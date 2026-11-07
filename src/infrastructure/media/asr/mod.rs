// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Transcription backends: vecboost local ONNX + Whisper HTTP API (both
//! behind feature `asr`).

#[cfg(feature = "asr")]
pub mod vecboost;
#[cfg(feature = "asr")]
pub mod whisper_api;
