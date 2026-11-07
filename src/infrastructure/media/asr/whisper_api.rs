// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Whisper-compatible HTTP transcription backend (feature `asr`).

#![cfg(feature = "asr")]

use async_trait::async_trait;

use crate::application::ports::{Availability, TranscribeProvider, Transcript, TranscriptSegment};
use crate::domain::error::AlchemizeError;

/// Configuration for a Whisper-compatible endpoint.
#[derive(Clone)]
pub struct WhisperApiConfig {
    /// Base URL (e.g. `https://api.openai.com`), no trailing slash.
    pub api_url: String,
    /// Bearer token (never rendered in Debug output).
    pub api_key: String,
    /// Model id.
    pub model: String,
}

impl std::fmt::Debug for WhisperApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // api_key is a secret: redacted.
        f.debug_struct("WhisperApiConfig")
            .field("api_url", &self.api_url)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl WhisperApiConfig {
    /// Create a config pointing at the given endpoint.
    pub fn new(api_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            api_key: api_key.into(),
            model: "whisper-1".to_string(),
        }
    }

    /// Override the model id.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

/// Transcriber backed by a Whisper-compatible `/v1/audio/transcriptions` API.
pub struct WhisperApiTranscriber {
    config: WhisperApiConfig,
}

impl WhisperApiTranscriber {
    /// Create with an explicit endpoint config (never auto-enabled).
    pub fn new(config: WhisperApiConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl TranscribeProvider for WhisperApiTranscriber {
    async fn transcribe(&self, audio_wav16k: &[u8]) -> Result<Transcript, AlchemizeError> {
        let url = format!("{}/v1/audio/transcriptions", self.config.api_url);
        let part = reqwest::multipart::Part::bytes(audio_wav16k.to_vec())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AlchemizeError::other(format!("mime error: {e}")))?;
        let form = reqwest::multipart::Form::new()
            .text("model", self.config.model.clone())
            .part("file", part);

        let response: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .timeout(std::time::Duration::from_secs(300))
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                AlchemizeError::AudioUnavailable(format!("whisper api request failed: {e}"))
            })?
            .error_for_status()
            .map_err(|e| AlchemizeError::AudioUnavailable(format!("whisper api status: {e}")))?
            .json()
            .await
            .map_err(|e| AlchemizeError::AudioUnavailable(format!("whisper api decode: {e}")))?;

        let text = response
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();

        Ok(Transcript {
            language: None,
            duration: None,
            segments: vec![TranscriptSegment {
                start_sec: 0.0,
                end_sec: 0.0,
                text,
            }],
        })
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }

    fn name(&self) -> &'static str {
        "whisper-api"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_api_key() {
        let config = WhisperApiConfig::new("https://api.example.com", "sk-secret-123");
        let rendered = format!("{config:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("sk-secret-123"));
        assert!(rendered.contains("https://api.example.com"));
    }

    #[test]
    fn model_override() {
        let config = WhisperApiConfig::new("u", "k").with_model("large-v3");
        assert_eq!(config.model, "large-v3");
    }
}
