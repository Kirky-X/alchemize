#![cfg(feature = "containers")]

//! 7z archive converter with recursive extraction (T027).
//!
//! Payloads are decoded fully in memory via `sevenz_rust2::SevenZReader`
//! (`for_each_entries`), bounded by the shared per-entry safety limit.

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

/// View over raw 7z bytes implementing the shared container skeleton.
pub struct SevenZArchive {
    bytes: Vec<u8>,
}

impl SevenZArchive {
    /// Wrap raw 7z bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl crate::infrastructure::containers::RecursiveContainer for SevenZArchive {
    fn format_name(&self) -> &'static str {
        "7z"
    }

    fn entries(&self) -> Result<Vec<ContainerEntry>, AlchemizeError> {
        extract_sevenz(&self.bytes)
    }
}

/// Converts 7z archives by extracting and converting each contained file.
///
/// Password-protected archives are not supported (the crate is built without
/// the `aes256` feature); they surface as an extraction error and the
/// registry degrades the entry accordingly.
pub struct SevenZConverter {
    base: ContainerBase,
}

impl SevenZConverter {
    /// Create with a deferred registry slot (builder wiring: filled once the
    /// enclosing registry is assembled). If the slot stays empty, entries
    /// degrade to attachments (`metadata.registry_wired = false`).
    pub fn with_registry_slot(slot: RegistrySlot) -> Self {
        Self {
            base: ContainerBase::with_slot(slot),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        Self {
            base: ContainerBase::with_arc(registry),
        }
    }
}

#[async_trait]
impl Converter for SevenZConverter {
    fn name(&self) -> &'static str {
        "7z"
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
                .map_err(|e| AlchemizeError::other(format!("failed to read 7z input: {e}")))?;
            let archive = SevenZArchive::new(bytes);
            self.base
                .convert_container(archive.format_name(), archive.entries())
                .await
        })
        .await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["7z"]
    }
}

/// Extract all regular-file entries from raw 7z bytes (in-memory decode).
///
/// `for_each_entries` yields entries folder by folder; returning `false`
/// would stop enumeration early, so we always return `true` and rely on the
/// per-entry size guard to reject oversized payloads.
fn extract_sevenz(bytes: &[u8]) -> Result<Vec<ContainerEntry>, AlchemizeError> {
    let mut reader =
        sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty())
            .map_err(|e| AlchemizeError::other(format!("failed to open 7z archive: {e}")))?;

    let mut out: Vec<ContainerEntry> = Vec::new();
    let mut total: u64 = 0;
    let limit = crate::infrastructure::containers::MAX_ENTRY_BYTES;
    reader
        .for_each_entries(|entry, data| {
            if entry.is_directory || entry.is_anti_item || !entry.has_stream {
                return Ok(true);
            }
            if is_metadata_entry(&entry.name) {
                return Ok(true);
            }
            if entry.size > limit {
                return Err(sevenz_rust2::Error::Other(
                    format!(
                        "entry '{}' is {} bytes, exceeds the {limit}-byte per-entry limit",
                        entry.name, entry.size
                    )
                    .into(),
                ));
            }
            let mut buf = Vec::new();
            data.take(limit + 1).read_to_end(&mut buf).map_err(|e| {
                sevenz_rust2::Error::Other(
                    format!("failed to read entry '{}': {e}", entry.name).into(),
                )
            })?;
            check_entry_size(&entry.name, buf.len() as u64)
                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            // Incremental aggregate check: `check_total_bytes` on the fully
            // materialized list would run after the peak allocation.
            total = total.saturating_add(buf.len() as u64);
            crate::infrastructure::containers::check_total_bytes(total)
                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            out.push(ContainerEntry {
                name: entry.name.clone(),
                bytes: buf,
            });
            Ok(true)
        })
        .map_err(|e| AlchemizeError::other(format!("failed to extract 7z entries: {e}")))?;

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::registry::ConverterRegistry;

    #[test]
    fn supported_extensions_and_name() {
        let converter = SevenZConverter::with_arc(Arc::new(ConverterRegistry::new()));
        assert_eq!(converter.supported_extensions(), &["7z"]);
        assert_eq!(converter.name(), "7z");
    }

    #[tokio::test]
    async fn convert_invalid_bytes_error() {
        let converter = SevenZConverter::with_arc(Arc::new(ConverterRegistry::new()));
        let err = converter
            .convert(Input::bytes(b"this is not a 7z archive".to_vec(), "bad.7z"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("7z"),
            "error must mention 7z, got: {err}"
        );
    }

    #[test]
    fn extract_invalid_bytes_error() {
        let err = extract_sevenz(b"\x37\x7A\xBC\xAF\x27\x1C truncated garbage").unwrap_err();
        assert!(err.to_string().contains("7z"));
    }

    #[test]
    fn format_name_via_view() {
        let archive = SevenZArchive::new(Vec::new());
        assert_eq!(archive.format_name(), "7z");
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::application::registry::ConverterRegistry;
    use crate::domain::input::Input;
    use crate::infrastructure::containers::RegistrySlot;
    use crate::infrastructure::document::text::TextConverter;
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
    use std::io::Cursor;

    fn sevenz_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).expect("7z writer");
        for (name, data) in entries {
            writer
                .push_archive_entry(ArchiveEntry::new_file(name), Some(&data[..]))
                .expect("push entry");
        }
        writer.finish().expect("finish").into_inner()
    }

    fn registry_with_text_and_slot() -> (
        Arc<ConverterRegistry>,
        crate::infrastructure::containers::RegistrySlot,
    ) {
        let slot: RegistrySlot = Arc::new(std::sync::OnceLock::new());
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        registry.register(Box::new(SevenZConverter::with_registry_slot(Arc::clone(
            &slot,
        ))));
        let registry = Arc::new(registry);
        let _ = slot.set(Arc::downgrade(&registry));
        (registry, slot)
    }

    #[tokio::test]
    async fn convert_7z_with_text_and_binary() {
        let (registry, _slot) = registry_with_text_and_slot();
        let bytes = sevenz_bytes(&[
            ("notes.txt", b"hello from 7z".as_slice()),
            ("blob.bin", [0x00, 0xFF, 0x10].as_slice()),
        ]);
        let result = registry
            .convert(&Input::bytes(bytes, "bundle.7z"))
            .await
            .unwrap();
        assert!(result.markdown.contains("hello from 7z"), "{result:?}");
        assert!(result.markdown.contains("blob.bin"), "{result:?}");
        assert_eq!(result.metadata["file_count"], 2);
        assert_eq!(result.metadata["converted_count"], 1);
        assert_eq!(result.assets.len(), 1);
    }

    #[tokio::test]
    async fn convert_7z_empty_archive() {
        let (registry, _slot) = registry_with_text_and_slot();
        let bytes = sevenz_bytes(&[]);
        let result = registry
            .convert(&Input::bytes(bytes, "empty.7z"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("no convertible files"),
            "{result:?}"
        );
        assert_eq!(result.metadata["file_count"], 0);
    }

    #[tokio::test]
    async fn extract_7z_skips_directories_and_metadata() {
        let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).expect("7z writer");
        writer
            .push_archive_entry::<&[u8]>(ArchiveEntry::new_directory("docs"), None)
            .expect("dir");
        writer
            .push_archive_entry(ArchiveEntry::new_file("._junk"), Some(&b"x"[..]))
            .expect("meta");
        writer
            .push_archive_entry(
                ArchiveEntry::new_file("docs/real.txt"),
                Some(&b"real body"[..]),
            )
            .expect("file");
        let bytes = writer.finish().expect("finish").into_inner();
        let entries = extract_sevenz(&bytes).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["docs/real.txt"], "{names:?}");
    }

    #[tokio::test]
    async fn convert_7z_nested_archive_recurses() {
        let inner = sevenz_bytes(&[("leaf.txt", b"leaf body".as_slice())]);
        let outer = sevenz_bytes(&[("inner.7z", inner.as_slice())]);
        let (registry, _slot) = registry_with_text_and_slot();
        let result = registry
            .convert(&Input::bytes(outer, "outer.7z"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("leaf body"),
            "7z->7z chain must recurse, got: {}",
            result.markdown
        );
        assert_eq!(result.metadata["depth_limited"], 0);
    }

    #[tokio::test]
    async fn sevenz_oversize_entry_rejected() {
        // 65 MiB of zeros compresses to ~70KB (LZMA): the writer builds it
        // cheaply and the per-entry limit must reject it.
        let big = vec![0u8; crate::infrastructure::containers::MAX_ENTRY_BYTES as usize + 1];
        let bytes = sevenz_bytes(&[("big.bin", big.as_slice())]);
        let err = extract_sevenz(&bytes).unwrap_err();
        assert!(err.to_string().contains("per-entry limit"), "{err}");
    }

    #[test]
    fn sevenz_view_format_name() {
        assert_eq!(SevenZArchive::new(Vec::new()).format_name(), "7z");
    }
}
