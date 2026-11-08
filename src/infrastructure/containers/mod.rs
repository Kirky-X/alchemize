#![cfg(feature = "containers")]

//! Recursive archive container converters (tar/tgz/gz/xz/zst/7z).
//!
//! Shared skeleton: [`ContainerBase`] holds the [`ConverterRegistry`] used to
//! convert extracted entries, enforces decompression-bomb safety limits, and
//! renders the per-entry markdown with a graceful fallback (fenced code block
//! plus attachment list) for entries no converter supports.

pub mod sevenz;
pub mod single;
pub mod tar;

pub use sevenz::SevenZConverter;
pub use single::{GzConverter, XzConverter, ZstConverter};
pub use tar::{TarConverter, TarGzConverter};

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::application::registry::ConverterRegistry;
use crate::domain::asset::Asset;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Maximum size of a single entry inside a container.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum total decompressed size of a container (bomb protection).
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum decompressed size for a single-file compressed stream (gz/xz/zst).
pub const MAX_SINGLE_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum container nesting depth when recursively converting archives.
pub const MAX_DEPTH: u32 = 3;

/// Upper bound for inlining fallback text into a fenced code block.
const FALLBACK_TEXT_BYTES: usize = 8 * 1024;

/// One extracted file from a container archive.
#[derive(Debug)]
pub struct ContainerEntry {
    /// Entry path as stored in the archive.
    pub name: String,
    /// Entry content.
    pub bytes: Vec<u8>,
}

/// A container format that can be enumerated into flat entries.
///
/// Implemented by per-archive view structs (e.g. [`tar::TarArchive`]); the
/// corresponding `Converter` impls construct the view from the input bytes
/// and hand the entries to [`ContainerBase::convert_container`].
pub trait RecursiveContainer {
    /// Stable format identifier (recorded as `metadata.source_format`).
    fn format_name(&self) -> &'static str;

    /// Extract all file entries. Directories, symlinks and pax/long-name
    /// headers are skipped; per-entry and total size limits are enforced.
    fn entries(&self) -> Result<Vec<ContainerEntry>, AlchemizeError>;
}

/// RAII guard for recursive container depth. Decrements the shared counter on
/// drop, so sibling entries are counted per conversion path.
pub struct DepthGuard<'a> {
    counter: &'a AtomicU32,
}

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Shared skeleton for recursive container converters.
pub struct ContainerBase {
    registry: Arc<ConverterRegistry>,
    depth: Arc<AtomicU32>,
}

impl ContainerBase {
    /// Create a base around an existing shared registry.
    pub fn new(registry: Arc<ConverterRegistry>) -> Self {
        Self {
            registry,
            depth: Arc::new(AtomicU32::new(0)),
        }
    }

    /// The registry used for nested entry conversion.
    pub fn registry(&self) -> &ConverterRegistry {
        &self.registry
    }

    /// Acquire one level of nesting depth.
    ///
    /// Fails once the depth reaches [`MAX_DEPTH`]; the error is surfaced by
    /// the registry to the enclosing entry, which falls back to an attachment
    /// instead of recursing forever (zip-bomb style nesting protection).
    pub fn acquire_depth(&self) -> Result<DepthGuard<'_>, AlchemizeError> {
        let prev = self.depth.fetch_add(1, Ordering::SeqCst);
        if prev >= MAX_DEPTH {
            self.depth.fetch_sub(1, Ordering::SeqCst);
            return Err(AlchemizeError::other(format!(
                "container nesting exceeds the depth limit of {MAX_DEPTH}"
            )));
        }
        Ok(DepthGuard {
            counter: &self.depth,
        })
    }

    /// Convert extracted entries into markdown, delegating each one to the
    /// registry. Entries without a matching converter degrade to a fenced
    /// code block (text) or an attachment (binary), both recorded in
    /// `metadata.attachments`.
    pub async fn convert_container(
        &self,
        source_format: &'static str,
        entries: Result<Vec<ContainerEntry>, AlchemizeError>,
    ) -> Result<ConvertResult, AlchemizeError> {
        let entries = entries?;

        let total: u64 = entries.iter().map(|e| e.bytes.len() as u64).sum();
        check_total_bytes(total)?;

        let mut parts: Vec<String> = Vec::new();
        let mut assets: Vec<Asset> = Vec::new();
        let mut attachments: Vec<String> = Vec::new();
        let file_count = entries.len();
        let mut converted_count = 0usize;

        for entry in entries {
            let nested = Input::bytes(entry.bytes.clone(), entry.name.clone());
            match self.registry.convert(&nested).await {
                Ok(mut result) => {
                    parts.push(format!("## {}\n", entry.name));
                    parts.push(std::mem::take(&mut result.markdown));
                    parts.push(String::new());
                    assets.append(&mut result.assets);
                    converted_count += 1;
                }
                Err(_) => {
                    let (markdown, asset) = fallback_entry(&entry.name, &entry.bytes);
                    parts.push(markdown);
                    parts.push(String::new());
                    if let Some(asset) = asset {
                        assets.push(asset);
                    }
                    attachments.push(entry.name);
                }
            }
        }

        let markdown = if parts.is_empty() {
            format!("*{source_format} archive contains no convertible files.*\n")
        } else {
            parts.join("\n")
        };

        let metadata = serde_json::json!({
            "source_format": source_format,
            "file_count": file_count,
            "converted_count": converted_count,
            "attachments": attachments,
        });

        let mut result = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        // Attachments extracted by the fallback path must reach the caller;
        // dropping them here would silently lose the payloads.
        for asset in assets {
            result = result.with_asset(asset);
        }
        Ok(result)
    }
}

/// Enforce the total decompressed size limit.
pub(crate) fn check_total_bytes(total: u64) -> Result<(), AlchemizeError> {
    if total > MAX_TOTAL_BYTES {
        return Err(AlchemizeError::other(format!(
            "decompressed container size {total} bytes exceeds the {MAX_TOTAL_BYTES}-byte safety limit"
        )));
    }
    Ok(())
}

/// Enforce the per-entry size limit.
pub(crate) fn check_entry_size(name: &str, size: u64) -> Result<(), AlchemizeError> {
    if size > MAX_ENTRY_BYTES {
        return Err(AlchemizeError::other(format!(
            "entry '{name}' is {size} bytes, exceeds the {MAX_ENTRY_BYTES}-byte per-entry limit"
        )));
    }
    Ok(())
}

/// Read `reader` to end, failing when the output exceeds `limit`
/// (decompression-bomb protection: returns an error instead of exhausting
/// memory).
pub(crate) fn read_limited(
    reader: impl std::io::Read,
    limit: u64,
    context: &str,
) -> Result<Vec<u8>, AlchemizeError> {
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| AlchemizeError::other(format!("{context}: {e}")))?;
    if buf.len() as u64 > limit {
        return Err(AlchemizeError::other(format!(
            "{context}: decompressed size exceeds the {limit}-byte safety limit"
        )));
    }
    Ok(buf)
}

/// Fallback rendering for an entry no converter supports.
///
/// Text (valid UTF-8, non-empty) is inlined into a fenced code block tagged
/// with the entry extension; binary content is listed as an attachment and
/// preserved as an [`Asset`]. Both kinds are recorded by the caller in
/// `metadata.attachments`.
pub(crate) fn fallback_entry(name: &str, bytes: &[u8]) -> (String, Option<Asset>) {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("text");

    if let Ok(text) = std::str::from_utf8(bytes) {
        if !text.trim().is_empty() {
            let body = if text.len() > FALLBACK_TEXT_BYTES {
                let mut cut = FALLBACK_TEXT_BYTES;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                format!(
                    "{}\n… (truncated, {} bytes total)",
                    &text[..cut],
                    text.len()
                )
            } else {
                text.to_string()
            };
            return (format!("```{ext}\n{body}\n```\n"), None);
        }
    }

    let markdown = format!(
        "> Attachment: `{name}` ({size} bytes) — unsupported inner format; content preserved as attachment.\n",
        size = bytes.len(),
    );
    (markdown, Some(Asset::new(name, bytes.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::document::text::TextConverter;

    fn registry_with_text() -> Arc<ConverterRegistry> {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        Arc::new(registry)
    }

    fn entries(names: &[&str]) -> Vec<ContainerEntry> {
        names
            .iter()
            .map(|n| ContainerEntry {
                name: (*n).to_string(),
                bytes: Vec::new(),
            })
            .collect()
    }

    #[tokio::test]
    async fn convert_container_renders_entries_and_fallback() {
        let base = ContainerBase::new(registry_with_text());
        let list = vec![
            ContainerEntry {
                name: "notes.txt".to_string(),
                bytes: b"hello from tar".to_vec(),
            },
            ContainerEntry {
                // 0xFF is invalid UTF-8: a genuine binary payload.
                name: "blob.bin".to_string(),
                bytes: vec![0xFF, 0xFE, 0x00],
            },
        ];
        let result = base.convert_container("tar", Ok(list)).await.unwrap();
        assert!(result.markdown.contains("## notes.txt"));
        assert!(result.markdown.contains("hello from tar"));
        // notes.txt is converted by TextConverter (raw text, no fence).
        // Binary entry: no registry match -> attachment, preserved as asset.
        assert!(result.markdown.contains("blob.bin"));
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].filename, "blob.bin");
        assert_eq!(result.metadata["file_count"], 2);
        assert_eq!(result.metadata["converted_count"], 1);
        let attachments = result.metadata["attachments"].as_array().unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0], "blob.bin");
    }

    #[tokio::test]
    async fn convert_container_empty() {
        let base = ContainerBase::new(registry_with_text());
        let result = base.convert_container("tar", Ok(Vec::new())).await.unwrap();
        assert!(result.markdown.contains("no convertible files"));
        assert_eq!(result.metadata["file_count"], 0);
    }

    #[tokio::test]
    async fn convert_container_propagates_entries_error() {
        let base = ContainerBase::new(registry_with_text());
        let err = base
            .convert_container("tar", Err(AlchemizeError::other("boom")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn depth_guard_counts_up_to_limit_and_releases() {
        let base = ContainerBase::new(Arc::new(ConverterRegistry::new()));
        let g1 = base.acquire_depth().unwrap();
        let g2 = base.acquire_depth().unwrap();
        let g3 = base.acquire_depth().unwrap();
        assert!(base.acquire_depth().is_err(), "4th level must be rejected");
        drop(g1);
        drop(g2);
        drop(g3);
        assert!(base.acquire_depth().is_ok(), "depth must release on drop");
    }

    #[test]
    fn entry_size_limit_rejects_oversize() {
        assert!(check_entry_size("big.bin", MAX_ENTRY_BYTES).is_ok());
        let err = check_entry_size("big.bin", MAX_ENTRY_BYTES + 1).unwrap_err();
        assert!(err.to_string().contains("big.bin"));
        assert!(err.to_string().contains("per-entry limit"));
    }

    #[test]
    fn total_size_limit_rejects_oversize() {
        assert!(check_total_bytes(MAX_TOTAL_BYTES).is_ok());
        let err = check_total_bytes(MAX_TOTAL_BYTES + 1).unwrap_err();
        assert!(err.to_string().contains("safety limit"));
    }

    #[test]
    fn read_limited_allows_exact_limit_and_rejects_over() {
        let data = vec![7u8; 10];
        assert_eq!(read_limited(&data[..], 10, "t").unwrap().len(), 10);
        let err = read_limited(&data[..], 9, "t").unwrap_err();
        assert!(err.to_string().contains("safety limit"));
    }

    #[test]
    fn fallback_entry_inlines_text_and_truncates() {
        let (md, asset) = fallback_entry("notes.log", b"plain text body");
        assert!(md.contains("```log"));
        assert!(md.contains("plain text body"));
        assert!(asset.is_none());

        let long = vec![b'a'; FALLBACK_TEXT_BYTES + 100];
        let (md, _) = fallback_entry("big.txt", &long);
        assert!(md.contains("truncated"));
    }

    #[test]
    fn fallback_entry_binary_becomes_asset() {
        let (md, asset) = fallback_entry("img.dat", &[0xFF, 0xD8, 0xFF, 0x00]);
        assert!(md.contains("Attachment"));
        assert!(md.contains("img.dat"));
        let asset = asset.unwrap();
        assert_eq!(asset.filename, "img.dat");
        assert_eq!(asset.bytes, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }
}
