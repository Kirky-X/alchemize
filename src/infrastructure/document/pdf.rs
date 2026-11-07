// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! PDF to Markdown converter using pdf-extract, with paper-flavoured
//! post-processing (TOC removal, artifact cleanup, structure heuristics).

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts PDF files to markdown by extracting text per page.
///
/// Local extraction is the fallback tier (quality `LocalFallback`): text
/// only, no images or formulas. Higher tiers (MinerU / anydoc) sit above
/// this converter in the registry.
pub struct PdfConverter;

impl PdfConverter {
    /// Create a new PDF converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PdfConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for PdfConverter {
    fn name(&self) -> &'static str {
        "pdf"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read PDF input: {e}")))?;

        let pages = pdf_extract::extract_text_from_mem_by_pages(&bytes)
            .map_err(|e| AlchemizeError::Pdf(format!("{e}")))?;

        let page_count = pages.len();
        let mut markdown = String::new();

        for (i, page_text) in pages.iter().enumerate() {
            let page_num = i + 1;
            let processed = postprocess(page_text);
            let trimmed = processed.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !markdown.is_empty() {
                markdown.push('\n');
            }
            markdown.push_str(&format!("## Page {page_num}\n\n{trimmed}\n"));
        }

        if markdown.is_empty() {
            markdown = "*PDF contains no extractable text.*\n".to_string();
        }

        let warnings = crate::domain::latex_validate::validate_markdown_math(&markdown);
        let metadata = serde_json::json!({
            "page_count": page_count,
            "source_format": "pdf",
            "latex_warnings": warnings,
        });

        Ok(ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFallback))
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }
}

/// Apply the paper post-processing pipeline: artifact cleanup → TOC removal
/// → structure heuristics.
pub fn postprocess(raw: &str) -> String {
    add_structure_heuristics(&remove_toc_block(&cleanup_text(raw)))
}

/// Remove PDF table-of-contents block from extracted text.
///
/// Detects TOC structurally by a sustained run (≥10 lines) of
/// numbered-outline / dot-leader entries within the first third of the
/// text; a preceding "Contents" heading is removed with the block.
pub fn remove_toc_block(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    let first_third = std::cmp::max(1, lines.len() / 3);

    let mut i = 0usize;
    while i < std::cmp::min(lines.len(), first_third) {
        if !is_toc_line(lines[i].trim()) {
            i += 1;
            continue;
        }

        let start = i;
        let mut toc_count = 0usize;
        while i < lines.len() {
            let s = lines[i].trim();
            if s.is_empty() {
                i += 1;
                continue;
            }
            if is_toc_line(s) {
                toc_count += 1;
                i += 1;
            } else {
                break;
            }
        }

        if toc_count >= 10 {
            let mut j = start as isize - 1;
            while j >= 0 && lines[j as usize].trim().is_empty() {
                j -= 1;
            }
            let mut start = start;
            if j >= 0 {
                let heading = lines[j as usize]
                    .trim()
                    .trim_start_matches('#')
                    .trim()
                    .to_lowercase();
                if heading == "contents" || heading == "table of contents" {
                    start = j as usize;
                }
            }
            lines.drain(start..i);
            break;
        }
        i = start + 1;
    }

    lines.join("\n")
}

/// True if a non-empty line looks like a PDF TOC entry.
fn is_toc_line(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Dot-leader lines (mostly dots)
    let dots = s.matches('.').count();
    if dots > 5 && dots as f64 / s.len().max(1) as f64 > 0.3 {
        return true;
    }
    // Standalone section number like "3.2.1"
    if is_bare_section_number(s) {
        return true;
    }
    // Numbered entry like "3.1 Token-level Memory" (short title only)
    if starts_with_section_number(s) {
        return s.chars().count() < 100;
    }
    false
}

/// Matches `^\d+(\.\d+)+\s*$` — a bare dotted number.
fn is_bare_section_number(s: &str) -> bool {
    let mut saw_dot_group = false;
    for (idx, part) in s.split('.').enumerate() {
        if part.is_empty() {
            return false;
        }
        if !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if idx > 0 {
            saw_dot_group = true;
        }
    }
    saw_dot_group && !s.contains(char::is_whitespace)
}

/// Matches `^\d+(\.\d+)*\s` — a leading section number followed by space.
fn starts_with_section_number(s: &str) -> bool {
    let mut rest = s;
    // leading integer
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    rest = &rest[digits.len()..];
    // zero or more (.digits) groups
    while let Some(stripped) = rest.strip_prefix('.') {
        let group: String = stripped.chars().take_while(|c| c.is_ascii_digit()).collect();
        if group.is_empty() {
            return false;
        }
        rest = &stripped[group.len()..];
    }
    rest.starts_with(char::is_whitespace)
}

/// Remove PDF extraction artifacts: form feeds, standalone page numbers,
/// TOC dot leaders, and single-character garbage lines.
pub fn cleanup_text(text: &str) -> String {
    let mut result = Vec::new();
    for line in text.split('\n') {
        let line = line.replace('\u{c}', "");
        let stripped = line.trim();

        if stripped.is_empty() {
            continue;
        }

        // Standalone page numbers (1-4 digit lines with no other content)
        if stripped.len() <= 4 && stripped.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        // TOC dot leaders (lines of only dots and spaces)
        if stripped.len() > 2 && stripped.chars().all(|c| c == '.' || c == ' ') {
            continue;
        }

        // Single-character garbage (isolated letters/punctuation)
        if stripped.chars().count() == 1 {
            continue;
        }

        result.push(line);
    }
    result.join("\n")
}

/// Add markdown heading structure heuristics to plain text.
///
/// Short lines starting with a section number or a canonical paper section
/// word become headings, with depth derived from the dot count.
pub fn add_structure_heuristics(text: &str) -> String {
    let canonical = [
        "Abstract",
        "Introduction",
        "Conclusion",
        "References",
        "Acknowledgment",
    ];
    let mut output = Vec::new();
    for line in text.split('\n') {
        let stripped = line.trim();
        let numbered_1 = stripped.starts_with("1 ") || stripped.starts_with("1. ");
        let numbered_11 = stripped.starts_with("1.1 ");
        let canonical_hit = canonical.iter().any(|w| stripped.starts_with(w));

        if !stripped.is_empty()
            && (numbered_1 || numbered_11 || canonical_hit)
            && stripped.chars().count() < 80
        {
            if stripped.starts_with("References") {
                output.push(format!("\n## {stripped}\n"));
            } else if stripped.starts_with(|c: char| c.is_ascii_digit())
                && stripped[..5.min(stripped.len())].contains('.')
            {
                let depth = stripped.matches('.').count() + 2;
                let depth = depth.min(4);
                output.push(format!("\n{} {stripped}\n", "#".repeat(depth)));
            } else {
                output.push(format!("\n## {stripped}\n"));
            }
            continue;
        }
        output.push(line.to_string());
    }
    output.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(PdfConverter::new().supported_extensions(), &["pdf"]);
    }

    // -- cleanup_text ---------------------------------------------------------

    #[test]
    fn cleanup_removes_standalone_page_numbers() {
        let text = "some text\n12\n344\n12345\nend";
        let out = cleanup_text(text);
        assert!(out.contains("some text"));
        assert!(!out.contains("\n12\n"));
        assert!(out.contains("12345"), "5-digit numbers are not page numbers");
    }

    #[test]
    fn cleanup_removes_dot_leaders_and_form_feeds() {
        let text = "Title\n.......\nother";
        let out = cleanup_text(&text.replace("other", "\u{c}other"));
        assert!(out.contains("Title"));
        assert!(!out.contains("......."));
        assert!(!out.contains('\u{c}'));
    }

    #[test]
    fn cleanup_removes_single_char_garbage() {
        let out = cleanup_text("a\nreal line\nZ");
        assert!(out.contains("real line"));
        assert_eq!(out.matches('\n').count(), 0);
    }

    // -- remove_toc_block -----------------------------------------------------

    #[test]
    fn toc_block_removed_when_sustained_run() {
        let toc: Vec<String> = (1..=12).map(|i| format!("{i} Section Number {i}")).collect();
        let mut text = String::from("Paper Title\n\n");
        text.push_str(&toc.join("\n"));
        text.push_str("\n\n1 Introduction\nBody content here.");
        let out = remove_toc_block(&text);
        assert!(!out.contains("Section Number 5"), "TOC entries must be removed");
        assert!(out.contains("Body content"));
    }

    #[test]
    fn toc_short_run_not_removed() {
        // Only 3 TOC-like lines — below the 10-line threshold, keep them.
        let text = "1 One\n2 Two\n3 Three\n\nbody";
        let out = remove_toc_block(text);
        assert!(out.contains("1 One"));
    }

    #[test]
    fn toc_contents_heading_removed_with_block() {
        let toc: Vec<String> = (1..=11).map(|i| format!("{i} Entry {i}")).collect();
        let text = format!("Paper\n\n## Contents\n\n{}\n\nbody", toc.join("\n"));
        let out = remove_toc_block(&text);
        assert!(!out.contains("Contents"));
        assert!(out.contains("body"));
    }

    #[test]
    fn bare_section_numbers_recognized() {
        // Python semantics: ^\d+(\.\d+)+ requires at least one dot group.
        assert!(is_bare_section_number("3.2.1"));
        assert!(is_bare_section_number("3.2"));
        assert!(!is_bare_section_number("8"), "bare integer is NOT a TOC line");
        assert!(!is_bare_section_number("3.2.1 "));
        assert!(!is_bare_section_number("3.a"));
    }

    #[test]
    fn numbered_entry_prefix_recognized() {
        assert!(starts_with_section_number("8 Conclusion"));
        assert!(starts_with_section_number("3.1 Token-level Memory"));
        assert!(!starts_with_section_number("Conclusion"));
        // Matches Python ^\d+(\.\d+)*\s — a year followed by space matches
        // too; the <100-char guard keeps this benign.
        assert!(starts_with_section_number("2023 results were mixed"));
    }

    // -- add_structure_heuristics ----------------------------------------------

    #[test]
    fn structure_references_becomes_h2() {
        let out = add_structure_heuristics("body\nReferences\n[1] first");
        assert!(out.contains("## References"));
    }

    #[test]
    fn structure_numbered_depth() {
        let out = add_structure_heuristics("1. Introduction to things");
        assert!(out.contains("### 1. Introduction to things"));
        // Python gates on the "1 "/"1. "/"1.1 " prefixes — "1.2.3" never
        // enters the heading branch; document that inherited behaviour.
        let out2 = add_structure_heuristics("1.2.3 Deep section");
        assert!(!out2.contains("#### 1.2.3"));
    }

    #[test]
    fn structure_long_lines_untouched() {
        let long = format!("1. {}x", "word ".repeat(40));
        let out = add_structure_heuristics(&long);
        assert!(!out.contains("## "));
    }

    // -- postprocess pipeline ---------------------------------------------------

    #[test]
    fn postprocess_pipeline_full_pass() {
        let toc: Vec<String> = (1..=12).map(|i| format!("{i} Toc item {i}")).collect();
        let mut raw = String::from("Paper\n\n12\n");
        raw.push_str(&toc.join("\n"));
        // Non-TOC line terminates the TOC run before body sections start.
        raw.push_str("\nPreamble sentence here.\n1 Introduction\nThe body text.");
        let out = postprocess(&raw);
        assert!(!out.contains("Toc item 5"));
        assert!(!out.contains("\n12\n"));
        assert!(out.contains("Preamble sentence here."));
        assert!(out.contains("1 Introduction"));
    }

    #[tokio::test]
    async fn convert_invalid_pdf_errors() {
        let converter = PdfConverter::new();
        let input = Input::bytes(b"not a pdf".to_vec(), "broken.pdf");
        let result = converter.convert(input).await;
        assert!(result.is_err());
    }
}
