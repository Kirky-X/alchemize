// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! XLSX/XLS/ODS to Markdown converter using calamine.
//!
//! Converts spreadsheet files to markdown tables. Each sheet becomes a
//! separate section with a header. Empty cells are rendered as empty strings.

use async_trait::async_trait;
use calamine::{Data, Reader, Sheets};

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

        // calamine 0.38 lazy Sheets only opens from a path (from_rs is
        // eager and unsupported for Sheets); stage bytes in a temp file.
        let suffix = format!(".{extension}");
        let tmp = tempfile::Builder::new()
            .suffix(&suffix)
            .tempfile()
            .map_err(|e| AlchemizeError::other(format!("spreadsheet temp file failed: {e}")))?;
        std::fs::write(tmp.path(), &bytes)
            .map_err(|e| AlchemizeError::other(format!("spreadsheet staging failed: {e}")))?;
        let mut workbook: Sheets<_> = calamine::open_workbook_auto(tmp.path())
            .map_err(|e| AlchemizeError::other(format!("failed to open spreadsheet: {e}")))?;

        let sheet_names = workbook.sheet_names();
        let mut markdown_parts = Vec::new();
        let mut total_rows = 0;
        let mut total_cells = 0;
        let mut numfmt_fallback = 0usize;

        for (sheet_index, sheet_name) in sheet_names.iter().enumerate() {
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

            let (formats, fmt_fallback) =
                read_cell_formats(&bytes, sheet_index);
            numfmt_fallback += fmt_fallback;
            let table =
                range_to_markdown_table(&range, &formats, &mut total_rows, &mut total_cells);
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
            "numfmt_fallback": numfmt_fallback,
            "source_format": extension,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["xlsx", "xls", "xlsb", "ods"]
    }
}

/// Convert a calamine Range to a markdown table.
fn range_to_markdown_table(
    range: &calamine::Range<Data>,
    formats: &CellFormats,
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
                let code = formats.get(&(r as u32, c as u32));
                format_cell_value_with_format(cell, code.map(String::as_str))
            })
            .collect();

        let row_line = format!("| {} |", row_cells.join(" | "));
        lines.push(row_line);
        *row_count += 1;
        *cell_count += width;
    }

    lines.join("\n")
}

/// Format a cell, applying the Excel number format when one applies.
///
/// Only numeric cells consult the format code; dates already arrive
/// pre-rendered from calamine. Unsupported codes fall back to the raw value.
fn format_cell_value_with_format(cell: Option<&Data>, format_code: Option<&str>) -> String {
    if let (Some(Data::Float(f)), Some(code)) = (cell, format_code)
        && let Some(rendered) = crate::infrastructure::document::office::numfmt::apply_format(*f, code)
    {
        return rendered;
    }
    format_cell_value(cell)
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


// ---- T014: number-format (numFmt) display values ------------------------
//
// calamine does not expose per-cell format codes, so the style chain is read
// from the raw package: `xl/styles.xml` maps cellXfs index → numFmtId →
// format code, and each sheet's `<c s="N">` attributes map cells to that
// index. Numeric cells then render through `numfmt::apply_format`.

/// Cell position → format code map for one sheet.
type CellFormats = std::collections::HashMap<(u32, u32), String>;

/// Read `xl/styles.xml` + worksheet XML into a cell→format-code map.
///
/// Returns an empty map for non-xlsx inputs or unreadable packages: number
/// formatting is a display nicety, never a conversion blocker.
fn read_cell_formats(bytes: &[u8], sheet_index: usize) -> (CellFormats, usize) {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::io::Read;

    let mut formats: CellFormats = std::collections::HashMap::new();
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return (formats, 0);
    };

    // 1) styles.xml: custom numFmts + cellXfs order.
    let mut code_by_id: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut xf_ids: Vec<u32> = Vec::new();
    let mut styles_xml = Vec::new();
    if let Ok(mut f) = zip.by_name("xl/styles.xml") {
        if f.read_to_end(&mut styles_xml).is_err() {
            return (formats, 0);
        }
    }
    if !styles_xml.is_empty() {
        let mut reader = Reader::from_reader(styles_xml.as_slice());
        let mut buf = Vec::new();
        let mut in_cell_xfs = false;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                    match local.as_str() {
                        "cellXfs" => in_cell_xfs = true,
                        "numFmt" => {
                            let mut id = None;
                            let mut code = None;
                            for a in e.attributes().flatten() {
                                match a.key.as_ref() {
                                    "numFmtId" => id = a.value.trim().parse::<u32>().ok(),
                                    "formatCode" => {
                                        code = Some(a.value.clone().into_owned());
                                    }
                                    _ => {}
                                }
                            }
                            if let (Some(id), Some(code)) = (id, code) {
                                code_by_id.insert(id, code);
                            }
                        }
                        "xf" if in_cell_xfs => {
                            let id = e
                                .attributes()
                                .flatten()
                                .find(|a| a.key.as_ref() == "numFmtId")
                                .and_then(|a| a.value.trim().parse::<u32>().ok())
                                .unwrap_or(0);
                            xf_ids.push(id);
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(e)) => {
                    if crate::infrastructure::document::local_tag_name(e.name().as_ref())
                        == "cellXfs"
                    {
                        in_cell_xfs = false;
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
    }
    if xf_ids.is_empty() {
        return (formats, 0);
    }

    // style index → format code (builtin ids resolved through the numfmt table).
    let style_codes: Vec<Option<String>> = xf_ids
        .iter()
        .map(|id| {
            code_by_id
                .get(id)
                .cloned()
                .or_else(|| crate::infrastructure::document::office::numfmt::builtin_format(*id as u16).map(|s| s.to_string()))
                .filter(|c| c != "General")
        })
        .collect();

    // 2) worksheet: <c r="A1" s="3"> → format code.
    let sheet_path = format!("xl/worksheets/sheet{}.xml", sheet_index + 1);
    let mut sheet_xml = Vec::new();
    let Ok(mut f) = zip.by_name(&sheet_path) else {
        return (formats, 0);
    };
    if f.read_to_end(&mut sheet_xml).is_err() {
        return (formats, 0);
    }
    let mut reader = Reader::from_reader(sheet_xml.as_slice());
    let mut buf = Vec::new();
    let mut fallback = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) != "c" {
                    buf.clear();
                    continue;
                }
                let mut pos = None;
                let mut style = None;
                for a in e.attributes().flatten() {
                    match a.key.as_ref() {
                        "r" => pos = cell_ref_to_pos(&a.value),
                        "s" => style = a.value.trim().parse::<usize>().ok(),
                        _ => {}
                    }
                }
                if let (Some(pos), Some(style)) = (pos, style) {
                    match style_codes.get(style).and_then(|c| c.clone()) {
                        Some(code) => {
                            formats.insert(pos, code);
                        }
                        None => fallback += 1,
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (formats, fallback)
}

/// `"B12"` → `(11, 1)` (0-based row, column).
fn cell_ref_to_pos(r: &str) -> Option<(u32, u32)> {
    // Excel refs are letters-then-digits ("B12"); anything else is invalid.
    let digits_at = r.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = r.split_at(digits_at);
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    if !letters.chars().all(|c| c.is_ascii_alphabetic())
        || !digits.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut col = 0u32;
    for ch in letters.chars() {
        col = col * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    let row: u32 = digits.parse().ok()?;
    if col == 0 || row == 0 {
        None
    } else {
        Some((row - 1, col - 1))
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        let converter = XlsxConverter::new();
        assert_eq!(converter.supported_extensions(), &["xlsx", "xls", "xlsb", "ods"]);
    }

    #[test]
    fn cell_ref_parsing() {
        assert_eq!(cell_ref_to_pos("A1"), Some((0, 0)));
        assert_eq!(cell_ref_to_pos("B12"), Some((11, 1)));
        assert_eq!(cell_ref_to_pos("AA3"), Some((2, 26)));
        assert_eq!(cell_ref_to_pos("1A"), None);
        assert_eq!(cell_ref_to_pos("$A$1"), None);
    }

    #[tokio::test]
    async fn styled_numeric_cells_render_percent_and_thousands() {
        // A real xlsx package with styles.xml wiring numFmtId 9 (0%) and
        // 3 (#,##0) onto two cells.
        use std::io::Write;
        let buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        let styles = r#"<?xml version="1.0"?><styleSheet xmlns="s">
<numFmts count="0"/>
<cellXfs count="3">
<xf numFmtId="0"/><xf numFmtId="9"/><xf numFmtId="3"/>
</cellXfs></styleSheet>"#;
        let sheet = r#"<?xml version="1.0"?><worksheet xmlns="s"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>rate</t></is></c><c r="B1" t="inlineStr"><is><t>count</t></is></c></row>
<row r="2"><c r="A2" s="1"><v>0.25</v></c><c r="B2" s="2"><v>1234567</v></c></row>
</sheetData></worksheet>"#;
        for (name, body) in [
            ("xl/styles.xml", styles.as_bytes()),
            ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
            (
                "xl/workbook.xml",
                br#"<?xml version="1.0"?><workbook xmlns="s"><sheets><sheet name="S1" sheetId="1" r:id="rId1" xmlns:r="r"/></sheets></workbook>"#,
            ),
        ] {
            w.start_file(name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        let bytes = w.finish().unwrap().into_inner();

        let (formats, fallback) = read_cell_formats(&bytes, 0);
        assert_eq!(fallback, 0, "no unsupported xf expected");
        assert_eq!(formats.get(&(1, 0)).map(String::as_str), Some("0%"));
        assert_eq!(formats.get(&(1, 1)).map(String::as_str), Some("#,##0"));

        // Values route through numfmt: 0.25 with 0% → 25%.
        assert_eq!(
            format_cell_value_with_format(Some(&Data::Float(0.25)), Some("0%")),
            "25%"
        );
        assert_eq!(
            format_cell_value_with_format(Some(&Data::Float(1234567.0)), Some("#,##0")),
            "1,234,567"
        );
        // Unknown code keeps the raw value (no silent loss).
        assert_eq!(
            format_cell_value_with_format(Some(&Data::Float(1.5)), Some("[Mystery]")),
            "1.5"
        );
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
        assert_eq!(converter.supported_extensions(), &["xlsx", "xls", "xlsb", "ods"]);
    }

    #[test]
    fn xlsb_extension_routed() {
        // Routing-level guarantee: the extension joins the engine table even
        // before a real binary fixture is available (calamine Xlsb path).
        let converter = XlsxConverter::new();
        assert!(converter.supported_extensions().contains(&"xlsb"));
    }

    #[tokio::test]
    #[ignore = "requires a real .xlsb binary fixture"]
    async fn real_xlsb_opens_via_calamine() {
        let path = std::env::var("ALCHEMIZE_XLSB_FIXTURE").expect("set ALCHEMIZE_XLSB_FIXTURE");
        let result = XlsxConverter::new()
            .convert(Input::Path(path.into()))
            .await
            .unwrap();
        assert!(!result.markdown.is_empty());
    }
}
