// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Image to Markdown converter using optional VisionProvider for OCR.

use std::sync::Arc;

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::application::ports::VisionProvider;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Supported image file extensions.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tiff", "tif", "svg",
];

/// Converts image files to markdown.
///
/// If a `VisionProvider` is available, uses it for OCR and image description.
/// Otherwise, returns basic metadata (filename, size) as markdown.
pub struct ImageConverter {
    vision: Option<Arc<dyn VisionProvider>>,
}

impl ImageConverter {
    /// Create an image converter without vision capabilities.
    pub fn new() -> Self {
        Self { vision: None }
    }

    /// Create an image converter with a vision provider for OCR and analysis.
    pub fn with_vision(vision: Arc<dyn VisionProvider>) -> Self {
        Self {
            vision: Some(vision),
        }
    }
}

impl Default for ImageConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for ImageConverter {
    fn name(&self) -> &'static str {
        "image"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read image input: {e}")))?;

        let filename = input.filename().unwrap_or("unknown").to_string();
        let byte_count = bytes.len();

        if let Some(ref vision) = self.vision {
            // Use vision provider for OCR + description.
            let ocr_text = vision
                .ocr(&bytes)
                .await
                .map_err(|e| AlchemizeError::VisionUnavailable(format!("OCR failed: {e}")))?;

            let description = vision
                .describe(&bytes, crate::domain::result::DetailLevel::Moderate)
                .await
                .map_err(|e| AlchemizeError::VisionUnavailable(format!("analysis failed: {e}")))?;

            let mut markdown = String::new();

            if !description.is_empty() {
                markdown.push_str(&format!("**Image description:** {description}\n\n"));
            }

            if !ocr_text.trim().is_empty() {
                markdown.push_str(&format!("**Extracted text:**\n\n{ocr_text}\n"));
            }

            if markdown.is_empty() {
                markdown = format!("*Image '{filename}' — no text or description extracted.*\n");
            }

            let metadata = serde_json::json!({
                "filename": filename,
                "byte_count": byte_count,
                "vision_provider": vision.name(),
                "source_format": "image",
            });

            Ok(
                ConvertResult::with_metadata(markdown, ContentType::Image, metadata)
                    .with_quality(ConversionQuality::LocalFull),
            )
        } else {
            // No vision provider — return metadata-only markdown.
            let markdown = format!(
                "*Image '{filename}' ({byte_count} bytes) — no vision provider \
                 configured for OCR.*\n"
            );

            let metadata = serde_json::json!({
                "filename": filename,
                "byte_count": byte_count,
                "source_format": "image",
            });

            Ok(
                ConvertResult::with_metadata(markdown, ContentType::Image, metadata)
                    .with_quality(ConversionQuality::LocalFallback),
            )
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        IMAGE_EXTENSIONS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_image_extensions() {
        let converter = ImageConverter::new();
        let exts = converter.supported_extensions();
        assert!(exts.contains(&"png"));
        assert!(exts.contains(&"jpg"));
        assert!(exts.contains(&"jpeg"));
        assert!(exts.contains(&"gif"));
        assert!(exts.contains(&"webp"));
    }

    #[test]
    fn can_convert_png() {
        let converter = ImageConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "photo.png".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn can_convert_jpeg() {
        let converter = ImageConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "photo.jpeg".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn cannot_convert_pdf() {
        let converter = ImageConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "doc.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(!converter.can_convert(&input));
    }

    #[tokio::test]
    async fn returns_metadata_without_vision() {
        let converter = ImageConverter::new();
        let input = Input::Bytes {
            data: vec![0x89, 0x50, 0x4E, 0x47],
            filename: "test.png".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await.unwrap();
        assert_eq!(result.content_type, ContentType::Image);
        assert!(result.markdown.contains("test.png"));
        assert!(result.markdown.contains("no vision provider"));
    }

    #[test]
    fn default_creates_instance() {
        let _converter = ImageConverter::default();
    }
}
