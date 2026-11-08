#![cfg(feature = "data")]

//! Apache Parquet converter — schema + rows rendered as a markdown table (T024).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Row cap: beyond this the table is truncated and flagged in metadata.
const MAX_ROWS: usize = 100_000;

/// Converts Apache Parquet files into a markdown table.
///
/// Rows stream through the parquet row iterator (row group by row group);
/// rendering stops at [`MAX_ROWS`] and `metadata.truncated` is set.
pub struct ParquetConverter;

impl ParquetConverter {
    /// Create a new parquet converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ParquetConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for ParquetConverter {
    fn name(&self) -> &'static str {
        "parquet"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read parquet input: {e}")))?;

        let reader = SerializedFileReader::new(Bytes::from(bytes))
            .map_err(|e| AlchemizeError::other(format!("failed to open parquet file: {e}")))?;

        let meta = reader.metadata();
        let total_rows = meta.file_metadata().num_rows();
        let columns: Vec<String> = meta
            .file_metadata()
            .schema_descr()
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let iter = reader
            .get_row_iter(None)
            .map_err(|e| AlchemizeError::other(format!("failed to read parquet rows: {e}")))?;

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut truncated = false;
        for row in iter {
            if rows.len() >= MAX_ROWS {
                truncated = true;
                break;
            }
            let row = row.map_err(|e| {
                AlchemizeError::other(format!("failed to read parquet row {}: {e}", rows.len()))
            })?;
            let cells: Vec<String> = row
                .get_column_iter()
                .map(|(_, field)| render_field(field))
                .collect();
            rows.push(cells);
        }

        let rendered_rows = rows.len();
        let markdown = render_table(&columns, &rows, truncated, total_rows);
        let metadata = serde_json::json!({
            "source_format": "parquet",
            "rows": rendered_rows,
            "cols": columns.len(),
            "total_rows": total_rows,
            "truncated": truncated,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["parquet"]
    }
}

/// Render one parquet field for a markdown table cell.
///
/// Primitive types render as plain values; null renders as an empty cell;
/// nested structures (groups/lists/maps) fall back to their `Display`.
/// Date/timestamp columns keep their raw numeric encoding (days/milliseconds
/// since epoch) — no timezone conversion is applied.
fn render_field(field: &Field) -> String {
    match field {
        Field::Null => String::new(),
        Field::Bool(v) => v.to_string(),
        Field::Byte(v) => v.to_string(),
        Field::Short(v) => v.to_string(),
        Field::Int(v) => v.to_string(),
        Field::Long(v) => v.to_string(),
        Field::UByte(v) => v.to_string(),
        Field::UShort(v) => v.to_string(),
        Field::UInt(v) => v.to_string(),
        Field::ULong(v) => v.to_string(),
        Field::Float(v) => v.to_string(),
        Field::Double(v) => v.to_string(),
        Field::Str(v) => v.clone(),
        Field::Bytes(v) => String::from_utf8_lossy(v.data()).into_owned(),
        Field::Date(days) => days.to_string(),
        Field::TimeMillis(v) => v.to_string(),
        Field::TimestampMillis(v) | Field::TimestampMicros(v) => v.to_string(),
        Field::Float16(v) => v.to_string(),
        other => other.to_string(),
    }
}

/// Escape a cell for a markdown table.
fn escape_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', " ")
        .replace('\r', " ")
}

/// Render headers + rows as a markdown table.
fn render_table(
    columns: &[String],
    rows: &[Vec<String>],
    truncated: bool,
    total_rows: i64,
) -> String {
    if columns.is_empty() {
        return "*Parquet file has no columns.*\n".to_string();
    }

    let mut out = String::new();
    out.push_str("| ");
    out.push_str(
        &columns
            .iter()
            .map(|c| escape_cell(c))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    out.push_str(" |\n|");
    for _ in columns {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in rows {
        out.push_str("| ");
        out.push_str(
            &row.iter()
                .map(|c| escape_cell(c))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        out.push_str(" |\n");
    }
    if truncated {
        out.push_str(&format!(
            "\n> Truncated: showing first {MAX_ROWS} of {total_rows} rows.\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::basic::{Repetition, Type as PhysicalType};
    use parquet::data_type::{BoolType, ByteArrayType, DoubleType, Int32Type, Int64Type};
    use parquet::file::writer::SerializedFileWriter;
    use parquet::schema::types;

    /// Build a real parquet file in memory: id INT32, score DOUBLE,
    /// name BYTE_ARRAY, flag BOOLEAN, ts INT64.
    fn build_parquet(rows: &[(i32, f64, &str, bool, i64)]) -> Vec<u8> {
        let schema = Arc::new(
            types::Type::group_type_builder("schema")
                .with_fields(vec![
                    Arc::new(
                        types::Type::primitive_type_builder("id", PhysicalType::INT32)
                            .with_repetition(Repetition::REQUIRED)
                            .build()
                            .unwrap(),
                    ),
                    Arc::new(
                        types::Type::primitive_type_builder("score", PhysicalType::DOUBLE)
                            .with_repetition(Repetition::REQUIRED)
                            .build()
                            .unwrap(),
                    ),
                    Arc::new(
                        types::Type::primitive_type_builder("name", PhysicalType::BYTE_ARRAY)
                            .with_repetition(Repetition::REQUIRED)
                            .build()
                            .unwrap(),
                    ),
                    Arc::new(
                        types::Type::primitive_type_builder("flag", PhysicalType::BOOLEAN)
                            .with_repetition(Repetition::REQUIRED)
                            .build()
                            .unwrap(),
                    ),
                    Arc::new(
                        types::Type::primitive_type_builder("ts", PhysicalType::INT64)
                            .with_repetition(Repetition::REQUIRED)
                            .build()
                            .unwrap(),
                    ),
                ])
                .build()
                .unwrap(),
        );
        let props = Default::default();
        let mut writer = SerializedFileWriter::new(Vec::new(), schema, props).unwrap();
        let mut row_group = writer.next_row_group().unwrap();

        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<Int32Type>()
            .write_batch(&rows.iter().map(|r| r.0).collect::<Vec<_>>(), None, None)
            .unwrap();
        col.close().unwrap();

        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<DoubleType>()
            .write_batch(&rows.iter().map(|r| r.1).collect::<Vec<_>>(), None, None)
            .unwrap();
        col.close().unwrap();

        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<ByteArrayType>()
            .write_batch(
                &rows.iter().map(|r| r.2.into()).collect::<Vec<_>>(),
                None,
                None,
            )
            .unwrap();
        col.close().unwrap();

        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<BoolType>()
            .write_batch(&rows.iter().map(|r| r.3).collect::<Vec<_>>(), None, None)
            .unwrap();
        col.close().unwrap();

        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<Int64Type>()
            .write_batch(&rows.iter().map(|r| r.4).collect::<Vec<_>>(), None, None)
            .unwrap();
        col.close().unwrap();

        row_group.close().unwrap();
        writer.into_inner().unwrap()
    }

    #[tokio::test]
    async fn convert_parquet_table() {
        let data = build_parquet(&[
            (1, 0.5, "alpha", true, 1_700_000_000_000),
            (2, 2.25, "beta", false, 1_700_000_001_000),
            (3, -1.0, "gamma", true, 1_700_000_002_000),
        ]);
        let converter = ParquetConverter::new();
        let result = converter
            .convert(Input::bytes(data, "sample.parquet"))
            .await
            .unwrap();

        assert!(
            result
                .markdown
                .contains("| id | score | name | flag | ts |")
        );
        assert!(
            result
                .markdown
                .contains("| 1 | 0.5 | alpha | true | 1700000000000 |")
        );
        assert!(
            result
                .markdown
                .contains("| 2 | 2.25 | beta | false | 1700000001000 |")
        );
        assert!(
            result
                .markdown
                .contains("| 3 | -1 | gamma | true | 1700000002000 |")
        );

        assert_eq!(result.metadata["source_format"], "parquet");
        assert_eq!(result.metadata["rows"], 3);
        assert_eq!(result.metadata["cols"], 5);
        assert_eq!(result.metadata["total_rows"], 3);
        assert_eq!(result.metadata["truncated"], false);
    }

    #[tokio::test]
    async fn convert_parquet_null_cell_is_empty() {
        // OPTIONAL column with def levels: row 2 is null.
        let schema = Arc::new(
            types::Type::group_type_builder("schema")
                .with_fields(vec![Arc::new(
                    types::Type::primitive_type_builder("v", PhysicalType::INT32)
                        .with_repetition(Repetition::OPTIONAL)
                        .build()
                        .unwrap(),
                )])
                .build()
                .unwrap(),
        );
        let mut writer =
            SerializedFileWriter::new(Vec::new(), Arc::clone(&schema), Default::default()).unwrap();
        let mut row_group = writer.next_row_group().unwrap();
        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<Int32Type>()
            .write_batch(&[10, 30], Some(&[1, 0, 1]), None)
            .unwrap();
        col.close().unwrap();
        row_group.close().unwrap();
        let data = writer.into_inner().unwrap();

        let converter = ParquetConverter::new();
        let result = converter
            .convert(Input::bytes(data, "nulls.parquet"))
            .await
            .unwrap();
        assert!(result.markdown.contains("| 10 |"));
        assert!(result.markdown.contains("|  |"));
        assert!(result.markdown.contains("| 30 |"));
    }

    #[tokio::test]
    async fn convert_parquet_truncates_at_max_rows() {
        let total = MAX_ROWS + 1;
        let ids: Vec<i32> = (0..total as i32).collect();
        let schema = Arc::new(
            types::Type::group_type_builder("schema")
                .with_fields(vec![Arc::new(
                    types::Type::primitive_type_builder("id", PhysicalType::INT32)
                        .with_repetition(Repetition::REQUIRED)
                        .build()
                        .unwrap(),
                )])
                .build()
                .unwrap(),
        );
        let mut writer = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap();
        let mut row_group = writer.next_row_group().unwrap();
        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<Int32Type>()
            .write_batch(&ids, None, None)
            .unwrap();
        col.close().unwrap();
        row_group.close().unwrap();
        let data = writer.into_inner().unwrap();

        let converter = ParquetConverter::new();
        let result = converter
            .convert(Input::bytes(data, "big.parquet"))
            .await
            .unwrap();
        assert_eq!(result.metadata["rows"], MAX_ROWS);
        assert_eq!(result.metadata["total_rows"], total as i64);
        assert_eq!(result.metadata["truncated"], true);
        assert!(
            result
                .markdown
                .contains(&format!("first {MAX_ROWS} of {total} rows"))
        );
    }

    #[tokio::test]
    async fn convert_parquet_invalid_bytes() {
        let converter = ParquetConverter::new();
        let err = converter
            .convert(Input::bytes(b"not parquet".to_vec(), "bad.parquet"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("parquet"));
    }

    #[test]
    fn supported_extensions_and_name() {
        let converter = ParquetConverter::new();
        assert_eq!(converter.supported_extensions(), &["parquet"]);
        assert_eq!(converter.name(), "parquet");
    }

    #[test]
    fn escape_cell_renders_pipes_and_newlines() {
        assert_eq!(escape_cell("a|b"), "a\\|b");
        assert_eq!(escape_cell("l1\nl2"), "l1 l2");
        assert_eq!(escape_cell("back\\slash"), "back\\\\slash");
    }
}
