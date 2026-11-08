// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Alchemize builder — fluent API for constructing a configured converter.

use std::sync::Arc;

use crate::application::ports::{TranscribeProvider, VisionProvider};
use crate::application::registry::ConverterRegistry;
use crate::domain::asset::{AssetSink, DirSink, InlineSink};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ConvertResult, VideoProcessingMode};

/// Configuration for MinerU remote conversion (local + cloud endpoints).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MineruConfig {
    /// Base URL for a local MinerU service.
    pub base_url: String,
    /// Base URL for the MinerU Agent API (cloud).
    pub agent_base_url: String,
    /// Base URL for the MinerU Precision API (cloud).
    pub precision_base_url: String,
    /// Optional bearer token for cloud endpoints (redacted in Debug below).
    #[serde(skip_serializing)]
    pub api_token: Option<String>,
    /// Per-request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for MineruConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8888".to_string(),
            agent_base_url: "https://mineru.net".to_string(),
            precision_base_url: "https://mineru.net".to_string(),
            api_token: None,
            timeout_secs: 300,
        }
    }
}

impl std::fmt::Debug for MineruConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // api_token deliberately excluded: never leak secrets in Debug output.
        f.debug_struct("MineruConfig")
            .field("base_url", &self.base_url)
            .field("agent_base_url", &self.agent_base_url)
            .field("precision_base_url", &self.precision_base_url)
            .field(
                "api_token",
                &if self.api_token.is_some() {
                    "<redacted>"
                } else {
                    "None"
                },
            )
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl std::fmt::Display for MineruConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // api_token deliberately excluded: never leak secrets in logs.
        write!(
            f,
            "MineruConfig{{base_url={}, agent={}, precision={}, timeout={}s, token={}}}",
            self.base_url,
            self.agent_base_url,
            self.precision_base_url,
            self.timeout_secs,
            if self.api_token.is_some() {
                "<redacted>"
            } else {
                "none"
            }
        )
    }
}

/// html-moli 引擎配置（feature `moli`）：链接绝对化基准与主内容提取开关。
#[derive(Debug, Clone, Default)]
pub struct MoliEngineConfig {
    /// 相对链接绝对化基准（None = 不改写链接）
    pub base_url: Option<String>,
    /// 主内容提取（dom_smoothie；false = 全页转换）
    pub extract_main: bool,
}

/// Where extracted assets are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetMode {
    /// Write to an `images/` directory next to the output (default).
    #[default]
    Dir,
    /// Inline as base64 data URLs (single-file delivery).
    Inline,
}

/// Built converter facade over the registry.
pub struct Alchemize {
    registry: std::sync::Arc<ConverterRegistry>,
    /// ZIP handled at the facade level: it shares the main registry for
    /// nested conversion; zip itself is not a registry member (unbounded
    /// self-recursion), so nested zips report unsupported.
    zip: Option<crate::infrastructure::document::ZipConverter>,
    asset_mode: AssetMode,
    no_remote: bool,
}

impl Alchemize {
    /// Convert an input to markdown using the fallback chain.
    pub async fn convert(&self, input: &Input) -> Result<ConvertResult, AlchemizeError> {
        self.convert_pref(input, None).await
    }

    /// Convert with a preferred engine name.
    pub async fn convert_pref(
        &self,
        input: &Input,
        prefer: Option<&str>,
    ) -> Result<ConvertResult, AlchemizeError> {
        if input.extension().as_deref() == Some("zip") {
            if let Some(ref zip) = self.zip {
                use crate::application::ports::Converter as _;
                return zip.convert(input.clone()).await;
            }
            return Err(AlchemizeError::UnsupportedFormat("zip".to_string()));
        }
        self.registry.convert_pref(input, prefer).await
    }

    /// Aggregate engine availability for diagnostics.
    pub fn availability(
        &self,
    ) -> Vec<(String, Vec<String>, crate::application::ports::Availability)> {
        let mut rows = self.registry.availability();
        if self.zip.is_some() {
            rows.push((
                "zip".to_string(),
                vec!["zip".to_string()],
                crate::application::ports::Availability::Available,
            ));
        }
        rows
    }

    /// Registered engine names.
    pub fn engines(&self) -> Vec<&'static str> {
        self.registry.converters().map(|c| c.name()).collect()
    }

    /// Number of registered engines (zip excluded; see [`Alchemize`]).
    pub fn engine_count(&self) -> usize {
        self.registry.len()
    }

    /// Whether remote engines were excluded at build time.
    pub fn is_no_remote(&self) -> bool {
        self.no_remote
    }

    /// Build an asset sink for an output location.
    pub fn asset_sink_for(&self, output_root: &std::path::Path) -> Box<dyn AssetSink> {
        match self.asset_mode {
            AssetMode::Dir => Box::new(DirSink::new(output_root.join("images"))),
            AssetMode::Inline => Box::new(InlineSink),
        }
    }
}

/// Builder for constructing a configured [`Alchemize`].
pub struct AlchemizeBuilder {
    max_file_size: usize,
    mineru: Option<MineruConfig>,
    vision_provider: Option<Arc<dyn VisionProvider>>,
    transcribe_provider: Option<Arc<dyn TranscribeProvider>>,
    video_mode: VideoProcessingMode,
    asset_mode: AssetMode,
    no_remote: bool,
    moli: Option<MoliEngineConfig>,
}

/// Maximum file size default: 100 MB.
const DEFAULT_MAX_FILE_SIZE: usize = 100 * 1024 * 1024;

impl Default for AlchemizeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AlchemizeBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self {
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            mineru: None,
            vision_provider: None,
            transcribe_provider: None,
            video_mode: VideoProcessingMode::default(),
            asset_mode: AssetMode::default(),
            no_remote: false,
            moli: None,
        }
    }

    /// Set the html-moli engine configuration (engine registered when the
    /// `moli` feature is compiled in).
    pub fn with_moli(mut self, config: MoliEngineConfig) -> Self {
        self.moli = Some(config);
        self
    }

    /// Set the MinerU endpoint configuration (enables MinerU engines when
    /// the `mineru` feature is compiled in).
    pub fn with_mineru(mut self, config: MineruConfig) -> Self {
        self.mineru = Some(config);
        self
    }

    /// Set the vision provider used by the image converters for
    /// description and OCR.
    ///
    /// Without a provider those converters degrade to metadata-only
    /// markdown; no built-in fallback provider exists.
    pub fn with_vision_provider(mut self, provider: Arc<dyn VisionProvider>) -> Self {
        self.vision_provider = Some(provider);
        self
    }

    /// Set the transcription provider used by audio/video converters.
    pub fn with_transcribe_provider(mut self, provider: Arc<dyn TranscribeProvider>) -> Self {
        self.transcribe_provider = Some(provider);
        self
    }

    /// Set video processing mode (Gpu, AudioOnly, Disabled).
    pub fn with_video_mode(mut self, mode: VideoProcessingMode) -> Self {
        self.video_mode = mode;
        self
    }

    /// Set the asset output mode (directory or inline base64).
    pub fn with_asset_mode(mut self, mode: AssetMode) -> Self {
        self.asset_mode = mode;
        self
    }

    /// Exclude remote (cloud API) engines from the registry.
    pub fn with_no_remote(mut self) -> Self {
        self.no_remote = true;
        self
    }

    /// Set the maximum allowed input file size in bytes.
    pub fn with_max_file_size(mut self, max_bytes: usize) -> Self {
        self.max_file_size = max_bytes;
        self
    }

    /// Build the converter with all configured engines registered.
    pub fn build(mut self) -> Alchemize {
        let mut registry = if self.no_remote {
            ConverterRegistry::new_no_remote()
        } else {
            ConverterRegistry::new()
        };

        // html-moli：feature moli 编译即注册（默认配置）；with_moli 仅定制
        // base_url/extract_main——与 anydoc「编译即可用」的注册语义一致。
        #[cfg(feature = "moli")]
        if self.moli.is_none() {
            self.moli = Some(MoliEngineConfig::default());
        }

        for converter in crate::infrastructure::engines::default_converters(
            &self.mineru,
            self.vision_provider.clone(),
            self.transcribe_provider.clone(),
            self.video_mode,
            self.max_file_size,
            &self.moli,
        ) {
            registry.register(converter);
        }

        // Shared registry: the facade and the zip converter (nested
        // conversion) use the same engine set; zip is not a member itself.
        let registry = std::sync::Arc::new(registry);
        let zip = crate::infrastructure::document::ZipConverter::with_arc(std::sync::Arc::clone(
            &registry,
        ));

        Alchemize {
            registry,
            zip: Some(zip),
            asset_mode: self.asset_mode,
            no_remote: self.no_remote,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mineru_config_defaults_match_contract() {
        let c = MineruConfig::default();
        assert_eq!(c.base_url, "http://localhost:8888");
        assert_eq!(c.agent_base_url, "https://mineru.net");
        assert_eq!(c.precision_base_url, "https://mineru.net");
        assert_eq!(c.timeout_secs, 300);
        assert!(c.api_token.is_none());
    }

    #[test]
    fn mineru_config_display_redacts_token() {
        let c = MineruConfig {
            api_token: Some("secret-token".into()),
            ..Default::default()
        };
        let msg = format!("{c}");
        assert!(msg.contains("<redacted>"));
        assert!(!msg.contains("secret-token"));
        // Manual Debug impl also redacts the token (derived Debug would print it).
        let dbg = format!("{c:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("secret-token"));
        // serde serialization also skips the token
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("secret-token"));
    }

    #[test]
    fn builder_defaults() {
        let alch = AlchemizeBuilder::new().build();
        assert!(!alch.is_no_remote());
        // Dir-mode sink construction must succeed for any output root.
        let _sink = alch.asset_sink_for(std::path::Path::new("/tmp/out"));
    }

    #[test]
    fn no_remote_propagates() {
        let alch = AlchemizeBuilder::new().with_no_remote().build();
        assert!(alch.is_no_remote());
    }

    #[test]
    fn mineru_engine_gated_by_config() {
        // Without with_mineru, no engine name contains "mineru" —
        // regardless of the cargo feature flag.
        let alch = AlchemizeBuilder::new().build();
        assert!(alch.engines().iter().all(|name| !name.contains("mineru")));
    }

    #[test]
    fn video_mode_default_is_audio_only() {
        assert_eq!(
            VideoProcessingMode::default(),
            VideoProcessingMode::AudioOnly
        );
    }
}
