// Copyright (c) 2026 Kirky.X🌠
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
    vision: Option<Arc<dyn VisionProvider>>,
    transcribe: Option<Arc<dyn TranscribeProvider>>,
    video_mode: VideoProcessingMode,
    _max_file_size: usize,
    moli: &Option<crate::application::builder::MoliEngineConfig>,
) -> Vec<Box<dyn Converter>> {
    let mut engines: Vec<Box<dyn Converter>> = Vec::new();

    // Text family (plain text + code/config fence).
    engines.push(Box::new(TextConverter::new()));
    // Office full-format backend (priority 15, above native parsers).
    #[cfg(feature = "anydoc")]
    engines.push(Box::new(
        crate::infrastructure::document::AnydocConverter::new(),
    ));
    // Office XML (fallback tier below anydoc).
    engines.push(Box::new(
        crate::infrastructure::document::DocxConverter::new(),
    ));
    // PDF local fallback tier.
    engines.push(Box::new(
        crate::infrastructure::document::PdfConverter::new(),
    ));
    // LaTeX source (pandoc-gated, fenced fallback).
    engines.push(Box::new(
        crate::infrastructure::document::LatexSourceConverter::new(),
    ));
    // Media family.
    let image_converter = match vision.clone() {
        Some(vision) => crate::infrastructure::media::ImageConverter::with_vision(vision),
        None => crate::infrastructure::media::ImageConverter::new(),
    };
    engines.push(Box::new(image_converter));

    let audio_converter = match transcribe.clone() {
        Some(provider) => crate::infrastructure::media::AudioConverter::with_transcriber(provider),
        None => crate::infrastructure::media::AudioConverter::new(),
    };
    engines.push(Box::new(audio_converter));

    let video_converter = match vision.clone() {
        Some(vision) => crate::infrastructure::media::VideoConverter::new(video_mode)
            .with_vision_provider(vision),
        None => crate::infrastructure::media::VideoConverter::new(video_mode),
    };
    engines.push(Box::new(video_converter));

    engines.push(Box::new(
        crate::infrastructure::media::SubtitleConverter::new(),
    ));

    // MinerU remote tier (config-gated; falls through to local on failure).
    #[cfg(feature = "mineru")]
    if let Some(cfg) = mineru {
        if let Ok(conv) = crate::infrastructure::remote::mineru::MinerUConverter::new(
            crate::infrastructure::remote::mineru::MinerUHttpConfig::from_config(cfg),
        ) {
            engines.push(Box::new(conv));
        }
    }
    engines.push(Box::new(
        crate::infrastructure::document::PptxConverter::new(),
    ));
    // Tables (csv/tsv).
    engines.push(Box::new(CsvConverter::new()));
    // Hypertext / e-book.
    engines.push(Box::new(
        crate::infrastructure::document::HtmlConverter::new(),
    ));
    engines.push(Box::new(
        crate::infrastructure::document::EpubConverter::new(),
    ));
    // Spreadsheets (xlsx/xls/ods).
    engines.push(Box::new(
        crate::infrastructure::document::XlsxConverter::new(),
    ));
    // Notebooks.
    engines.push(Box::new(
        crate::infrastructure::document::IpynbConverter::new(),
    ));

    // html-moli：crawlrs 吸收的 moli→htmd 转换链（feature moli 门控，
    // priority 15 压过 html/htmd 的 10；base_url/extract_main 显式选项）。
    #[cfg(feature = "moli")]
    if let Some(cfg) = moli {
        engines.push(Box::new(
            crate::infrastructure::document::html_moli::HtmlMoliConverter::new(
                cfg.base_url.clone(),
                cfg.extract_main,
            ),
        ));
    }

    // ZipConverter needs a registry for nested conversion: built without it
    // here (nested support wired when the registry is fully assembled — the
    // CLI layer constructs zip with a clone of the final registry).
    let _ = mineru; // wired by the MinerU migration task
    let _ = _max_file_size;
    #[cfg(not(feature = "moli"))]
    let _ = moli;
    engines
}
