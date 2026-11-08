#![cfg(feature = "containers")]

//! Single-file compressed stream converters: gz / xz / zst (T021).
//!
//! Each converter decompresses one file, then re-routes the payload through
//! the registry: the inner filename (compression suffix stripped) decides the
//! first attempt; magic-byte detection (`domain::input::detect_format`)
//! provides a second attempt when the filename hint fails. Payloads nothing
//! can handle degrade to a fenced code block / attachment.

use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::application::registry::ConverterRegistry;
use crate::domain::error::AlchemizeError;
use crate::domain::input::{detect_format, Input};
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};
use crate::infrastructure::containers::{
    fallback_entry, read_limited, ContainerBase, MAX_SINGLE_DECOMPRESSED_BYTES,
};

/// Converts gzip-compressed single files (`.gz`).
pub struct GzConverter {
    base: ContainerBase,
}

/// Converts xz-compressed single files (`.xz`).
pub struct XzConverter {
    base: ContainerBase,
}

/// Converts zstd-compressed single files (`.zst`).
pub struct ZstConverter {
    base: ContainerBase,
}

impl GzConverter {
    /// Create a new gz converter owning a registry for nested handling.
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

impl XzConverter {
    /// Create a new xz converter owning a registry for nested handling.
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

impl ZstConverter {
    /// Create a new zst converter owning a registry for nested handling.
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

fn gz_decompress(bytes: &[u8]) -> Result<Vec<u8>, AlchemizeError> {
    read_limited(
        flate2::read::GzDecoder::new(Cursor::new(bytes)),
        MAX_SINGLE_DECOMPRESSED_BYTES,
        "gz",
    )
}

fn xz_decompress(bytes: &[u8]) -> Result<Vec<u8>, AlchemizeError> {
    read_limited(
        xz2::read::XzDecoder::new(Cursor::new(bytes)),
        MAX_SINGLE_DECOMPRESSED_BYTES,
        "xz",
    )
}

fn zst_decompress(bytes: &[u8]) -> Result<Vec<u8>, AlchemizeError> {
    let decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::other(format!("zst: failed to start decoder: {e}")))?;
    read_limited(
        decoder,
        MAX_SINGLE_DECOMPRESSED_BYTES,
        "zst",
    )
}

/// Strip a compression suffix from a filename (`archive.tar.gz` ->
/// `archive.tar`); filenames without a compression suffix pass through.
fn strip_compression_suffix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for suffix in [".gz", ".xz", ".zst"] {
        if lower.ends_with(suffix) && name.len() > suffix.len() {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name.to_string()
}

/// Filename hint for the payload according to magic bytes, when it differs
/// from the filename-derived extension (e.g. `img.bin.gz` holding a PNG).
fn magic_suggested_name(data: &[u8], inner_name: &str) -> Option<String> {
    let input = Input::bytes(data.to_vec(), inner_name.to_string());
    let format = detect_format(&input)?;
    let canonical = format.extension();
    let current = std::path::Path::new(inner_name)
        .extension()
        .and_then(|e| e.to_str())?;
    if current.eq_ignore_ascii_case(canonical) {
        return None;
    }
    Some(match inner_name.rfind('.') {
        Some(dot) => format!("{}.{}", &inner_name[..dot], canonical),
        None => format!("{inner_name}.{canonical}"),
    })
}

/// Shared conversion pipeline for all single-file converters.
async fn convert_single(
    base: &ContainerBase,
    input: Input,
    source_format: &'static str,
    decompress: fn(&[u8]) -> Result<Vec<u8>, AlchemizeError>,
) -> Result<ConvertResult, AlchemizeError> {
    let _depth = base.acquire_depth()?;
    let bytes = input
        .to_bytes()
        .await
        .map_err(|e| AlchemizeError::other(format!("failed to read {source_format} input: {e}")))?;
    let filename = input.filename().unwrap_or("payload").to_string();
    let inner_name = strip_compression_suffix(&filename);
    let data = decompress(&bytes)?;

    // 1) Filename-derived routing: the registry matches on the inner name.
    if let Ok(mut result) = base
        .registry()
        .convert(&Input::bytes(data.clone(), inner_name.clone()))
        .await
    {
        return Ok(finish_inner(source_format, &inner_name, &mut result));
    }

    // 2) Magic-byte assist: rename by detected format and retry once.
    if let Some(suggested) = magic_suggested_name(&data, &inner_name) {
        if let Ok(mut result) = base
            .registry()
            .convert(&Input::bytes(data.clone(), suggested.clone()))
            .await
        {
            return Ok(finish_inner(source_format, &suggested, &mut result));
        }
    }

    // 3) Degrade: fenced code block for text, attachment for binary.
    // Use the magic-suggested name when one exists so the attachment carries
    // the real format extension (no image converter is registered here, but
    // the payload's type must not be lost).
    let fallback_name = magic_suggested_name(&data, &inner_name).unwrap_or_else(|| inner_name.clone());
    let (markdown, asset) = fallback_entry(&fallback_name, &data);
    let mut metadata = serde_json::json!({
        "source_format": source_format,
        "inner_name": inner_name,
        "decompressed_bytes": data.len(),
        "converted": false,
        "attachments": [fallback_name],
    });
    if let Some(asset) = asset {
        metadata["attachment_extracted"] = serde_json::Value::Bool(true);
        return Ok(ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull)
            .with_asset(asset));
    }
    Ok(ConvertResult::with_metadata(
        markdown,
        ContentType::Document,
        metadata,
    ))
}

/// Wrap a successfully converted inner result with source metadata.
fn finish_inner(
    source_format: &'static str,
    inner_name: &str,
    result: &mut ConvertResult,
) -> ConvertResult {
    let metadata = serde_json::json!({
        "source_format": source_format,
        "inner_name": inner_name,
        "converted": true,
        "attachments": [],
    });
    ConvertResult::with_metadata(std::mem::take(&mut result.markdown), ContentType::Document, metadata)
        .with_quality(ConversionQuality::LocalFull)
}

#[async_trait]
impl Converter for GzConverter {
    fn name(&self) -> &'static str {
        "gz"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        convert_single(&self.base, input, "gz", gz_decompress).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["gz"]
    }
}

#[async_trait]
impl Converter for XzConverter {
    fn name(&self) -> &'static str {
        "xz"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        convert_single(&self.base, input, "xz", xz_decompress).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["xz"]
    }
}

#[async_trait]
impl Converter for ZstConverter {
    fn name(&self) -> &'static str {
        "zst"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        convert_single(&self.base, input, "zst", zst_decompress).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["zst"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::document::csv::CsvConverter;
    use crate::infrastructure::document::text::TextConverter;
    use std::io::Write;

    fn registry() -> Arc<ConverterRegistry> {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        registry.register(Box::new(CsvConverter::new()));
        Arc::new(registry)
    }

    fn gz_bytes(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn xz_bytes(data: &[u8]) -> Vec<u8> {
        let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn zst_bytes(data: &[u8]) -> Vec<u8> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn supported_extensions_and_names() {
        assert_eq!(GzConverter::new(ConverterRegistry::new()).supported_extensions(), &["gz"]);
        assert_eq!(XzConverter::new(ConverterRegistry::new()).supported_extensions(), &["xz"]);
        assert_eq!(ZstConverter::new(ConverterRegistry::new()).supported_extensions(), &["zst"]);
        assert_eq!(GzConverter::new(ConverterRegistry::new()).name(), "gz");
        assert_eq!(XzConverter::new(ConverterRegistry::new()).name(), "xz");
        assert_eq!(ZstConverter::new(ConverterRegistry::new()).name(), "zst");
    }

    #[test]
    fn strip_compression_suffix_variants() {
        assert_eq!(strip_compression_suffix("archive.tar.gz"), "archive.tar");
        assert_eq!(strip_compression_suffix("notes.TXT.GZ"), "notes.TXT");
        assert_eq!(strip_compression_suffix("data.ZST"), "data");
        assert_eq!(strip_compression_suffix("plain.txt"), "plain.txt");
        assert_eq!(strip_compression_suffix("noext"), "noext");
        // A bare ".gz" would strip to nothing; keep it intact.
        assert_eq!(strip_compression_suffix(".gz"), ".gz");
    }

    #[tokio::test]
    async fn gz_routes_by_filename_to_text() {
        let converter = GzConverter::with_arc(registry());
        let result = converter
            .convert(Input::bytes(gz_bytes(b"gz body text"), "notes.txt.gz"))
            .await
            .unwrap();
        assert!(result.markdown.contains("gz body text"));
        assert_eq!(result.metadata["source_format"], "gz");
        assert_eq!(result.metadata["inner_name"], "notes.txt");
        assert_eq!(result.metadata["converted"], true);
        assert_eq!(result.assets.len(), 0);
    }

    #[tokio::test]
    async fn gz_routes_by_filename_to_csv() {
        let converter = GzConverter::with_arc(registry());
        let csv = b"a,b\n1,2\n".as_slice();
        let result = converter
            .convert(Input::bytes(gz_bytes(csv), "table.csv.gz"))
            .await
            .unwrap();
        assert!(result.markdown.contains('a'));
        assert_eq!(result.metadata["inner_name"], "table.csv");
    }

    #[tokio::test]
    async fn gz_magic_assist_when_filename_hint_unknown() {
        // PNG payload under a name the registry cannot match by extension.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        png.extend_from_slice(b"tiny");
        let converter = GzConverter::with_arc(Arc::new(ConverterRegistry::new()));
        let result = converter
            .convert(Input::bytes(gz_bytes(&png), "img.bin.gz"))
            .await
            .unwrap();
        // No image converter registered: falls back, but the inner name must
        // carry the magic-detected extension.
        assert!(result.markdown.contains("img.png"));
    }

    #[tokio::test]
    async fn gz_fallback_for_binary_payload() {
        let converter = GzConverter::with_arc(registry());
        let binary: Vec<u8> = (0..=255u8).collect();
        let result = converter
            .convert(Input::bytes(gz_bytes(&binary), "blob.bin.gz"))
            .await
            .unwrap();
        assert!(result.markdown.contains("Attachment"));
        assert_eq!(result.metadata["converted"], false);
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].filename, "blob.bin");
    }

    #[tokio::test]
    async fn xz_routes_by_filename() {
        let converter = XzConverter::with_arc(registry());
        let result = converter
            .convert(Input::bytes(xz_bytes(b"xz payload"), "doc.txt.xz"))
            .await
            .unwrap();
        assert!(result.markdown.contains("xz payload"));
        assert_eq!(result.metadata["source_format"], "xz");
        assert_eq!(result.metadata["inner_name"], "doc.txt");
    }

    #[tokio::test]
    async fn xz_invalid_bytes_error() {
        let converter = XzConverter::with_arc(registry());
        let err = converter
            .convert(Input::bytes(b"not xz at all".to_vec(), "bad.xz"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("xz"));
    }

    #[tokio::test]
    async fn zst_routes_by_filename() {
        let converter = ZstConverter::with_arc(registry());
        let result = converter
            .convert(Input::bytes(zst_bytes(b"zstd payload"), "log.txt.zst"))
            .await
            .unwrap();
        assert!(result.markdown.contains("zstd payload"));
        assert_eq!(result.metadata["source_format"], "zst");
        assert_eq!(result.metadata["inner_name"], "log.txt");
    }

    #[tokio::test]
    async fn zst_invalid_bytes_error() {
        let converter = ZstConverter::with_arc(registry());
        let err = converter
            .convert(Input::bytes(b"garbage".to_vec(), "bad.zst"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("zst"));
    }

    #[tokio::test]
    async fn gz_nested_tar_recursion() {
        // A tar.gz routes gz -> tar through the registry when both are wired.
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        registry.register(Box::new(crate::infrastructure::containers::TarConverter::with_arc(
            Arc::new(ConverterRegistry::new()),
        )));

        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        let data = b"nested tar content".as_slice();
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append_data(&mut header, "inner.txt", data).unwrap();
        let tar_bytes = builder.into_inner().unwrap();

        let converter = GzConverter::with_arc(Arc::new(registry));
        let result = converter
            .convert(Input::bytes(gz_bytes(&tar_bytes), "archive.tar.gz"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("nested tar content"),
            "gz->tar chain must convert the inner entry, got: {}",
            result.markdown
        );
    }

    #[tokio::test]
    async fn gzip_bomb_rejected_not_oome() {
        // 100MB of zeros compress to a few hundred KB; decompression must
        // stop at the 64MB limit with an explicit error.
        let converter = GzConverter::with_arc(registry());
        let chunk = vec![0u8; 1024 * 1024];
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(1));
        for _ in 0..100 {
            encoder.write_all(&chunk).unwrap();
        }
        let bomb = encoder.finish().unwrap();
        assert!(bomb.len() < 1024 * 1024, "compressed bomb should be tiny");

        let err = converter
            .convert(Input::bytes(bomb, "bomb.txt.gz"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("safety limit"),
            "expected bomb protection error, got: {err}"
        );
    }
}
