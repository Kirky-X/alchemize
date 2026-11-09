// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Asset pipeline — binary assets extracted in document order.

use sha2::{Digest, Sha256};

/// Validate a converter-provided asset filename before it reaches any
/// write boundary (`DirSink::put`, the CLI asset loop).
///
/// Rejects path-traversal and absolute-path shapes; accepts the
/// `images/{seq:04}-{sha8}.{ext}` relative convention. Converters must
/// fail loudly on rejection — silently rewriting would desynchronize the
/// markdown reference from the written file.
pub fn validate_asset_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() {
        return Err("empty asset filename".into());
    }
    if filename.contains('\0') {
        return Err("asset filename contains NUL".into());
    }
    if filename.contains('\\') {
        return Err("asset filename contains backslash".into());
    }
    if filename.starts_with('/') || filename.contains(r":\") {
        return Err(format!("absolute asset filename: {filename:?}"));
    }
    // Windows drive prefix ("C:...") in either separator flavor.
    if filename.len() >= 2 && filename.as_bytes()[1] == b':' {
        return Err(format!("drive-prefixed asset filename: {filename:?}"));
    }
    for component in filename.split('/') {
        if component.is_empty() {
            return Err(format!("empty path component in {filename:?}"));
        }
        if component == "." || component == ".." {
            return Err(format!("path traversal component in {filename:?}"));
        }
    }
    Ok(())
}

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
        sha8_hex(&self.bytes)
    }

    /// Extension hint from the filename, if any.
    pub fn extension(&self) -> Option<&str> {
        self.filename.rsplit_once('.').map(|(_, ext)| ext)
    }
}

/// Leading 8 hex chars of the SHA-256 digest — the single source behind
/// `Asset::sha8` and every asset naming site.
pub fn sha8_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Shared content-deduplicating asset namer — the single implementation of
/// the `images/{seq:04}-{sha8}.{ext}` naming contract. The hash and the
/// filename come from the same source, so they cannot drift apart.
pub struct AssetNamer {
    by_sha: std::collections::HashMap<String, String>,
    seq: usize,
    assets: Vec<Asset>,
}

impl AssetNamer {
    /// Create an empty namer.
    pub fn new() -> Self {
        Self {
            by_sha: std::collections::HashMap::new(),
            seq: 0,
            assets: Vec::new(),
        }
    }

    /// Register borrowed bytes; returns the canonical `images/…` reference.
    /// Identical content reuses the first reference and adds no asset.
    pub fn reference(&mut self, bytes: &[u8], ext: &str) -> String {
        let sha8 = sha8_hex(bytes);
        if let Some(existing) = self.by_sha.get(&sha8) {
            return existing.clone();
        }
        let filename = format!("images/{:04}-{sha8}.{ext}", self.seq);
        self.seq += 1;
        self.assets
            .push(Asset::new(filename.clone(), bytes.to_vec()));
        self.by_sha.insert(sha8, filename.clone());
        filename
    }

    /// Register owned bytes without cloning them — for call sites that
    /// already own the payload (e.g. `mem::take`).
    pub fn reference_owned(&mut self, bytes: Vec<u8>, ext: &str) -> String {
        let sha8 = sha8_hex(&bytes);
        if let Some(existing) = self.by_sha.get(&sha8) {
            return existing.clone();
        }
        let filename = format!("images/{:04}-{sha8}.{ext}", self.seq);
        self.seq += 1;
        self.assets.push(Asset::new(filename.clone(), bytes));
        self.by_sha.insert(sha8, filename.clone());
        filename
    }

    /// Consume the namer, returning the collected assets in first-seen
    /// order.
    pub fn into_assets(self) -> Vec<Asset> {
        self.assets
    }
}

impl Default for AssetNamer {
    fn default() -> Self {
        Self::new()
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
        validate_asset_filename(&asset.filename)
            .map_err(|reason| std::io::Error::new(std::io::ErrorKind::InvalidData, reason))?;
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
    fn validate_accepts_canonical_images_reference() {
        assert_eq!(validate_asset_filename("images/0000-ab12cd34.png"), Ok(()));
    }

    #[test]
    fn validate_rejects_traversal_and_absolute_shapes() {
        for bad in [
            "../evil.png",
            "images/../x.png",
            "/abs/path.png",
            "C:\\evil.png",
            "C:/evil.png",
            "a\\b.png",
            "",
            "images/.",
        ] {
            assert!(validate_asset_filename(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn dir_sink_rejects_illegal_filename_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sink = DirSink::new(tmp.path());
        let err = sink
            .put(Asset::new("../evil.png", vec![1]))
            .expect_err("traversal must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            std::fs::read_dir(tmp.path()).unwrap().count(),
            0,
            "nothing written"
        );
    }

    #[test]
    fn asset_namer_dedups_by_content() {
        let mut namer = AssetNamer::new();
        let r1 = namer.reference(&[1, 2, 3], "png");
        let r2 = namer.reference(&[1, 2, 3], "png");
        assert_eq!(r1, r2, "same content must reuse the first reference");
        let r3 = namer.reference_owned(vec![9, 9], "jpg");
        assert_ne!(r3, r1, "different content gets the next seq");
        let assets = namer.into_assets();
        assert_eq!(assets.len(), 2, "one asset per distinct content");
        assert_eq!(assets[0].filename, r1);
        assert_eq!(assets[1].filename, r3);
    }

    #[test]
    fn asset_namer_format_matches_dir_sink_contract() {
        let mut namer = AssetNamer::new();
        let reference = namer.reference(&[7; 16], "png");
        // images/{seq:04}-{sha8}.{ext} with the sha8 of the bytes embedded.
        let expected_sha = sha8_hex(&[7; 16]);
        assert_eq!(
            reference,
            format!("images/0000-{expected_sha}.png"),
            "naming contract single source"
        );
        let second = namer.reference(&[8; 16], "jpg");
        assert!(second.starts_with("images/0001-"), "{second}");
    }

    #[test]
    fn sha8_hex_matches_asset_sha8() {
        let asset = Asset::new("x.png", vec![1, 2, 3, 4]);
        assert_eq!(sha8_hex(&asset.bytes), asset.sha8(), "single source");
    }

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
