// Copyright © 2026 Kirky.X. All rights reserved.
//! DOCX to Markdown converter using ZIP + quick-xml.

use std::io::Read;

use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

use super::local_tag_name;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts DOCX files to markdown by parsing the ZIP-embedded XML.
///
/// A DOCX file is a ZIP archive containing `word/document.xml`. Text content
/// is inside `<w:t>` tags within the WordprocessingML namespace. Paragraph
/// structure (`<w:p>`) is preserved as separate lines.
pub struct DocxConverter;

impl DocxConverter {
    /// Create a new DOCX converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DocxConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for DocxConverter {
    fn name(&self) -> &'static str {
        "docx"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read DOCX input: {e}")))?;

        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| AlchemizeError::Zip(format!("{e}")))?;

        let mut xml_content = String::new();
        {
            let mut doc_file = archive
                .by_name("word/document.xml")
                .map_err(|e| AlchemizeError::Zip(format!("word/document.xml not found: {e}")))?;
            doc_file
                .read_to_string(&mut xml_content)
                .map_err(|e| AlchemizeError::Zip(format!("{e}")))?;
        }

        let paragraphs = extract_paragraphs_from_docx_xml(&xml_content)?;

        let paragraph_count = paragraphs.len();
        let markdown = if paragraphs.is_empty() {
            "*DOCX contains no extractable text.*\n".to_string()
        } else {
            paragraphs.join("\n\n") + "\n"
        };

        let metadata = serde_json::json!({
            "paragraph_count": paragraph_count,
            "source_format": "docx",
        });

        Ok(ConvertResult::with_metadata(
            markdown,
            ContentType::Document,
            metadata,
        ))
    }

    fn supported_extensions(&self) -> &[&str] {
        &["docx"]
    }
}

/// Extract paragraphs from `word/document.xml`.
///
/// Each `<w:p>` element produces one paragraph. Text runs (`<w:r>`) containing
/// `<w:t>` elements are concatenated within each paragraph. An empty `<w:p>`
/// produces an empty line to preserve document spacing.
fn extract_paragraphs_from_docx_xml(xml: &str) -> Result<Vec<String>, AlchemizeError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current_paragraph = String::new();
    let mut in_paragraph = false;
    let mut in_text = false;
    let mut paragraph_has_content = false;

    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = local_tag_name(e.name().as_ref());
                match local_name.as_str() {
                    "p" => {
                        in_paragraph = true;
                        current_paragraph.clear();
                        paragraph_has_content = false;
                    }
                    "t" if in_paragraph => {
                        in_text = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = local_tag_name(e.name().as_ref());
                match local_name.as_str() {
                    "p" => {
                        if paragraph_has_content {
                            paragraphs.push(current_paragraph.trim().to_string());
                        } else {
                            paragraphs.push(String::new());
                        }
                        in_paragraph = false;
                        current_paragraph.clear();
                    }
                    "t" => {
                        in_text = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) if in_text => {
                let text = String::from_utf8_lossy(e.as_bytes()).to_string();
                current_paragraph.push_str(&text);
                if !text.trim().is_empty() {
                    paragraph_has_content = true;
                }
            }
            Ok(Event::Text(_)) => {}
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(AlchemizeError::Xml(format!(
                    "XML parse error at position {}: {e}",
                    reader.error_position()
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    // Trim trailing empty paragraphs.
    while paragraphs.last().is_some_and(|p| p.is_empty()) {
        paragraphs.pop();
    }

    Ok(paragraphs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_docx_extension() {
        let converter = DocxConverter::new();
        assert_eq!(converter.supported_extensions(), &["docx"]);
    }

    #[test]
    fn can_convert_docx_input() {
        let converter = DocxConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "document.docx".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn cannot_convert_non_docx() {
        let converter = DocxConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "test.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(!converter.can_convert(&input));
    }

    #[test]
    fn extract_paragraphs_simple() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Hello</w:t></w:r></w:p>
                <w:p><w:r><w:t>World</w:t></w:r></w:p>
            </w:body>
        </document>"#;
        let paragraphs = extract_paragraphs_from_docx_xml(xml).unwrap();
        assert_eq!(paragraphs, vec!["Hello", "World"]);
    }

    #[test]
    fn extract_paragraphs_concatenates_runs() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p>
                    <w:r><w:t>Hello </w:t></w:r>
                    <w:r><w:t>World</w:t></w:r>
                </w:p>
            </w:body>
        </document>"#;
        let paragraphs = extract_paragraphs_from_docx_xml(xml).unwrap();
        assert_eq!(paragraphs, vec!["Hello World"]);
    }

    #[test]
    fn extract_paragraphs_preserves_empty_lines() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>First</w:t></w:r></w:p>
                <w:p></w:p>
                <w:p><w:r><w:t>Third</w:t></w:r></w:p>
            </w:body>
        </document>"#;
        let paragraphs = extract_paragraphs_from_docx_xml(xml).unwrap();
        assert_eq!(paragraphs, vec!["First", "", "Third"]);
    }

    #[test]
    fn extract_paragraphs_trims_trailing_empty() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:body>
                <w:p><w:r><w:t>Content</w:t></w:r></w:p>
                <w:p></w:p>
            </w:body>
        </document>"#;
        let paragraphs = extract_paragraphs_from_docx_xml(xml).unwrap();
        assert_eq!(paragraphs, vec!["Content"]);
    }

    #[tokio::test]
    async fn rejects_invalid_zip_bytes() {
        let converter = DocxConverter::new();
        let input = Input::Bytes {
            data: b"not a zip".to_vec(),
            filename: "bad.docx".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn default_creates_instance() {
        let _converter = DocxConverter;
    }
}
