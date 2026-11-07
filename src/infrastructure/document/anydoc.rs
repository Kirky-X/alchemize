// SPDX-License-Identifier: Apache-2.0
//! AnydocConverter — feature-gated wrapper around the `anydoc` crate
//! (firecrawl/anydoc, MIT).
//!
//! Office formats to GitHub-Flavored Markdown across anydoc's full
//! extension table (legacy binary Office included). Runs conversion inside
//! `catch_unwind` on the blocking pool: anydoc may panic on malformed
//! input, which surfaces as a converter error instead of a process abort.

#![cfg(feature = "anydoc")]

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// anydoc's full extension table (20 extensions).
const SUPPORTED_EXTS: &[&str] = &[
    "doc", "docx", "docm", "odt", "pdf", "pptx", "pptm", "ppsx", "ppsm", "ppt", "pps", "pot",
    "rtf", "epub", "xlsx", "xlsm", "xlsb", "xls", "ods", "odp", "csv",
];

/// Converter backed by the `anydoc` crate.
pub struct AnydocConverter;

impl AnydocConverter {
    /// Create a new anydoc converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AnydocConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for AnydocConverter {
    fn name(&self) -> &'static str {
        "anydoc"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = match input.to_bytes().await {
            Ok(b) => b,
            Err(e) => return Err(AlchemizeError::file_read(input_path(&input), io_of(&e))),
        };

        // Panic protection: anydoc may panic on malformed input.
        let result = tokio::task::spawn_blocking(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                anydoc::to_markdown_bytes(&bytes, None)
            }))
        })
        .await
        .map_err(|e| AlchemizeError::other(format!("anydoc spawn_blocking panicked: {e}")))?;

        let markdown = result
            .map_err(|_| AlchemizeError::other("anydoc panicked on malformed input"))?
            .map_err(|e| AlchemizeError::other(format!("anydoc conversion failed: {e}")))?;

        let markdown = if markdown.trim().is_empty() {
            "*Document contains no extractable content.*\n".to_string()
        } else {
            markdown
        };

        let metadata = serde_json::json!({
            "source_format": input.extension().unwrap_or_else(|| "unknown".into()),
        });

        Ok(ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull))
    }

    fn supported_extensions(&self) -> &[&str] {
        SUPPORTED_EXTS
    }

    fn priority(&self) -> u8 {
        15
    }
}

/// Best-effort path string for error context.
fn input_path(input: &Input) -> String {
    match input {
        Input::Path(p) => p.display().to_string(),
        Input::Bytes { filename, .. } => filename.clone(),
    }
}

/// Coerce an anyhow error into a plain io::Error for the FileRead variant.
fn io_of(e: &anyhow::Error) -> std::io::Error {
    match e.downcast_ref::<std::io::Error>() {
        Some(io) => std::io::Error::new(io.kind(), io.to_string()),
        None => std::io::Error::other(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_full_table() {
        let converter = AnydocConverter::new();
        for ext in [
            "doc", "docm", "pps", "pot", "xlsm", "xlsb", "odp", "rtf", "pdf",
        ] {
            assert!(
                converter.supported_extensions().contains(&ext),
                "missing {ext}"
            );
        }
    }

    #[test]
    fn priority_above_native_office() {
        assert_eq!(AnydocConverter::new().priority(), 15);
    }

    #[tokio::test]
    async fn malformed_zip_errors_not_panics() {
        let converter = AnydocConverter::new();
        // Truncated zip: must return Err, never abort the process.
        let input = Input::bytes(vec![0x50, 0x4B, 0x03, 0x04, 0x00], "broken.docx");
        let result = converter.convert(input).await;
        assert!(result.is_err());
    }
}
