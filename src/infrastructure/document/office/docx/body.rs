// SPDX-License-Identifier: Apache-2.0
//! DOCX body parsing: the `word/document.xml` event stream.
//!
//! Handles paragraphs (with the style cascade applied), runs, hyperlinks,
//! notes references, drawings (image assets), and tables (grid spans and
//! vertical merges).

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::domain::document::{
    Block, Cell, CellSlot, ImageSource, Inline, LinkTarget, Table, TableKind,
};
use crate::domain::error::AlchemizeError;

use super::styles::{RunProps, StyleTable, attr_value, has_attr_false};

/// Body parsing context: style cascade, relationships, and media access.
pub struct BodyCtx<'a> {
    /// Resolved style cascade.
    pub style_table: &'a StyleTable,
    /// Relationship id → target.
    pub rels: &'a HashMap<String, String>,
    /// Counters surfaced into metadata.
    pub warnings: &'a mut super::converter::Warnings,
    /// Media payloads collected so far.
    pub assets: Vec<super::converter::RawAsset>,
    /// Cumulative media bytes; capped by the shared output budget so many
    /// embedded parts cannot sum unboundedly.
    pub assets_bytes: usize,
    /// Package-relative part → index into `assets`.
    pub asset_index: HashMap<String, usize>,
    /// The DOCX package.
    pub zip: &'a mut zip::ZipArchive<Cursor<&'a [u8]>>,
}

impl BodyCtx<'_> {
    /// Map a relationship id to an asset index, reading media on demand.
    pub fn asset_for(&mut self, rel_id: &str) -> Option<usize> {
        let target = self.rels.get(rel_id)?;
        let part = normalize_target(target);
        if let Some(idx) = self.asset_index.get(&part) {
            return Some(*idx);
        }
        let bytes = read_part(self.zip, &part).ok()?;
        if self.assets_bytes.saturating_add(bytes.len())
            > crate::infrastructure::document::shared::MAX_OUTPUT_BYTES
        {
            // Budget exhausted: degrade the reference instead of growing
            // memory without bound.
            return None;
        }
        self.assets_bytes += bytes.len();
        let media_type = guess_media_type(&part);
        let idx = self.assets.len();
        self.asset_index.insert(part.clone(), idx);
        self.assets.push(super::converter::RawAsset {
            part,
            media_type,
            bytes,
        });
        Some(idx)
    }
}

/// Classify a relationship target: absolute URLs stay external, everything
/// else is resolved against the `word/` package root.
pub fn classify_target(target: &str) -> LinkTarget {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("ftp://")
    {
        LinkTarget::External(target.to_string())
    } else {
        LinkTarget::Relative(normalize_target(target))
    }
}

/// Resolve a relationship target against the `word/` package root.
///
/// Relationship targets are relative to the part's folder (`word/`), may
/// start with `/` (package root) and may carry a `../` prefix.
pub fn normalize_target(target: &str) -> String {
    let trimmed = target.strip_prefix('/').unwrap_or(target);
    if trimmed.starts_with("word/") {
        return trimmed.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    let mut escaped_word_root = false;
    for seg in trimmed.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    // Popping past the word/ root: the target escapes the
                    // `word/` folder entirely (e.g. "../customXml/x.xml").
                    escaped_word_root = true;
                }
            }
            other => parts.push(other),
        }
    }
    if escaped_word_root {
        parts.join("/")
    } else {
        format!("word/{}", parts.join("/"))
    }
}

/// Guess a media type from the part extension.
pub fn guess_media_type(part: &str) -> String {
    let ext = part.rsplit('.').next().unwrap_or("bin");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "emf" | "wmf" => "image/x-emf",
        "tiff" | "tif" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Read one part out of the package (bounded by the shared per-part limit).
pub fn read_part(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, AlchemizeError> {
    let mut file = zip
        .by_name(name)
        .map_err(|e| AlchemizeError::Zip(format!("part {name} not found: {e}")))?;
    crate::infrastructure::document::shared::read_part_limited(&mut file, name)
}

/// Vertical merge state of a table cell.
#[derive(Clone, Copy, PartialEq)]
pub enum VMerge {
    /// The cell starts a vertical merge (spans the rows below).
    Restart,
    /// The cell continues a merge started above.
    Continue,
}

/// Parse `word/document.xml` into blocks.
pub fn parse_body(xml: &[u8], ctx: &mut BodyCtx) -> Result<Vec<Block>, AlchemizeError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    // Paragraph accumulation state.
    let mut in_paragraph = false;
    let mut para_inlines: Vec<Inline> = Vec::new();
    let mut para_outline: Option<u8> = None;
    let mut para_props = RunProps::default();
    let mut run_props = RunProps::default();
    let mut in_heading = false;
    let mut heading_level: Option<u8> = None;
    let mut in_run = false;
    let mut in_rpr = false;
    let mut run_char_style: Option<String> = None;
    // Nested hyperlink buffers: (target, collected content).
    let mut link_stack: Vec<(LinkTarget, Vec<Inline>)> = Vec::new();

    // Table state.
    let mut table_grid: Vec<Vec<CellSlot>> = Vec::new();
    let mut row_cells: Vec<CellSlot> = Vec::new();
    let mut cell_blocks: Vec<Block> = Vec::new();
    let mut in_table = false;
    let mut in_cell = false;
    let mut cell_span: u32 = 1;
    let mut cell_vmerge: Option<VMerge> = None;
    let mut cell_col: usize = 0;
    let mut row_index: usize = 0;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "r" if in_paragraph => {
                        in_run = true;
                        run_props = para_props;
                        run_char_style = None;
                    }
                    "rPr" if in_run => in_rpr = true,
                    "rStyle" if in_rpr => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            run_char_style = Some(v);
                        }
                    }
                    "p" => {
                        in_paragraph = true;
                        para_inlines.clear();
                        para_outline = None;
                        para_props = ctx.style_table.defaults;
                        run_props = para_props;
                        link_stack.clear();
                        in_heading = false;
                        heading_level = None;
                    }
                    "pStyle" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            let (props, outline, heading) = ctx.style_table.resolve_paragraph(&v);
                            para_props = props;
                            run_props = props;
                            if let Some(lvl) = outline {
                                para_outline = Some(lvl);
                                in_heading = true;
                            }
                            if let Some(lvl) = heading {
                                heading_level = Some(lvl);
                                in_heading = true;
                            }
                        }
                    }
                    "outlineLvl" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(lvl) = v.trim().parse::<u8>()
                        {
                            para_outline = Some(lvl);
                            in_heading = true;
                        }
                    }
                    "b" if in_rpr => {
                        run_props.bold = Some(!has_attr_false(&e));
                    }
                    "i" if in_rpr => {
                        run_props.italic = Some(!has_attr_false(&e));
                    }
                    "sz" if in_rpr => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(hp) = v.trim().parse::<u32>()
                        {
                            run_props.size_pt = Some(hp as f32 / 2.0);
                        }
                    }
                    "tbl" => {
                        in_table = true;
                        table_grid.clear();
                        row_index = 0;
                    }
                    "tr" if in_table => {
                        row_cells.clear();
                        cell_col = 0;
                    }
                    "tc" if in_table => {
                        in_cell = true;
                        cell_blocks.clear();
                        cell_span = 1;
                        cell_vmerge = None;
                    }
                    "gridSpan" if in_cell => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            cell_span = v.trim().parse().unwrap_or(1);
                        }
                    }
                    "vMerge" if in_cell => {
                        let val = attr_value(&e, "w:val");
                        cell_vmerge = Some(match val.as_deref() {
                            Some("restart") => VMerge::Restart,
                            _ => VMerge::Continue,
                        });
                    }
                    "hyperlink" if in_paragraph => {
                        // Hyperlink: collect the children into the link body.
                        let rid = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref().ends_with("id"))
                            .map(|a| a.value.clone().into_owned());
                        let target = rid
                            .as_ref()
                            .and_then(|rid| ctx.rels.get(rid))
                            .map(|t| classify_target(t));
                        let link_target =
                            target.unwrap_or_else(|| LinkTarget::Relative(String::new()));
                        link_stack.push((link_target, Vec::new()));
                    }
                    "blip" => {
                        if let Some(idx) = blip_asset(&e, ctx) {
                            para_inlines.push(Inline::Image {
                                alt: String::new(),
                                source: ImageSource::Asset(idx),
                            });
                        }
                    }
                    "oMath" => {
                        // Table-driven OMML → LaTeX; unknown fragments fall
                        // back to \text{} and bump the counter (no silent loss).
                        let (latex, used_fallback) =
                            crate::infrastructure::document::office::omml::convert_island_pub(
                                &mut reader,
                            );
                        if used_fallback {
                            ctx.warnings.omml_fallback += 1;
                        }
                        if !latex.trim().is_empty() {
                            para_inlines.push(Inline::Math(latex));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::GeneralRef(r)) if in_paragraph => {
                let text = crate::infrastructure::document::decode_entity(r.as_ref());
                if !text.is_empty() {
                    let mut props = run_props;
                    if let Some(style_id) = &run_char_style {
                        props = props.merge(ctx.style_table.resolve_character(style_id));
                    }
                    let inline = Inline::Text {
                        text,
                        style: props.to_style(),
                    };
                    match link_stack.last_mut() {
                        Some((_, body)) => body.push(inline),
                        None => para_inlines.push(inline),
                    }
                }
            }
            Ok(Event::Text(t)) if in_paragraph => {
                let text = String::from_utf8_lossy(t.as_bytes()).into_owned();
                // Pretty-printed XML puts indentation between elements; a
                // whitespace-only run carries no content and would otherwise
                // leak newlines into headings and cells.
                if text.trim().is_empty() {
                    buf.clear();
                    continue;
                }
                let mut props = run_props;
                if let Some(style_id) = &run_char_style {
                    props = props.merge(ctx.style_table.resolve_character(style_id));
                }
                let inline = Inline::Text {
                    text,
                    style: props.to_style(),
                };
                match link_stack.last_mut() {
                    Some((_, body)) => body.push(inline),
                    None => para_inlines.push(inline),
                }
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "hyperlink" => {
                        if let Some((target, body)) = link_stack.pop() {
                            let link = Inline::Link {
                                content: body,
                                target,
                            };
                            match link_stack.last_mut() {
                                Some((_, parent)) => parent.push(link),
                                None => para_inlines.push(link),
                            }
                        }
                    }
                    "rPr" => in_rpr = false,
                    "r" => {
                        in_run = false;
                        run_props = para_props;
                        run_char_style = None;
                    }
                    "p" => {
                        in_paragraph = false;
                        if in_cell {
                            if !para_inlines.is_empty() {
                                cell_blocks
                                    .push(Block::Paragraph(std::mem::take(&mut para_inlines)));
                            }
                        } else if !para_inlines.is_empty() {
                            let inlines = std::mem::take(&mut para_inlines);
                            if in_heading {
                                let lvl = heading_level
                                    .or_else(|| para_outline.map(|l| l + 1))
                                    .unwrap_or(2)
                                    .clamp(1, 6);
                                blocks.push(Block::Heading {
                                    level: lvl,
                                    content: inlines,
                                });
                            } else {
                                blocks.push(Block::Paragraph(inlines));
                            }
                        }
                    }
                    "tc" if in_table => {
                        in_cell = false;
                        match cell_vmerge {
                            // Continuation only makes sense below a restart row.
                            // A malformed first-row continuation keeps its own
                            // content instead of silently becoming covered.
                            Some(VMerge::Continue) if row_index > 0 => {
                                // Vertical merge continuation: the covered
                                // position points back at the origin cell.
                                row_cells.push(CellSlot::Covered {
                                    origin_row: row_index - 1,
                                    origin_col: cell_col,
                                });
                            }
                            _ => {
                                row_cells.push(CellSlot::Origin(Cell {
                                    blocks: std::mem::take(&mut cell_blocks),
                                    col_span: cell_span,
                                    row_span: 1,
                                }));
                            }
                        }
                        cell_col += cell_span as usize;
                    }
                    "tr" if in_table => {
                        table_grid.push(std::mem::take(&mut row_cells));
                        row_index += 1;
                    }
                    "tbl" => {
                        in_table = false;
                        let grid = std::mem::take(&mut table_grid);
                        blocks.push(Block::Table(Table {
                            grid,
                            header_rows: 1,
                            kind: TableKind::Data,
                        }));
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                // Self-closing attribute carriers (pStyle/outlineLvl/gridSpan…):
                // dispatch through the same handler as Start.
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "pStyle" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            let (props, outline, heading) = ctx.style_table.resolve_paragraph(&v);
                            para_props = props;
                            run_props = props;
                            if let Some(lvl) = outline {
                                para_outline = Some(lvl);
                                in_heading = true;
                            }
                            if let Some(lvl) = heading {
                                heading_level = Some(lvl);
                                in_heading = true;
                            }
                        }
                    }
                    "outlineLvl" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(lvl) = v.trim().parse::<u8>()
                        {
                            para_outline = Some(lvl);
                            in_heading = true;
                        }
                    }
                    "gridSpan" if in_cell => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            cell_span = v.trim().parse().unwrap_or(1);
                        }
                    }
                    "vMerge" if in_cell => {
                        let val = attr_value(&e, "w:val");
                        cell_vmerge = Some(match val.as_deref() {
                            Some("restart") => VMerge::Restart,
                            _ => VMerge::Continue,
                        });
                    }
                    "b" if in_rpr => {
                        run_props.bold = Some(!has_attr_false(&e));
                    }
                    "i" if in_rpr => {
                        run_props.italic = Some(!has_attr_false(&e));
                    }
                    "sz" if in_rpr => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(hp) = v.trim().parse::<u32>()
                        {
                            run_props.size_pt = Some(hp as f32 / 2.0);
                        }
                    }
                    "footnoteReference" | "endnoteReference" => {
                        // Note reference: the id links to the notes part.
                        if let Some(v) = attr_value(&e, "w:id")
                            && v != "0"
                            && v != "-1"
                        {
                            para_inlines.push(Inline::NoteRef(v));
                        }
                    }
                    "br" if in_paragraph => {
                        para_inlines.push(Inline::LineBreak);
                    }
                    "tab" if in_paragraph => {
                        para_inlines.push(Inline::Text {
                            text: "\t".into(),
                            style: run_props.to_style(),
                        });
                    }
                    "blip" => {
                        if let Some(idx) = blip_asset(&e, ctx) {
                            para_inlines.push(Inline::Image {
                                alt: String::new(),
                                source: ImageSource::Asset(idx),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(AlchemizeError::Xml(format!(
                    "document.xml parse error: {e}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(blocks)
}

/// Resolve a `<a:blip r:embed="…"/>` element to an asset index.
fn blip_asset(e: &quick_xml::events::BytesStart, ctx: &mut BodyCtx) -> Option<usize> {
    let rid = e
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref().ends_with("embed"))
        .map(|a| a.value.clone().into_owned())?;
    ctx.asset_for(&rid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_targets_normalize_against_word_root() {
        assert_eq!(
            normalize_target("media/image1.png"),
            "word/media/image1.png"
        );
        assert_eq!(normalize_target("/word/media/a.png"), "word/media/a.png");
        assert_eq!(
            normalize_target("../customXml/item1.xml"),
            "customXml/item1.xml"
        );
    }

    #[test]
    fn classify_target_keeps_absolute_urls_external() {
        for target in [
            "http://x.test/a",
            "HTTPS://x.test/a",
            "mailto:a@b.c",
            "ftp://x.test/f",
        ] {
            assert!(
                matches!(classify_target(target), LinkTarget::External(ref t) if t == target),
                "{target} must stay external"
            );
        }
        assert_eq!(
            classify_target("media/pic.png"),
            LinkTarget::Relative("word/media/pic.png".into())
        );
        assert_eq!(
            classify_target("../customXml/x.xml"),
            LinkTarget::Relative("customXml/x.xml".into())
        );
    }

    #[test]
    fn normalize_target_handles_dots_and_double_slashes() {
        assert_eq!(normalize_target("./media/a.png"), "word/media/a.png");
        assert_eq!(normalize_target("media//a.png"), "word/media/a.png");
        assert_eq!(normalize_target("/media/a.png"), "word/media/a.png");
        assert_eq!(normalize_target("../a"), "a");
    }

    #[test]
    fn media_type_guessing() {
        assert_eq!(guess_media_type("word/media/a.png"), "image/png");
        assert_eq!(guess_media_type("word/media/a.jpeg"), "image/jpeg");
        assert_eq!(
            guess_media_type("word/media/a.bin"),
            "application/octet-stream"
        );
        for (part, expected) in [
            ("word/media/a.gif", "image/gif"),
            ("word/media/a.bmp", "image/bmp"),
            ("word/media/a.svg", "image/svg+xml"),
            ("word/media/a.webp", "image/webp"),
            ("word/media/a.wmf", "image/x-emf"),
            ("word/media/a.tif", "image/tiff"),
            ("no_extension", "application/octet-stream"),
        ] {
            assert_eq!(guess_media_type(part), expected, "{part}");
        }
    }
}
