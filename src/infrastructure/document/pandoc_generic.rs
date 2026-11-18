// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Generic pandoc-backed converter: org / rst / asciidoc / opml / mediawiki
//! Without pandoc on PATH the source degrades to a fenced code block
//! so content is never lost.

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter, EngineProvenanceDescriptor};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::provenance::{ExternalToolKind, ProvenanceEnvelope, SanitizationSummary};
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};
use crate::infrastructure::document::shared::fence_for;
use crate::infrastructure::tools::pandoc::PandocTool;

/// Extension → pandoc input format.
const FORMATS: &[(&str, &str)] = &[
    ("org", "org"),
    ("rst", "rst"),
    ("adoc", "asciidoc"),
    ("asciidoc", "asciidoc"),
    ("opml", "opml"),
    ("mediawiki", "mediawiki"),
    ("wiki", "mediawiki"),
    ("textile", "textile"),
];

/// Pandoc-backed generic converter.
pub struct PandocGenericConverter;

impl PandocGenericConverter {
    /// Create a new converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PandocGenericConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Pandoc format name for an extension.
fn pandoc_format(ext: &str) -> Option<&'static str> {
    FORMATS
        .iter()
        .find(|(e, _)| e.eq_ignore_ascii_case(ext))
        .map(|(_, f)| *f)
}

#[async_trait]
impl Converter for PandocGenericConverter {
    fn name(&self) -> &'static str {
        "pandoc-generic"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let ext = input.extension().unwrap_or_default();
        let format = pandoc_format(&ext)
            .ok_or_else(|| AlchemizeError::UnsupportedFormat(format!("pandoc-generic: .{ext}")))?;
        let bytes = input.to_bytes().await.map_err(|e| {
            AlchemizeError::other(crate::tr_args(
                "failed-to-read-pandoc-input",
                &[("ext", ext.to_string()), ("source", e.to_string())],
            ))
        })?;
        let (source, encoding) = crate::infrastructure::document::detect_encoding(&bytes);

        match PandocTool::probe() {
            Availability::Available => {
                let raw = PandocTool::to_gfm(format, &source, None)
                    .await
                    .map_err(|e| {
                        AlchemizeError::other(crate::tr_args(
                            "pandoc-failed",
                            &[("source", e.to_string())],
                        ))
                    })?;
                let metadata = serde_json::json!({
                    "source_format": ext,
                    "engine": "pandoc",
                    "pandoc_format": format,
                    "encoding": encoding,
                });
                Ok(
                    ConvertResult::with_metadata(raw, ContentType::Document, metadata)
                        .with_quality(ConversionQuality::LocalFull),
                )
            }
            Availability::Unavailable { reason } => {
                // Degrade: keep the source verbatim in a fenced block.
                let body = source.trim_end();
                let fence = fence_for(body);
                let markdown = format!("{fence}{format}\n{body}\n{fence}\n");
                let metadata = serde_json::json!({
                    "source_format": ext,
                    "engine": "fallback",
                    "pandoc_unavailable": reason,
                    "encoding": encoding,
                });
                // The fence reinforcement is an engine-applied output
                // transform: declare it on the result (the registry merges
                // it into the envelope).
                Ok(
                    ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                        .with_quality(ConversionQuality::LocalFallback)
                        .with_provenance(ProvenanceEnvelope {
                            sanitization: SanitizationSummary {
                                fence_reinforced: true,
                                ..SanitizationSummary::default()
                            },
                            ..ProvenanceEnvelope::default()
                        }),
                )
            }
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        // Static list mirroring FORMATS (trait requires &'static ct).
        &[
            "org",
            "rst",
            "adoc",
            "asciidoc",
            "opml",
            "mediawiki",
            "wiki",
            "textile",
        ]
    }

    fn priority(&self) -> u8 {
        // Above TextConverter (10): pandoc renders real markup instead of
        // passing the source through as plain text.
        12
    }

    fn availability(&self) -> Availability {
        // Always converts: without pandoc it degrades to a fenced block.
        Availability::Available
    }

    /// Drives the pandoc child process when available (the degraded fence
    /// fallback path runs no subprocess).
    fn provenance_descriptor(&self) -> EngineProvenanceDescriptor {
        EngineProvenanceDescriptor {
            external_tools: vec![ExternalToolKind::Pandoc],
            ..EngineProvenanceDescriptor::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_map_to_pandoc_names() {
        assert_eq!(pandoc_format("org"), Some("org"));
        assert_eq!(pandoc_format("rst"), Some("rst"));
        assert_eq!(pandoc_format("adoc"), Some("asciidoc"));
        assert_eq!(pandoc_format("wiki"), Some("mediawiki"));
        assert_eq!(pandoc_format("docx"), None);
    }

    #[test]
    fn contract() {
        let c = PandocGenericConverter::new();
        assert_eq!(c.name(), "pandoc-generic");
        assert_eq!(c.availability(), Availability::Available);
        assert!(c.supported_extensions().contains(&"org"));
    }

    #[test]
    fn provenance_descriptor_declares_pandoc() {
        let d = PandocGenericConverter::new().provenance_descriptor();
        assert_eq!(d.external_tools, vec![ExternalToolKind::Pandoc]);
        assert!(d.network_egress.is_none());
        assert!(d.model_bundles.is_empty());
    }

    #[tokio::test]
    async fn asciidoc_converts_through_pandoc() {
        let adoc = "= Title\n\nSome *bold* text.\n\n== Section\n\nBody.\n";
        let result = PandocGenericConverter::new()
            .convert(Input::bytes(adoc.as_bytes().to_vec(), "doc.adoc"))
            .await
            .unwrap();
        if result.metadata["engine"] == "pandoc" {
            // pandoc's asciidoc reader puts `= Title` into document metadata,
            // so the body assertion targets the section + inline markup.
            assert!(result.markdown.contains("Section"), "{}", result.markdown);
            assert!(result.markdown.contains("**bold**"), "{}", result.markdown);
        } else {
            // No pandoc: the fenced fallback keeps the source intact.
            assert!(
                result.markdown.starts_with("````asciidoc"),
                "{}",
                result.markdown
            );
            assert!(result.markdown.contains("*bold*"));
        }
    }

    #[tokio::test]
    async fn org_mode_converts() {
        let org = "* Heading\n\nSome /italic/ text with a [[https://x.test][link]].\n";
        let result = PandocGenericConverter::new()
            .convert(Input::bytes(org.as_bytes().to_vec(), "notes.org"))
            .await
            .unwrap();
        assert!(result.markdown.contains("Heading"), "{}", result.markdown);
        if result.metadata["engine"] == "pandoc" {
            assert!(
                result.markdown.contains("https://x.test"),
                "{}",
                result.markdown
            );
        }
    }

    #[tokio::test]
    async fn unsupported_extension_errors() {
        let err = PandocGenericConverter::new()
            .convert(Input::bytes(b"x".to_vec(), "a.docx"))
            .await;
        assert!(err.is_err());
    }
}
