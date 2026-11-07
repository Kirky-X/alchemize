// SPDX-License-Identifier: Apache-2.0
//! Engine wiring — assemble the default converter set for the builder.
//!
//! Converters are registered here as infrastructure tasks land; engines whose
//! cargo feature is off simply never appear in the returned list.

use std::sync::Arc;

use crate::application::builder::MineruConfig;
use crate::application::ports::{Converter, TranscribeProvider, VisionProvider};
use crate::domain::result::VideoProcessingMode;

/// Assemble the default engine list from configuration and feature flags.
///
/// - `mineru`: MinerU remote engines; `None` (or feature off) omits them.
/// - `vision` / `transcribe`: optional providers; `None` degrades those
///   converters to metadata-only markdown.
pub fn default_converters(
    mineru: &Option<MineruConfig>,
    _vision: Option<Arc<dyn VisionProvider>>,
    _transcribe: Option<Arc<dyn TranscribeProvider>>,
    _video_mode: VideoProcessingMode,
    _max_file_size: usize,
) -> Vec<Box<dyn Converter>> {
    let engines: Vec<Box<dyn Converter>> = Vec::new();
    let _ = mineru; // wired by the MinerU migration task
    engines
}
