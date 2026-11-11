// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Alchemize error types.
//!
//! Display messages are user-facing and therefore internationalized:
//! every variant renders via [`crate::tr_args`] against the Fluent
//! catalog (`locales/{en,zh}/messages.ftl`). Keys are the English source
//! text, so the en bundle renders byte-identical to the pre-i18n output.

use std::fmt;
use std::path::PathBuf;

/// Errors produced by the alchemize crate.
#[derive(Debug, thiserror::Error)]
pub enum AlchemizeError {
    /// The input format is not supported by any registered converter.
    UnsupportedFormat(String),

    /// Recursive container nesting exceeded the configured depth limit.
    DepthLimitExceeded {
        /// Maximum nesting depth allowed per conversion chain.
        limit: u32,
    },

    /// No converter accepted the input.
    NoConverterAccepted,

    /// All backends in the fallback chain failed, with per-engine attempts.
    AllBackendsFailed {
        /// File extension of the failing input.
        ext: String,
        /// Ordered (engine name, failure reason) pairs, one per attempted backend.
        attempts: Vec<(String, String)>,
    },

    /// Failed to read the input file.
    FileRead {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Failed to extract text from PDF.
    Pdf(String),

    /// Failed to parse a ZIP-based format (PPTX/DOCX/ODF).
    Zip(String),

    /// Failed to parse XML content within a ZIP archive.
    Xml(String),

    /// Video processing is not available (no ffmpeg or disabled).
    VideoUnavailable(String),

    /// Audio transcription is not available.
    AudioUnavailable(String),

    /// Vision provider (OCR/image analysis) is not available.
    VisionUnavailable(String),

    /// MinerU service is unavailable (connection refused, timeout, etc.).
    MineruUnavailable(String),

    /// MinerU API returned an error or conversion failed.
    MineruConversion(String),

    /// Security violation detected (SSRF, invalid URL, etc.).
    SecurityViolation {
        kind: SecurityViolationType,
        message: String,
    },

    /// HTTP response body exceeded the configured size limit.
    ResponseTooLarge { actual: u64, limit: u64 },

    /// A generic error from a converter implementation.
    Other(String),

    /// The synchronous conversion port cannot serve this request (path
    /// input, or no sync-capable engine for the format).
    NotSyncCapable(String),
}

/// Security violation types for URL validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecurityViolationType {
    /// URL scheme is not HTTPS.
    InvalidScheme,
    /// URL points to loopback address (127.0.0.0/8, ::1, localhost).
    LoopbackAddress,
    /// URL points to private IPv4 range (RFC1918: 10/8, 172.16/12, 192.168/16).
    PrivateAddress,
    /// URL points to link-local address (169.254/16).
    LinkLocalAddress,
    /// URL hostname is not in the allowlist.
    HostnameNotAllowed,
    /// URL is missing required presigned signature.
    MissingSignature,
}

impl AlchemizeError {
    /// Create a file read error.
    pub fn file_read(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::FileRead {
            path: path.into(),
            source,
        }
    }

    /// Convenience constructor for a generic error from any Display source.
    pub fn other(msg: impl std::fmt::Display) -> Self {
        Self::Other(msg.to_string())
    }

    /// Stable, non-sensitive variant label for logs and metrics.
    ///
    /// Converter errors can embed remote response bodies or internal
    /// endpoints; logs must use this instead of `Display`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnsupportedFormat(_) => "unsupported_format",
            Self::DepthLimitExceeded { .. } => "depth_limit_exceeded",
            Self::NoConverterAccepted => "no_converter_accepted",
            Self::AllBackendsFailed { .. } => "all_backends_failed",
            Self::FileRead { .. } => "file_read",
            Self::Pdf(_) => "pdf",
            Self::Zip(_) => "zip",
            Self::Xml(_) => "xml",
            Self::VideoUnavailable(_) => "video_unavailable",
            Self::AudioUnavailable(_) => "audio_unavailable",
            Self::VisionUnavailable(_) => "vision_unavailable",
            Self::MineruUnavailable(_) => "mineru_unavailable",
            Self::MineruConversion(_) => "mineru_conversion",
            Self::SecurityViolation { .. } => "security_violation",
            Self::ResponseTooLarge { .. } => "response_too_large",
            Self::Other(_) => "other",
            Self::NotSyncCapable(_) => "not_sync_capable",
        }
    }
}

impl fmt::Display for AlchemizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // One tr_args call per variant; the key is the English source text
        // so the en bundle and the no-i18n fallback render identically.
        let s = match self {
            Self::UnsupportedFormat(what) => {
                crate::tr_args("unsupported-format-for-input", &[("what", what.clone())])
            }
            Self::DepthLimitExceeded { limit } => crate::tr_args(
                "container-nesting-exceeds-the-depth-limit-of",
                &[("limit", limit.to_string())],
            ),
            Self::NoConverterAccepted => crate::tr("no-converter-accepted-the-input"),
            Self::AllBackendsFailed { ext, attempts } => {
                // Debug-format the attempts vec, matching the old
                // `{attempts:?}` interpolation exactly.
                let rendered = format!("{attempts:?}");
                crate::tr_args(
                    "all-backends-failed-for",
                    &[("ext", ext.clone()), ("attempts", rendered)],
                )
            }
            Self::FileRead { path, source } => crate::tr_args(
                "failed-to-read-input-file",
                &[
                    ("path", path.display().to_string()),
                    ("source", source.to_string()),
                ],
            ),
            Self::Pdf(what) => crate::tr_args("pdf-extraction-failed", &[("what", what.clone())]),
            Self::Zip(what) => crate::tr_args("zip-archive-error", &[("what", what.clone())]),
            Self::Xml(what) => crate::tr_args("xml-parsing-error", &[("what", what.clone())]),
            Self::VideoUnavailable(what) => {
                crate::tr_args("video-processing-unavailable", &[("what", what.clone())])
            }
            Self::AudioUnavailable(what) => {
                crate::tr_args("audio-transcription-unavailable", &[("what", what.clone())])
            }
            Self::VisionUnavailable(what) => {
                crate::tr_args("vision-provider-unavailable", &[("what", what.clone())])
            }
            Self::MineruUnavailable(what) => {
                crate::tr_args("mineru-service-unavailable", &[("what", what.clone())])
            }
            Self::MineruConversion(what) => {
                crate::tr_args("mineru-conversion-error", &[("what", what.clone())])
            }
            Self::SecurityViolation { kind, message } => crate::tr_args(
                "security-violation",
                &[("kind", kind.to_string()), ("message", message.clone())],
            ),
            Self::ResponseTooLarge { actual, limit } => crate::tr_args(
                "response-body-too-large",
                &[("actual", actual.to_string()), ("limit", limit.to_string())],
            ),
            Self::Other(msg) => msg.clone(),
            Self::NotSyncCapable(what) => crate::tr_args(
                "synchronous-conversion-not-available",
                &[("what", what.clone())],
            ),
        };
        f.write_str(&s)
    }
}

impl fmt::Display for SecurityViolationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::InvalidScheme => crate::tr("invalid-scheme"),
            Self::LoopbackAddress => crate::tr("loopback-address"),
            Self::PrivateAddress => crate::tr("private-address"),
            Self::LinkLocalAddress => crate::tr("link-local-address"),
            Self::HostnameNotAllowed => crate::tr("hostname-not-allowed"),
            Self::MissingSignature => crate::tr("missing-signature"),
        };
        f.write_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Display tests are locale-independent by construction: the expected
    // string is built through the same catalog API (`translate_for` with
    // the currently detected locale), so the assertions verify the
    // key/arg wiring of each variant, not any particular language. The
    // languages themselves are pinned by i18n::catalog::tests.

    #[test]
    fn error_display_unsupported_format() {
        let err = AlchemizeError::UnsupportedFormat("xyz".to_string());
        let expected = alchemize_i18n_expected("unsupported-format-for-input", &[("what", "xyz")]);
        assert_eq!(err.to_string(), expected);
    }

    #[test]
    fn error_display_not_sync_capable_carries_reason_and_kind() {
        let err = AlchemizeError::NotSyncCapable("path input".to_string());
        let expected = alchemize_i18n_expected(
            "synchronous-conversion-not-available",
            &[("what", "path input")],
        );
        assert_eq!(err.to_string(), expected);
        assert_eq!(err.kind(), "not_sync_capable");
    }

    #[test]
    fn error_display_all_backends_failed_with_attempts() {
        let err = AlchemizeError::AllBackendsFailed {
            ext: "pdf".to_string(),
            attempts: vec![
                ("mineru".to_string(), "timeout".to_string()),
                ("pdf".to_string(), "corrupted xref".to_string()),
            ],
        };
        let msg = err.to_string();
        assert!(msg.contains("pdf"));
        assert!(msg.contains("mineru"));
        assert!(msg.contains("corrupted xref"));
    }

    #[test]
    fn error_display_file_read() {
        let err = AlchemizeError::file_read("/tmp/missing.pdf", std::io::Error::other("not found"));
        let msg = err.to_string();
        assert!(msg.contains("/tmp/missing.pdf"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn error_display_domain_variants() {
        // Locale-independent invariants: product/format identifiers must
        // survive translation in every bundle.
        assert!(
            AlchemizeError::Pdf("bad".into())
                .to_string()
                .contains("PDF")
        );
        assert!(
            AlchemizeError::Zip("bad".into())
                .to_string()
                .contains("ZIP")
        );
        assert!(
            AlchemizeError::Xml("bad".into())
                .to_string()
                .contains("XML")
        );
        assert!(
            AlchemizeError::VideoUnavailable("no ffmpeg".into())
                .to_string()
                .contains("no ffmpeg")
        );
        assert!(
            AlchemizeError::AudioUnavailable("no engine".into())
                .to_string()
                .contains("no engine")
        );
        assert!(
            AlchemizeError::MineruUnavailable("refused".into())
                .to_string()
                .contains("MinerU")
        );
    }

    #[test]
    fn error_display_response_too_large() {
        let err = AlchemizeError::ResponseTooLarge {
            actual: 100_000_000,
            limit: 50_000_000,
        };
        let msg = err.to_string();
        assert!(msg.contains("100000000"));
        assert!(msg.contains("50000000"));
    }

    #[test]
    fn file_read_constructor_and_other_helper() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err = AlchemizeError::file_read("/path/to/file", io_err);
        match &err {
            AlchemizeError::FileRead { path, .. } => {
                assert_eq!(path.to_str(), Some("/path/to/file"))
            }
            _ => panic!("expected FileRead variant"),
        }
        assert_eq!(AlchemizeError::other(42).to_string(), "42");
    }

    /// Build the expected Display text through the catalog directly —
    /// mirrors what `AlchemizeError::fmt` should produce for the given
    /// key/args under the host locale, without hardcoding a language.
    fn alchemize_i18n_expected(key: &str, args: &[(&str, &str)]) -> String {
        let owned: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, (*v).to_string())).collect();
        let lang = crate::i18n::locale::current_locale();
        crate::i18n::translate_for(&lang, key, &owned)
    }
}
