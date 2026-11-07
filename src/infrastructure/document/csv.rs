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
        let header = decoded.lines().next().unwrap_or("");
        let tabs = header.matches('\t').count();
        let commas = header.matches(',').count();
        if tabs > commas {
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

        let mut reader = ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(true)
            .flexible(true)
            .from_reader(decoded.as_bytes());

        let headers = reader
            .headers()
            .map_err(|e| AlchemizeError::other(format!("failed to read CSV headers: {e}")))?;

        let header_vec: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
        let col_count = header_vec.len();

        let mut lines = Vec::new();
        lines.push(format!("| {} |", header_vec.join(" | ")));
        lines.push(format!(
            "|{}|",
            (0..col_count).map(|_| "---").collect::<Vec<_>>().join("|")
        ));

        let mut row_count = 0;
        for result in reader.records() {
            let record =
                result.map_err(|e| AlchemizeError::other(format!("CSV parse error: {e}")))?;
            let cells: Vec<String> = record.iter().map(|s| s.to_string()).collect();

            let cells_len = cells.len();
            let padded_cells: Vec<String> = if cells_len < col_count {
                cells
                    .into_iter()
                    .chain((cells_len..col_count).map(|_| String::new()))
                    .collect()
            } else {
                cells
            };

            lines.push(format!("| {} |", padded_cells.join(" | ")));
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
        &["csv", "tsv"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(CsvConverter::new().supported_extensions(), &["csv", "tsv"]);
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
        assert_eq!(
            converter.effective_delimiter("csv", "a;b\nc;d"),
            b';'
        );
    }
}
