//! Transcription backends: vecboost local ONNX + Whisper HTTP API (both
//! behind feature `asr`).

#[cfg(feature = "asr")]
pub mod vecboost;
#[cfg(feature = "asr")]
pub mod whisper_api;
