// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! ZIP archive converter with recursive extraction.

use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use zip::ZipArchive;

use crate::application::ports::Converter;
use crate::application::registry::ConverterRegistry;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts ZIP archives by extracting and converting each contained file.
///
/// Uses the ConverterRegistry to handle each extracted file according to its
/// extension. Files that cannot be converted are skipped (counted in
/// metadata as skipped, never silently dropped from the record).
pub struct ZipConverter {
    registry: Arc<ConverterRegistry>,
}

impl ZipConverter {
    /// Create a new ZIP converter owning a registry for nested handling.
    pub fn new(registry: ConverterRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }

    /// Create with an existing shared registry.
    pub fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        Self { registry }
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

        let extracted_files = extract_zip_contents(&bytes)?;

        let mut markdown_parts = Vec::new();
        let file_count = extracted_files.len();
        let mut converted_count = 0;
        let mut attachments: Vec<String> = Vec::new();
        let mut assets: Vec<crate::domain::asset::Asset> = Vec::new();

        for (filename, content) in extracted_files {
            let nested_input = Input::Bytes {
                data: content.clone(),
                filename: filename.clone(),
                mimetype: None,
                extension: None,
            };

            match self.registry.convert(&nested_input).await {
                Ok(result) => {
                    markdown_parts.push(format!("### {filename}\n"));
                    markdown_parts.push(result.markdown);
                    markdown_parts.push(String::new());
                    converted_count += 1;
                }
                Err(_) => {
                    // No engine for this member: never drop it silently —
                    // text becomes a fenced block, binary stays an attachment.
                    markdown_parts.push(format!("### {filename}\n"));
                    if let Ok(text) = std::str::from_utf8(&content)
                        && !text.trim().is_empty()
                        && !text
                            .chars()
                            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
                    {
                        let ext = filename.rsplit('.').next().unwrap_or("txt");
                        markdown_parts.push(format!("```{ext}\n{}\n```\n", text.trim_end()));
                    } else {
                        markdown_parts.push(format!(
                            "> Attachment: `{filename}` ({} bytes) — unsupported inner format; content preserved as attachment.\n",
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
            "attachments": attachments,
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

/// Extract all file contents from a ZIP archive (synchronous).
fn extract_zip_contents(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, AlchemizeError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::Zip(format!("failed to open ZIP archive: {e}")))?;

    let mut files = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| AlchemizeError::Zip(format!("failed to read ZIP entry {i}: {e}")))?;

        if file.is_dir() {
            continue;
        }

        let filename = file
            .name()
            .map_err(|e| AlchemizeError::Zip(format!("failed to read ZIP entry name {i}: {e}")))?
            .to_string();

        if filename.starts_with('.') || filename.contains("__MACOSX") {
            continue;
        }

        let mut content = Vec::new();
        std::io::copy(&mut file, &mut content)
            .map_err(|e| AlchemizeError::Zip(format!("failed to read file content: {e}")))?;

        files.push((filename, content));
    }

    Ok(files)
}

#[cfg(test)]
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

    #[tokio::test]
    async fn convert_empty_zip() {
        let converter = ZipConverter::new(ConverterRegistry::new());
        let input = Input::bytes(create_zip_bytes(&[]), "empty.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("no convertible files"));
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
            ("__MACOSX/._file", b"mac metadata"),
            ("visible.txt", b"visible content"),
        ]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(!result.markdown.contains("hidden content"));
        assert!(!result.markdown.contains("mac metadata"));
        assert!(result.markdown.contains("visible content"));
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
        assert!(extract_zip_contents(b"this is not a zip file").is_err());
    }

    #[test]
    fn extract_zip_contents_multiple_files() {
        let zip_data = create_zip_bytes(&[("file1.txt", b"content1"), ("file2.txt", b"content2")]);
        let files = extract_zip_contents(&zip_data).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "file1.txt");
    }
}
