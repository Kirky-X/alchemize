// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Asset pipeline — binary assets extracted in document order.

use sha2::{Digest, Sha256};

/// Role an asset plays in the source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRole {
    /// Inline or referenced image.
    Image,
    /// Extracted video frame.
    Frame,
    /// Image used as OCR source (rendered region).
    OcrSource,
}

/// A binary asset extracted during conversion.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    /// Suggested filename (final name may gain seq prefix via the sink).
    pub filename: String,
    /// Raw bytes.
    pub bytes: Vec<u8>,
    /// Role in the source document.
    pub role: AssetRole,
}

impl Asset {
    /// Create a new image-role asset.
    pub fn new(filename: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            filename: filename.into(),
            bytes,
            role: AssetRole::Image,
        }
    }

    /// Create an asset with an explicit role.
    pub fn with_role(filename: impl Into<String>, bytes: Vec<u8>, role: AssetRole) -> Self {
        Self {
            filename: filename.into(),
            bytes,
            role,
        }
    }

    /// Leading 8 hex chars of the SHA-256 digest of the bytes.
    pub fn sha8(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.bytes);
        let digest = hasher.finalize();
        digest[..4].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Extension hint from the filename, if any.
    pub fn extension(&self) -> Option<&str> {
        self.filename.rsplit_once('.').map(|(_, ext)| ext)
    }
}

/// Destination for extracted assets; returns the markdown reference path
/// for each asset so converters can emit it at the extraction point.
pub trait AssetSink: Send + Sync {
    /// Store an asset and return the markdown reference (relative path or data URL).
    fn put(&mut self, asset: Asset) -> std::io::Result<String>;
}

/// Sink that writes assets to an `images/` directory, referencing them by
/// relative path. Filenames are `{seq:04}-{sha8}.{ext}`; identical content
/// (same sha8) reuses the first reference without rewriting the file.
pub struct DirSink {
    root: std::path::PathBuf,
    seq: u32,
    seen: std::collections::HashMap<String, String>,
}

impl DirSink {
    /// Create a sink rooted at the given directory (created on first write).
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root: root.into(),
            seq: 0,
            seen: std::collections::HashMap::new(),
        }
    }

    /// The configured root directory.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
}

impl AssetSink for DirSink {
    fn put(&mut self, asset: Asset) -> std::io::Result<String> {
        let sha8 = asset.sha8();
        if let Some(existing) = self.seen.get(&sha8) {
            return Ok(existing.clone());
        }
        let ext = asset
            .extension()
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let name = format!("{:04}-{sha8}{ext}", self.seq);
        self.seq += 1;

        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join(&name);
        std::fs::write(&path, &asset.bytes)?;

        let reference = format!("images/{name}");
        self.seen.insert(sha8, reference.clone());
        Ok(reference)
    }
}

/// Sink that inlines assets as base64 data URLs (single-file delivery).
pub struct InlineSink;

impl AssetSink for InlineSink {
    fn put(&mut self, asset: Asset) -> std::io::Result<String> {
        use base64::Engine as _;
        let mime = match asset.extension() {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("svg") => "image/svg+xml",
            Some("tiff") | Some("tif") => "image/tiff",
            _ => "application/octet-stream",
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&asset.bytes);
        Ok(format!("data:{mime};base64,{encoded}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn asset_sha8_stable_prefix() {
        let a = Asset::new("img.png", vec![1, 2, 3]);
        assert_eq!(a.sha8().len(), 8);
        let b = Asset::new("other.png", vec![1, 2, 3]);
        assert_eq!(a.sha8(), b.sha8(), "same bytes must yield same sha8");
    }

    #[test]
    fn asset_extension_hint() {
        assert_eq!(Asset::new("photo.jpeg", vec![]).extension(), Some("jpeg"));
        assert_eq!(Asset::new("noext", vec![]).extension(), None);
    }

    #[test]
    fn dir_sink_preserves_order_and_names() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sink = DirSink::new(tmp.path());
        let r1 = sink.put(Asset::new("a.png", vec![1])).unwrap();
        let r2 = sink.put(Asset::new("b.png", vec![2])).unwrap();
        let r3 = sink.put(Asset::new("c.png", vec![3])).unwrap();
        assert!(r1.starts_with("images/0000-") && r1.ends_with(".png"));
        assert!(r2.starts_with("images/0001-"));
        assert!(r3.starts_with("images/0002-"));
    }

    #[test]
    fn dir_sink_dedups_identical_content() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sink = DirSink::new(tmp.path());
        let first = sink.put(Asset::new("a.png", vec![9, 9])).unwrap();
        let second = sink
            .put(Asset::new("different-name.png", vec![9, 9]))
            .unwrap();
        assert_eq!(first, second, "same sha must reuse the first reference");

        let written: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(written.len(), 1, "no duplicate file on disk");
    }

    #[test]
    fn dir_sink_writes_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sink = DirSink::new(tmp.path());
        let reference = sink.put(Asset::new("a.png", vec![7, 7, 7])).unwrap();
        let name = reference.trim_start_matches("images/");
        let mut buf = Vec::new();
        std::fs::File::open(tmp.path().join(name))
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        assert_eq!(buf, vec![7, 7, 7]);
    }

    #[test]
    fn inline_sink_data_url_roundtrip() {
        use base64::Engine as _;
        let mut sink = InlineSink;
        let url = sink.put(Asset::new("a.png", vec![1, 2])).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        let b64 = url.trim_start_matches("data:image/png;base64,");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
            vec![1, 2]
        );
    }

    #[test]
    fn inline_sink_mime_by_extension() {
        let mut sink = InlineSink;
        assert!(sink
            .put(Asset::new("a.svg", vec![]))
            .unwrap()
            .starts_with("data:image/svg+xml"));
        assert!(sink
            .put(Asset::new("a.bin", vec![]))
            .unwrap()
            .starts_with("data:application/octet-stream"));
    }
}
