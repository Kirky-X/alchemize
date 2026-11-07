// SPDX-License-Identifier: Apache-2.0
//! Engine wiring — assemble the default converter set for the builder.
//!
//! Converters are registered here as infrastructure tasks land; engines whose
//! cargo feature is off simply never appear in the returned list.

use std::sync::Arc;

use crate::application::builder::MineruConfig;
use crate::application::ports::{Converter, TranscribeProvider, VisionProvider};
use crate::domain::result::VideoProcessingMode;
use crate::infrastructure::document::{CsvConverter, TextConverter};

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
    let mut engines: Vec<Box<dyn Converter>> = Vec::new();

    // Text family (plain text + code/config fence).
    engines.push(Box::new(TextConverter::new()));
    // Office XML (fallback tier below anydoc).
    engines.push(Box::new(crate::infrastructure::document::DocxConverter::new()));
    engines.push(Box::new(crate::infrastructure::document::PptxConverter::new()));
    // Tables (csv/tsv).
    engines.push(Box::new(CsvConverter::new()));
    // Hypertext / e-book.
    engines.push(Box::new(crate::infrastructure::document::HtmlConverter::new()));
    engines.push(Box::new(crate::infrastructure::document::EpubConverter::new()));
    // Spreadsheets (xlsx/xls/ods).
    engines.push(Box::new(crate::infrastructure::document::XlsxConverter::new()));
    // Notebooks.
    engines.push(Box::new(crate::infrastructure::document::IpynbConverter::new()));

    // ZipConverter needs a registry for nested conversion: built without it
    // here (nested support wired when the registry is fully assembled — the
    // CLI layer constructs zip with a clone of the final registry).
    let _ = mineru; // wired by the MinerU migration task
    let _ = _max_file_size;
    engines
}
