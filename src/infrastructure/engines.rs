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
/// - `container_registry_slot`: deferred handle the container converters
///   (tar/tgz/gz/xz/zst/7z) dispatch nested entries through. A caller that
///   assembles its own registry must fill the slot with
///   `Arc::downgrade(&registry)` after wrapping the registry in an `Arc`
///   (the builder does this). If the slot stays empty, nested entries
///   degrade to attachments with `metadata.registry_wired = false` — the
///   archive itself still converts.
pub fn default_converters(
    mineru: &Option<MineruConfig>,
    vision: Option<Arc<dyn VisionProvider>>,
    transcribe: Option<Arc<dyn TranscribeProvider>>,
    video_mode: VideoProcessingMode,
    html_deep: &Option<crate::application::builder::HtmlDeepEngineConfig>,
    container_registry_slot: &std::sync::Arc<
        std::sync::OnceLock<std::sync::Weak<crate::application::registry::ConverterRegistry>>,
    >,
) -> Vec<Box<dyn Converter>> {
    let mut engines: Vec<Box<dyn Converter>> = vec![
        // Text family (plain text + code/config fence).
        Box::new(TextConverter::new()),
    ];
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
    #[cfg(feature = "pdf-local")]
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
    #[cfg(feature = "pdf-cloud")]
    if let Some(cfg) = mineru
        && let Ok(conv) = crate::infrastructure::remote::mineru::MinerUConverter::new(
            crate::infrastructure::remote::mineru::MinerUHttpConfig::from_config(cfg),
        )
    {
        engines.push(Box::new(conv));
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
    // Spreadsheets (xlsx/xls/xlsb/ods): unsupported. The calamine backend
    // was removed because its materialization (shared strings, styles/theme,
    // ODS repeats and text expansion) cannot be bounded by a pre-check for
    // untrusted input.
    // Notebooks.
    engines.push(Box::new(
        crate::infrastructure::document::IpynbConverter::new(),
    ));

    // Legacy Office (doc/ppt/rtf) through LibreOffice — feature gated at 15.
    #[cfg(feature = "legacy-office")]
    {
        engines.push(Box::new(
            crate::infrastructure::document::LegacyOfficeConverter::new(),
        ));
        engines.push(Box::new(
            crate::infrastructure::document::LegacyPptConverter::new(),
        ));
    }

    // Recursive archive containers — per-format features (capability-named):
    // tar / gzip / xz / zstd / sevenz. The combined `.tgz` format requires
    // both `tar` and `gzip` (cfg(all)). Nested conversion dispatches through
    // the assembled registry; the slot is filled by the builder right after
    // assembly (same post-assembly wiring as the zip facade). An empty slot
    // degrades entries to attachments (`registry_wired: false`), so direct
    // callers still work.
    #[cfg(any(
        feature = "tar",
        feature = "gzip",
        feature = "xz",
        feature = "zstd",
        feature = "sevenz"
    ))]
    {
        use crate::infrastructure::containers::RegistrySlot;
        let slot: RegistrySlot = Arc::clone(container_registry_slot);
        #[cfg(feature = "tar")]
        engines.push(Box::new(
            crate::infrastructure::containers::TarConverter::with_registry_slot(Arc::clone(&slot)),
        ));
        #[cfg(all(feature = "tar", feature = "gzip"))]
        engines.push(Box::new(
            crate::infrastructure::containers::TarGzConverter::with_registry_slot(Arc::clone(
                &slot,
            )),
        ));
        #[cfg(feature = "gzip")]
        engines.push(Box::new(
            crate::infrastructure::containers::GzConverter::with_registry_slot(Arc::clone(&slot)),
        ));
        #[cfg(feature = "xz")]
        engines.push(Box::new(
            crate::infrastructure::containers::XzConverter::with_registry_slot(Arc::clone(&slot)),
        ));
        #[cfg(feature = "zstd")]
        engines.push(Box::new(
            crate::infrastructure::containers::ZstConverter::with_registry_slot(Arc::clone(&slot)),
        ));
        #[cfg(feature = "sevenz")]
        engines.push(Box::new(
            crate::infrastructure::containers::SevenZConverter::with_registry_slot(slot),
        ));
    }
    #[cfg(not(any(
        feature = "tar",
        feature = "gzip",
        feature = "xz",
        feature = "zstd",
        feature = "sevenz"
    )))]
    let _ = container_registry_slot;

    // E-mail (eml) — feature gated.
    #[cfg(feature = "email")]
    engines.push(Box::new(
        crate::infrastructure::document::EmlConverter::new(),
    ));

    // Structured data engines — per-format features (capability-named).
    #[cfg(feature = "parquet")]
    engines.push(Box::new(
        crate::infrastructure::data::ParquetConverter::new(),
    ));
    #[cfg(feature = "sqlite")]
    engines.push(Box::new(crate::infrastructure::data::SqliteConverter::new()));
    #[cfg(feature = "bibtex")]
    engines.push(Box::new(crate::infrastructure::data::BibConverter::new()));

    // html：自研引擎主 + htmd 兜底转换链（feature html 门控，
    // priority 15 压过 html/htmd 的 10；base_url/extract_main 显式选项）。
    #[cfg(feature = "html")]
    if let Some(cfg) = html_deep {
        engines.push(Box::new(
            crate::infrastructure::document::html_deep::HtmlDeepConverter::new(
                cfg.base_url.clone(),
                cfg.extract_main,
            ),
        ));
    }

    // ZipConverter is wired at the facade level (`AlchemizeBuilder::build`):
    // it needs the fully assembled registry for nested conversion and must
    // stay out of the registry itself — otherwise a nested .zip entry would
    // recurse without bound.
    //
    // Feature-off silencing: `pdf-cloud`/`html_deep` are consumed above only
    // when their cargo feature is compiled in.
    #[cfg(not(feature = "pdf-cloud"))]
    let _ = mineru;
    #[cfg(not(feature = "html"))]
    let _ = html_deep;
    engines
}
