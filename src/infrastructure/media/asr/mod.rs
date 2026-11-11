// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Transcription backends: vecboost local ONNX + Whisper HTTP API (both
//! behind feature `audio`).

#[cfg(feature = "audio")]
pub mod vecboost;
#[cfg(feature = "audio")]
pub mod whisper_api;
