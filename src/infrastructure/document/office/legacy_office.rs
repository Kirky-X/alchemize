// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Legacy Office engine (feature `libreoffice`): `.doc`/`.ppt`/`.rtf` through
//! a headless LibreOffice round-trip.
//!
//! Strategy: convert the legacy format to its OOXML equivalent (`.docx` /
//! `.pptx`) and hand the bytes to the self-hosted deep parser, so legacy
//! documents get the same style/footnote/table fidelity as native OOXML.
//! RTF converts straight to markdown when the installed LibreOffice offers a
//! Markdown filter; otherwise it goes through the same `.docx` hop.

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};
use crate::infrastructure::tools::libreoffice::LibreOfficeTool;

/// Legacy Word/RTF converter (`doc`, `rtf`).
pub struct LegacyOfficeConverter;

impl LegacyOfficeConverter {
    /// Create a new legacy-office converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LegacyOfficeConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Format hop chosen for one source extension.
struct Hop {
    /// Extension soffice is asked to produce.
    target: &'static str,
    /// LibreOffice filter name for that target.
    filter: &'static str,
}

fn hop_for(ext: &str) -> Hop {
    match ext {
        // Word 97-2003 and RTF both round-trip through WordprocessingML.
        _ => Hop {
            target: "docx",
            filter: "Office Open XML Text",
        },
    }
}

#[async_trait]
impl Converter for LegacyOfficeConverter {
    fn name(&self) -> &'static str {
        "legacy-office"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let ext = input.extension().unwrap_or_default();
        let hop = hop_for(&ext);
        let filename = input.filename().unwrap_or("input.bin").to_string();
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read {ext} input: {e}")))?;

        let work = tempfile::Builder::new()
            .prefix("alch-legacy-")
            .tempdir()
            .map_err(|e| AlchemizeError::other(format!("tempdir failed: {e}")))?;
        let input_path = work.path().join(&filename);
        std::fs::write(&input_path, &bytes)
            .map_err(|e| AlchemizeError::other(format!("staging failed: {e}")))?;

        let runner = crate::infrastructure::tools::libreoffice::LibreOfficeRunner::new()
            .map_err(|e| AlchemizeError::other(format!("soffice profile failed: {e}")))?;
        let out_dir = work.path().join("out");
        let produced = runner
            .convert(&input_path, hop.target, hop.filter, &out_dir)
            .await
            .map_err(|e| AlchemizeError::other(format!("soffice conversion failed: {e}")))?;
        let converted = std::fs::read(&produced)
            .map_err(|e| AlchemizeError::other(format!("converted output unreadable: {e}")))?;

        // Second hop: self-hosted deep parsers handle the OOXML.
        let result = match hop.target {
            "docx" => {
                crate::infrastructure::document::office::docx::converter::DocxDeepConverter::new()
                    .convert(Input::bytes(converted, format!("{}.docx", stem(&filename))))
                    .await?
            }
            other => {
                return Err(AlchemizeError::other(format!(
                    "unsupported intermediate format '{other}'"
                )));
            }
        };

        let mut out = ConvertResult::with_metadata(
            result.markdown,
            ContentType::Document,
            serde_json::json!({
                "source_format": ext,
                "engine": "libreoffice",
                "intermediate": hop.target,
                "asset_count": result.assets.len(),
            }),
        )
        .with_quality(ConversionQuality::LocalFull);
        let mut out2 = out;
        for asset in result.assets {
            out2 = out2.with_asset(asset);
        }
        out = out2;
        Ok(out)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["doc", "rtf"]
    }

    fn priority(&self) -> u8 {
        15
    }

    fn availability(&self) -> Availability {
        if LibreOfficeTool::on_path() {
            Availability::Available
        } else {
            Availability::Unavailable {
                reason: "'soffice' not found on PATH (legacy doc/rtf need LibreOffice)".to_string(),
            }
        }
    }
}

/// File stem without its extension (`a/b.doc` → `a/b` → `b`).
fn stem(filename: &str) -> String {
    let base = filename.rsplit('/').next().unwrap_or(filename);
    base.rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| base.to_string())
}

/// Legacy PowerPoint converter (`ppt` → `pptx` → deep parser).
pub struct LegacyPptConverter;

impl LegacyPptConverter {
    /// Create a new legacy-PowerPoint converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LegacyPptConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for LegacyPptConverter {
    fn name(&self) -> &'static str {
        "legacy-ppt"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let filename = input.filename().unwrap_or("input.ppt").to_string();
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read ppt input: {e}")))?;

        let work = tempfile::Builder::new()
            .prefix("alch-legacy-ppt-")
            .tempdir()
            .map_err(|e| AlchemizeError::other(format!("tempdir failed: {e}")))?;
        let input_path = work.path().join(&filename);
        std::fs::write(&input_path, &bytes)
            .map_err(|e| AlchemizeError::other(format!("staging failed: {e}")))?;

        let runner = crate::infrastructure::tools::libreoffice::LibreOfficeRunner::new()
            .map_err(|e| AlchemizeError::other(format!("soffice profile failed: {e}")))?;
        let out_dir = work.path().join("out");
        let produced = runner
            .convert(
                &input_path,
                "pptx",
                "Office Open XML Presentation",
                &out_dir,
            )
            .await
            .map_err(|e| AlchemizeError::other(format!("soffice conversion failed: {e}")))?;
        let converted = std::fs::read(&produced)
            .map_err(|e| AlchemizeError::other(format!("converted output unreadable: {e}")))?;

        let result = crate::infrastructure::document::office::pptx_deep::PptxDeepConverter::new()
            .convert(Input::bytes(converted, format!("{}.pptx", stem(&filename))))
            .await?;

        let mut out = ConvertResult::with_metadata(
            result.markdown,
            ContentType::Document,
            serde_json::json!({
                "source_format": "ppt",
                "engine": "libreoffice",
                "intermediate": "pptx",
                "asset_count": result.assets.len(),
            }),
        )
        .with_quality(ConversionQuality::LocalFull);
        for asset in result.assets {
            out = out.with_asset(asset);
        }
        Ok(out)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["ppt", "pps", "pot"]
    }

    fn priority(&self) -> u8 {
        15
    }

    fn availability(&self) -> Availability {
        if LibreOfficeTool::on_path() {
            Availability::Available
        } else {
            Availability::Unavailable {
                reason: "'soffice' not found on PATH (legacy ppt needs LibreOffice)".to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_converter_contract() {
        let c = LegacyOfficeConverter::new();
        assert_eq!(c.name(), "legacy-office");
        assert_eq!(c.supported_extensions(), &["doc", "rtf"]);
        assert_eq!(c.priority(), 15);
        let p = LegacyPptConverter::new();
        assert_eq!(p.supported_extensions(), &["ppt", "pps", "pot"]);
    }

    #[test]
    fn availability_degrades_without_soffice() {
        let _serial = crate::test_support::env_lock();
        let saved = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", "/nonexistent-dir") };
        match LegacyOfficeConverter::new().availability() {
            Availability::Unavailable { reason } => {
                assert!(reason.contains("soffice"), "{reason}");
            }
            Availability::Available => panic!("must be unavailable with empty PATH"),
        }
        if let Some(saved) = saved {
            unsafe { std::env::set_var("PATH", saved) };
        }
    }

    #[test]
    fn stem_strips_directory_and_extension() {
        assert_eq!(stem("a/b/report.doc"), "report");
        assert_eq!(stem("report.rtf"), "report");
        assert_eq!(stem("noext"), "noext");
    }

    #[tokio::test]
    async fn missing_soffice_errors_instead_of_silently_degrading() {
        // availability() reports Unavailable, so the registry skips this
        // engine; a direct convert() call must still fail loudly.
        let _serial = crate::test_support::env_lock();
        let saved = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", "/nonexistent-dir") };
        let err = LegacyOfficeConverter::new()
            .convert(Input::bytes(b"{\\rtf1}".to_vec(), "x.rtf"))
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("soffice"), "err: {msg}");
        if let Some(saved) = saved {
            unsafe { std::env::set_var("PATH", saved) };
        }
    }

    #[tokio::test]
    #[ignore = "requires a real soffice binary on PATH"]
    async fn real_soffice_converts_rtf_to_markdown() {
        let rtf = br"{\rtf1\ansi\deff0 {\fonttbl {\f0 Times;}}\f0\fs24 Legacy RTF body\par}";
        let result = LegacyOfficeConverter::new()
            .convert(Input::bytes(rtf.to_vec(), "sample.rtf"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("Legacy RTF body"),
            "md: {}",
            result.markdown
        );
    }
}
