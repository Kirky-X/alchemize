// SPDX-License-Identifier: Apache-2.0
//! Conversion result types.

use serde::{Deserialize, Serialize};

use super::asset::Asset;

/// Converter result — markdown text with metadata and extracted assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertResult {
    /// The extracted markdown content.
    pub markdown: String,
    /// Arbitrary metadata about the conversion (page count, title, etc.).
    pub metadata: serde_json::Value,
    /// The kind of content that was converted.
    pub content_type: ContentType,
    /// Quality level of the conversion.
    pub quality: ConversionQuality,
    /// Extracted title (≤20 tokens), derived from content prefix or "Untitled".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Binary assets (images, media) extracted in document order.
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl ConvertResult {
    /// Create a minimal result with just markdown text.
    pub fn from_markdown(markdown: impl Into<String>, content_type: ContentType) -> Self {
        Self {
            markdown: markdown.into(),
            metadata: serde_json::Value::Null,
            content_type,
            quality: ConversionQuality::LocalFull,
            title: None,
            assets: Vec::new(),
        }
    }

    /// Create a result with markdown and JSON metadata.
    pub fn with_metadata(
        markdown: impl Into<String>,
        content_type: ContentType,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            markdown: markdown.into(),
            metadata,
            content_type,
            quality: ConversionQuality::LocalFull,
            title: None,
            assets: Vec::new(),
        }
    }

    /// Set the conversion quality level.
    pub fn with_quality(mut self, quality: ConversionQuality) -> Self {
        self.quality = quality;
        self
    }

    /// Set the extracted title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Append an extracted asset.
    pub fn with_asset(mut self, asset: Asset) -> Self {
        self.assets.push(asset);
        self
    }
}

/// Quality level of a format conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConversionQuality {
    /// High-quality conversion via remote API (e.g. MinerU cloud).
    Api,
    /// Full local conversion — all features available.
    #[default]
    LocalFull,
    /// Degraded local conversion — some features missing (e.g. no OCR for images in PDF).
    LocalFallback,
}

impl std::fmt::Display for ConversionQuality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversionQuality::Api => write!(f, "api"),
            ConversionQuality::LocalFull => write!(f, "local_full"),
            ConversionQuality::LocalFallback => write!(f, "local_fallback"),
        }
    }
}

/// Broad category of the source content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    /// Document formats: PDF, PPTX, DOCX, etc.
    Document,
    /// Image formats: PNG, JPEG, etc.
    Image,
    /// Audio formats: WAV, MP3, etc.
    Audio,
    /// Video formats: MP4, etc.
    Video,
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Document => write!(f, "document"),
            ContentType::Image => write!(f, "image"),
            ContentType::Audio => write!(f, "audio"),
            ContentType::Video => write!(f, "video"),
        }
    }
}

/// Video processing mode controlled by feature flag and hardware availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoProcessingMode {
    /// GPU-capable mode — frame analysis via VisionProvider is planned but not implemented.
    Gpu,
    /// No GPU — audio extraction only, skip frame analysis.
    #[default]
    AudioOnly,
    /// Video processing completely disabled.
    Disabled,
}

impl VideoProcessingMode {
    /// Whether frame extraction and analysis is possible.
    pub fn supports_frame_analysis(self) -> bool {
        matches!(self, VideoProcessingMode::Gpu)
    }

    /// Whether audio extraction is possible.
    pub fn supports_audio(self) -> bool {
        !matches!(self, VideoProcessingMode::Disabled)
    }
}

impl std::fmt::Display for VideoProcessingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoProcessingMode::Gpu => write!(f, "gpu"),
            VideoProcessingMode::AudioOnly => write!(f, "audio_only"),
            VideoProcessingMode::Disabled => write!(f, "disabled"),
        }
    }
}

/// Detail level hint for vision providers describing an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DetailLevel {
    /// Low-resolution, fast pass.
    Low,
    /// Balanced default.
    #[default]
    Moderate,
    /// Maximum fidelity.
    High,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_result_from_markdown_defaults() {
        let result = ConvertResult::from_markdown("# Hello", ContentType::Document);
        assert_eq!(result.markdown, "# Hello");
        assert_eq!(result.content_type, ContentType::Document);
        assert!(result.metadata.is_null());
        assert_eq!(result.quality, ConversionQuality::LocalFull);
        assert!(result.title.is_none());
        assert!(result.assets.is_empty());
    }

    #[test]
    fn convert_result_with_metadata() {
        let meta = serde_json::json!({"page_count": 5});
        let result = ConvertResult::with_metadata("text", ContentType::Image, meta.clone());
        assert_eq!(result.metadata, meta);
        assert_eq!(result.content_type, ContentType::Image);
    }

    #[test]
    fn convert_result_serde_roundtrip_without_assets_field() {
        // Old JSON (pre-assets) must deserialize with an empty assets vec.
        let json = r#"{"markdown":"content","metadata":null,"content_type":"audio","quality":"local_full"}"#;
        let decoded: ConvertResult = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.markdown, "content");
        assert_eq!(decoded.content_type, ContentType::Audio);
        assert!(decoded.assets.is_empty());
    }

    #[test]
    fn convert_result_serde_roundtrip_with_assets() {
        let result = ConvertResult::from_markdown("x", ContentType::Document)
            .with_asset(Asset::new("img.png", vec![1, 2, 3]));
        let json = serde_json::to_string(&result).unwrap();
        let decoded: ConvertResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.assets.len(), 1);
        assert_eq!(decoded.assets[0].filename, "img.png");
    }

    #[test]
    fn builder_methods_chain() {
        let result = ConvertResult::with_metadata(
            "# Hello",
            ContentType::Image,
            serde_json::json!({"key": "value"}),
        )
        .with_quality(ConversionQuality::LocalFallback)
        .with_title("Chained");
        assert_eq!(result.quality, ConversionQuality::LocalFallback);
        assert_eq!(result.title.as_deref(), Some("Chained"));
    }

    #[test]
    fn conversion_quality_display_and_default() {
        assert_eq!(ConversionQuality::default(), ConversionQuality::LocalFull);
        assert_eq!(format!("{}", ConversionQuality::Api), "api");
        assert_eq!(
            format!("{}", ConversionQuality::LocalFallback),
            "local_fallback"
        );
    }

    #[test]
    fn content_type_display() {
        assert_eq!(format!("{}", ContentType::Document), "document");
        assert_eq!(format!("{}", ContentType::Video), "video");
    }

    #[test]
    fn video_mode_capabilities() {
        assert!(VideoProcessingMode::Gpu.supports_frame_analysis());
        assert!(!VideoProcessingMode::AudioOnly.supports_frame_analysis());
        assert!(VideoProcessingMode::AudioOnly.supports_audio());
        assert!(!VideoProcessingMode::Disabled.supports_audio());
        assert_eq!(
            VideoProcessingMode::default(),
            VideoProcessingMode::AudioOnly
        );
    }
}
