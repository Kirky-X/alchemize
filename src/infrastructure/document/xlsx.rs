// Copyright © 2026 Kirky.X. All rights reserved.
//! XLSX/XLS/ODS to Markdown converter using calamine.
//!
//! Converts spreadsheet files to markdown tables. Each sheet becomes a
//! separate section with a header. Empty cells are rendered as empty strings.

use async_trait::async_trait;
use calamine::{open_workbook_from_rs, Data, Reader, Sheets};
use std::io::Cursor;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts XLSX/XLS/ODS spreadsheet files to markdown tables.
///
/// Uses the `calamine` crate to read Excel and OpenDocument spreadsheet formats.
/// Each worksheet is converted to a markdown table with proper column alignment.
pub struct XlsxConverter;

impl XlsxConverter {
    /// Create a new spreadsheet converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for XlsxConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for XlsxConverter {
    fn name(&self) -> &'static str {
        "xlsx"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read spreadsheet input: {e}")))?;

        let filename = input.filename().unwrap_or("spreadsheet.xlsx");

        // Determine format from extension
        let extension = filename.rsplit('.').next().unwrap_or("xlsx").to_lowercase();

        let mut workbook: Sheets<_> = open_workbook_from_rs(Cursor::new(&bytes))
            .map_err(|e| AlchemizeError::Other(format!("failed to open spreadsheet: {e}")))?;

        let sheet_names = workbook.sheet_names();
        let mut markdown_parts = Vec::new();
        let mut total_rows = 0;
        let mut total_cells = 0;

        for sheet_name in &sheet_names {
            // Skip hidden sheets (usually start with underscore or have special names)
            if sheet_name.starts_with('_') || sheet_name.to_lowercase().contains("hidden") {
                continue;
            }

            let range = workbook.worksheet_range(sheet_name).map_err(|e| {
                AlchemizeError::Other(format!("failed to read sheet '{sheet_name}': {e}"))
            })?;

            if range.is_empty() {
                continue;
            }

            markdown_parts.push(format!("## {}\n", sheet_name));

            let table = range_to_markdown_table(&range, &mut total_rows, &mut total_cells);
            markdown_parts.push(table);
            markdown_parts.push(String::new()); // Empty line between sheets
        }

        let markdown = if markdown_parts.is_empty() {
            "*Spreadsheet contains no extractable data.*\n".to_string()
        } else {
            markdown_parts.join("\n")
        };

        let metadata = serde_json::json!({
            "sheet_count": sheet_names.len(),
            "total_rows": total_rows,
            "total_cells": total_cells,
            "source_format": extension,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["xlsx", "xls", "ods"]
    }
}

/// Convert a calamine Range to a markdown table.
fn range_to_markdown_table(
    range: &calamine::Range<Data>,
    row_count: &mut usize,
    cell_count: &mut usize,
) -> String {
    // Get dimensions using width and height methods
    let height = range.height();
    let width = range.width();

    if height == 0 || width == 0 {
        return String::new();
    }

    let mut lines = Vec::new();

    // Build header row - use first row content or column letters
    let header_cells: Vec<String> = (0..width)
        .map(|c| {
            let cell = range.get((0, c));
            if let Some(Data::String(s)) = cell {
                if !s.trim().is_empty() {
                    return s.clone();
                }
            }
            column_letter(c)
        })
        .collect();

    let header_line = format!("| {} |", header_cells.join(" | "));
    lines.push(header_line);

    let separator_line = format!(
        "|{}|",
        (0..width).map(|_| "---").collect::<Vec<_>>().join("|")
    );
    lines.push(separator_line);

    // Build data rows (skip header row)
    for r in 1..height {
        let row_cells: Vec<String> = (0..width)
            .map(|c| {
                let cell = range.get((r, c));
                format_cell_value(cell)
            })
            .collect();

        let row_line = format!("| {} |", row_cells.join(" | "));
        lines.push(row_line);
        *row_count += 1;
        *cell_count += width;
    }

    lines.join("\n")
}

/// Format a cell value as string for markdown.
fn format_cell_value(cell: Option<&Data>) -> String {
    match cell {
        None => String::new(),
        Some(Data::Empty) => String::new(),
        Some(Data::String(s)) => s.clone(),
        Some(Data::Float(f)) => format!("{f}"),
        Some(Data::Int(i)) => format!("{i}"),
        Some(Data::Bool(b)) => format!("{b}"),
        Some(Data::DateTime(dt)) => format!("{dt}"),
        Some(Data::DateTimeIso(s)) => s.clone(),
        Some(Data::DurationIso(s)) => s.clone(),
        Some(Data::Error(e)) => format!("[ERROR: {e}]"),
    }
}

/// Convert column index to letter (0 -> A, 1 -> B, etc.).
fn column_letter(idx: usize) -> String {
    if idx < 26 {
        ((idx + 65) as u8 as char).to_string()
    } else {
        let first = (idx / 26 - 1 + 65) as u8 as char;
        let second = (idx % 26 + 65) as u8 as char;
        format!("{first}{second}")
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        let converter = XlsxConverter::new();
        assert_eq!(converter.supported_extensions(), &["xlsx", "xls", "ods"]);
    }

    #[test]
    fn column_letter_a() {
        assert_eq!(column_letter(0), "A");
    }

    #[test]
    fn column_letter_z() {
        assert_eq!(column_letter(25), "Z");
    }

    #[test]
    fn column_letter_aa() {
        assert_eq!(column_letter(26), "AA");
    }

    #[test]
    fn format_empty_cell() {
        assert_eq!(format_cell_value(None), "");
        assert_eq!(format_cell_value(Some(&Data::Empty)), "");
    }

    #[test]
    fn format_string_cell() {
        assert_eq!(
            format_cell_value(Some(&Data::String("hello".to_string()))),
            "hello"
        );
    }

    #[test]
    fn format_int_cell() {
        assert_eq!(format_cell_value(Some(&Data::Int(42))), "42");
    }

    #[test]
    fn format_float_cell() {
        let result = format_cell_value(Some(&Data::Float(2.5)));
        assert!(result.contains("2.5"));
    }

    #[test]
    fn format_bool_cell() {
        assert_eq!(format_cell_value(Some(&Data::Bool(true))), "true");
        assert_eq!(format_cell_value(Some(&Data::Bool(false))), "false");
    }

    #[test]
    fn default_creates_instance() {
        let converter = XlsxConverter;
        assert_eq!(converter.supported_extensions(), &["xlsx", "xls", "ods"]);
    }
}
