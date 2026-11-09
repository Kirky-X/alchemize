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
///   Input size enforcement lives at the facade (`Alchemize::convert_pref`),
///   not per-converter.
/// - `vision` / `transcribe`: optional providers; `None` degrades those
///   converters to metadata-only markdown.
pub fn default_converters(
    mineru: &Option<MineruConfig>,
    vision: Option<Arc<dyn VisionProvider>>,
    transcribe: Option<Arc<dyn TranscribeProvider>>,
    video_mode: VideoProcessingMode,
    html_deep: &Option<crate::application::builder::HtmlDeepEngineConfig>,
) -> Vec<Box<dyn Converter>> {
    let mut engines: Vec<Box<dyn Converter>> = Vec::new();

    // Text family (plain text + code/config fence).
    engines.push(Box::new(TextConverter::new()));
    // Self-hosted deep DOCX parser (priority 18): style cascade, footnotes,
    // merged cells, drawing assets. Above the native docx (10).
    engines.push(Box::new(
        crate::infrastructure::document::DocxDeepConverter::new(),
    ));
    // Self-hosted deep office parsers (priority 18): ODT/ODP/PPTX.
    engines.push(Box::new(
        crate::infrastructure::document::OdtConverter::new(),
    ));
    engines.push(Box::new(
        crate::infrastructure::document::OdpConverter::new(),
    ));
    engines.push(Box::new(
        crate::infrastructure::document::PptxDeepConverter::new(),
    ));
    // Office XML (fallback tier below the deep parsers).
    engines.push(Box::new(
        crate::infrastructure::document::DocxConverter::new(),
    ));
    // pdfium 本地高保真 PDF（spec R-self-002：PDF 链 priority 20 顶层）。
    // 动态库缺失时 availability=Unavailable，registry 跳过并记录原因，
    // 落到下方 lopdf extract_text 档。
    #[cfg(feature = "pdfium")]
    engines.push(Box::new(
        crate::infrastructure::document::PdfiumConverter::new(),
    ));
    // PDF local fallback tier.
    engines.push(Box::new(
        crate::infrastructure::document::PdfConverter::new(),
    ));
    // LaTeX source (pandoc-gated, fenced fallback).
    engines.push(Box::new(
        crate::infrastructure::document::LatexSourceConverter::new(),
    ));
    // Generic pandoc formats (org/rst/asciidoc/opml/mediawiki).
    engines.push(Box::new(
        crate::infrastructure::document::PandocGenericConverter::new(),
    ));
    // Media family.
    let image_converter = match vision.clone() {
        Some(vision) => crate::infrastructure::media::ImageConverter::with_vision(vision),
        None => crate::infrastructure::media::ImageConverter::new(),
    };
    engines.push(Box::new(image_converter));
    // HEIC/HEIF: OS decoder (sips) or metadata-only fallback.
    engines.push(Box::new(crate::infrastructure::media::HeicConverter::new()));

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

    // Legacy Office (doc/ppt/rtf) through LibreOffice — feature gated at 15.
    #[cfg(feature = "libreoffice")]
    {
        engines.push(Box::new(
            crate::infrastructure::document::LegacyOfficeConverter::new(),
        ));
        engines.push(Box::new(
            crate::infrastructure::document::LegacyPptConverter::new(),
        ));
    }

    // Recursive archive containers (tar/tgz/gz/xz/zst/7z) — feature gated.
    #[cfg(feature = "containers")]
    {
        let arc = std::sync::Arc::new(crate::application::registry::ConverterRegistry::new());
        engines.push(Box::new(
            crate::infrastructure::containers::TarConverter::with_arc(arc.clone()),
        ));
        engines.push(Box::new(
            crate::infrastructure::containers::TarGzConverter::with_arc(arc.clone()),
        ));
        engines.push(Box::new(
            crate::infrastructure::containers::GzConverter::with_arc(arc.clone()),
        ));
        engines.push(Box::new(
            crate::infrastructure::containers::XzConverter::with_arc(arc.clone()),
        ));
        engines.push(Box::new(
            crate::infrastructure::containers::ZstConverter::with_arc(arc.clone()),
        ));
        engines.push(Box::new(
            crate::infrastructure::containers::SevenZConverter::with_arc(arc.clone()),
        ));
    }

    // E-mail (eml) — feature gated.
    #[cfg(feature = "eml")]
    engines.push(Box::new(
        crate::infrastructure::document::EmlConverter::new(),
    ));

    // Structured data engines (parquet/sqlite/bib) — feature gated.
    #[cfg(feature = "data")]
    {
        engines.push(Box::new(
            crate::infrastructure::data::ParquetConverter::new(),
        ));
        engines.push(Box::new(crate::infrastructure::data::SqliteConverter::new()));
        engines.push(Box::new(crate::infrastructure::data::BibConverter::new()));
    }

    // html-deep：自研引擎主 + htmd 兜底转换链（feature html-deep 门控，
    // priority 15 压过 html/htmd 的 10；base_url/extract_main 显式选项）。
    #[cfg(feature = "html-deep")]
    if let Some(cfg) = html_deep {
        engines.push(Box::new(
            crate::infrastructure::document::html_deep::HtmlDeepConverter::new(
                cfg.base_url.clone(),
                cfg.extract_main,
            ),
        ));
    }

    // ZipConverter needs a registry for nested conversion: built without it
    // here (nested support wired when the registry is fully assembled — the
    // CLI layer constructs zip with a clone of the final registry).
    let _ = mineru; // wired by the MinerU migration task
    #[cfg(not(feature = "html-deep"))]
    let _ = html_deep;
    engines
}
