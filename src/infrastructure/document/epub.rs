// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! EPUB to Markdown converter using rbook + htmd.

use std::io::Cursor;

use async_trait::async_trait;
use htmd::HtmlToMarkdown;
use rbook::epub::Epub;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts EPUB ebook files to markdown.
///
/// Each spine item is extracted and converted with htmd; the book title
/// (from EPUB metadata) becomes the top heading when present.
pub struct EpubConverter {
    html_converter: HtmlToMarkdown,
}

impl EpubConverter {
    /// Create a new EPUB converter.
    pub fn new() -> Self {
        Self {
            html_converter: HtmlToMarkdown::new(),
        }
    }
}

impl Default for EpubConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for EpubConverter {
    fn name(&self) -> &'static str {
        "epub"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read EPUB input: {e}")))?;

        let epub = Epub::read(Cursor::new(bytes))
            .map_err(|e| AlchemizeError::Zip(format!("failed to open EPUB: {e}")))?;

        let mut markdown_parts = Vec::new();
        let mut chapter_count = 0;
        let mut chapter_bytes = 0usize;
        let mut failed_chapters = 0usize;
        let mut last_error: Option<String> = None;

        if let Some(title) = epub.metadata().title() {
            markdown_parts.push(format!(
                "# {}\n",
                crate::infrastructure::document::shared::escape_markdown_label(title.value())
            ));
        }

        let mut reader = epub.reader();
        while let Some(result) = reader.read_next() {
            // Degrade rather than fail (project principle): a broken chapter
            // is recorded and skipped so the remaining chapters still reach
            // the caller; only a book with zero successful chapters errors.
            let data = match result {
                Ok(data) => data,
                Err(e) => {
                    failed_chapters += 1;
                    last_error = Some(format!("EPUB chapter read failed: {e}"));
                    continue;
                }
            };
            let content = data.content();
            chapter_bytes = chapter_bytes.saturating_add(content.len());
            crate::infrastructure::document::shared::check_output_budget(chapter_bytes)?;

            if content.trim().is_empty() {
                continue;
            }

            let chapter_md = self.html_converter.convert(content).map_err(|e| {
                AlchemizeError::other(format!("chapter HTML conversion failed: {e}"))
            })?;

            if !chapter_md.trim().is_empty() {
                let chapter_md = if !chapter_md.trim().starts_with('#') {
                    format!("## Chapter {}\n\n{}", chapter_count + 1, chapter_md)
                } else {
                    chapter_md
                };

                markdown_parts.push(chapter_md.trim().to_string());
                markdown_parts.push(String::new());
                chapter_count += 1;
            }
        }

        // Every chapter failed: surface the reason instead of a title-only
        // document that hides the loss.
        if chapter_count == 0
            && failed_chapters > 0
            && let Some(error) = last_error
        {
            return Err(AlchemizeError::other(error));
        }

        let markdown = if markdown_parts.is_empty() {
            "*EPUB contains no extractable content.*\n".to_string()
        } else {
            markdown_parts.join("\n") + "\n"
        };

        let metadata = serde_json::json!({
            "source_format": "epub",
            "chapter_count": chapter_count,
            "failed_chapters": failed_chapters,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["epub"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(EpubConverter::new().supported_extensions(), &["epub"]);
    }

    #[tokio::test]
    async fn convert_invalid_epub_errors() {
        let converter = EpubConverter::new();
        let input = Input::bytes(b"not an epub".to_vec(), "broken.epub");
        let result = converter.convert(input).await;
        assert!(result.is_err());
    }
}

#[test]
fn default_matches_new() {
    let _ = EpubConverter::default();
}
