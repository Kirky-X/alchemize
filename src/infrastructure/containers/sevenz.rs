#![cfg(feature = "containers")]

//! 7z archive converter with recursive extraction (T027).
//!
//! Payloads are decoded fully in memory via `sevenz_rust2::SevenZReader`
//! (`for_each_entries`), bounded by the shared per-entry safety limit.

use std::io::{Cursor, Read};
use std::sync::Arc;

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::application::registry::ConverterRegistry;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::infrastructure::containers::{
    check_entry_size, ContainerBase, ContainerEntry, RecursiveContainer,
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
    /// Create a new 7z converter owning a registry for nested handling.
    pub fn new(registry: ConverterRegistry) -> Self {
        Self {
            base: ContainerBase::new(Arc::new(registry)),
        }
    }

    /// Create with an existing shared registry.
    pub fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        Self {
            base: ContainerBase::new(registry),
        }
    }
}

#[async_trait]
impl Converter for SevenZConverter {
    fn name(&self) -> &'static str {
        "7z"
    }

    async fn convert(&self, input: Input) -> Result<crate::domain::result::ConvertResult, AlchemizeError> {
        let _depth = self.base.acquire_depth()?;
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read 7z input: {e}")))?;
        let archive = SevenZArchive::new(bytes);
        self.base
            .convert_container(archive.format_name(), archive.entries())
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
    let mut reader = sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty())
    .map_err(|e| AlchemizeError::other(format!("failed to open 7z archive: {e}")))?;

    let mut out: Vec<ContainerEntry> = Vec::new();
    let limit = crate::infrastructure::containers::MAX_ENTRY_BYTES;
    reader
        .for_each_entries(|entry, data| {
            if entry.is_directory || entry.is_anti_item || !entry.has_stream {
                return Ok(true);
            }
            if entry.size > limit {
                return Err(sevenz_rust2::Error::Other(format!(
                    "entry '{}' is {} bytes, exceeds the {limit}-byte per-entry limit",
                    entry.name, entry.size
                )
                .into()));
            }
            let mut buf = Vec::new();
            data.take(limit + 1)
                .read_to_end(&mut buf)
                .map_err(|e| sevenz_rust2::Error::Other(format!("failed to read entry '{}': {e}", entry.name).into()))?;
            check_entry_size(&entry.name, buf.len() as u64)
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
        let converter = SevenZConverter::new(ConverterRegistry::new());
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
