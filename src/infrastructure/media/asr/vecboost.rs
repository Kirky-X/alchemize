// SPDX-License-Identifier: Apache-2.0
//! Local transcription backend wrapping the in-house vecboost facade
//! (Qwen3-ASR ONNX three-graph + silero VAD, feature `transcribe`).

#![cfg(feature = "asr")]

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::application::ports::{Availability, Transcript, TranscribeProvider, TranscriptSegment};
use crate::domain::error::AlchemizeError;

/// Environment variable that names the transcribe model bundle directory.
pub const ENV_TRANSCRIBE_MODELS: &str = "ALCHEMIZE_TRANSCRIBE_MODELS";

/// Default probe path for the bundle (three ONNX graphs + silero VAD +
/// tokenizer), mirroring the layout documented in vecboost's CONTRACT.md.
pub fn default_models_dir() -> PathBuf {
    dirs_cache().clone()
}

fn dirs_cache() -> &'static PathBuf {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".cache/alchemize/models/transcribe")
    })
}

/// Resolve the model bundle: explicit env var first, then the default cache
/// path. Returns `None` (with reason string) when neither exists.
fn resolve_models_dir() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var(ENV_TRANSCRIBE_MODELS) {
        let path = PathBuf::from(explicit);
        if path.is_dir() {
            return Ok(path);
        }
        return Err(format!(
            "{ENV_TRANSCRIBE_MODELS}={} does not exist; expected qwen3-*.onnx + silero_vad.onnx + tokenizer/",
            path.display()
        ));
    }
    let default = default_models_dir();
    if default.is_dir() {
        return Ok(default);
    }
    Err(format!(
        "no transcribe model bundle: set {ENV_TRANSCRIBE_MODELS} or place the bundle at {} \
         (qwen3-conv_frontend.onnx, qwen3-encoder.int8.onnx, qwen3-decoder.int8.onnx, \
         silero_vad.onnx, tokenizer/) — download from the vecboost project",
        default.display()
    ))
}

/// Whether the ORT dynamic library loads (vecboost ort uses load-dynamic).
fn ort_dylib_available() -> bool {
    // vecboost reads ORT_DYLIB_PATH itself; a missing env var surfaces as an
    // error on session open, which we translate to Unavailable below. Here
    // we only pre-check the obvious absence for a clearer reason string.
    std::env::var("ORT_DYLIB_PATH").map(|v| !v.is_empty()).unwrap_or(false)
}

/// Transcriber backed by the local vecboost ONNX engine.
pub struct VecboostTranscriber {
    models_dir: PathBuf,
}

impl VecboostTranscriber {
    /// Probe the environment and construct (Availability reflects probe).
    pub fn new() -> Self {
        let models_dir = resolve_models_dir().unwrap_or_else(|_| default_models_dir());
        Self { models_dir }
    }
}

impl Default for VecboostTranscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TranscribeProvider for VecboostTranscriber {
    async fn transcribe(&self, audio_wav16k: &[u8]) -> Result<Transcript, AlchemizeError> {
        // WAV bytes land in a temp file: the vecboost facade reads 16kHz
        // mono WAV from disk (read_wav_16k_mono).
        let tmp = tempfile::NamedTempFile::new()
            .map_err(|e| AlchemizeError::AudioUnavailable(format!("tempfile failed: {e}")))?;
        std::fs::write(tmp.path(), audio_wav16k)
            .map_err(|e| AlchemizeError::AudioUnavailable(format!("wav write failed: {e}")))?;

        let opts = vecboost::transcribe::TranscribeOpts::default();
        let wav_path = tmp.path().to_path_buf();

        // ONNX inference is CPU-bound: run on the blocking pool.
        let models_dir = self.models_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let samples = vecboost::transcribe::read_wav_16k_mono(&wav_path)?;
            let session = vecboost::transcribe::open(&models_dir, &opts)?;
            let segments = session.detect_segments(&samples)?;
            let mut out = Vec::with_capacity(segments.len());
            for seg in &segments {
                let slice = &samples[seg.start_sample as usize..seg.end_sample as usize];
                let text = session.transcribe_segment(slice)?;
                out.push(TranscriptSegment {
                    start_sec: seg.start_sec,
                    end_sec: seg.end_sec,
                    text,
                });
            }
            let duration = samples.len() as f64 / vecboost::transcribe::TRANSCRIBE_SAMPLE_RATE_HZ;
            Ok::<_, vecboost::VecboostError>(Transcript {
                language: Some("auto".to_string()),
                duration: Some(duration),
                segments: out,
            })
        })
        .await
        .map_err(|e| AlchemizeError::AudioUnavailable(format!("vecboost task panicked: {e}")))?;

        result
            .map_err(|e| AlchemizeError::AudioUnavailable(format!("vecboost transcription failed: {e}")))
    }

    fn availability(&self) -> Availability {
        match resolve_models_dir() {
            Err(reason) => Availability::Unavailable { reason },
            Ok(dir) => {
                if !ort_dylib_available() {
                    return Availability::Unavailable {
                        reason: "ORT_DYLIB_PATH is not set; vecboost loads onnxruntime dynamically"
                            .to_string(),
                    };
                }
                if !dir.join("silero_vad.onnx").is_file() {
                    return Availability::Unavailable {
                        reason: format!(
                            "bundle at {} misses silero_vad.onnx; incomplete bundle",
                            dir.display()
                        ),
                    };
                }
                Availability::Available
            }
        }
    }

    fn name(&self) -> &'static str {
        "vecboost"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bundle_reports_reason_with_guidance() {
        // Force an env var pointing nowhere; availability must explain.
        unsafe { std::env::set_var(ENV_TRANSCRIBE_MODELS, "/nonexistent/alchemize-test") };
        let t = VecboostTranscriber::new();
        match t.availability() {
            Availability::Unavailable { reason } => {
                assert!(reason.contains("ALCHEMIZE_TRANSCRIBE_MODELS"));
                assert!(reason.contains("does not exist"));
            }
            Availability::Available => panic!("must be unavailable"),
        }
        unsafe { std::env::remove_var(ENV_TRANSCRIBE_MODELS) };
    }

    #[test]
    fn transcript_disclaimer_mentions_vad() {
        assert!(Transcript::DISCLAIMER.contains("VAD"));
    }

    #[tokio::test]
    #[ignore = "requires local vecboost model bundle + ORT_DYLIB_PATH"]
    async fn real_model_roundtrip() {
        let t = VecboostTranscriber::new();
        assert_eq!(t.availability(), Availability::Available);
        // Feed assets/smoke10s.wav equivalent placed next to the bundle.
        let path = std::env::var("ALCHEMIZE_ASR_TEST_WAV").expect("set ALCHEMIZE_ASR_TEST_WAV");
        let wav = std::fs::read(path).unwrap();
        let transcript = t.transcribe(&wav).await.unwrap();
        assert!(!transcript.segments.is_empty());
        assert!(transcript.duration.unwrap() > 0.0);
    }
}
