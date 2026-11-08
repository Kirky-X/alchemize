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
                let bridged = bridge_to_selfhost(doc);
                let markdown =
                    crate::infrastructure::document::gfm_render::render_markdown(&bridged);
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

/// Bridge the anydoc model to the self-hosted document model.
///
/// Transitional (deleted with the anydoc retirement): maps every variant
/// one-to-one; asset references are already rewritten to External by then.
fn bridge_to_selfhost(doc: anydoc::model::Document) -> crate::domain::document::Document {
    use crate::domain::document as dm;

    fn map_blocks(blocks: Vec<anydoc::model::Block>) -> Vec<dm::Block> {
        blocks
            .into_iter()
            .map(|b| match b {
                anydoc::model::Block::Heading { level, content, .. } => dm::Block::Heading {
                    level,
                    content: map_inlines(content),
                },
                anydoc::model::Block::Paragraph(i) => dm::Block::Paragraph(map_inlines(i)),
                anydoc::model::Block::List(l) => dm::Block::List(dm::List {
                    marker: match l.marker {
                        anydoc::model::MarkerKind::Bullet => dm::MarkerKind::Bullet,
                        anydoc::model::MarkerKind::Decimal => dm::MarkerKind::Decimal,
                        anydoc::model::MarkerKind::LowerAlpha => dm::MarkerKind::LowerAlpha,
                        anydoc::model::MarkerKind::UpperAlpha => dm::MarkerKind::UpperAlpha,
                        anydoc::model::MarkerKind::LowerRoman => dm::MarkerKind::LowerRoman,
                        anydoc::model::MarkerKind::UpperRoman => dm::MarkerKind::UpperRoman,
                    },
                    start: l.start,
                    items: l
                        .items
                        .into_iter()
                        .map(|it| dm::ListItem {
                            blocks: map_blocks(it.blocks),
                        })
                        .collect(),
                }),
                anydoc::model::Block::Table(t) => dm::Block::Table(map_table(t)),
                anydoc::model::Block::BlockQuote(nested) => {
                    dm::Block::BlockQuote(map_blocks(nested))
                }
                anydoc::model::Block::CodeBlock { lang, text } => {
                    dm::Block::CodeBlock { lang, text }
                }
                anydoc::model::Block::Rule => dm::Block::Rule,
                anydoc::model::Block::Math(tex) => dm::Block::Math(tex),
            })
            .collect()
    }

    fn map_table(t: anydoc::model::Table) -> dm::Table {
        dm::Table {
            grid: t
                .grid
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|slot| match slot {
                            anydoc::model::CellSlot::Origin(c) => dm::CellSlot::Origin(dm::Cell {
                                blocks: map_blocks(c.blocks),
                                col_span: c.col_span,
                                row_span: c.row_span,
                            }),
                            anydoc::model::CellSlot::Covered { origin_row, origin_col } => {
                                dm::CellSlot::Covered { origin_row, origin_col }
                            }
                        })
                        .collect()
                })
                .collect(),
            header_rows: t.header_rows,
            kind: match t.kind {
                anydoc::model::TableKind::Data => dm::TableKind::Data,
                anydoc::model::TableKind::Layout => dm::TableKind::Layout,
            },
        }
    }

    fn map_inlines(inlines: Vec<anydoc::model::Inline>) -> Vec<dm::Inline> {
        inlines
            .into_iter()
            .map(|i| match i {
                anydoc::model::Inline::Text { text, style } => dm::Inline::Text {
                    text,
                    style: dm::Style {
                        bold: style.bold,
                        italic: style.italic,
                        code: style.code,
                        strike: style.strike,
                    },
                },
                anydoc::model::Inline::Link { content, target } => dm::Inline::Link {
                    content: map_inlines(content),
                    target: match target {
                        anydoc::model::LinkTarget::External(u) => dm::LinkTarget::External(u),
                        anydoc::model::LinkTarget::Relative(u) => dm::LinkTarget::Relative(u),
                        anydoc::model::LinkTarget::Anchor(id) => {
                            dm::LinkTarget::Anchor(format!("{id:?}"))
                        }
                    },
                },
                anydoc::model::Inline::Image { alt, source } => dm::Inline::Image {
                    alt,
                    source: match source {
                        anydoc::model::ImageSource::External(u) => dm::ImageSource::External(u),
                        anydoc::model::ImageSource::Asset(id) => {
                            dm::ImageSource::Asset(id.0)
                        }
                        anydoc::model::ImageSource::Unavailable => dm::ImageSource::Unavailable,
                    },
                },
                anydoc::model::Inline::Math(tex) => dm::Inline::Math(tex),
                anydoc::model::Inline::Checkbox(c) => dm::Inline::Checkbox(c),
                anydoc::model::Inline::Anchor(id) => dm::Inline::Anchor(format!("{id:?}")),
                anydoc::model::Inline::NoteRef(id) => dm::Inline::NoteRef(id),
                anydoc::model::Inline::LineBreak => dm::Inline::LineBreak,
            })
            .collect()
    }

    dm::Document {
        blocks: map_blocks(doc.blocks),
        notes: doc
            .notes
            .into_iter()
            .map(|n| dm::Note {
                id: n.id,
                kind: match n.kind {
                    anydoc::model::NoteKind::Footnote => dm::NoteKind::Footnote,
                    anydoc::model::NoteKind::Endnote => dm::NoteKind::Endnote,
                },
                blocks: map_blocks(n.blocks),
            })
            .collect(),
        assets: doc
            .assets
            .into_iter()
            .map(|a| dm::Asset {
                media_type: a.media_type,
                origin_part: a.origin_part,
                bytes: a.bytes,
            })
            .collect(),
    }
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
