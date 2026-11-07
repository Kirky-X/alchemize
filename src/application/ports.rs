// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Ports — traits the infrastructure layer implements.

use async_trait::async_trait;

use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ConvertResult, DetailLevel};

/// Availability status of a converter (self-described, used by `alch doctor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Ready to serve conversions.
    Available,
    /// Not usable right now, with a human-readable reason.
    Unavailable {
        /// Diagnostic reason surfaced by doctor and attempts records.
        reason: String,
    },
}

/// A converter transforms an input file (or raw bytes) into markdown.
///
/// Implementations handle specific formats (PDF, PPTX, DOCX, Image, Audio,
/// Video). Each converter declares which file extensions it supports and
/// whether it can handle a given input.
#[async_trait]
pub trait Converter: Send + Sync {
    /// Convert the input to markdown.
    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError>;

    /// File extensions this converter handles (lowercase, without dot).
    ///
    /// Example: `["pdf"]` or `["pptx"]` or `["png", "jpg", "jpeg"]`.
    fn supported_extensions(&self) -> &[&str];

    /// Check if this converter can handle the given input.
    ///
    /// The default implementation matches the input's extension against
    /// [`supported_extensions`](Self::supported_extensions).
    fn can_convert(&self, input: &Input) -> bool {
        let Some(ext) = input.extension() else {
            return false;
        };
        self.supported_extensions()
            .iter()
            .any(|&supported| supported == ext)
    }

    /// Priority of this converter (higher number = tried earlier).
    ///
    /// When multiple converters support the same extension, the one with
    /// the highest priority value is tried first.
    ///
    /// Default: 10 (medium priority).
    fn priority(&self) -> u8 {
        10
    }

    /// Stable lowercase identifier for this converter.
    ///
    /// Used by `--engine` to select a specific backend by name and recorded
    /// on results as `metadata.converter`.
    fn name(&self) -> &'static str {
        "unknown"
    }

    /// Whether this converter is usable in the current environment.
    ///
    /// Default: `Available`. Converters with external dependencies (MinerU
    /// HTTP endpoint, vision provider, transcribe models) override to return
    /// `Unavailable` with a diagnostic reason when their dependency is
    /// missing or unreachable.
    fn availability(&self) -> Availability {
        Availability::Available
    }

    /// Whether this engine requires remote connectivity (cloud API).
    ///
    /// Remote engines are excluded from candidate chains when the registry
    /// runs in no-remote mode (`--no-remote`). Default: local engine.
    fn is_remote(&self) -> bool {
        false
    }
}

/// Vision provider port — image description and OCR.
///
/// Implementations call external vision models (e.g. Ollama VLM, cloud API);
/// alchemize ships none by default. Without a provider, image converters
/// degrade to metadata-only markdown.
#[async_trait]
pub trait VisionProvider: Send + Sync {
    /// Describe an image at the requested detail level.
    async fn describe(&self, image_data: &[u8], detail: DetailLevel) -> anyhow::Result<String>;

    /// Perform OCR on an image, returning extracted text.
    async fn ocr(&self, image_data: &[u8]) -> anyhow::Result<String>;

    /// Human-readable provider name for logging.
    fn name(&self) -> &'static str;
}

/// Transcript of an audio/video input, with VAD segment boundaries.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Transcript {
    /// Detected or configured language tag, if known.
    pub language: Option<String>,
    /// Total duration in seconds, if known.
    pub duration: Option<f64>,
    /// VAD segments in chronological order (no speaker labels — VAD carries
    /// no speaker identity; attribution is intentionally not fabricated).
    pub segments: Vec<TranscriptSegment>,
}

/// One VAD segment with its transcription.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSegment {
    /// Segment start in seconds.
    pub start_sec: f64,
    /// Segment end in seconds.
    pub end_sec: f64,
    /// Transcribed text for the segment.
    pub text: String,
}

impl Transcript {
    /// The disclaimer carried with every transcript output.
    pub const DISCLAIMER: &'static str =
        "分段由 VAD（语音活动检测）产生，非说话人识别；不提供说话人归属。";

    /// Concatenated text of all segments.
    pub fn full_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Transcription provider port — audio to text.
///
/// The default backend wraps the in-house vecboost transcribe facade
/// (Qwen3-ASR ONNX + silero VAD); a Whisper-compatible HTTP API backend is
/// also available. When no backend is available, audio converters degrade
/// to metadata-only markdown.
#[async_trait]
pub trait TranscribeProvider: Send + Sync {
    /// Transcribe 16kHz mono WAV bytes into a segment transcript.
    async fn transcribe(&self, audio_wav16k: &[u8]) -> Result<Transcript, AlchemizeError>;

    /// Whether this provider is usable in the current environment.
    fn availability(&self) -> Availability;

    /// Provider name for logging and engine selection.
    fn name(&self) -> &'static str;
}

/// External binary tool port (pandoc / ffmpeg) — build-time absent, runtime probed.
pub trait ExternalTool: Send + Sync {
    /// Program name probed on PATH.
    fn program(&self) -> &str;

    /// Probe availability: program resolvable on PATH plus any version check.
    fn probe(&self) -> Availability {
        if which_program(self.program()) {
            Availability::Available
        } else {
            Availability::Unavailable {
                reason: format!("'{}' not found on PATH", self.program()),
            }
        }
    }
}

/// Check whether a program is resolvable on PATH (no shell invocation).
pub fn which_program(program: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = if cfg!(windows) {
            dir.join(format!("{program}.exe"))
        } else {
            dir.join(program)
        };
        candidate.is_file()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_equality() {
        assert_eq!(Availability::Available, Availability::Available);
        assert_eq!(
            Availability::Unavailable {
                reason: "x".to_string()
            },
            Availability::Unavailable {
                reason: "x".to_string()
            }
        );
        assert_ne!(
            Availability::Available,
            Availability::Unavailable {
                reason: "x".to_string()
            }
        );
    }

    #[test]
    fn transcript_full_text_joins_segments() {
        let t = Transcript {
            language: Some("zh".into()),
            duration: Some(10.0),
            segments: vec![
                TranscriptSegment {
                    start_sec: 0.0,
                    end_sec: 2.5,
                    text: "你好".into(),
                },
                TranscriptSegment {
                    start_sec: 3.0,
                    end_sec: 5.0,
                    text: "世界".into(),
                },
            ],
        };
        assert_eq!(t.full_text(), "你好 世界");
        assert!(Transcript::DISCLAIMER.contains("VAD"));
    }

    #[test]
    fn which_program_finds_ls_and_rejects_obscure() {
        if cfg!(unix) {
            assert!(which_program("ls"));
        }
        assert!(!which_program("definitely-not-a-real-program-xyz"));
    }

    #[test]
    fn transcript_serde_roundtrip() {
        let t = Transcript {
            language: None,
            duration: Some(1.5),
            segments: vec![TranscriptSegment {
                start_sec: 0.0,
                end_sec: 1.5,
                text: "hi".into(),
            }],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(back.segments.len(), 1);
        assert_eq!(back.duration, Some(1.5));
    }
}
