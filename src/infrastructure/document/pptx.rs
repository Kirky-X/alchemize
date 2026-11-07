// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! PPTX to Markdown converter using ZIP + quick-xml.

use std::io::Read;

use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

use super::local_tag_name;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConvertResult};

/// Converts PPTX files to markdown by parsing ZIP-embedded XML slides.
///
/// A PPTX file is a ZIP archive containing XML files. Each slide is stored
/// as `ppt/slides/slideN.xml`. Text content is inside `<a:t>` tags within
/// the DrawingML namespace. Slides are grouped under `## Slide N` headings.
pub struct PptxConverter;

impl PptxConverter {
    /// Create a new PPTX converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PptxConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for PptxConverter {
    fn name(&self) -> &'static str {
        "pptx"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read PPTX input: {e}")))?;

        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| AlchemizeError::Zip(format!("{e}")))?;

        // Collect slide files and sort by slide number.
        let mut slide_entries: Vec<(usize, String)> = Vec::new();
        for i in 0..archive.len() {
            let file = archive
                .by_index(i)
                .map_err(|e| AlchemizeError::Zip(format!("{e}")))?;
            // zip 9.x：条目名解码可能失败（非法编码），错误直接冒泡而非吞掉
            let name = file
                .name()
                .map_err(|e| AlchemizeError::Zip(format!("{e}")))?
                .to_string();
            if let Some(num) = extract_slide_number(&name) {
                slide_entries.push((num, name));
            }
        }
        slide_entries.sort_by_key(|(num, _)| *num);

        let slide_count = slide_entries.len();
        let mut markdown = String::new();

        for (slide_num, entry_name) in &slide_entries {
            let mut file = archive
                .by_name(entry_name)
                .map_err(|e| AlchemizeError::Zip(format!("{e}")))?;
            let mut xml_content = String::new();
            file.read_to_string(&mut xml_content)
                .map_err(|e| AlchemizeError::Zip(format!("{e}")))?;

            let text_blocks = extract_text_from_slide_xml(&xml_content)?;
            if text_blocks.is_empty() {
                continue;
            }

            if !markdown.is_empty() {
                markdown.push('\n');
            }
            markdown.push_str(&format!("## Slide {slide_num}\n\n"));
            for block in &text_blocks {
                markdown.push_str(block);
                markdown.push('\n');
            }
        }

        if markdown.is_empty() {
            markdown = "*PPTX contains no extractable text.*\n".to_string();
        }

        let metadata = serde_json::json!({
            "slide_count": slide_count,
            "source_format": "pptx",
        });

        Ok(ConvertResult::with_metadata(
            markdown,
            ContentType::Document,
            metadata,
        ))
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pptx"]
    }
}

/// Extract the slide number from a path like `ppt/slides/slide3.xml`.
fn extract_slide_number(path: &str) -> Option<usize> {
    let path = path.replace('\\', "/");
    let filename = path.rsplit('/').next()?;
    if !filename.starts_with("slide") || !filename.ends_with(".xml") {
        return None;
    }
    let num_str = &filename[5..filename.len() - 4];
    num_str.parse().ok()
}

/// Extract text blocks from a slide XML document.
///
/// Text is grouped by paragraphs (`<a:p>`), with each paragraph becoming
/// a separate line in the output. Within a paragraph, all `<a:t>` text
/// fragments are concatenated.
fn extract_text_from_slide_xml(xml: &str) -> Result<Vec<String>, AlchemizeError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current_paragraph = String::new();
    let mut in_paragraph = false;
    let mut in_text = false;

    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = local_tag_name(e.name().as_ref());
                match local_name.as_str() {
                    "p" => {
                        // New paragraph — flush previous if non-empty.
                        if in_paragraph && !current_paragraph.trim().is_empty() {
                            paragraphs.push(current_paragraph.trim().to_string());
                        }
                        current_paragraph.clear();
                        in_paragraph = true;
                    }
                    "t" => {
                        in_text = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = local_tag_name(e.name().as_ref());
                match local_name.as_str() {
                    "p" => {
                        if !current_paragraph.trim().is_empty() {
                            paragraphs.push(current_paragraph.trim().to_string());
                        }
                        current_paragraph.clear();
                        in_paragraph = false;
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

    // Flush any remaining paragraph.
    if in_paragraph && !current_paragraph.trim().is_empty() {
        paragraphs.push(current_paragraph.trim().to_string());
    }

    Ok(paragraphs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_pptx_extension() {
        let converter = PptxConverter::new();
        assert_eq!(converter.supported_extensions(), &["pptx"]);
    }

    #[test]
    fn can_convert_pptx_input() {
        let converter = PptxConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "presentation.pptx".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn cannot_convert_non_pptx() {
        let converter = PptxConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "test.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(!converter.can_convert(&input));
    }

    #[test]
    fn extract_slide_number_valid() {
        assert_eq!(extract_slide_number("ppt/slides/slide1.xml"), Some(1));
        assert_eq!(extract_slide_number("ppt/slides/slide12.xml"), Some(12));
        assert_eq!(extract_slide_number("ppt\\slides\\slide3.xml"), Some(3));
    }

    #[test]
    fn extract_slide_number_invalid() {
        assert_eq!(
            extract_slide_number("ppt/slides/_rels/slide1.xml.rels"),
            None
        );
        assert_eq!(extract_slide_number("content.xml"), None);
        assert_eq!(extract_slide_number("ppt/presentation.xml"), None);
    }

    #[test]
    fn extract_text_from_slide_xml_simple() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <a:p><a:r><a:t>Hello</a:t></a:r></a:p>
            <a:p><a:r><a:t>World</a:t></a:r></a:p>
        </sld>"#;
        let blocks = extract_text_from_slide_xml(xml).unwrap();
        assert_eq!(blocks, vec!["Hello", "World"]);
    }

    #[test]
    fn extract_text_from_slide_xml_concatenates_fragments() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <a:p>
                <a:r><a:t>Hello </a:t></a:r>
                <a:r><a:t>World</a:t></a:r>
            </a:p>
        </sld>"#;
        let blocks = extract_text_from_slide_xml(xml).unwrap();
        assert_eq!(blocks, vec!["Hello World"]);
    }

    #[test]
    fn extract_text_from_slide_xml_empty() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <a:p><a:r><a:t></a:t></a:r></a:p>
        </sld>"#;
        let blocks = extract_text_from_slide_xml(xml).unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn local_tag_name_strips_namespace() {
        assert_eq!(local_tag_name("a:t"), "t");
        assert_eq!(local_tag_name("a:p"), "p");
        assert_eq!(local_tag_name("p"), "p");
    }

    #[tokio::test]
    async fn rejects_invalid_zip_bytes() {
        let converter = PptxConverter::new();
        let input = Input::Bytes {
            data: b"not a zip".to_vec(),
            filename: "bad.pptx".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn default_creates_instance() {
        let _converter = PptxConverter;
    }
}
