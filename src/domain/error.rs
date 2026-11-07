// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Alchemize error types.

use std::path::PathBuf;

/// Errors produced by the alchemize crate.
#[derive(Debug, thiserror::Error)]
pub enum AlchemizeError {
    /// The input format is not supported by any registered converter.
    #[error("unsupported format for input: {0}")]
    UnsupportedFormat(String),

    /// No converter accepted the input.
    #[error("no converter accepted the input")]
    NoConverterAccepted,

    /// All backends in the fallback chain failed, with per-engine attempts.
    #[error("all backends failed for {ext}: {attempts:?}")]
    AllBackendsFailed {
        /// File extension of the failing input.
        ext: String,
        /// Ordered (engine name, failure reason) pairs, one per attempted backend.
        attempts: Vec<(String, String)>,
    },

    /// Failed to read the input file.
    #[error("failed to read input file '{path}': {source}")]
    FileRead {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Failed to extract text from PDF.
    #[error("PDF extraction failed: {0}")]
    Pdf(String),

    /// Failed to parse a ZIP-based format (PPTX/DOCX/ODF).
    #[error("ZIP archive error: {0}")]
    Zip(String),

    /// Failed to parse XML content within a ZIP archive.
    #[error("XML parsing error: {0}")]
    Xml(String),

    /// Video processing is not available (no ffmpeg or disabled).
    #[error("video processing unavailable: {0}")]
    VideoUnavailable(String),

    /// Audio transcription is not available.
    #[error("audio transcription unavailable: {0}")]
    AudioUnavailable(String),

    /// Vision provider (OCR/image analysis) is not available.
    #[error("vision provider unavailable: {0}")]
    VisionUnavailable(String),

    /// MinerU service is unavailable (connection refused, timeout, etc.).
    #[error("MinerU service unavailable: {0}")]
    MineruUnavailable(String),

    /// MinerU API returned an error or conversion failed.
    #[error("MinerU conversion error: {0}")]
    MineruConversion(String),

    /// Security violation detected (SSRF, invalid URL, etc.).
    #[error("security violation ({kind}): {message}")]
    SecurityViolation {
        kind: SecurityViolationType,
        message: String,
    },

    /// HTTP response body exceeded the configured size limit.
    #[error("response body too large: {actual} bytes exceeds limit of {limit} bytes")]
    ResponseTooLarge { actual: u64, limit: u64 },

    /// A generic error from a converter implementation.
    #[error("{0}")]
    Other(String),
}

/// Security violation types for URL validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecurityViolationType {
    /// URL scheme is not HTTPS.
    #[error("invalid scheme")]
    InvalidScheme,
    /// URL points to loopback address (127.0.0.0/8, ::1, localhost).
    #[error("loopback address")]
    LoopbackAddress,
    /// URL points to private IPv4 range (RFC1918: 10/8, 172.16/12, 192.168/16).
    #[error("private address")]
    PrivateAddress,
    /// URL points to link-local address (169.254/16).
    #[error("link-local address")]
    LinkLocalAddress,
    /// URL hostname is not in the allowlist.
    #[error("hostname not allowed")]
    HostnameNotAllowed,
    /// URL is missing required presigned signature.
    #[error("missing signature")]
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_unsupported_format() {
        let err = AlchemizeError::UnsupportedFormat("xyz".to_string());
        assert!(err.to_string().contains("unsupported format"));
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
                .contains("video processing")
        );
        assert!(
            AlchemizeError::AudioUnavailable("no engine".into())
                .to_string()
                .contains("audio transcription")
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
        assert!(msg.contains("response body too large"));
        assert!(msg.contains("100000000"));
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
}
