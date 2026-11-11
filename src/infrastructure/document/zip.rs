// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! ZIP archive converter with recursive extraction.

use std::io::{Cursor, Read};
use std::sync::Arc;

use async_trait::async_trait;
use zip::ZipArchive;

use crate::application::ports::Converter;
use crate::application::registry::ConverterRegistry;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};
use crate::infrastructure::document::shared::{
    MAX_INLINE_TEXT_BYTES, code_span_label, escape_markdown_label, fence_for,
};

/// Cumulative decompression ceiling across all entries — the zip counterpart
/// of the lopdf call-site policy (bounded per stream + bounded in aggregate).
///
/// The facade caps the compressed input (`max_file_size`, default 100 MB) and
/// the per-entry ceiling caps each member; deflate's theoretical ratio is
/// ~1000:1, so this aggregate bound stops a many-entry bomb while leaving
/// ample headroom for legitimate text archives (roughly 10:1 expansion).
const MAX_TOTAL_EXTRACTED_BYTES: usize = 1024 * 1024 * 1024;

/// Extraction outcome: members within the limits plus the names of entries
/// that exceeded the per-entry ceiling (reported, never silently dropped).
struct Extracted {
    files: Vec<(String, Vec<u8>)>,
    oversize: Vec<String>,
}

/// Converts ZIP archives by extracting and converting each contained file.
///
/// Uses the ConverterRegistry to handle each extracted file according to its
/// extension. Entries that cannot be converted are preserved — text becomes a
/// fenced block, binary stays an attachment — and listed under the metadata
/// `attachments` key; nothing is silently dropped from the record.
pub struct ZipConverter {
    registry: Arc<ConverterRegistry>,
    /// Nested-entry size ceiling in bytes (`None` = unbounded). Entries
    /// over it are skipped with an explicit count — the zip counterpart of
    /// the facade-level `max_file_size`, which nested conversion bypasses
    /// (this converter calls the registry directly).
    max_entry_size: Option<usize>,
}

impl ZipConverter {
    /// Create a new ZIP converter owning a registry for nested handling.
    pub fn new(registry: ConverterRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            max_entry_size: None,
        }
    }

    /// Create with an existing shared registry.
    pub fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        Self {
            registry,
            max_entry_size: None,
        }
    }

    /// Set the nested-entry size ceiling (bytes). Over-ceiling entries are
    /// skipped and counted, not converted.
    pub fn with_max_entry_size(mut self, max: usize) -> Self {
        self.max_entry_size = Some(max);
        self
    }
}

#[async_trait]
impl Converter for ZipConverter {
    fn name(&self) -> &'static str {
        "zip"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read ZIP input: {e}")))?;

        let Extracted { files, oversize } = extract_zip_contents(&bytes, self.max_entry_size)?;

        let mut markdown_parts = Vec::new();
        let file_count = files.len() + oversize.len();
        let mut converted_count = 0;
        let skipped_oversize = oversize.len();
        let mut attachments: Vec<String> = Vec::new();
        let mut assets: Vec<crate::domain::asset::Asset> = Vec::new();
        let mut depth_limited = 0usize;
        let mut nested_attachments = 0usize;

        // Oversize members are reported explicitly (heading + reason), never
        // dropped: extraction already stopped reading them.
        for filename in oversize {
            markdown_parts.push(format!("### {}\n", escape_markdown_label(&filename)));
            markdown_parts.push(format!(
                "> Skipped: entry exceeds the nested-entry limit of {} bytes.\n",
                self.max_entry_size.unwrap_or_default()
            ));
            markdown_parts.push(String::new());
            attachments.push(filename);
        }

        for (filename, content) in files {
            let nested_input = Input::Bytes {
                data: content.clone(),
                filename: filename.clone(),
                mimetype: None,
                extension: None,
            };

            match self.registry.convert(&nested_input).await {
                Ok(mut result) => {
                    // A nested container's depth rejections must surface at
                    // the top-level result, not die with the child metadata.
                    depth_limited = depth_limited.saturating_add(
                        result
                            .metadata
                            .get("depth_limited")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize,
                    );
                    nested_attachments = nested_attachments
                        .saturating_add(
                            result
                                .metadata
                                .get("attachments")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0),
                        )
                        .saturating_add(
                            result
                                .metadata
                                .get("nested_attachments")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize,
                        );
                    markdown_parts.push(format!("### {}\n", escape_markdown_label(&filename)));
                    markdown_parts.push(std::mem::take(&mut result.markdown));
                    markdown_parts.push(String::new());
                    // Inner assets (e.g. images from a docx member) must reach
                    // the caller; dropping them leaves dangling references.
                    assets.append(&mut result.assets);
                    converted_count += 1;
                }
                Err(error) => {
                    // No engine for this member: never drop it silently —
                    // text becomes a fenced block, binary stays an attachment.
                    // Attachments record *which* member failed. Details stay
                    // in the log: converter errors can carry internal
                    // endpoints/response bodies and must not enter the
                    // document output.
                    let depth_hit = matches!(error, AlchemizeError::DepthLimitExceeded { .. });
                    if depth_hit {
                        depth_limited += 1;
                    }
                    markdown_parts.push(format!("### {}\n", escape_markdown_label(&filename)));
                    if !matches!(
                        error,
                        AlchemizeError::UnsupportedFormat(_)
                            | AlchemizeError::DepthLimitExceeded { .. }
                    ) {
                        tracing::warn!(
                            entry = %crate::infrastructure::document::shared::sanitize_label(&filename),
                            error_kind = error.kind(),
                            "zip member conversion failed"
                        );
                        markdown_parts.push(
                            "> Converter error (details logged, not written to the document).\n"
                                .to_string(),
                        );
                    }
                    if !depth_hit
                        && let Ok(text) = std::str::from_utf8(&content)
                        && !text.trim().is_empty()
                        && !text
                            .chars()
                            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
                    {
                        // Fence language: alphanumerics only — an entry name
                        // must not smuggle markdown through the info string.
                        let ext: String = filename
                            .rsplit('.')
                            .next()
                            .unwrap_or("txt")
                            .chars()
                            .filter(|c| c.is_ascii_alphanumeric())
                            .take(16)
                            .collect();
                        let ext = if ext.is_empty() {
                            "txt".to_string()
                        } else {
                            ext
                        };
                        let trimmed = text.trim_end();
                        // Bound inline size: also bounds the fence derived
                        // from the body (a backtick-only member must not
                        // amplify output).
                        let body = if trimmed.len() > MAX_INLINE_TEXT_BYTES {
                            let mut cut = MAX_INLINE_TEXT_BYTES;
                            while !trimmed.is_char_boundary(cut) {
                                cut -= 1;
                            }
                            format!(
                                "{}\n… (truncated, {} bytes total)",
                                &trimmed[..cut],
                                trimmed.len()
                            )
                        } else {
                            trimmed.to_string()
                        };
                        let fence = fence_for(&body);
                        markdown_parts.push(format!("{fence}{ext}\n{body}\n{fence}\n"));
                    } else {
                        let reason = if depth_hit {
                            "nested container exceeds the depth limit"
                        } else {
                            "unsupported inner format"
                        };
                        markdown_parts.push(format!(
                            "> Attachment: `{}` ({} bytes) — {reason}; content preserved as attachment.\n",
                            code_span_label(&filename),
                            content.len()
                        ));
                        assets.push(crate::domain::asset::Asset::new(&filename, content));
                    }
                    markdown_parts.push(String::new());
                    attachments.push(filename);
                }
            }
        }

        let markdown = if markdown_parts.is_empty() {
            "*ZIP archive contains no convertible files.*\n".to_string()
        } else {
            markdown_parts.join("\n")
        };

        let metadata = serde_json::json!({
            "source_format": "zip",
            "file_count": file_count,
            "converted_count": converted_count,
            "skipped_oversize": skipped_oversize,
            "attachments": attachments,
            "nested_attachments": nested_attachments,
            "depth_limited": depth_limited,
        });

        let mut result = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in assets {
            result = result.with_asset(asset);
        }
        Ok(result)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["zip", "cbz"]
    }
}

/// Extract file contents from a ZIP archive (synchronous).
///
/// Reads are bounded twice: per entry by `max_entry_size` (declared size
/// pre-check + a `take` bound that catches lying headers) and in aggregate
/// by [`MAX_TOTAL_EXTRACTED_BYTES`]. Over-limit entries are returned as names
/// in `oversize`; breaching the aggregate bound aborts the conversion.
fn extract_zip_contents(
    bytes: &[u8],
    max_entry_size: Option<usize>,
) -> Result<Extracted, AlchemizeError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::Zip(format!("failed to open ZIP archive: {e}")))?;

    let mut files = Vec::new();
    let mut oversize = Vec::new();
    let mut total_extracted: usize = 0;

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| AlchemizeError::Zip(format!("failed to read ZIP entry {i}: {e}")))?;

        if file.is_dir() {
            continue;
        }

        let filename = file
            .name()
            .map_err(|e| AlchemizeError::Zip(format!("failed to read ZIP entry name {i}: {e}")))?
            .to_string();

        // Hidden-file filter on the basename: a nested dotfile like
        // `subdir/.gitkeep` must be skipped too, not just root-level ones.
        let basename = filename.rsplit(['/', '\\']).next().unwrap_or(&filename);
        if basename.starts_with('.') || filename.contains("__MACOSX") {
            continue;
        }

        // Honest headers let us skip the decompression work entirely; the
        // `take` bound below is the enforcement for lying headers.
        if let Some(max) = max_entry_size
            && file.size() > max as u64
        {
            oversize.push(filename);
            continue;
        }

        let limit = max_entry_size.unwrap_or(usize::MAX).saturating_add(1) as u64;
        let mut content = Vec::new();
        file.take(limit)
            .read_to_end(&mut content)
            .map_err(|e| AlchemizeError::Zip(format!("failed to read file content: {e}")))?;

        if max_entry_size.is_some_and(|max| content.len() > max) {
            oversize.push(filename);
            continue;
        }

        total_extracted = total_extracted.saturating_add(content.len());
        if total_extracted > MAX_TOTAL_EXTRACTED_BYTES {
            return Err(AlchemizeError::Zip(format!(
                "cumulative extracted size exceeds {MAX_TOTAL_EXTRACTED_BYTES} bytes — aborting (possible decompression bomb)"
            )));
        }

        files.push((filename, content));
    }

    Ok(Extracted { files, oversize })
}

#[cfg(all(test, feature = "text"))]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn create_zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(buf);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, content) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn registry_with_text() -> Arc<ConverterRegistry> {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(
            crate::infrastructure::document::text::TextConverter::new(),
        ));
        Arc::new(registry)
    }

    #[test]
    fn supported_extensions() {
        let converter = ZipConverter::new(ConverterRegistry::new());
        assert_eq!(converter.supported_extensions(), &["zip", "cbz"]);
    }

    struct AssetConverter;

    #[async_trait::async_trait]
    impl Converter for AssetConverter {
        fn name(&self) -> &'static str {
            "asset"
        }

        async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
            Ok(ConvertResult::with_metadata(
                "body".to_string(),
                ContentType::Document,
                serde_json::json!({"attachments": ["inner.bin"]}),
            )
            .with_asset(crate::domain::asset::Asset::new(
                "images/inner.png",
                vec![1, 2, 3],
            )))
        }

        fn supported_extensions(&self) -> &[&str] {
            &["asset"]
        }
    }

    #[tokio::test]
    async fn nested_assets_and_counters_survive_the_zip_wrapper() {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(AssetConverter));
        let converter = ZipConverter::with_arc(Arc::new(registry));
        let zip_data = create_zip_bytes(&[("member.asset", b"payload")]);
        let result = converter
            .convert(Input::bytes(zip_data, "assets.zip"))
            .await
            .unwrap();
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].filename, "images/inner.png");
        assert_eq!(result.metadata["nested_attachments"], 1);
    }

    #[tokio::test]
    async fn convert_empty_zip() {
        let converter = ZipConverter::new(ConverterRegistry::new());
        let input = Input::bytes(create_zip_bytes(&[]), "empty.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("no convertible files"));
    }

    #[tokio::test]
    async fn oversize_nested_entry_skipped_with_count() {
        let big = vec![0u8; 64];
        let zip_data = create_zip_bytes(&[("big.txt", &big), ("ok.txt", b"tiny")]);
        let converter = ZipConverter::with_arc(registry_with_text()).with_max_entry_size(16);
        let result = converter
            .convert(Input::bytes(zip_data, "t.zip"))
            .await
            .unwrap();
        assert_eq!(result.metadata["skipped_oversize"], 1);
        assert!(result.markdown.contains("exceeds the nested-entry limit"));
        assert!(
            result.markdown.contains("tiny"),
            "within-limit entry still converts"
        );
        assert_eq!(result.metadata["converted_count"], 1);
    }

    #[tokio::test]
    async fn convert_zip_with_text_file() {
        let converter = ZipConverter::with_arc(registry_with_text());
        let zip_data = create_zip_bytes(&[("readme.txt", b"Hello from ZIP")]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("readme.txt"));
        assert!(result.markdown.contains("Hello from ZIP"));
    }

    #[tokio::test]
    async fn convert_zip_skips_hidden_and_macosx() {
        let converter = ZipConverter::with_arc(registry_with_text());
        let zip_data = create_zip_bytes(&[
            (".hidden.txt", b"hidden content"),
            ("subdir/.gitkeep", b"nested dotfile content"),
            ("__MACOSX/._file", b"mac metadata"),
            ("visible.txt", b"visible content"),
        ]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(!result.markdown.contains("hidden content"));
        assert!(
            !result.markdown.contains("nested dotfile content"),
            "subdirectory dotfiles must be filtered by basename"
        );
        assert!(!result.markdown.contains("mac metadata"));
        assert!(result.markdown.contains("visible content"));
    }

    #[tokio::test]
    async fn convert_zip_sanitizes_heading_labels() {
        // Attacker-controlled entry names must not inject markdown structure.
        let converter = ZipConverter::with_arc(registry_with_text());
        let zip_data = create_zip_bytes(&[("evil\n# INJECTED.txt", b"body")]);
        let result = converter
            .convert(Input::bytes(zip_data, "archive.zip"))
            .await
            .unwrap();
        assert!(
            !result.markdown.contains("\n# INJECTED"),
            "newline must be stripped from the heading label: {}",
            result.markdown
        );
        assert!(result.markdown.contains("evil # INJECTED.txt"));
    }

    #[tokio::test]
    async fn convert_zip_metadata_counts() {
        let converter = ZipConverter::with_arc(registry_with_text());
        let zip_data = create_zip_bytes(&[("a.txt", b"aaa"), ("b.txt", b"bbb")]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert_eq!(result.metadata["file_count"], 2);
        assert_eq!(result.metadata["converted_count"], 2);
    }

    #[tokio::test]
    async fn convert_zip_no_converter_keeps_members() {
        // No engine registered: text members degrade to fenced blocks and
        // binary members to attachments — never dropped.
        let converter = ZipConverter::new(ConverterRegistry::new());
        let zip_data = create_zip_bytes(&[("data.txt", b"some text")]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(
            result.markdown.contains("data.txt"),
            "member name must appear: {}",
            result.markdown
        );
        assert!(
            result.markdown.contains("some text"),
            "member text must survive: {}",
            result.markdown
        );
        assert_eq!(result.metadata["converted_count"], 0);
    }

    #[tokio::test]
    async fn convert_zip_binary_member_becomes_attachment() {
        let converter = ZipConverter::new(ConverterRegistry::new());
        let zip_data = create_zip_bytes(&[("blob.bin", &[0xFFu8, 0xFE, 0x00])]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(
            result.markdown.contains("Attachment"),
            "{}",
            result.markdown
        );
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].filename, "blob.bin");
        assert_eq!(result.assets[0].bytes, vec![0xFF, 0xFE, 0x00]);
    }

    #[test]
    fn extract_zip_contents_invalid_data() {
        assert!(extract_zip_contents(b"this is not a zip file", None).is_err());
    }

    #[test]
    fn extract_zip_contents_multiple_files() {
        let zip_data = create_zip_bytes(&[("file1.txt", b"content1"), ("file2.txt", b"content2")]);
        let extracted = extract_zip_contents(&zip_data, None).unwrap();
        assert_eq!(extracted.files.len(), 2);
        assert!(extracted.oversize.is_empty());
        assert_eq!(extracted.files[0].0, "file1.txt");
    }

    #[test]
    fn extract_zip_contents_oversize_skips_without_reading() {
        // Declared size over the ceiling: reported as oversize, no content.
        let big = vec![0u8; 64];
        let zip_data = create_zip_bytes(&[("big.txt", &big), ("ok.txt", b"tiny")]);
        let extracted = extract_zip_contents(&zip_data, Some(16)).unwrap();
        assert_eq!(extracted.oversize, vec!["big.txt".to_string()]);
        assert_eq!(extracted.files.len(), 1);
        assert_eq!(extracted.files[0].0, "ok.txt");
    }
}

#[tokio::test]
async fn cumulative_bomb_aborts() {
    // Two 600MB-of-zeros entries: each passes the per-entry check (no
    // max_entry_size), the aggregate 1 GiB ceiling must abort.
    let zeros = vec![0u8; 600 * 1024 * 1024];
    let mut buf = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        use std::io::Write as _;
        for name in ["a.bin", "b.bin"] {
            writer.start_file(name, options).unwrap();
            writer.write_all(&zeros).unwrap();
        }
        writer.finish().unwrap();
    }
    let converter = ZipConverter::with_arc(Arc::new(ConverterRegistry::new()));
    let err = converter
        .convert(Input::bytes(buf.into_inner(), "bomb.zip"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("cumulative extracted size"),
        "{err}"
    );
}

#[test]
fn converter_name_is_zip() {
    assert_eq!(ZipConverter::new(ConverterRegistry::new()).name(), "zip");
}

#[tokio::test]
async fn unreadable_input_errors() {
    let converter = ZipConverter::new(ConverterRegistry::new());
    let err = converter
        .convert(Input::Path(std::path::PathBuf::from(
            "/nonexistent/bomb.zip",
        )))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("ZIP"), "{err:?}");
}
