// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! CSV/TSV to Markdown table converter.

use async_trait::async_trait;
use csv::ReaderBuilder;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

use super::shared::detect_encoding;

/// Converts CSV (and TSV) files to markdown tables.
///
/// The first row is treated as the header row. The delimiter is auto-sensed
/// per file: `.tsv` forces tab; `.csv` samples the header line and picks tab
/// when tab separators outnumber commas (mislabelled TSV).
pub struct CsvConverter {
    delimiter: Option<u8>,
}

impl CsvConverter {
    /// Create a converter with auto-sensed delimiters.
    pub fn new() -> Self {
        Self { delimiter: None }
    }

    /// Create a converter with a fixed custom delimiter.
    pub fn with_delimiter(delimiter: u8) -> Self {
        Self {
            delimiter: Some(delimiter),
        }
    }

    /// Resolve the effective delimiter for a decoded sample.
    fn effective_delimiter(&self, ext: &str, decoded: &str) -> u8 {
        if let Some(fixed) = self.delimiter {
            return fixed;
        }
        if ext == "tsv" {
            return b'\t';
        }
        if ext == "psv" {
            return b'|';
        }
        // Sample the first few non-empty lines instead of only the header
        // row: a header cell with embedded commas (or a leading blank line)
        // would otherwise flip the vote on tab-separated data.
        let sample: String = decoded
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(5)
            .collect::<Vec<_>>()
            .join("\n");
        let tabs = sample.matches('\t').count();
        let commas = sample.matches(',').count();
        let pipes = sample.matches('|').count();
        // Pipe-separated files are frequently mislabelled `.csv`.
        if pipes > commas && pipes > tabs {
            b'|'
        } else if tabs > commas {
            b'\t'
        } else {
            b','
        }
    }
}

impl Default for CsvConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for CsvConverter {
    fn name(&self) -> &'static str {
        "csv"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read CSV input: {e}")))?;

        let (decoded, encoding_name) = detect_encoding(&bytes);
        let ext = input.extension().unwrap_or_default();
        let delimiter = self.effective_delimiter(&ext, &decoded);

        // Pass 1: widest row. The table width is the widest row — truncating
        // would lose data and a ragged table would break GFM rendering. Only
        // widths are kept; buffering all cell contents would expand a 100 MB
        // CSV of tiny cells into gigabytes.
        let mut data_width = 0usize;
        {
            let mut probe = ReaderBuilder::new()
                .delimiter(delimiter)
                .has_headers(true)
                .flexible(true)
                .from_reader(decoded.as_bytes());
            for result in probe.records() {
                let record =
                    result.map_err(|e| AlchemizeError::other(format!("CSV parse error: {e}")))?;
                data_width = data_width.max(record.len());
            }
        }

        let mut reader = ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(true)
            .flexible(true)
            .from_reader(decoded.as_bytes());

        let headers = reader
            .headers()
            .map_err(|e| AlchemizeError::other(format!("failed to read CSV headers: {e}")))?;

        let mut header_vec: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
        let col_count = header_vec.len().max(data_width);

        // Pad the header to the final width with synthetic column names.
        let padded_header_columns = col_count - header_vec.len();
        while header_vec.len() < col_count {
            let index = header_vec.len() + 1;
            header_vec.push(format!("column_{index}"));
        }

        let mut lines = Vec::new();
        // Rows are charged against the shared output budget as they are
        // rendered: a 100 MB CSV of tiny cells must fail closed instead of
        // materializing a multi-hundred-MB table.
        let mut output_bytes = 0usize;
        let mut push_row = |line: String, bytes: &mut usize| -> Result<(), AlchemizeError> {
            *bytes = bytes.saturating_add(line.len());
            crate::infrastructure::document::shared::check_output_budget(*bytes)?;
            lines.push(line);
            Ok(())
        };
        push_row(format!("| {} |", header_vec.join(" | ")), &mut output_bytes)?;
        push_row(
            format!(
                "|{}|",
                (0..col_count).map(|_| "---").collect::<Vec<_>>().join("|")
            ),
            &mut output_bytes,
        )?;

        // Pass 2: emit rows one at a time.
        let mut row_count = 0usize;
        for result in reader.records() {
            let record =
                result.map_err(|e| AlchemizeError::other(format!("CSV parse error: {e}")))?;
            let mut padded: Vec<String> = record.iter().map(|s| s.to_string()).collect();
            padded.resize(col_count, String::new());
            push_row(format!("| {} |", padded.join(" | ")), &mut output_bytes)?;
            row_count += 1;
        }

        let markdown = if row_count == 0 {
            "*CSV file contains no data rows.*\n".to_string()
        } else {
            lines.join("\n") + "\n"
        };

        let metadata = serde_json::json!({
            "source_format": ext,
            "column_count": col_count,
            "padded_header_columns": padded_header_columns,
            "row_count": row_count,
            "encoding": encoding_name,
            "delimiter": match delimiter {
                b',' => "comma",
                b'\t' => "tab",
                _ => "custom",
            },
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["csv", "tsv", "psv"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimiter_sensing_covers_tsv_psv_and_csv() {
        let c = CsvConverter::new();
        assert_eq!(c.effective_delimiter("tsv", "a\tb"), b'\t');
        assert_eq!(c.effective_delimiter("psv", "a|b"), b'|');
        assert_eq!(c.effective_delimiter("csv", "a,b,c"), b',');
        // Mislabelled: pipes dominate in a .csv.
        assert_eq!(c.effective_delimiter("csv", "a|b|c,d"), b'|');
        // Mislabelled: tabs dominate.
        assert_eq!(c.effective_delimiter("csv", "a\tb\tc,d"), b'\t');
        assert_eq!(
            CsvConverter::with_delimiter(b';').effective_delimiter("csv", "a,b"),
            b';'
        );
    }

    #[test]
    fn supported_extensions() {
        assert_eq!(
            CsvConverter::new().supported_extensions(),
            &["csv", "tsv", "psv"]
        );
    }

    #[tokio::test]
    async fn convert_simple_csv() {
        let converter = CsvConverter::new();
        let csv = "Name,Age\nAlice,30\nBob,25\n";
        let input = Input::bytes(csv.as_bytes().to_vec(), "test.csv");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Name"));
        assert!(result.markdown.contains("Alice"));
        assert!(result.markdown.contains("---"));
    }

    #[tokio::test]
    async fn convert_tsv_forced_tab() {
        let converter = CsvConverter::new();
        let tsv = "Name\tAge\nAlice\t30\n";
        let input = Input::bytes(tsv.as_bytes().to_vec(), "test.tsv");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Alice"));
        assert_eq!(result.metadata["delimiter"], "tab");
    }

    #[tokio::test]
    async fn convert_mislabelled_tsv_sensed_from_content() {
        // A .csv file whose content is actually tab-separated
        let converter = CsvConverter::new();
        let tsv = "Name\tAge\tCity\nAlice\t30\tBeijing\n";
        let input = Input::bytes(tsv.as_bytes().to_vec(), "data.csv");
        let result = converter.convert(input).await.unwrap();
        assert_eq!(result.metadata["delimiter"], "tab");
        assert!(result.markdown.contains("Beijing"));
    }

    #[tokio::test]
    async fn convert_empty_csv() {
        let converter = CsvConverter::new();
        let input = Input::bytes(b"Header1,Header2\n".to_vec(), "test.csv");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("no data rows"));
    }

    #[test]
    fn effective_delimiter_fixed_override() {
        let converter = CsvConverter::with_delimiter(b';');
        assert_eq!(converter.effective_delimiter("csv", "a;b\nc;d"), b';');
    }

    #[test]
    fn effective_delimiter_samples_multiple_lines() {
        let converter = CsvConverter::new();
        // Header has embedded commas, data rows are tab-separated: the
        // multi-line vote must still pick tabs.
        assert_eq!(
            converter.effective_delimiter("csv", "name, full\nAlice\t30\nBob\t25"),
            b'\t'
        );
        // Leading blank line does not break the vote.
        assert_eq!(
            converter.effective_delimiter("csv", "\n\na\tb\nc\td"),
            b'\t'
        );
    }

    #[tokio::test]
    async fn convert_widens_table_for_ragged_rows() {
        let converter = CsvConverter::new();
        let csv = "a,b\n1,2,3,4\n5,6\n";
        let input = Input::bytes(csv.as_bytes().to_vec(), "wide.csv");
        let result = converter.convert(input).await.unwrap();
        // The table widens to the widest row: no cell is dropped.
        assert_eq!(result.metadata["column_count"], 4);
        assert_eq!(result.metadata["padded_header_columns"], 2);
        assert!(result.markdown.contains("| a | b | column_3 | column_4 |"));
        assert!(result.markdown.contains("| 1 | 2 | 3 | 4 |"));
        // Short rows are padded, never truncated.
        assert!(result.markdown.contains("| 5 | 6 |  |  |"));
        // Every rendered row keeps exactly the final column count.
        for line in result.markdown.lines().filter(|l| l.starts_with('|')) {
            assert_eq!(line.matches('|').count(), 5, "ragged row: {line}");
        }
    }
}

#[test]
fn default_matches_new() {
    let _ = CsvConverter::default();
}

#[tokio::test]
async fn missing_input_path_errors() {
    let converter = CsvConverter::new();
    let err = converter
        .convert(Input::Path(std::path::PathBuf::from(
            "/nonexistent/data.csv",
        )))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("CSV"), "{err:?}");
}
