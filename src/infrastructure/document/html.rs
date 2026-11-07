// SPDX-License-Identifier: Apache-2.0
//! HTML to Markdown converter using htmd.

use async_trait::async_trait;
use htmd::HtmlToMarkdown;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts HTML files to markdown using the htmd crate.
///
/// Handles headings, paragraphs, lists, links, images, tables, and code
/// blocks. LaTeXML-style math/bibliography preprocessing lands with the
/// fidelity pass.
pub struct HtmlConverter {
    converter: HtmlToMarkdown,
}

impl HtmlConverter {
    /// Create a new HTML converter.
    pub fn new() -> Self {
        Self {
            converter: HtmlToMarkdown::new(),
        }
    }
}

impl Default for HtmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for HtmlConverter {
    fn name(&self) -> &'static str {
        "html"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read HTML input: {e}")))?;

        let html = String::from_utf8(bytes).map_err(|e| {
            AlchemizeError::other(format!("HTML content is not valid UTF-8: {e}"))
        })?;

        let markdown = self
            .converter
            .convert(&html)
            .map_err(|e| AlchemizeError::other(format!("HTML conversion failed: {e}")))?;

        let markdown = if markdown.trim().is_empty() {
            "*HTML document contains no extractable content.*\n".to_string()
        } else {
            markdown.trim().to_string() + "\n"
        };

        let metadata = serde_json::json!({
            "source_format": "html",
            "original_size": html.len(),
            "output_size": markdown.len(),
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["html", "htm"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(HtmlConverter::new().supported_extensions(), &["html", "htm"]);
    }

    #[tokio::test]
    async fn convert_simple_html() {
        let converter = HtmlConverter::new();
        let input = Input::bytes(
            "<h1>Title</h1><p>Hello world</p>".as_bytes().to_vec(),
            "test.html",
        );
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Title"));
        assert!(result.markdown.contains("Hello world"));
    }

    #[tokio::test]
    async fn convert_html_with_list() {
        let converter = HtmlConverter::new();
        let input = Input::bytes(
            "<ul><li>Item 1</li><li>Item 2</li></ul>".as_bytes().to_vec(),
            "test.html",
        );
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Item 1"));
        assert!(result.markdown.contains("Item 2"));
    }

    #[tokio::test]
    async fn convert_empty_html() {
        let converter = HtmlConverter::new();
        let input = Input::bytes(
            "<html><body></body></html>".as_bytes().to_vec(),
            "test.html",
        );
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("no extractable content"));
    }
}
