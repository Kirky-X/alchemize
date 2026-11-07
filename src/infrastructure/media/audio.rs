// Copyright © 2026 Kirky.X. All rights reserved.
//! Audio to Markdown converter.
//!
//! Requires an ASR (Automatic Speech Recognition) provider for transcription.
//! Without one, returns metadata-only markdown.

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Supported audio file extensions.
const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "ogg", "flac", "aac", "m4a", "wma", "opus"];

use std::sync::Arc;

use crate::application::ports::TranscribeProvider;

/// Converts audio files to markdown.
///
/// If an ASR provider is available (injected at construction time), transcribes
/// the audio to text. Otherwise, returns basic metadata as markdown.
///
/// The ASR provider is passed as a callback closure to avoid coupling to
/// mnemis-model. When the `with-model` feature is enabled, callers can pass
/// the mnemis-model Transcriber through this interface.
pub struct AudioConverter {
    transcribe: Option<Arc<dyn TranscribeProvider>>,
}

impl AudioConverter {
    /// Create an audio converter without ASR capabilities.
    pub fn new() -> Self {
        Self {
            transcribe: None,
        }
    }

    /// Create an audio converter with a transcription function.
    ///
    /// The closure receives `(audio_bytes, filename)` and returns the
    /// transcribed text.
    pub fn with_transcriber(provider: Arc<dyn TranscribeProvider>) -> Self {
        Self {
            transcribe: Some(provider),
        }
    }
}

impl Default for AudioConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for AudioConverter {
    fn name(&self) -> &'static str {
        "audio"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read audio input: {e}")))?;

        let filename = input.filename().unwrap_or("unknown").to_string();
        let byte_count = bytes.len();

        if let Some(ref provider) = self.transcribe {
            let transcript = provider
                .transcribe(&bytes)
                .await
                .map_err(|e| AlchemizeError::AudioUnavailable(format!("transcription failed: {e}")))?;
            let transcript = transcript.full_text();

            let markdown = if transcript.trim().is_empty() {
                format!("*Audio '{filename}' — transcription produced no text.*\n")
            } else {
                format!("## Transcript\n\n{transcript}\n")
            };

            let metadata = serde_json::json!({
                "filename": filename,
                "byte_count": byte_count,
                "transcribed": true,
                "source_format": "audio",
            });

            Ok(
                ConvertResult::with_metadata(markdown, ContentType::Audio, metadata)
                    .with_quality(ConversionQuality::LocalFull),
            )
        } else {
            let markdown = format!(
                "*Audio '{filename}' ({byte_count} bytes) — no ASR provider \
                 configured.*\n"
            );

            let metadata = serde_json::json!({
                "filename": filename,
                "byte_count": byte_count,
                "transcribed": false,
                "source_format": "audio",
            });

            Ok(
                ConvertResult::with_metadata(markdown, ContentType::Audio, metadata)
                    .with_quality(ConversionQuality::LocalFallback),
            )
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        AUDIO_EXTENSIONS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_audio_extensions() {
        let converter = AudioConverter::new();
        let exts = converter.supported_extensions();
        assert!(exts.contains(&"wav"));
        assert!(exts.contains(&"mp3"));
        assert!(exts.contains(&"flac"));
        assert!(exts.contains(&"ogg"));
    }

    #[test]
    fn can_convert_mp3() {
        let converter = AudioConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "recording.mp3".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn cannot_convert_pdf() {
        let converter = AudioConverter::new();
        let input = Input::Bytes {
            data: vec![],
            filename: "doc.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(!converter.can_convert(&input));
    }

    #[tokio::test]
    async fn returns_metadata_without_transcriber() {
        let converter = AudioConverter::new();
        let input = Input::Bytes {
            data: vec![0x52, 0x49, 0x46, 0x46],
            filename: "speech.wav".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await.unwrap();
        assert_eq!(result.content_type, ContentType::Audio);
        assert!(result.markdown.contains("speech.wav"));
        assert!(result.markdown.contains("no ASR provider"));
    }

    #[tokio::test]
    async fn uses_transcriber_when_provided() {
        struct StubTranscriber;
        #[async_trait::async_trait]
        impl crate::application::ports::TranscribeProvider for StubTranscriber {
            async fn transcribe(
                &self,
                _audio_wav16k: &[u8],
            ) -> Result<crate::application::ports::Transcript, crate::domain::error::AlchemizeError>
            {
                Ok(crate::application::ports::Transcript {
                    language: None,
                    duration: None,
                    segments: vec![crate::application::ports::TranscriptSegment {
                        start_sec: 0.0,
                        end_sec: 1.0,
                        text: "Hello world".to_string(),
                    }],
                })
            }
            fn availability(&self) -> crate::application::ports::Availability {
                crate::application::ports::Availability::Available
            }
            fn name(&self) -> &'static str {
                "stub"
            }
        }
        let converter =
            AudioConverter::with_transcriber(std::sync::Arc::new(StubTranscriber));
        let input = Input::Bytes {
            data: vec![0x52, 0x49, 0x46, 0x46],
            filename: "speech.wav".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Hello world"));
        assert!(result.markdown.contains("Transcript"));
    }

    #[test]
    fn default_creates_instance() {
        let _converter = AudioConverter::default();
    }
}
