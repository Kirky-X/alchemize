// SPDX-License-Identifier: Apache-2.0
//! PPTX deep parser: PresentationML slides → the unified document model.
//!
//! A PPTX package is a zip: `ppt/slides/slideN.xml` (shape trees),
//! `ppt/notesSlides/notesSlideN.xml` (speaker notes), `ppt/slides/_rels/`
//! (per-slide relationships), `ppt/media/*` (image payloads).
//!
//! Per slide the shape tree is walked: `p:sp` shapes are classified through
//! their `p:ph` placeholder (`title`/`ctrTitle` → level-1 heading, everything
//! else → paragraph); `p:graphicFrame` tables become `Block::Table`;
//! `p:pic` images resolve through slide rels into assets. Slides sort by
//! their numeric suffix (slide10 after slide2, never lexicographically).
//!
//! quick-xml 0.42 notes baked in from the DOCX parser: attribute keys are
//! QNames (`&str`) and values are `Cow<str>`; self-closing elements such as
//! `<p:ph type="title"/>` or `<a:rPr b="1"/>` arrive as `Event::Empty`, not
//! `Event::Start`, so attribute-carrying elements dispatch on both events.

use std::collections::HashMap;
use std::io::Cursor;

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::application::ports::{Availability, Converter};
use crate::domain::asset::Asset;
use crate::domain::document::{
    Block, CellSlot, Document, ImageSource, Inline, Note, NoteKind, Style, Table, TableKind,
};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Deep PPTX converter: PresentationML → the unified document model.
pub struct PptxDeepConverter;

impl PptxDeepConverter {
    /// Create a new deep PPTX converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PptxDeepConverter {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Converter for PptxDeepConverter {
    fn name(&self) -> &'static str {
        "pptx-deep"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read PPTX input: {e}")))?;

        let result = tokio::task::spawn_blocking(move || parse(&bytes))
            .await
            .map_err(|e| AlchemizeError::other(format!("pptx-deep task panicked: {e}")))?;

        let parsed = result?;
        let slide_count = parsed.slide_count;
        let notes_count = parsed.document.notes.len();
        let asset_count = parsed.document.assets.len();
        let markdown =
            crate::infrastructure::document::gfm_render::render_markdown(&parsed.document);

        let metadata = serde_json::json!({
            "source_format": "pptx",
            "slide_count": slide_count,
            "asset_count": asset_count,
            "notes_count": notes_count,
            "parser": "pptx-deep",
        });

        let mut out = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in parsed.raw_assets {
            out = out.with_asset(Asset::new(asset.filename, asset.bytes));
        }
        Ok(out)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pptx"]
    }

    fn priority(&self) -> u8 {
        18
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

/// Media payload with its final sha8-based filename.
#[derive(Debug)]
pub struct PptxAsset {
    /// Final on-disk filename: `{seq:04}-{sha8}.{ext}`.
    pub filename: String,
    /// Raw media bytes.
    pub bytes: Vec<u8>,
}

/// Parse result: the document model plus named assets.
#[derive(Debug)]
pub struct ParsedPptx {
    /// Unified document model.
    pub document: Document,
    /// Number of slide parts parsed.
    pub slide_count: usize,
    /// Assets with final filenames, in first-reference order.
    pub raw_assets: Vec<PptxAsset>,
}

/// Parse PPTX bytes into the unified model.
pub fn parse(bytes: &[u8]) -> Result<ParsedPptx, AlchemizeError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::Zip(format!("failed to open PPTX: {e}")))?;

    // Slide and note parts, sorted numerically: slide10 sorts after slide2.
    let slide_parts = numbered_parts(&mut zip, "ppt/slides/slide", ".xml")?;
    let note_parts = numbered_parts(&mut zip, "ppt/notesSlides/notesSlide", ".xml")?;
    let mut media = MediaCtx {
        zip: &mut zip,
        assets: Vec::new(),
        asset_index: HashMap::new(),
    };

    let mut blocks: Vec<Block> = Vec::new();
    for (pos, (number, part)) in slide_parts.iter().enumerate() {
        let xml = read_part(media.zip, part)?;
        // Rels live beside the part: ppt/slides/_rels/<basename>.rels
        let base = part.rsplit('/').next().unwrap_or(part);
        let rels = read_slide_rels(media.zip, &format!("ppt/slides/_rels/{base}.rels"))?
            .unwrap_or_default();
        if pos > 0 {
            blocks.push(Block::Rule);
        }
        // A slide's own title placeholder already carries the heading; only
        // synthesize `Slide N` when the body has no level-1 heading.
        let slide_blocks = parse_slide(&xml, &rels, &mut media)?;
        let has_own_title = slide_blocks
            .iter()
            .any(|b| matches!(b, Block::Heading { level: 1, .. }));
        if !has_own_title {
            blocks.push(Block::Heading {
                level: 1,
                content: vec![Inline::Text {
                    text: format!("Slide {number}"),
                    style: Style::PLAIN,
                }],
            });
        }
        blocks.extend(slide_blocks);
    }
    let slide_count = slide_parts.len();

    // Speaker notes: `notesSlideN.xml` → Note id "slide{N}-notes" (Endnote).
    let mut notes: Vec<Note> = Vec::new();
    for (number, part) in note_parts {
        let xml = read_part(media.zip, &part)?;
        let text = collect_all_text(&xml);
        if !text.is_empty() {
            notes.push(Note {
                id: format!("slide{number}-notes"),
                kind: NoteKind::Endnote,
                blocks: vec![Block::Paragraph(vec![Inline::Text {
                    text,
                    style: Style::PLAIN,
                }])],
            });
        }
    }

    // Model assets keep provenance; ConvertResult assets get sha8 names.
    let doc_assets: Vec<crate::domain::document::Asset> = media
        .assets
        .iter()
        .map(|a| crate::domain::document::Asset {
            media_type: a.media_type.clone(),
            origin_part: a.part.clone(),
            bytes: a.bytes.clone(),
        })
        .collect();
    let mut refs: HashMap<usize, String> = HashMap::new();
    let mut collected: Vec<PptxAsset> = Vec::new();
    let mut seq = 0usize;
    rewrite_image_refs(
        &mut blocks,
        &doc_assets,
        &mut refs,
        &mut collected,
        &mut seq,
    );

    Ok(ParsedPptx {
        document: Document {
            blocks,
            notes,
            assets: doc_assets,
        },
        slide_count,
        raw_assets: collected,
    })
}

/// Shared package context: the zip plus media collected on demand.
struct MediaCtx<'a> {
    zip: &'a mut zip::ZipArchive<Cursor<&'a [u8]>>,
    /// Media payloads collected so far, in first-reference order.
    assets: Vec<RawMedia>,
    /// Package part name → index into `assets`.
    asset_index: HashMap<String, usize>,
}

/// Raw media part before sha8 renaming.
struct RawMedia {
    part: String,
    media_type: String,
    bytes: Vec<u8>,
}

/// Zip parts matching `prefix{digits}suffix`, sorted by the digits.
fn numbered_parts(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    prefix: &str,
    suffix: &str,
) -> Result<Vec<(u32, String)>, AlchemizeError> {
    let mut out: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let name = zip
            .by_index(i)
            .map_err(|e| AlchemizeError::Zip(format!("{e}")))?
            .name()
            .map_err(|e| AlchemizeError::Zip(format!("{e}")))?
            .to_string();
        if let Some(n) = slide_number_of(&name, prefix, suffix) {
            out.push((n, name));
        }
    }
    out.sort_by_key(|(n, _)| *n);
    Ok(out)
}

/// Extract the numeric suffix of a part name (`prefix{N}suffix`).
fn slide_number_of(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    let rest = name.strip_prefix(prefix)?;
    let num = rest.strip_suffix(suffix)?;
    num.parse().ok()
}

/// Read one part out of the package.
fn read_part(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, AlchemizeError> {
    use std::io::Read;
    let mut file = zip
        .by_name(name)
        .map_err(|e| AlchemizeError::Zip(format!("part {name} not found: {e}")))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| AlchemizeError::Zip(format!("part {name} read failed: {e}")))?;
    Ok(buf)
}

/// Read a slide's relationships part (optional).
fn read_slide_rels(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Option<HashMap<String, String>>, AlchemizeError> {
    use std::io::Read;
    let Ok(mut file) = zip.by_name(name) else {
        return Ok(None);
    };
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| AlchemizeError::Zip(format!("part {name} read failed: {e}")))?;
    Ok(Some(parse_rels(&buf)))
}

/// Parse a relationships part into id → target.
///
/// `<Relationship …/>` is self-closing: the attributes ride on `Event::Empty`
/// (and sometimes `Event::Start`), so both variants dispatch here.
fn parse_rels(xml: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref())
                    != "Relationship"
                {
                    continue;
                }
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
                    out.insert(id, target);
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

/// Resolve a relationship target relative to `ppt/slides/` against the
/// package root: `../media/image1.png` → `ppt/media/image1.png`.
fn normalize_slide_target(target: &str) -> String {
    let trimmed = target.strip_prefix('/').unwrap_or(target);
    if trimmed.starts_with("ppt/") {
        return trimmed.to_string();
    }
    // Relationship targets are relative to the part's folder: slide rels live
    // in `ppt/slides/`, so one `../` lands at `ppt/`, two at the package root.
    // Base folder is `ppt/slides/`; a bare `media/x.png` is treated as
    // package-relative to `ppt/` (PowerPoint's own emitters always use
    // `../media/`, but hand-built packages often omit it).
    let mut parts: Vec<&str> = if trimmed.starts_with("media/") {
        vec!["ppt"]
    } else {
        vec!["ppt", "slides"]
    };
    for seg in trimmed.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Guess a media type from the part extension.
fn guess_media_type(part: &str) -> String {
    let ext = part.rsplit('.').next().unwrap_or("bin");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "tiff" | "tif" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Resolve a `r:embed` id to an asset index, reading media on demand.
fn asset_for(ctx: &mut MediaCtx, rels: &HashMap<String, String>, rid: &str) -> Option<usize> {
    let target = rels.get(rid)?;
    let part = normalize_slide_target(target);
    if let Some(idx) = ctx.asset_index.get(&part) {
        return Some(*idx);
    }
    let bytes = read_part(ctx.zip, &part).ok()?;
    let media_type = guess_media_type(&part);
    let idx = ctx.assets.len();
    ctx.asset_index.insert(part.clone(), idx);
    ctx.assets.push(RawMedia {
        part,
        media_type,
        bytes,
    });
    Some(idx)
}

/// Placeholder classification of a `p:sp` shape.
#[derive(Clone, Copy, PartialEq)]
enum PhKind {
    /// `title` / `ctrTitle` placeholder → level-1 heading.
    Title,
    /// `body` / `subTitle` / `obj` / untyped placeholder → paragraph.
    Body,
}

/// Accumulation state across one slide's event stream.
#[derive(Default)]
struct SlideState {
    in_sp: bool,
    ph: Option<PhKind>,
    in_table: bool,
    in_cell: bool,
    cell_text: String,
    grid: Vec<Vec<CellSlot>>,
    row: Vec<CellSlot>,
    in_para: bool,
    in_text: bool,
    in_run: bool,
    run_bold: Option<bool>,
    run_italic: Option<bool>,
    run_size: Option<f32>,
    para_inlines: Vec<Inline>,
}

impl SlideState {
    /// Current run style (unspecified flags default to off).
    fn run_style(&self) -> Style {
        Style {
            bold: self.run_bold.unwrap_or(false),
            italic: self.run_italic.unwrap_or(false),
            code: false,
            strike: false,
            size_pt: self.run_size,
        }
    }
}

/// Parse one slide's XML into blocks, resolving images through `rels`.
fn parse_slide(
    xml: &[u8],
    rels: &HashMap<String, String>,
    ctx: &mut MediaCtx,
) -> Result<Vec<Block>, AlchemizeError> {
    let mut st = SlideState::default();
    let mut blocks: Vec<Block> = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(ev) => ev,
            Err(e) => return Err(AlchemizeError::Xml(format!("slide parse error: {e}"))),
            _ => break,
        };
        match event {
            Event::Eof => break,
            Event::Start(e) => handle_tag(&e, rels, ctx, &mut st, &mut blocks)?,
            Event::Empty(e) => handle_tag(&e, rels, ctx, &mut st, &mut blocks)?,
            Event::GeneralRef(r) => {
                if st.in_text {
                    let text = crate::infrastructure::document::decode_entity(r.as_ref());
                    if st.in_cell {
                        st.cell_text.push_str(&text);
                    } else if st.in_para {
                        st.para_inlines.push(Inline::Text {
                            text,
                            style: st.run_style(),
                        });
                    }
                }
            }
            Event::Text(t) if st.in_text => {
                let text = String::from_utf8_lossy(t.as_bytes()).into_owned();
                if st.in_cell {
                    st.cell_text.push_str(&text);
                } else if st.in_para {
                    st.para_inlines.push(Inline::Text {
                        text,
                        style: st.run_style(),
                    });
                }
            }
            Event::End(e) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "r" => {
                        st.in_run = false;
                        st.run_bold = None;
                        st.run_italic = None;
                        st.run_size = None;
                    }
                    "t" => st.in_text = false,
                    "p" => {
                        st.in_para = false;
                        if st.in_cell {
                            // Cell paragraphs join with a space.
                            if !st.cell_text.trim().is_empty() {
                                st.cell_text.push(' ');
                            }
                        } else if st.in_sp && !st.para_inlines.is_empty() {
                            let inlines = std::mem::take(&mut st.para_inlines);
                            if st.ph == Some(PhKind::Title) {
                                blocks.push(Block::Heading {
                                    level: 1,
                                    content: inlines,
                                });
                            } else {
                                blocks.push(Block::Paragraph(inlines));
                            }
                        } else if !st.in_sp && !st.para_inlines.is_empty() {
                            // Bare paragraph (e.g. inside a group): paragraph.
                            blocks.push(Block::Paragraph(std::mem::take(&mut st.para_inlines)));
                        }
                    }
                    "tc" => {
                        st.in_cell = false;
                        let text = st.cell_text.trim().to_string();
                        st.row
                            .push(CellSlot::Origin(crate::domain::document::Cell::text(text)));
                        st.cell_text.clear();
                    }
                    "tr" => st.grid.push(std::mem::take(&mut st.row)),
                    "tbl" => {
                        st.in_table = false;
                        let grid = std::mem::take(&mut st.grid);
                        if !grid.is_empty() {
                            blocks.push(Block::Table(Table {
                                grid,
                                header_rows: 1,
                                kind: TableKind::Data,
                            }));
                        }
                    }
                    "sp" => {
                        st.in_sp = false;
                        st.ph = None;
                    }
                    "pic" => {}
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(blocks)
}

/// Shared dispatch for `Event::Start` and `Event::Empty`: self-closing
/// attribute carriers (`p:ph`, `a:rPr`, `a:blip`) must be handled identically
/// on both variants.
fn handle_tag(
    e: &quick_xml::events::BytesStart,
    rels: &HashMap<String, String>,
    ctx: &mut MediaCtx,
    st: &mut SlideState,
    blocks: &mut Vec<Block>,
) -> Result<(), AlchemizeError> {
    let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
    match local.as_str() {
        "sp" => {
            st.in_sp = true;
            st.ph = None;
        }
        "ph" if st.in_sp => {
            let ty = e
                .attributes()
                .flatten()
                .find(|a| a.key.as_ref() == "type")
                .map(|a| a.value.clone().into_owned())
                .unwrap_or_default();
            st.ph = Some(match ty.as_str() {
                "title" | "ctrTitle" => PhKind::Title,
                _ => PhKind::Body,
            });
        }
        "tbl" => {
            st.in_table = true;
            st.grid.clear();
        }
        "tr" if st.in_table => st.row.clear(),
        "tc" if st.in_table => {
            st.in_cell = true;
            st.cell_text.clear();
        }
        "p" if !st.in_table => {
            st.in_para = true;
            st.para_inlines.clear();
        }
        "r" if st.in_para => {
            st.in_run = true;
            st.run_bold = None;
            st.run_italic = None;
            st.run_size = None;
        }
        "rPr" if st.in_run => {
            // `<a:rPr b="1" sz="1800"/>` is usually self-closing.
            for attr in e.attributes().flatten() {
                match attr.key.as_ref() {
                    "b" => st.run_bold = Some(attr.value.as_ref() == "1"),
                    "i" => st.run_italic = Some(attr.value.as_ref() == "1"),
                    "sz" => {
                        if let Ok(hp) = attr.value.trim().parse::<u32>() {
                            st.run_size = Some(hp as f32 / 100.0);
                        }
                    }
                    _ => {}
                }
            }
        }
        "t" => st.in_text = true,
        "blip" => {
            // `<a:blip r:embed="rIdN"/>` — namespace-prefixed key.
            let rid = e
                .attributes()
                .flatten()
                .find(|a| a.key.as_ref().ends_with("embed"))
                .map(|a| a.value.clone().into_owned());
            if let Some(rid) = rid
                && let Some(idx) = asset_for(ctx, rels, &rid)
            {
                let image = Inline::Image {
                    alt: String::new(),
                    source: ImageSource::Asset(idx),
                };
                if st.in_para {
                    st.para_inlines.push(image);
                } else {
                    // Standalone picture outside any paragraph.
                    blocks.push(Block::Paragraph(vec![image]));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Concatenate every `a:t` text fragment in an XML part (used for notes).
fn collect_all_text(xml: &[u8]) -> String {
    let mut out = String::new();
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut in_text = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == "t" {
                    in_text = true;
                }
            }
            Ok(Event::Text(t)) if in_text => {
                out.push_str(&String::from_utf8_lossy(t.as_bytes()));
            }
            Ok(Event::End(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == "t" {
                    in_text = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out.trim().to_string()
}

/// Rewrite `ImageSource::Asset(idx)` into sha8-named `images/…` references
/// (the DOCX convention) and collect named assets in first-reference
/// order. Identical bytes reuse the first reference.
fn rewrite_image_refs(
    blocks: &mut Vec<Block>,
    doc_assets: &[crate::domain::document::Asset],
    refs: &mut HashMap<usize, String>,
    collected: &mut Vec<PptxAsset>,
    seq: &mut usize,
) {
    for block in blocks.iter_mut() {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                rewrite_inlines(content, doc_assets, refs, collected, seq);
            }
            Block::Table(table) => {
                for row in table.grid.iter_mut() {
                    for slot in row.iter_mut() {
                        if let CellSlot::Origin(cell) = slot {
                            rewrite_image_refs(&mut cell.blocks, doc_assets, refs, collected, seq);
                        }
                    }
                }
            }
            Block::List(list) => {
                for item in list.items.iter_mut() {
                    rewrite_image_refs(&mut item.blocks, doc_assets, refs, collected, seq);
                }
            }
            Block::BlockQuote(nested) => {
                rewrite_image_refs(nested, doc_assets, refs, collected, seq);
            }
            Block::CodeBlock { .. } | Block::Rule | Block::Math(_) => {}
        }
    }
}

/// Rewrite image inlines of one paragraph/heading.
fn rewrite_inlines(
    inlines: &mut [Inline],
    doc_assets: &[crate::domain::document::Asset],
    refs: &mut HashMap<usize, String>,
    collected: &mut Vec<PptxAsset>,
    seq: &mut usize,
) {
    for inline in inlines.iter_mut() {
        let Inline::Image { source, .. } = inline else {
            continue;
        };
        let ImageSource::Asset(id) = source else {
            continue;
        };
        let Some(asset) = doc_assets.get(*id) else {
            continue;
        };
        if !refs.contains_key(id) {
            let sha8 = crate::domain::asset::sha8_hex(&asset.bytes);
            let ext = asset
                .origin_part
                .rsplit('.')
                .next()
                .unwrap_or("bin")
                .to_lowercase();
            let name = format!("{:04}-{sha8}.{ext}", *seq);
            *seq += 1;
            collected.push(PptxAsset {
                filename: name.clone(),
                bytes: asset.bytes.clone(),
            });
            refs.insert(*id, format!("images/{name}"));
        }
        let reference = refs[id].clone();
        *source = ImageSource::External(reference);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::Cell;

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    /// Build a minimal PPTX zip from (part name, bytes) pairs.
    fn pptx_zip(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
        use std::io::Write;
        let buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, content) in parts {
            w.start_file(*name, opts).unwrap();
            w.write_all(content).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn str(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    fn slide(title_xml: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree>{title_xml}</p:spTree></p:cSld></p:sld>"#
        )
    }

    #[test]
    fn converter_metadata_shape() {
        let c = PptxDeepConverter::new();
        assert_eq!(c.name(), "pptx-deep");
        assert_eq!(c.priority(), 18);
        assert_eq!(c.supported_extensions(), &["pptx"]);
        assert_eq!(c.availability(), Availability::Available);
    }

    #[test]
    fn slide_number_parsing_is_numeric() {
        assert_eq!(
            slide_number_of("ppt/slides/slide10.xml", "ppt/slides/slide", ".xml"),
            Some(10)
        );
        assert_eq!(
            slide_number_of("ppt/slides/slide2.xml", "ppt/slides/slide", ".xml"),
            Some(2)
        );
        assert_eq!(
            slide_number_of(
                "ppt/slides/_rels/slide2.xml.rels",
                "ppt/slides/slide",
                ".xml"
            ),
            None
        );
        assert_eq!(
            slide_number_of(
                "ppt/notesSlides/notesSlide3.xml",
                "ppt/notesSlides/notesSlide",
                ".xml"
            ),
            Some(3)
        );
    }

    #[test]
    fn relationship_targets_normalize_against_ppt_root() {
        assert_eq!(
            normalize_slide_target("../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            normalize_slide_target("media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            normalize_slide_target("/ppt/media/image1.png"),
            "ppt/media/image1.png"
        );
    }

    #[tokio::test]
    async fn title_placeholder_becomes_heading() {
        // `<p:ph type="title"/>` is self-closing: it must be read from
        // Event::Empty, not Event::Start.
        let bytes = pptx_zip(&[(
            "ppt/slides/slide1.xml",
            str(&slide(
                r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-US"/><a:t>Quarterly Review</a:t></a:r></a:p></p:txBody></p:sp>"#,
            )),
        )]);
        let doc = parse(&bytes).unwrap().document;
        let heading = doc.blocks.iter().find_map(|b| match b {
            Block::Heading { level, content } => Some((*level, content.clone())),
            _ => None,
        });
        let (level, content) = heading.expect("title heading expected");
        assert_eq!(level, 1);
        assert!(matches!(&content[0], Inline::Text { text, .. } if text == "Quarterly Review"));
    }

    #[tokio::test]
    async fn ctr_title_also_heading_body_placeholder_paragraph() {
        let bytes = pptx_zip(&[(
            "ppt/slides/slide1.xml",
            str(&slide(
                r#"<p:sp><p:nvSpPr><p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Centered</a:t></a:r></a:p></p:txBody></p:sp>"#,
            )),
        )]);
        let doc = parse(&bytes).unwrap().document;
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(b, Block::Heading { level: 1, .. }))
        );
    }

    #[tokio::test]
    async fn run_styles_bold_italic_and_size() {
        // `<a:rPr b="1" sz="1800"/>` self-closing with attributes.
        let bytes = pptx_zip(&[(
            "ppt/slides/slide1.xml",
            str(&slide(
                r#"<p:sp><p:nvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:rPr b="1" i="1" sz="1800"/><a:t>Bold body</a:t></a:r></a:p></p:txBody></p:sp>"#,
            )),
        )]);
        let doc = parse(&bytes).unwrap().document;
        let para = doc.blocks.iter().find_map(|b| match b {
            Block::Paragraph(inlines) => Some(inlines.clone()),
            _ => None,
        });
        let inlines = para.expect("body paragraph expected");
        let Some(Inline::Text { text, style }) = inlines.first() else {
            panic!("expected text run");
        };
        assert_eq!(text, "Bold body");
        assert!(style.bold);
        assert!(style.italic);
        assert_eq!(style.size_pt, Some(18.0));
    }

    #[tokio::test]
    async fn graphic_frame_table_becomes_table_block() {
        let bytes = pptx_zip(&[(
            "ppt/slides/slide1.xml",
            str(&slide(
                r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Table 3"/><p:nvPr/></p:nvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblGrid/><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Region</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Sales</a:t></a:r></a:p></a:txBody></a:tc></a:tr><a:tr><a:tc><a:txBody><a:p><a:r><a:t>North</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>1200</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
            )),
        )]);
        let doc = parse(&bytes).unwrap().document;
        let table = doc.blocks.iter().find_map(|b| match b {
            Block::Table(t) => Some(t.clone()),
            _ => None,
        });
        let table = table.expect("table block expected");
        assert_eq!(table.grid.len(), 2);
        assert_eq!(table.width(), 2);
        let CellSlot::Origin(cell) = &table.grid[1][1] else {
            panic!("origin cell expected");
        };
        let Some(Block::Paragraph(inlines)) = cell.blocks.first() else {
            panic!("cell paragraph expected");
        };
        let Some(Inline::Text { text, .. }) = inlines.first() else {
            panic!("cell text expected");
        };
        assert_eq!(text, "1200");
    }

    #[tokio::test]
    async fn picture_resolves_to_sha8_named_asset() {
        let bytes = pptx_zip(&[
            (
                "ppt/slides/slide1.xml",
                str(&slide(
                    r#"<p:pic><p:nvPicPr><p:cNvPr id="5" name="Picture 4"/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr/></p:pic>"#,
                )),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                str(
                    r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#,
                ),
            ),
            ("ppt/media/image1.png", PNG_1X1.to_vec()),
        ]);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.document.assets.len(), 1);
        assert_eq!(
            parsed.document.assets[0].origin_part,
            "ppt/media/image1.png"
        );
        assert_eq!(parsed.raw_assets.len(), 1);
        let name = &parsed.raw_assets[0].filename;
        assert!(
            name.len() == "0000-12345678.png".len(),
            "sha8 name shape: {name}"
        );
        assert!(name.ends_with(".png"));
        assert_eq!(parsed.raw_assets[0].bytes, PNG_1X1);
        // Model inline points at the rewritten reference.
        let image = parsed.document.blocks.iter().find_map(|b| match b {
            Block::Paragraph(inlines) => inlines.iter().find_map(|i| match i {
                Inline::Image { source, .. } => Some(source.clone()),
                _ => None,
            }),
            _ => None,
        });
        let Some(ImageSource::External(reference)) = image else {
            panic!("external image reference expected");
        };
        assert_eq!(reference, format!("images/{name}"));
    }

    #[tokio::test]
    async fn notes_slide_becomes_endnote() {
        let bytes = pptx_zip(&[
            (
                "ppt/slides/slide1.xml",
                str(&slide(
                    r#"<p:sp><p:txBody><a:p><a:r><a:t>Body</a:t></a:r></a:p></p:txBody></p:sp>"#,
                )),
            ),
            (
                "ppt/notesSlides/notesSlide1.xml",
                str(
                    r#"<?xml version="1.0"?><p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Speaker note</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#,
                ),
            ),
        ]);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.document.notes.len(), 1);
        assert_eq!(parsed.document.notes[0].id, "slide1-notes");
        assert_eq!(parsed.document.notes[0].kind, NoteKind::Endnote);
        let Some(Block::Paragraph(inlines)) = parsed.document.notes[0].blocks.first() else {
            panic!("note paragraph expected");
        };
        let Some(Inline::Text { text, .. }) = inlines.first() else {
            panic!("note text expected");
        };
        assert_eq!(text, "Speaker note");
    }

    #[tokio::test]
    async fn slides_sort_numerically_not_lexically() {
        // slide1, slide2, slide10 — lexicographic would put slide10 first.
        let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
        for n in [1u32, 2, 10] {
            parts.push((
                format!("ppt/slides/slide{n}.xml"),
                str(&slide(&format!(
                    r#"<p:sp><p:txBody><a:p><a:r><a:t>Content {n}</a:t></a:r></a:p></p:txBody></p:sp>"#
                ))),
            ));
        }
        let refs: Vec<(&str, Vec<u8>)> =
            parts.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let bytes = pptx_zip(&refs);
        let result = PptxDeepConverter::new()
            .convert(Input::bytes(bytes, "deck.pptx"))
            .await
            .unwrap();
        let md = &result.markdown;
        let p1 = md.find("# Slide 1").expect("slide 1 header");
        let p2 = md.find("# Slide 2").expect("slide 2 header");
        let p10 = md.find("# Slide 10").expect("slide 10 header");
        assert!(p1 < p2 && p2 < p10, "numeric order required: {md}");
        // Rule between pages renders as `---`.
        assert!(md.contains("---"), "rule between slides expected: {md}");
        assert_eq!(result.metadata["slide_count"].as_u64(), Some(3));
    }

    #[tokio::test]
    async fn empty_slides_still_get_headers() {
        let bytes = pptx_zip(&[("ppt/slides/slide1.xml", str(&slide("")))]);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.slide_count, 1);
        assert!(
            parsed
                .document
                .blocks
                .iter()
                .any(|b| matches!(b, Block::Heading { level: 1, .. }))
        );
    }

    #[tokio::test]
    async fn invalid_zip_errors() {
        let err = parse(b"not a zip").unwrap_err();
        assert!(matches!(err, AlchemizeError::Zip(_)));
    }

    #[tokio::test]
    async fn real_pptx_recall_and_metadata() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real/real.pptx");
        let result = PptxDeepConverter::new()
            .convert(Input::Path(path))
            .await
            .unwrap();
        assert_eq!(result.metadata["slide_count"].as_u64(), Some(13));
        assert_eq!(result.metadata["source_format"], "pptx");
        // Real content keywords read from slide XML before the test was
        // written (brand-kit deck).
        for keyword in ["Brand kit", "Atmosphere", "Color palette (1/6)"] {
            assert!(
                result.markdown.contains(keyword),
                "keyword {keyword:?} missing"
            );
        }
        assert!(result.markdown.contains("# Slide 13"));
    }

    #[test]
    fn cell_helper_plain() {
        let c = Cell::text("x");
        assert_eq!(c.col_span, 1);
        assert_eq!(c.row_span, 1);
    }
}
