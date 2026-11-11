#![cfg(feature = "tar")]

//! TAR archive converter with recursive extraction (T020).

use std::io::{Cursor, Read};
#[cfg(test)]
use std::sync::Arc;

use async_trait::async_trait;

use crate::application::ports::Converter;
#[cfg(test)]
use crate::application::registry::ConverterRegistry;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::infrastructure::containers::{
    ContainerBase, ContainerEntry, RecursiveContainer, RegistrySlot, acquire_depth,
    check_entry_size, is_metadata_entry, with_depth_scope,
};
// Byte-budget helpers used only by the .tgz path (single-format builds without
// the gzip feature must not import them).
#[cfg(all(feature = "tar", feature = "gzip"))]
use crate::infrastructure::containers::{charge_chain_bytes, read_limited};

/// View over raw tar bytes implementing the shared container skeleton.
pub struct TarArchive {
    bytes: Vec<u8>,
}

impl TarArchive {
    /// Wrap raw tar bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl RecursiveContainer for TarArchive {
    fn format_name(&self) -> &'static str {
        "tar"
    }

    fn entries(&self) -> Result<Vec<ContainerEntry>, AlchemizeError> {
        extract_tar(&self.bytes)
    }
}

/// Converts uncompressed tar archives by extracting and converting each
/// contained regular file.
pub struct TarConverter {
    base: ContainerBase,
}

impl TarConverter {
    /// Create with a deferred registry slot (builder wiring: filled once the
    /// enclosing registry is assembled). If the slot stays empty, entries
    /// degrade to attachments (`metadata.registry_wired = false`).
    pub fn with_registry_slot(slot: RegistrySlot) -> Self {
        Self {
            base: ContainerBase::with_slot(slot),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)] // used by feature-gated test modules
    pub(crate) fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        Self {
            base: ContainerBase::with_arc(registry),
        }
    }
}

#[async_trait]
impl Converter for TarConverter {
    fn name(&self) -> &'static str {
        "tar"
    }

    async fn convert(
        &self,
        input: Input,
    ) -> Result<crate::domain::result::ConvertResult, AlchemizeError> {
        with_depth_scope(async {
            let _depth = acquire_depth()?;
            let bytes = input
                .to_bytes()
                .await
                .map_err(|e| AlchemizeError::other(format!("failed to read tar input: {e}")))?;
            let archive = TarArchive::new(bytes);
            self.base
                .convert_container(archive.format_name(), archive.entries())
                .await
        })
        .await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["tar"]
    }
}

/// Converts gzip-compressed tar archives (`.tgz` / `.tar.gz`).
///
/// Note: because [`Input::extension`] only reports the last suffix, a file
/// named `archive.tar.gz` routes to the gz single-file converter first, which
/// detects the inner tar and recurses into [`TarConverter`] via the registry.
/// This converter handles `.tgz` directly and `.tar.gz` when an explicit
/// extension override names it.
#[cfg(all(feature = "tar", feature = "gzip"))]
pub struct TarGzConverter {
    base: ContainerBase,
}

#[cfg(all(feature = "tar", feature = "gzip"))]
impl TarGzConverter {
    /// Create with a deferred registry slot (builder wiring: filled once the
    /// enclosing registry is assembled). If the slot stays empty, entries
    /// degrade to attachments (`metadata.registry_wired = false`).
    pub fn with_registry_slot(slot: RegistrySlot) -> Self {
        Self {
            base: ContainerBase::with_slot(slot),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)] // used by feature-gated test modules
    pub(crate) fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        Self {
            base: ContainerBase::with_arc(registry),
        }
    }
}

#[async_trait]
#[cfg(all(feature = "tar", feature = "gzip"))]
impl Converter for TarGzConverter {
    fn name(&self) -> &'static str {
        "tar-gz"
    }

    async fn convert(
        &self,
        input: Input,
    ) -> Result<crate::domain::result::ConvertResult, AlchemizeError> {
        with_depth_scope(async {
            let _depth = acquire_depth()?;
            let bytes = input
                .to_bytes()
                .await
                .map_err(|e| AlchemizeError::other(format!("failed to read tar.gz input: {e}")))?;
            // The decompressed tar is itself a container, so it is bounded by
            // the total limit rather than the single-file limit.
            let tar_bytes = read_limited(
                flate2::read::GzDecoder::new(Cursor::new(&bytes)),
                crate::infrastructure::containers::MAX_TOTAL_BYTES,
                "tar.gz",
            )?;
            charge_chain_bytes(tar_bytes.len() as u64)?;
            let archive = TarArchive::new(tar_bytes);
            self.base
                .convert_container(archive.format_name(), archive.entries())
                .await
        })
        .await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["tgz", "tar.gz"]
    }
}

/// Extract all regular-file entries from raw tar bytes.
fn extract_tar(bytes: &[u8]) -> Result<Vec<ContainerEntry>, AlchemizeError> {
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut out = Vec::new();
    let mut total: u64 = 0;

    for entry in archive
        .entries()
        .map_err(|e| AlchemizeError::other(format!("failed to read tar archive: {e}")))?
    {
        let entry =
            entry.map_err(|e| AlchemizeError::other(format!("failed to read tar entry: {e}")))?;

        // Only real file contents. Directories, symlinks, pax extended
        // headers (XHeader/XGlobalHeader) and GNU long-name markers are
        // structural, not payload — skip them.
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }

        let name = entry
            .path()
            .map_err(|e| AlchemizeError::other(format!("failed to read tar entry name: {e}")))?
            .to_string_lossy()
            .into_owned();

        // OS metadata (AppleDouble `._*`, `__MACOSX`) is not document
        // content; the zip converter filters the same class of entries.
        if is_metadata_entry(&name) {
            continue;
        }

        let declared = entry
            .header()
            .size()
            .map_err(|e| AlchemizeError::other(format!("failed to read tar entry size: {e}")))?;
        check_entry_size(&name, declared)?;

        // `take` guards against malformed headers declaring a smaller size
        // than the payload stream actually yields.
        let mut data = Vec::new();
        entry
            .take(crate::infrastructure::containers::MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| {
                AlchemizeError::other(format!("failed to read tar entry '{name}': {e}"))
            })?;
        check_entry_size(&name, data.len() as u64)?;
        // Incremental aggregate check: the post-materialization check would
        // run after the peak allocation.
        total = total.saturating_add(data.len() as u64);
        crate::infrastructure::containers::check_total_bytes(total)?;

        out.push(ContainerEntry { name, bytes: data });
    }

    Ok(out)
}

#[cfg(all(test, feature = "text"))]
mod tests {
    use super::*;
    use crate::infrastructure::document::text::TextConverter;
    #[cfg(all(feature = "tar", feature = "gzip"))]
    use std::io::Write;
    use tar::{Builder, Header};

    fn registry_with_text() -> Arc<ConverterRegistry> {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        Arc::new(registry)
    }

    /// Build a tar archive in memory with the given file entries.
    fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());
        for &(name, data) in entries {
            let mut header = Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    /// Build a tar archive containing a directory entry plus files.
    fn build_tar_with_dir() -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());
        let mut dir_header = Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder
            .append_data(&mut dir_header, "docs/", std::io::empty())
            .unwrap();

        let mut header = Header::new_gnu();
        let data = b"inside dir".as_slice();
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, "docs/note.txt", data)
            .unwrap();
        builder.into_inner().unwrap()
    }

    #[test]
    fn supported_extensions() {
        let converter = TarConverter::with_arc(Arc::new(ConverterRegistry::new()));
        assert_eq!(converter.supported_extensions(), &["tar"]);
        assert_eq!(converter.name(), "tar");
    }

    /// `.tgz`/`.tar.gz` 的组合格式能力需要 tar+gzip 同时在场，测试同样
    /// 按 feature 门控（安全审查 MEDIUM 修复：tar-only 构建不得引用该类型）。
    #[cfg(all(feature = "tar", feature = "gzip"))]
    #[test]
    fn supported_extensions_tar_gz() {
        let gz = TarGzConverter::with_arc(Arc::new(ConverterRegistry::new()));
        assert_eq!(gz.supported_extensions(), &["tgz", "tar.gz"]);
        assert_eq!(gz.name(), "tar-gz");
    }

    #[tokio::test]
    async fn convert_tar_with_text_and_binary() {
        let converter = TarConverter::with_arc(registry_with_text());
        let tar_bytes = build_tar(&[
            ("readme.md", b"# Tar Fixture\nbody text".as_slice()),
            ("payload.bin", &[0x00, 0xFF, 0x10]),
        ]);
        let result = converter
            .convert(Input::bytes(tar_bytes, "bundle.tar"))
            .await
            .unwrap();
        assert!(result.markdown.contains("## readme.md"));
        assert!(result.markdown.contains("body text"));
        assert!(result.markdown.contains("payload.bin"));
        assert!(result.markdown.contains("Attachment"));
        assert_eq!(result.metadata["source_format"], "tar");
        assert_eq!(result.metadata["file_count"], 2);
        assert_eq!(result.metadata["converted_count"], 1);
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].filename, "payload.bin");
    }

    #[tokio::test]
    async fn convert_tar_skips_directories() {
        let converter = TarConverter::with_arc(registry_with_text());
        let tar_bytes = build_tar_with_dir();
        let result = converter
            .convert(Input::bytes(tar_bytes, "dir.tar"))
            .await
            .unwrap();
        // Directory itself is not listed as an entry; the file inside is.
        assert!(result.markdown.contains("docs/note.txt"));
        assert!(result.markdown.contains("inside dir"));
        assert_eq!(result.metadata["file_count"], 1);
    }

    #[tokio::test]
    async fn convert_tar_handles_pax_long_names() {
        let converter = TarConverter::with_arc(registry_with_text());
        let long_name = format!("{}.txt", "deep/".repeat(30) + "x");
        let tar_bytes = build_tar(&[(long_name.as_str(), b"long name content")]);
        let result = converter
            .convert(Input::bytes(tar_bytes, "pax.tar"))
            .await
            .unwrap();
        // Pax extended headers are skipped; the real entry still converts.
        assert!(
            result.markdown.contains(&long_name),
            "long name must survive"
        );
        assert!(result.markdown.contains("long name content"));
    }

    #[tokio::test]
    async fn convert_tar_empty_archive() {
        let converter = TarConverter::with_arc(registry_with_text());
        let tar_bytes = build_tar(&[]);
        let result = converter
            .convert(Input::bytes(tar_bytes, "empty.tar"))
            .await
            .unwrap();
        assert!(result.markdown.contains("no convertible files"));
        assert_eq!(result.metadata["file_count"], 0);
    }

    #[cfg(all(feature = "tar", feature = "gzip"))]
    #[tokio::test]
    async fn convert_tgz_roundtrip() {
        let converter = TarGzConverter::with_arc(registry_with_text());
        let tar_bytes = build_tar(&[("story.txt", b"once upon a tgz")]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        let gz_bytes = gz.finish().unwrap();
        let result = converter
            .convert(Input::bytes(gz_bytes, "bundle.tgz"))
            .await
            .unwrap();
        assert!(result.markdown.contains("story.txt"));
        assert!(result.markdown.contains("once upon a tgz"));
        assert_eq!(result.metadata["source_format"], "tar");
    }

    #[tokio::test]
    async fn convert_tar_invalid_bytes() {
        let converter = TarConverter::with_arc(registry_with_text());
        let err = converter
            .convert(Input::bytes(b"definitely not a tar".to_vec(), "bad.tar"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("tar"));
    }

    #[test]
    fn extract_tar_declared_oversize_rejected() {
        // Header claims >64MB; extraction must refuse before reading payload.
        let mut builder = Builder::new(Vec::new());
        let mut header = Header::new_gnu();
        header.set_size(crate::infrastructure::containers::MAX_ENTRY_BYTES + 1);
        header.set_cksum();
        builder
            .append_data(&mut header, "huge.bin", std::io::empty())
            .unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let err = extract_tar(&tar_bytes).unwrap_err();
        assert!(err.to_string().contains("per-entry limit"));
    }

    #[tokio::test]
    async fn four_level_nesting_rejects_and_propagates_depth_counter() {
        // leaf.tar -> l2.tar -> l3.tar -> l4.tar: the 4th container exceeds
        // MAX_DEPTH=3. The top-level result must still report the rejection.
        let mut inner = build_tar(&[("leaf.txt", b"leaf body")]);
        for _ in 0..3 {
            let mut builder = Builder::new(Vec::new());
            let mut header = Header::new_gnu();
            header.set_size(inner.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "nested.tar", inner.as_slice())
                .unwrap();
            inner = builder.into_inner().unwrap();
        }

        // Self-referential wiring: the slot points at the registry that holds
        // the converter (same post-assembly wiring the builder performs).
        let slot: crate::infrastructure::containers::RegistrySlot =
            Arc::new(std::sync::OnceLock::new());
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        registry.register(Box::new(TarConverter::with_registry_slot(Arc::clone(
            &slot,
        ))));
        let registry = Arc::new(registry);
        let _ = slot.set(Arc::downgrade(&registry));

        let result = registry
            .convert(&Input::bytes(inner, "outer.tar"))
            .await
            .unwrap();
        assert_eq!(
            result.metadata["depth_limited"], 1,
            "top-level result must expose the nested depth rejection: {}",
            result.metadata
        );
        assert!(
            result.markdown.contains("depth limit"),
            "rendered reason must say depth limit, got: {:?}",
            result.markdown
        );
    }
}
