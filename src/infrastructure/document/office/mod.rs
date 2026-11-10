//! Self-hosted office-format deep parsers.

pub mod docx;
pub mod odp;
pub mod odt;
pub mod omml;
pub mod pptx_deep;

#[cfg(feature = "libreoffice")]
pub mod legacy_office;

pub use docx::DocxDeepConverter;
#[cfg(feature = "libreoffice")]
pub use legacy_office::{LegacyOfficeConverter, LegacyPptConverter};
pub use odp::OdpConverter;
pub use odt::OdtConverter;
pub use pptx_deep::PptxDeepConverter;

use crate::domain::asset::{Asset, AssetNamer};
use crate::domain::document::{Block, CellSlot, Document, ImageSource, Inline};

/// One raw media payload as parsed from a package part.
pub(crate) struct RawMedia<'a> {
    /// Package-relative part name (used for the file extension).
    pub part: &'a str,
    /// Media type from the package (fallback for the extension).
    pub media_type: &'a str,
    /// Payload bytes.
    pub bytes: &'a [u8],
}

/// Rewrite `ImageSource::Asset(idx)` references into sha8-named file
/// references (`images/{seq:04}-{sha8}.{ext}`), returning the assets in
/// first-reference order. Identical bytes reuse the first reference.
///
/// The CLI writes each returned asset at exactly that filename under the
/// output directory, so the markdown image links resolve.
pub(crate) fn rewrite_image_assets(document: &mut Document, raw: &[RawMedia<'_>]) -> Vec<Asset> {
    use std::collections::HashMap;

    struct Ctx<'a> {
        raw: &'a [RawMedia<'a>],
        by_idx: HashMap<usize, String>,
        namer: AssetNamer,
    }

    impl Ctx<'_> {
        fn reference_for(&mut self, idx: usize) -> Option<String> {
            if let Some(existing) = self.by_idx.get(&idx) {
                return Some(existing.clone());
            }
            let media = self.raw.get(idx)?;
            let ext = asset_ext(media.part, media.media_type);
            let reference = self.namer.reference(media.bytes, &ext);
            self.by_idx.insert(idx, reference.clone());
            Some(reference)
        }
    }

    fn walk_inlines(inlines: &mut [Inline], ctx: &mut Ctx<'_>) {
        for inline in inlines {
            match inline {
                Inline::Image { source, .. } => {
                    if let ImageSource::Asset(idx) = source
                        && let Some(reference) = ctx.reference_for(*idx)
                    {
                        *source = ImageSource::External(reference);
                    }
                }
                Inline::Link { content, .. } => walk_inlines(content, ctx),
                _ => {}
            }
        }
    }

    fn walk_blocks(blocks: &mut [Block], ctx: &mut Ctx<'_>) {
        for block in blocks {
            match block {
                Block::Heading { content, .. } | Block::Paragraph(content) => {
                    walk_inlines(content, ctx);
                }
                Block::List(list) => {
                    for item in &mut list.items {
                        walk_blocks(&mut item.blocks, ctx);
                    }
                }
                Block::Table(table) => {
                    for row in &mut table.grid {
                        for slot in row {
                            if let CellSlot::Origin(cell) = slot {
                                walk_blocks(&mut cell.blocks, ctx);
                            }
                        }
                    }
                }
                Block::BlockQuote(nested) => walk_blocks(nested, ctx),
                Block::CodeBlock { .. } | Block::Rule | Block::Math(_) => {}
            }
        }
    }

    let mut ctx = Ctx {
        raw,
        by_idx: HashMap::new(),
        namer: AssetNamer::new(),
    };
    walk_blocks(&mut document.blocks, &mut ctx);
    for note in &mut document.notes {
        walk_blocks(&mut note.blocks, &mut ctx);
    }
    ctx.namer.into_assets()
}

/// File extension for an extracted asset: the part suffix when present,
/// otherwise the media-type subtype.
fn asset_ext(part: &str, media_type: &str) -> String {
    if let Some((_, ext)) = part.rsplit_once('.')
        && !ext.is_empty()
        && !ext.contains('/')
    {
        return ext.to_lowercase();
    }
    let sub = media_type.rsplit('/').next().unwrap_or("bin");
    if sub.is_empty() || sub == "octet-stream" {
        "bin".to_string()
    } else {
        sub.replace('+', "-")
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::asset::sha8_hex;

    use super::*;
    use crate::domain::document::{Cell, List, ListItem, Note, NoteKind, Style, Table, TableKind};

    fn image(idx: usize) -> Inline {
        Inline::Image {
            alt: String::new(),
            source: ImageSource::Asset(idx),
        }
    }

    fn text(value: &str) -> Inline {
        Inline::Text {
            text: value.into(),
            style: Style::PLAIN,
        }
    }

    #[test]
    fn rewrites_every_container_and_dedupes_identical_bytes() {
        let raw = [
            RawMedia {
                part: "word/media/a.png",
                media_type: "image/png",
                bytes: b"png-bytes",
            },
            RawMedia {
                part: "word/media/raw",
                media_type: "image/svg+xml",
                bytes: b"svg-bytes",
            },
            RawMedia {
                part: "word/media/blob",
                media_type: "application/octet-stream",
                bytes: b"blob",
            },
        ];
        let mut doc = Document {
            blocks: vec![
                Block::Paragraph(vec![text("a"), image(0)]),
                Block::List(List {
                    marker: crate::domain::document::MarkerKind::Decimal,
                    start: 1,
                    items: vec![ListItem {
                        blocks: vec![Block::Paragraph(vec![image(1)])],
                    }],
                }),
                Block::Table(Table {
                    grid: vec![vec![CellSlot::Origin(Cell {
                        blocks: vec![Block::Paragraph(vec![
                            Inline::Link {
                                content: vec![image(0)],
                                target: crate::domain::document::LinkTarget::External(
                                    "https://x.test".into(),
                                ),
                            },
                            image(2),
                        ])],
                        col_span: 1,
                        row_span: 1,
                    })]],
                    header_rows: 1,
                    kind: TableKind::Data,
                }),
                Block::BlockQuote(vec![Block::Paragraph(vec![image(99)])]),
            ],
            notes: vec![Note {
                id: "1".into(),
                kind: NoteKind::Footnote,
                blocks: vec![Block::Paragraph(vec![image(1)])],
            }],
            assets: Vec::new(),
        };
        let assets = rewrite_image_assets(&mut doc, &raw);
        // idx 0 shared (two references), idx 1 deduped by bytes, idx 2 blob.
        assert_eq!(assets.len(), 3, "{:?}", assets);
        assert_eq!(
            assets[0].filename,
            format!("images/0000-{}.png", sha8_hex(b"png-bytes"))
        );
        assert_eq!(
            assets[1].filename,
            format!("images/0001-{}.svg-xml", sha8_hex(b"svg-bytes"))
        );
        assert_eq!(
            assets[2].filename,
            format!("images/0002-{}.bin", sha8_hex(b"blob"))
        );
        // The unresolved index is left untouched (no panic, no phantom asset).
        assert!(matches!(
            doc.blocks[3],
            Block::BlockQuote(ref nested)
                if matches!(&nested[0], Block::Paragraph(inlines)
                    if matches!(inlines[0], Inline::Image { source: ImageSource::Asset(99), .. }))
        ));
        // Markdown now carries resolvable links.
        let md = crate::infrastructure::document::gfm_render::render_markdown(&doc);
        assert!(md.contains("](images/0000-"), "{md}");
    }
}
