// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Application layer — orchestration, use cases, and ports.

pub mod builder;
pub mod ports;
pub mod registry;

pub use builder::{Alchemize, AlchemizeBuilder, AssetMode, MineruConfig};
pub use ports::{
    Availability, Converter, ExternalTool, TranscribeProvider, Transcript, TranscriptSegment,
    VisionProvider,
};
pub use registry::ConverterRegistry;
