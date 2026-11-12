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
#[cfg(feature = "csv")]
use crate::infrastructure::document::CsvConverter;
#[cfg(feature = "text")]
use crate::infrastructure::document::TextConverter;

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
    // Text family (plain text + code/config fence) seeds the list; builds
    // without it start empty (clippy::vec_init_then_push cannot see the cfg).
    #[cfg(feature = "text")]
    #[allow(unused_mut)]
    let mut engines: Vec<Box<dyn Converter>> = vec![Box::new(TextConverter::new())];
    #[cfg(not(feature = "text"))]
    #[allow(unused_mut)]
    let mut engines: Vec<Box<dyn Converter>> = Vec::new();
    // Self-hosted deep DOCX parser (priority 18): style cascade, footnotes,
    // merged cells, drawing assets. Above the native docx (10).
    #[cfg(feature = "docx")]
    engines.push(Box::new(
        crate::infrastructure::document::DocxDeepConverter::new(),
    ));
    // Self-hosted deep office parsers (priority 18): ODT/ODP/PPTX.
    #[cfg(feature = "odt")]
    engines.push(Box::new(
        crate::infrastructure::document::OdtConverter::new(),
    ));
    #[cfg(feature = "odp")]
    engines.push(Box::new(
        crate::infrastructure::document::OdpConverter::new(),
    ));
    #[cfg(feature = "pptx")]
    engines.push(Box::new(
        crate::infrastructure::document::PptxDeepConverter::new(),
    ));
    // Office XML (fallback tier below the deep parsers).
    #[cfg(feature = "docx")]
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
    #[cfg(feature = "pdf")]
    engines.push(Box::new(
        crate::infrastructure::document::PdfConverter::new(),
    ));
    // LaTeX source (pandoc-gated, fenced fallback).
    #[cfg(feature = "latex")]
    engines.push(Box::new(
        crate::infrastructure::document::LatexSourceConverter::new(),
    ));
    // Generic pandoc formats (org/rst/asciidoc/opml/mediawiki).
    #[cfg(feature = "pandoc")]
    engines.push(Box::new(
        crate::infrastructure::document::PandocGenericConverter::new(),
    ));
    // Media family: image / audio / video / subtitle.
    #[cfg(feature = "media")]
    {
        let image_converter = match vision.clone() {
            Some(vision) => crate::infrastructure::media::ImageConverter::with_vision(vision),
            None => crate::infrastructure::media::ImageConverter::new(),
        };
        engines.push(Box::new(image_converter));

        let audio_converter = match transcribe.clone() {
            Some(provider) => {
                crate::infrastructure::media::AudioConverter::with_transcriber(provider)
            }
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
    }
    // HEIC/HEIF: OS decoder (sips) or metadata-only fallback.
    #[cfg(feature = "heic")]
    engines.push(Box::new(crate::infrastructure::media::HeicConverter::new()));

    // MinerU remote tier (config-gated; falls through to local on failure).
    #[cfg(feature = "pdf-cloud")]
    if let Some(cfg) = mineru
        && let Ok(conv) = crate::infrastructure::remote::mineru::MinerUConverter::new(
            crate::infrastructure::remote::mineru::MinerUHttpConfig::from_config(cfg),
        )
    {
        engines.push(Box::new(conv));
    }
    #[cfg(feature = "pptx")]
    engines.push(Box::new(
        crate::infrastructure::document::PptxConverter::new(),
    ));
    // Tables (csv/tsv).
    #[cfg(feature = "csv")]
    engines.push(Box::new(CsvConverter::new()));
    // Hypertext / e-book.
    #[cfg(feature = "html")]
    engines.push(Box::new(
        crate::infrastructure::document::HtmlConverter::new(),
    ));
    #[cfg(feature = "epub")]
    engines.push(Box::new(
        crate::infrastructure::document::EpubConverter::new(),
    ));
    // Spreadsheets (xlsx/xls/xlsb/ods): unsupported. The calamine backend
    // was removed because its materialization (shared strings, styles/theme,
    // ODS repeats and text expansion) cannot be bounded by a pre-check for
    // untrusted input.
    // Notebooks.
    #[cfg(feature = "ipynb")]
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
    // Feature-off silencing: parameters are consumed above only when their
    // cargo feature is compiled in.
    #[cfg(not(feature = "pdf-cloud"))]
    let _ = mineru;
    #[cfg(not(feature = "html"))]
    let _ = html_deep;
    #[cfg(not(feature = "media"))]
    let _ = (vision, transcribe, video_mode);
    engines
}

#[cfg(test)]
mod feature_registration_tests {
    use super::*;

    /// Assemble the default engine list the way the builder does: the two
    /// config-driven engines (MinerU / html-deep) receive default configs so
    /// their registration gates are exercised rather than skipped.
    fn engine_names() -> Vec<&'static str> {
        let slot = std::sync::Arc::new(std::sync::OnceLock::new());
        let mineru = Some(MineruConfig::default());
        let html_deep = Some(crate::application::builder::HtmlDeepEngineConfig::default());
        default_converters(
            &mineru,
            None,
            None,
            VideoProcessingMode::default(),
            &html_deep,
            &slot,
        )
        .iter()
        .map(|c| c.name())
        .collect()
    }

    /// 每个能力 feature 的注册在场性：feature 开 → 引擎必须在场；feature 关 →
    /// 引擎必须缺席。把「cfg 漏改导致的静默丢失/越界注册」变成测试失败。
    #[test]
    fn feature_gates_register_exactly_the_enabled_engines() {
        let names = engine_names();

        macro_rules! assert_gated {
            ($feature:literal, $engine:literal) => {
                #[cfg(feature = $feature)]
                assert!(
                    names.contains(&$engine),
                    "feature '{}' is on but engine '{}' is not registered",
                    $feature,
                    $engine
                );
                #[cfg(not(feature = $feature))]
                assert!(
                    !names.contains(&$engine),
                    "feature '{}' is off but engine '{}' is registered",
                    $feature,
                    $engine
                );
            };
        }

        // 格式族
        assert_gated!("text", "text");
        assert_gated!("csv", "csv");
        assert_gated!("html", "html");
        assert_gated!("html", "html-deep");
        assert_gated!("pdf", "pdf");
        assert_gated!("pdf-local", "pdfium");
        assert_gated!("docx", "docx");
        assert_gated!("docx", "docx-deep");
        assert_gated!("pptx", "pptx");
        assert_gated!("pptx", "pptx-deep");
        assert_gated!("odt", "odt-deep");
        assert_gated!("odp", "odp-deep");
        assert_gated!("epub", "epub");
        assert_gated!("ipynb", "ipynb");
        assert_gated!("latex", "latex-source");
        assert_gated!("pandoc", "pandoc-generic");
        assert_gated!("media", "image");
        assert_gated!("media", "audio");
        assert_gated!("media", "video");
        assert_gated!("media", "subtitle");
        assert_gated!("heic", "heic");
        // 后端 / 数据 / 容器
        assert_gated!("pdf-cloud", "mineru");
        assert_gated!("legacy-office", "legacy-office");
        assert_gated!("legacy-office", "legacy-ppt");
        assert_gated!("email", "eml");
        assert_gated!("tar", "tar");
        assert_gated!("gzip", "gz");
        assert_gated!("xz", "xz");
        assert_gated!("zstd", "zst");
        assert_gated!("sevenz", "7z");
        assert_gated!("parquet", "parquet");
        assert_gated!("sqlite", "sqlite");
        assert_gated!("bibtex", "bib");

        // --- feature_map binding & completeness ---------------------------
        // (1) binding — every build: for each table entry whose features are
        // all enabled here, a converter that this build actually registered
        // (or the facade zip converter) must accept the extension. A wrong
        // feature attribution therefore fails in that feature's own cell.
        let exts_of_this_build: Vec<String> = {
            let slot = std::sync::Arc::new(std::sync::OnceLock::new());
            let mineru = Some(MineruConfig::default());
            let html_deep = Some(crate::application::builder::HtmlDeepEngineConfig::default());
            let converters = default_converters(
                &mineru,
                None,
                None,
                VideoProcessingMode::default(),
                &html_deep,
                &slot,
            );
            converters
                .iter()
                .flat_map(|c| c.supported_extensions().iter().map(|s| s.to_string()))
                .collect()
        };
        let enabled = crate::domain::feature_map::enabled_features();
        for (ext, required) in crate::domain::feature_map::EXTENSION_FEATURES {
            if !required.iter().all(|f| enabled.contains(f)) {
                continue;
            }
            if required.contains(&"zip") {
                // Facade-level converter, not a registry member: check the
                // extension against the facade's own list, not a name.
                #[cfg(feature = "zip")]
                {
                    let zip = crate::infrastructure::document::ZipConverter::with_arc(
                        std::sync::Arc::new(crate::application::registry::ConverterRegistry::new()),
                    );
                    assert!(
                        zip.supported_extensions().contains(ext),
                        "feature_map says '{ext}' is served by the zip facade, but it is not"
                    );
                }
                #[cfg(not(feature = "zip"))]
                assert!(
                    crate::domain::feature_map::missing_features(ext).is_some(),
                    "zip feature is off but '{ext}' reports no missing feature"
                );
                continue;
            }
            assert!(
                exts_of_this_build.iter().any(|e| e == ext),
                "feature_map says '{ext}' needs {required:?}, all of which are enabled, \
                 but no registered converter accepts it"
            );
        }

        // Zip-off direction: the loop above skips entries whose features are
        // not all enabled, so assert the refusal directly.
        #[cfg(not(feature = "zip"))]
        assert!(
            crate::domain::feature_map::missing_features("zip").is_some(),
            "zip feature is off but .zip reports no missing feature"
        );

        // (2) completeness + (3) feature-set — verified under `full`, where
        // every converter is registered.
        #[cfg(feature = "full")]
        {
            // (2a) every converter-declared extension is in the table.
            for ext in &exts_of_this_build {
                assert!(
                    crate::domain::feature_map::features_for_extension(ext).is_some(),
                    "converter declares '{ext}' but the feature map does not list it"
                );
            }
            // (2b) the facade zip converter's extensions are in the table too.
            {
                let zip = crate::infrastructure::document::ZipConverter::with_arc(
                    std::sync::Arc::new(crate::application::registry::ConverterRegistry::new()),
                );
                for ext in zip.supported_extensions() {
                    assert!(
                        crate::domain::feature_map::features_for_extension(ext).is_some(),
                        "zip facade declares '{ext}' but the feature map does not list it"
                    );
                }
            }
            // (3) enabled_features == Cargo.toml `full`; every table feature
            // is a conversion feature (i.e. a member of `full`); every other
            // declared feature is either `full` itself or one of the two
            // non-conversion tool faces.
            let cargo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
                .expect("read Cargo.toml");
            let full: std::collections::BTreeSet<&str> =
                parse_cargo_list(&cargo, "full").into_iter().collect();
            let listed: std::collections::BTreeSet<&str> = enabled.into_iter().collect();
            assert_eq!(
                listed, full,
                "enabled_features() and Cargo.toml `full` must be the same set"
            );
            for (ext, required) in crate::domain::feature_map::EXTENSION_FEATURES {
                for f in *required {
                    assert!(
                        full.contains(f),
                        "feature_map entry '{ext}' names '{f}', which is not a conversion feature in `full`"
                    );
                }
            }
            // The CI isolation loop is a hand-written list; keep it equal to
            // `full` (+ the tar,gzip combination), so a forgotten cell fails
            // here instead of never being isolated.
            let ci = std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/.github/workflows/ci.yml"
            ))
            .expect("read ci.yml");
            let loop_set: std::collections::BTreeSet<&str> =
                parse_ci_loop(&ci).into_iter().collect();
            let mut expected = full.clone();
            expected.insert("tar,gzip");
            assert_eq!(
                loop_set, expected,
                "CI feature-isolation loop must list exactly `full` plus tar,gzip"
            );
            // Every declared feature must be a conversion feature, `full`
            // itself, `default`, or a non-conversion tool face — so adding a
            // Cargo feature without wiring it into `full`/enabled_features
            // cannot ship unguarded. (`kit` is the trait-kit integration
            // gate — adapter modules only, no converter surface.)
            let tool_faces = ["default", "cli", "mcp", "full", "kit"];
            for key in parse_cargo_feature_keys(&cargo) {
                assert!(
                    full.contains(key) || tool_faces.contains(&key),
                    "Cargo.toml declares feature '{key}' outside `full` and outside the tool faces"
                );
            }
        }

        // `.tgz` is a combined capability: present only with both features.
        #[cfg(all(feature = "tar", feature = "gzip"))]
        assert!(names.contains(&"tar-gz"), "tar+gzip: tar-gz must register");
        #[cfg(not(all(feature = "tar", feature = "gzip")))]
        assert!(
            !names.contains(&"tar-gz"),
            "tar-gz must stay unregistered unless both tar and gzip are on"
        );
    }

    /// Extract a quoted string list (e.g. `full = ["a", "b"]`) from Cargo.toml.
    #[cfg(feature = "full")]
    fn parse_cargo_list<'a>(cargo: &'a str, key: &str) -> Vec<&'a str> {
        let marker = format!("{key} = [");
        let start = cargo
            .find(&marker)
            .unwrap_or_else(|| panic!("Cargo.toml has no `{key}` list"))
            + marker.len();
        let rest = &cargo[start..];
        let end = rest.find(']').expect("unterminated list");
        rest[..end]
            .split(',')
            .filter_map(|item| {
                let item = item.trim();
                item.strip_prefix('"')?.strip_suffix('"')
            })
            .collect()
    }

    /// Extract the feature tokens of the CI isolation loop
    /// (`for f in ... ; do`), stripping shell line continuations.
    #[cfg(feature = "full")]
    fn parse_ci_loop(ci: &str) -> Vec<&str> {
        let start =
            ci.find("for f in ").expect("ci.yml has the isolation loop") + "for f in ".len();
        let rest = &ci[start..];
        let end = rest.find("; do").expect("loop terminator");
        rest[..end]
            .split_whitespace()
            .map(|tok| tok.trim_matches('"').trim_end_matches('\\'))
            .filter(|tok| !tok.is_empty())
            .collect()
    }

    /// Extract the keys of Cargo.toml's `[features]` table.
    #[cfg(feature = "full")]
    fn parse_cargo_feature_keys(cargo: &str) -> Vec<&str> {
        let section = cargo
            .split("[features]")
            .nth(1)
            .expect("Cargo.toml has a [features] section");
        let mut keys = Vec::new();
        for line in section.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                break; // end of [features]; later sections list deps, not features
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, _)) = line.split_once(" = ") {
                keys.push(key.trim());
            }
        }
        keys
    }
}
