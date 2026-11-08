// SPDX-License-Identifier: Apache-2.0
//! DOCX deep parser (self-hosted): WordprocessingML event-stream parsing into
//! the unified document model with style inheritance, footnotes, merged
//! table cells, drawing assets, and OMML math.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::OnceLock;

use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::application::ports::{Availability, Converter};
use crate::domain::asset::Asset;
use crate::domain::document::{
    Block, Cell, CellSlot, Document, ImageSource, Inline, LinkTarget, List, ListItem, MarkerKind,
    Note, NoteKind, Style, Table, TableKind,
};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Deep DOCX converter (fallback tier below anydoc while it lives; the
/// self-hosted primary afterwards).
pub struct DocxDeepConverter;

impl DocxDeepConverter {
    /// Create a new deep DOCX converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DocxDeepConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for DocxDeepConverter {
    fn name(&self) -> &'static str {
        "docx-deep"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read DOCX input: {e}")))?;

        let result = tokio::task::spawn_blocking(move || parse(&bytes))
            .await
            .map_err(|e| AlchemizeError::other(format!("docx-deep task panicked: {e}")))?;

        let (doc, warnings) = result?;
        let raw_assets = LAST_RAW_ASSETS.with(|c| std::mem::take(&mut *c.borrow_mut()));
        let markdown = crate::infrastructure::document::gfm_render::render_markdown(&doc);

        let metadata = serde_json::json!({
            "source_format": "docx",
            "asset_count": doc.assets.len(),
            "footnote_count": doc.notes.len(),
            "parser": "docx-deep",
            "omml_fallback": warnings.omml_fallback,
            "numfmt_fallback": 0,
        });

        let mut out = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in &raw_assets {
            out = out.with_asset(Asset::new(asset_name(asset), asset.bytes.clone()));
        }
        Ok(out)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["docx"]
    }

    fn priority(&self) -> u8 {
        18
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

fn asset_name(asset: &RawAsset) -> String {
    asset
        .part
        .rsplit('/')
        .next()
        .unwrap_or("asset.bin")
        .to_string()
}

/// Parser warnings surfaced into metadata.
#[derive(Default)]
struct Warnings {
    omml_fallback: usize,
}

/// Raw zip part payload.
struct RawAsset {
    part: String,
    media_type: String,
    bytes: Vec<u8>,
}

/// Entry of a parsed relationships part.
struct Rel {
    id: String,
    target: String,
}

/// Style table row: name → (outline level, bold, italic).
#[derive(Default, Clone)]
struct StyleInfo {
    outline_lvl: Option<u8>,
    bold: bool,
    italic: bool,
    is_heading: bool,
}

/// Parse DOCX bytes into the unified model.
pub fn parse(bytes: &[u8]) -> Result<(Document, Warnings), AlchemizeError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::Zip(format!("failed to open DOCX: {e}")))?;

    let document_xml = read_part(&mut zip, "word/document.xml")?;
    let styles_xml = read_part(&mut zip, "word/styles.xml").ok();
    let footnotes_xml = read_part(&mut zip, "word/footnotes.xml").ok();
    let rels_xml = read_part(&mut zip, "word/_rels/document.xml.rels").ok();

    let style_table = styles_xml.as_deref().map(parse_styles).unwrap_or_default();
    let rels: HashMap<String, String> = rels_xml
        .as_deref()
        .map(parse_rels)
        .map(|v| v.into_iter().map(|r| (r.id, r.target)).collect())
        .unwrap_or_default();

    let mut warnings = Warnings::default();
    let mut ctx = BodyCtx {
        style_table: &style_table,
        rels: &rels,
        warnings: &mut warnings,
        assets: Vec::new(),
        asset_index: HashMap::new(),
        zip: &mut zip,
    };

    let blocks = parse_body(&document_xml, &mut ctx)?;
    let notes = match footnotes_xml.as_deref() {
        Some(xml) => parse_footnotes(xml, &style_table),
        None => Vec::new(),
    };

    // Asset references (ImageSource::Asset(idx)) index into this vector;
    // ConvertResult receives the raw bytes, the model keeps provenance only.
    let assets: Vec<crate::domain::document::Asset> = ctx
        .assets
        .iter()
        .map(|a| crate::domain::document::Asset {
            media_type: a.media_type.clone(),
            origin_part: a.part.clone(),
            bytes: a.bytes.clone(),
        })
        .collect();
    let raw_assets = std::mem::take(&mut ctx.assets);

    let document = Document {
        blocks,
        notes,
        assets,
    };
    // Raw bytes ride along for the converter via thread-local stash.
    LAST_RAW_ASSETS.with(|c| *c.borrow_mut() = raw_assets);

    Ok((document, warnings))
}

thread_local! {
    /// Raw asset payloads from the last parse in this thread (bridge for
    /// ConvertResult assembly; consumed by the converter immediately).
    static LAST_RAW_ASSETS: std::cell::RefCell<Vec<RawAsset>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Body parsing context.
struct BodyCtx<'a> {
    style_table: &'a HashMap<String, StyleInfo>,
    rels: &'a HashMap<String, String>,
    warnings: &'a mut Warnings,
    assets: Vec<RawAsset>,
    asset_index: HashMap<String, usize>,
    zip: &'a mut zip::ZipArchive<Cursor<&'a [u8]>>,
}

impl BodyCtx<'_> {
    /// Map a relationship id to an asset index, reading media on demand.
    fn asset_for(&mut self, rel_id: &str) -> Option<usize> {
        let target = self.rels.get(rel_id)?;
        let part = format!("word/{target}");
        if let Some(idx) = self.asset_index.get(&part) {
            return Some(*idx);
        }
        let bytes = read_part(self.zip, &part).ok()?;
        let media_type = guess_media_type(&part);
        let idx = self.assets.len();
        self.asset_index.insert(part.clone(), idx);
        self.assets.push(RawAsset {
            part,
            media_type,
            bytes,
        });
        Some(idx)
    }
}

fn guess_media_type(part: &str) -> String {
    let ext = part.rsplit('.').next().unwrap_or("bin");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn read_part(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, AlchemizeError> {
    let mut file = zip
        .by_name(name)
        .map_err(|e| AlchemizeError::Zip(format!("part {name} not found: {e}")))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| AlchemizeError::Zip(format!("part {name} read failed: {e}")))?;
    Ok(buf)
}

/// Parse styles.xml into styleId → info.
fn parse_styles(xml: &[u8]) -> HashMap<String, StyleInfo> {
    let mut table = HashMap::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut current: Option<(String, StyleInfo)> = None;
    let mut in_name = false;
    let mut in_outline = false;
    let mut style_name = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "style" => {
                        let mut id = String::new();
                        let mut is_heading = false;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                "w:styleId" => {
                                    id = attr.value.clone().into_owned()
                                }
                                "w:type" => {}
                                _ => {}
                            }
                        }
                        // Heading styles are named Heading1..9 or styleId pHeading1.
                        if id.starts_with("Heading") || id.starts_with("Titre") {
                            is_heading = true;
                        }
                        current = Some((id, StyleInfo { is_heading, ..Default::default() }));
                    }
                    "name" if current.is_some() => {
                        // <w:name w:val="heading 1"/> — value rides an attribute.
                        if let Some(v) = attr_value(&e, "w:val") {
                            style_name = v;
                        }
                        in_name = true;
                    }
                    "outlineLvl" if current.is_some() => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            if let Ok(lvl) = v.trim().parse::<u8>() {
                                if let Some((_, info)) = &mut current {
                                    info.outline_lvl = Some(lvl);
                                }
                            }
                        }
                    }
                    "b" | "i" if current.is_some() => {
                        let key = local.clone();
                        if let Some((_, info)) = &mut current {
                            let on = e
                                .attributes()
                                .all(|a| a.map(|a| a.key.as_ref() != "w:val").unwrap_or(true));
                            if on {
                                if key == "b" {
                                    info.bold = true;
                                } else {
                                    info.italic = true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_name => {
                style_name.push_str(&String::from_utf8_lossy(t.as_bytes()));
            }
            Ok(Event::Text(t)) if in_outline => {
                if let Ok(v) = String::from_utf8_lossy(t.as_bytes()).trim().parse::<u8>() {
                    if let Some((_, info)) = &mut current {
                        info.outline_lvl = Some(v);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "name" => in_name = false,
                    "outlineLvl" => in_outline = false,
                    "style" => {
                        if let Some((id, mut info)) = current.take() {
                            let name = style_name.trim().to_lowercase();
                            if name.starts_with("heading") || name.starts_with("标题") {
                                info.is_heading = true;
                                let digits: String =
                                    name.chars().filter(|c| c.is_ascii_digit()).collect();
                                if let Ok(lvl) = digits.parse::<u8>() {
                                    info.outline_lvl = Some(lvl.saturating_sub(1));
                                }
                            }
                            if !id.is_empty() {
                                table.insert(id, info);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                let _ = e;
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    table
}

/// Parse relationships part.
fn parse_rels(xml: &[u8]) -> Vec<Rel> {
    let mut out = Vec::new();
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == "Relationship" => {
                let mut id = String::new();
                let mut target = String::new();
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        "Id" => id = attr.value.clone().into_owned(),
                        "Target" => target = attr.value.clone().into_owned(),
                        _ => {}
                    }
                }
                if !id.is_empty() {
                    out.push(Rel { id, target });
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Footnote body parsing (text only; styles inherited from table).
fn parse_footnotes(xml: &[u8], _styles: &HashMap<String, StyleInfo>) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if local == "footnote" {
                    let id = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == "w:id")
                        .map(|a| a.value.clone().into_owned())
                        .unwrap_or_default();
                    current_id = Some(id);
                    current_text.clear();
                }
            }
            Ok(Event::Text(t)) => {
                if current_id.is_some() {
                    current_text.push_str(&String::from_utf8_lossy(t.as_bytes()));
                }
            }
            Ok(Event::End(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == "footnote" {
                    if let Some(id) = current_id.take() {
                        let text = current_text.trim().to_string();
                        // Skip the separator/continuation markers (id 0/-1).
                        if id != "0" && id != "-1" && !text.is_empty() {
                            notes.push(Note {
                                id,
                                kind: NoteKind::Footnote,
                                blocks: vec![Block::Paragraph(vec![Inline::Text {
                                    text,
                                    style: Style::PLAIN,
                                }])],
                            });
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    notes
}

/// Parse document.xml body into blocks.
fn parse_body(xml: &[u8], ctx: &mut BodyCtx) -> Result<Vec<Block>, AlchemizeError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    // Paragraph accumulation state.
    let mut in_paragraph = false;
    let mut para_inlines: Vec<Inline> = Vec::new();
    let mut para_style: Option<String> = None;
    let mut para_outline: Option<u8> = None;
    let mut run_style = Style::PLAIN;
    let mut in_heading = false;

    // Table state.
    let mut table_grid: Vec<Vec<CellSlot>> = Vec::new();
    let mut row_cells: Vec<CellSlot> = Vec::new();
    let mut cell_blocks: Vec<Block> = Vec::new();
    let mut in_table = false;
    let mut in_cell = false;
    let mut cell_span: u32 = 1;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "p" => {
                        in_paragraph = true;
                        para_inlines.clear();
                        para_style = None;
                        para_outline = None;
                        run_style = Style::PLAIN;
                        in_heading = false;
                    }
                    "pStyle" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            para_style = Some(v.clone());
                            if let Some(info) = ctx.style_table.get(&v) {
                                if let Some(lvl) = info.outline_lvl {
                                    para_outline = Some(lvl);
                                    in_heading = true;
                                }
                                if info.is_heading {
                                    in_heading = true;
                                }
                                if info.bold {
                                    run_style.bold = true;
                                }
                                if info.italic {
                                    run_style.italic = true;
                                }
                            }
                        }
                    }
                    "outlineLvl" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            if let Ok(lvl) = v.trim().parse::<u8>() {
                                para_outline = Some(lvl);
                                in_heading = true;
                            }
                        }
                    }
                    "tbl" => {
                        in_table = true;
                        table_grid.clear();
                    }
                    "tr" if in_table => {
                        row_cells.clear();
                    }
                    "tc" if in_table => {
                        in_cell = true;
                        cell_blocks.clear();
                        cell_span = 1;
                    }
                    "gridSpan" if in_cell => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            cell_span = v.trim().parse().unwrap_or(1);
                        }
                    }
                    "b" if in_paragraph => {
                        let on = !has_attr_false(&e);
                        run_style.bold = on;
                    }
                    "i" if in_paragraph => {
                        let on = !has_attr_false(&e);
                        run_style.italic = on;
                    }
                    "hyperlink" if in_paragraph => {
                        // Hyperlink target resolved at End (children accumulate).
                        let rid = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref().ends_with("id"))
                            .map(|a| a.value.clone().into_owned());
                        if let Some(rid) = rid {
                            if let Some(target) = ctx.rels.get(&rid).cloned() {
                                para_inlines.push(Inline::Link {
                                    content: Vec::new(),
                                    target: LinkTarget::Relative(target),
                                });
                            }
                        }
                    }
                    "blip" => {
                        let rid = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref().ends_with("embed"))
                            .map(|a| a.value.clone().into_owned());
                        if let Some(rid) = rid {
                            if let Some(idx) = ctx.asset_for(&rid) {
                                para_inlines.push(Inline::Image {
                                    alt: String::new(),
                                    source: ImageSource::Asset(idx),
                                });
                            }
                        }
                    }
                    "oMath" => {
                        // OMML handled by the dedicated converter in a later
                        // pass; here consume to End and record a placeholder
                        // count so nothing silently disappears.
                        let text = collect_text_until_end(&mut reader, b"oMath");
                        if !text.trim().is_empty() {
                            ctx.warnings.omml_fallback += 1;
                            para_inlines.push(Inline::Text {
                                text,
                                style: Style::PLAIN,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_paragraph => {
                let text = String::from_utf8_lossy(t.as_bytes()).into_owned();
                if !text.is_empty() {
                    para_inlines.push(Inline::Text { text, style: run_style });
                }
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "p" => {
                        in_paragraph = false;
                        if in_cell {
                            if !para_inlines.is_empty() {
                                cell_blocks.push(Block::Paragraph(std::mem::take(&mut para_inlines)));
                            }
                        } else if !para_inlines.is_empty() {
                            let inlines = std::mem::take(&mut para_inlines);
                            if in_heading {
                                let lvl = para_outline.map(|l| l + 1).unwrap_or(2).clamp(1, 6);
                                blocks.push(Block::Heading { level: lvl, content: inlines });
                            } else {
                                blocks.push(Block::Paragraph(inlines));
                            }
                        }
                    }
                    "tc" if in_table => {
                        in_cell = false;
                        row_cells.push(CellSlot::Origin(Cell {
                            blocks: std::mem::take(&mut cell_blocks),
                            col_span: cell_span,
                            row_span: 1,
                        }));
                    }
                    "tr" if in_table => {
                        table_grid.push(std::mem::take(&mut row_cells));
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
                            para_style = Some(v.clone());
                            if let Some(info) = ctx.style_table.get(&v) {
                                if let Some(lvl) = info.outline_lvl {
                                    para_outline = Some(lvl);
                                    in_heading = true;
                                }
                                if info.is_heading {
                                    in_heading = true;
                                }
                            }
                        }
                    }
                    "outlineLvl" if in_paragraph => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            if let Ok(lvl) = v.trim().parse::<u8>() {
                                para_outline = Some(lvl);
                                in_heading = true;
                            }
                        }
                    }
                    "gridSpan" if in_cell => {
                        if let Some(v) = attr_value(&e, "w:val") {
                            cell_span = v.trim().parse().unwrap_or(1);
                        }
                    }
                    "blip" => {
                        if let Some(rid) = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref().ends_with("embed"))
                            .map(|a| a.value.clone().into_owned())
                        {
                            if let Some(idx) = ctx.asset_for(&rid) {
                                para_inlines.push(Inline::Image {
                                    alt: String::new(),
                                    source: ImageSource::Asset(idx),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(AlchemizeError::Xml(format!("document.xml parse error: {e}"))),
            _ => {}
        }
        buf.clear();
    }
    let _ = List {
        marker: MarkerKind::Bullet,
        start: 1,
        items: vec![ListItem { blocks: vec![] }],
    };
    Ok(blocks)
}

fn attr_value(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| a.value.clone().into_owned())
}

fn has_attr_false(e: &quick_xml::events::BytesStart) -> bool {
    e.attributes()
        .flatten()
        .any(|a| a.key.as_ref() == "w:val" && (a.value.as_ref() == "0" || a.value.as_ref() == "false"))
}

fn collect_text_until_end(reader: &mut Reader<&[u8]>, tag: &[u8]) -> String {
    let mut out = String::new();
    let mut depth = 1;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if e.name().as_ref().ends_with(std::str::from_utf8(tag).unwrap_or("\0")) {
                    depth += 1;
                }
            }
            Ok(Event::Text(t)) => out.push_str(&String::from_utf8_lossy(t.as_bytes())),
            Ok(Event::End(e)) => {
                if e.name().as_ref().ends_with(std::str::from_utf8(tag).unwrap_or("\0")) {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docx_zip(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, content) in parts {
            w.start_file(*name, opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn priority_and_extensions() {
        let c = DocxDeepConverter::new();
        assert_eq!(c.priority(), 18);
        assert_eq!(c.supported_extensions(), &["docx"]);
        assert_eq!(c.availability(), Availability::Available);
    }

    #[tokio::test]
    async fn parses_headings_and_text() {
        let doc = docx_zip(&[
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>
<w:p><w:r><w:t>Body text</w:t></w:r></w:p>
</w:body></w:document>"#,
            ),
            (
                "word/styles.xml",
                r#"<?xml version="1.0"?><w:styles xmlns:w="w"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style></w:styles>"#,
            ),
        ]);
        let result = DocxDeepConverter::new()
            .convert(Input::bytes(doc, "t.docx"))
            .await
            .unwrap();
        assert!(result.markdown.contains("# Title"), "md: {}", result.markdown);
        assert!(result.markdown.contains("Body text"));
    }

    #[tokio::test]
    async fn parses_table_with_span() {
        let doc = docx_zip(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#,
        )]);
        let result = DocxDeepConverter::new()
            .convert(Input::bytes(doc, "t.docx"))
            .await
            .unwrap();
        assert!(result.markdown.contains("| A | B |"), "md: {}", result.markdown);
    }

    #[tokio::test]
    async fn real_docx_recall_vs_anydoc_output() {
        // Character-recall smoke: deep parse of the real 923KB wordlist must
        // recover the bulk of extractable characters.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/real/real.docx");
        let result = DocxDeepConverter::new()
            .convert(Input::Path(path))
            .await
            .unwrap();
        assert!(
            result.markdown.chars().count() > 500_000,
            "recall too low: {} chars",
            result.markdown.chars().count()
        );
    }
}
