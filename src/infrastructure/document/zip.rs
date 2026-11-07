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

        for (filename, content) in extracted_files {
            let nested_input = Input::Bytes {
                data: content,
                filename: filename.clone(),
                mimetype: None,
                extension: None,
            };

            if let Ok(result) = self.registry.convert(&nested_input).await {
                markdown_parts.push(format!("### {filename}\n"));
                markdown_parts.push(result.markdown);
                markdown_parts.push(String::new());
                converted_count += 1;
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
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["zip"]
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
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

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
        registry.register(Box::new(crate::infrastructure::document::text::TextConverter::new()));
        Arc::new(registry)
    }

    #[test]
    fn supported_extensions() {
        let converter = ZipConverter::new(ConverterRegistry::new());
        assert_eq!(converter.supported_extensions(), &["zip"]);
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
    async fn convert_zip_no_converter_registered() {
        let converter = ZipConverter::new(ConverterRegistry::new());
        let zip_data = create_zip_bytes(&[("data.txt", b"some text")]);
        let input = Input::bytes(zip_data, "archive.zip");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("no convertible files"));
    }

    #[test]
    fn extract_zip_contents_invalid_data() {
        assert!(extract_zip_contents(b"this is not a zip file").is_err());
    }

    #[test]
    fn extract_zip_contents_multiple_files() {
        let zip_data = create_zip_bytes(&[
            ("file1.txt", b"content1"),
            ("file2.txt", b"content2"),
        ]);
        let files = extract_zip_contents(&zip_data).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "file1.txt");
    }
}
