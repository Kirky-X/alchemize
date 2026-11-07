// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! AnydocConverter — feature-gated wrapper around the `anydoc` crate
//! (firecrawl/anydoc, MIT).
//!
//! Office formats to GitHub-Flavored Markdown across anydoc's full
//! extension table (legacy binary Office included). Runs conversion inside
//! `catch_unwind` on the blocking pool: anydoc may panic on malformed
//! input, which surfaces as a converter error instead of a process abort.

#![cfg(feature = "anydoc")]

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// anydoc's full extension table (20 extensions).
const SUPPORTED_EXTS: &[&str] = &[
    "doc", "docx", "docm", "odt", "pdf", "pptx", "pptm", "ppsx", "ppsm", "ppt", "pps", "pot",
    "rtf", "epub", "xlsx", "xlsm", "xlsb", "xls", "ods", "odp", "csv",
];

/// Converter backed by the `anydoc` crate.
pub struct AnydocConverter;

impl AnydocConverter {
    /// Create a new anydoc converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AnydocConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for AnydocConverter {
    fn name(&self) -> &'static str {
        "anydoc"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = match input.to_bytes().await {
            Ok(b) => b,
            Err(e) => return Err(AlchemizeError::file_read(input_path(&input), io_of(&e))),
        };

        // Panic protection: anydoc may panic on malformed input.
        let outcome = tokio::task::spawn_blocking(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // PDF bypasses the document model (no assets to extract).
                let doc = anydoc::to_document(&bytes, None)?;
                let mut doc = doc;
                let assets = rewrite_asset_references(&mut doc);
                let markdown =
                    crate::infrastructure::document::anydoc_render::render_markdown(&doc);
                Ok::<_, anydoc::ConvertError>((markdown, assets))
            }))
        })
        .await
        .map_err(|e| AlchemizeError::other(format!("anydoc spawn_blocking panicked: {e}")))?;

        let (markdown, assets) = outcome
            .map_err(|_| AlchemizeError::other("anydoc panicked on malformed input"))?
            .map_err(|e| AlchemizeError::other(format!("anydoc conversion failed: {e}")))?;

        let markdown = if markdown.trim().is_empty() {
            "*Document contains no extractable content.*\n".to_string()
        } else {
            markdown
        };

        let metadata = serde_json::json!({
            "source_format": input.extension().unwrap_or_else(|| "unknown".into()),
            "asset_count": assets.len(),
        });

        let mut result = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in assets {
            result = result.with_asset(asset);
        }
        Ok(result)
    }

    fn supported_extensions(&self) -> &[&str] {
        SUPPORTED_EXTS
    }

    fn priority(&self) -> u8 {
        15
    }
}

/// Rewrite `ImageSource::Asset(id)` references into predicted file
/// references (`images/{seq:04}-{sha8}.{ext}`) in document order, returning
/// the assets in that same order. Identical asset ids reuse the first
/// reference. The filenames in the returned assets are the final on-disk
/// names; the CLI writes them under `<output>/images/`.
fn rewrite_asset_references(doc: &mut anydoc::model::Document) -> Vec<crate::domain::asset::Asset> {
    use anydoc::model::{Block, CellSlot, ImageSource, Inline};
    use std::collections::HashMap;

    let mut refs: HashMap<usize, String> = HashMap::new();
    let mut collected: Vec<crate::domain::asset::Asset> = Vec::new();

    // Mirror of the sink naming rule: {seq:04}-{sha8}.{ext} under images/.
    let reference_for = |asset: &anydoc::model::Asset, seq: usize| -> String {
        let sha8 = {
            use sha2::{Digest as _, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&asset.bytes);
            let digest = hasher.finalize();
            digest[..4]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let ext = asset
            .media_type
            .rsplit('/')
            .next()
            .map(|e| e.replace('+', "-"))
            .unwrap_or_else(|| "bin".into());
        format!("images/{seq:04}-{sha8}.{ext}")
    };

    // First pass (immutable): collect assets in document order is impossible
    // without walking; instead assign references during the mutable walk by
    // pre-indexing assets that will be encountered. Simpler correct approach:
    // assign every doc asset a deterministic reference keyed by first-use
    // order discovered during the walk; unused assets are dropped.
    let mut seq_counter = 0usize;

    fn walk_blocks(
        blocks: &mut Vec<Block>,
        refs: &mut HashMap<usize, String>,
        collected: &mut Vec<crate::domain::asset::Asset>,
        doc_assets: &[anydoc::model::Asset],
        seq_counter: &mut usize,
        reference_for: &dyn Fn(&anydoc::model::Asset, usize) -> String,
    ) {
        for block in blocks.iter_mut() {
            match block {
                Block::Heading { content, .. } | Block::Paragraph(content) => {
                    walk_inlines(
                        content,
                        refs,
                        collected,
                        doc_assets,
                        seq_counter,
                        reference_for,
                    );
                }
                Block::List(list) => {
                    for item in list.items.iter_mut() {
                        walk_blocks(
                            &mut item.blocks,
                            refs,
                            collected,
                            doc_assets,
                            seq_counter,
                            reference_for,
                        );
                    }
                }
                Block::Table(table) => {
                    for row in table.grid.iter_mut() {
                        for slot in row.iter_mut() {
                            if let CellSlot::Origin(cell) = slot {
                                walk_blocks(
                                    &mut cell.blocks,
                                    refs,
                                    collected,
                                    doc_assets,
                                    seq_counter,
                                    reference_for,
                                );
                            }
                        }
                    }
                }
                Block::BlockQuote(nested) => {
                    walk_blocks(
                        nested,
                        refs,
                        collected,
                        doc_assets,
                        seq_counter,
                        reference_for,
                    );
                }
                Block::CodeBlock { .. } | Block::Rule | Block::Math(_) => {}
            }
        }
    }

    fn walk_inlines(
        inlines: &mut Vec<Inline>,
        refs: &mut HashMap<usize, String>,
        collected: &mut Vec<crate::domain::asset::Asset>,
        doc_assets: &[anydoc::model::Asset],
        seq_counter: &mut usize,
        reference_for: &dyn Fn(&anydoc::model::Asset, usize) -> String,
    ) {
        for inline in inlines.iter_mut() {
            match inline {
                Inline::Image { source, .. } => {
                    let asset_idx = match source {
                        ImageSource::Asset(id) => id.0,
                        _ => continue,
                    };
                    if !refs.contains_key(&asset_idx) {
                        let Some(doc_asset) = doc_assets.get(asset_idx) else {
                            continue;
                        };
                        let reference = reference_for(doc_asset, *seq_counter);
                        let filename = reference.trim_start_matches("images/").to_string();
                        collected.push(crate::domain::asset::Asset::new(
                            filename,
                            doc_asset.bytes.clone(),
                        ));
                        refs.insert(asset_idx, reference);
                        *seq_counter += 1;
                    }
                    let reference = refs[&asset_idx].clone();
                    *source = ImageSource::External(reference);
                }
                Inline::Link { content, .. } => {
                    walk_inlines(
                        content,
                        refs,
                        collected,
                        doc_assets,
                        seq_counter,
                        reference_for,
                    );
                }
                _ => {}
            }
        }
    }

    // Notes may also reference assets.
    let mut notes = std::mem::take(&mut doc.notes);
    for note in notes.iter_mut() {
        walk_blocks(
            &mut note.blocks,
            &mut refs,
            &mut collected,
            &doc.assets,
            &mut seq_counter,
            &reference_for,
        );
    }
    doc.notes = notes;

    let mut blocks = std::mem::take(&mut doc.blocks);
    walk_blocks(
        &mut blocks,
        &mut refs,
        &mut collected,
        &doc.assets,
        &mut seq_counter,
        &reference_for,
    );
    doc.blocks = blocks;

    collected
}

/// Best-effort path string for error context.
fn input_path(input: &Input) -> String {
    match input {
        Input::Path(p) => p.display().to_string(),
        Input::Bytes { filename, .. } => filename.clone(),
    }
}

/// Coerce an anyhow error into a plain io::Error for the FileRead variant.
fn io_of(e: &anyhow::Error) -> std::io::Error {
    match e.downcast_ref::<std::io::Error>() {
        Some(io) => std::io::Error::new(io.kind(), io.to_string()),
        None => std::io::Error::other(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_full_table() {
        let converter = AnydocConverter::new();
        for ext in [
            "doc", "docm", "pps", "pot", "xlsm", "xlsb", "odp", "rtf", "pdf",
        ] {
            assert!(
                converter.supported_extensions().contains(&ext),
                "missing {ext}"
            );
        }
    }

    #[test]
    fn priority_above_native_office() {
        assert_eq!(AnydocConverter::new().priority(), 15);
    }

    #[tokio::test]
    async fn malformed_zip_errors_not_panics() {
        let converter = AnydocConverter::new();
        // Truncated zip: must return Err, never abort the process.
        let input = Input::bytes(vec![0x50, 0x4B, 0x03, 0x04, 0x00], "broken.docx");
        let result = converter.convert(input).await;
        assert!(result.is_err());
    }
}
