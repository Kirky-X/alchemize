// SPDX-License-Identifier: Apache-2.0
//! ODT deep parser (self-hosted): the OpenDocument event stream parsed into
//! the unified document model with automatic-style resolution, footnotes,
//! merged table cells, list numbering, and image assets.
//!
//! ODF layout: a zip package holding `content.xml` (body + automatic styles),
//! `styles.xml` (named styles), `Pictures/*`, and `META-INF/manifest.xml`.
//! There is no `_rels` part — image hrefs point straight at package paths.
//!
//! The walker in this file also backs the ODP converter ([`super::odp`]):
//! `draw:page` slides and `presentation:notes` are handled by the same
//! event loop, so odp.rs only carries its `Converter` and its tests.

use std::collections::HashMap;
use std::io::Cursor;

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::docx::body::{guess_media_type, read_part};
use super::docx::styles::RunProps;
use crate::application::ports::{Availability, Converter};
use crate::domain::asset::Asset;
use crate::domain::document::{
    Block, Cell, CellSlot, Document, ImageSource, Inline, LinkTarget, List, ListItem, MarkerKind,
    Note, NoteKind, Style, Table, TableKind,
};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

use crate::infrastructure::document::local_tag_name;

/// Upper bound for `table:number-columns-repeated` / `-rows-repeated`
/// expansion; beyond it the repeat is truncated and counted.
const REPEAT_CAP: u32 = 1000;
/// Upper bound for a single `text:c` space run.
const SPACE_CAP: usize = 10_000;

/// An automatic or named style declaration.
#[derive(Default, Clone)]
pub(crate) struct OdfStyleDef {
    /// `style:parent-style-name` — inheritance chain.
    parent: Option<String>,
    /// `fo:font-weight` (`bold` / `normal`).
    bold: Option<bool>,
    /// `fo:font-style` (`italic` / `normal`).
    italic: Option<bool>,
    /// `fo:font-size` in points.
    size_pt: Option<f32>,
    /// List marker derived from `text:list-level-style-number`.
    marker: Option<MarkerKind>,
}

/// Raw zip part payload (media read on demand).
pub(crate) struct RawOdfAsset {
    /// Package-relative part name (provenance and extension source).
    pub(crate) part: String,
    /// Guessed media type.
    pub(crate) media_type: String,
    /// Raw bytes.
    pub(crate) bytes: Vec<u8>,
}

/// Parse result: the document model plus parser counters.
pub struct ParsedOdf {
    /// Unified document model (image refs still `ImageSource::Asset`).
    pub(crate) document: Document,
    /// Media payloads in first-reference order.
    pub(crate) raw_assets: Vec<RawOdfAsset>,
    /// Paragraph and heading blocks emitted.
    pub(crate) paragraph_count: usize,
    /// Number of repeat attributes that exceeded [`REPEAT_CAP`].
    pub(crate) repeated_truncated: usize,
}

/// Table being built (stacked so nested tables inside cells work).
struct TableState {
    grid: Vec<Vec<CellSlot>>,
    row: Vec<CellSlot>,
    col: usize,
    row_index: usize,
    header_rows: usize,
    in_header: bool,
    row_repeat: u32,
    /// Covered position → its origin, for `table:covered-table-cell`.
    coverage: HashMap<(usize, usize), (usize, usize)>,
}

impl TableState {
    fn new() -> Self {
        Self {
            grid: Vec::new(),
            row: Vec::new(),
            col: 0,
            row_index: 0,
            header_rows: 0,
            in_header: false,
            row_repeat: 1,
            coverage: HashMap::new(),
        }
    }
}

/// Open block container: paragraphs/lists/cells/notes/links all terminate by
/// flushing into the container below them on the stack.
enum Container {
    /// `text:p` / `text:h`; `spans` is the nested `text:span` style chain.
    Para {
        heading: Option<u8>,
        inlines: Vec<Inline>,
        props: RunProps,
        spans: Vec<RunProps>,
    },
    /// `text:list`; `current` accumulates the in-flight item's blocks.
    List {
        marker: MarkerKind,
        items: Vec<ListItem>,
        current: Vec<Block>,
    },
    /// `table:table-cell` with its span attributes.
    Cell {
        blocks: Vec<Block>,
        col_span: u32,
        row_span: u32,
        repeat: u32,
    },
    /// `text:note` / `presentation:notes`; id may arrive via the citation.
    Note {
        id: Option<String>,
        kind: NoteKind,
        blocks: Vec<Block>,
        emit_ref: bool,
    },
    /// `text:a` hyperlink wrapping inline content.
    Link {
        content: Vec<Inline>,
        target: LinkTarget,
    },
}

/// Shared ODF event-stream walker (ODT body and ODP slides alike).
pub(crate) struct OdfWalker<'a> {
    zip: &'a mut zip::ZipArchive<Cursor<&'a [u8]>>,
    styles: HashMap<String, OdfStyleDef>,
    pub(crate) raw_assets: Vec<RawOdfAsset>,
    asset_index: HashMap<String, usize>,
    pub(crate) blocks: Vec<Block>,
    pub(crate) notes: Vec<Note>,
    pub(crate) paragraph_count: usize,
    pub(crate) repeated_truncated: usize,
    page_no: usize,
    note_seq: usize,
    containers: Vec<Container>,
    tables: Vec<TableState>,
    citation: Option<String>,
    /// Captured citation text of the current note (after `note-citation`
    /// closes); the id fallback reads it when the note ends.
    last_citation: Option<String>,
    binary: Option<String>,
}

impl<'a> OdfWalker<'a> {
    fn new(
        zip: &'a mut zip::ZipArchive<Cursor<&'a [u8]>>,
        styles: HashMap<String, OdfStyleDef>,
    ) -> Self {
        Self {
            zip,
            styles,
            raw_assets: Vec::new(),
            asset_index: HashMap::new(),
            blocks: Vec::new(),
            notes: Vec::new(),
            paragraph_count: 0,
            repeated_truncated: 0,
            page_no: 0,
            note_seq: 0,
            containers: Vec::new(),
            tables: Vec::new(),
            citation: None,
            last_citation: None,
            binary: None,
        }
    }

    // ---- sinks ------------------------------------------------------------

    /// Route a block into the nearest receiving container (cell, note, list
    /// item) or the top-level blocks. A paragraph landing inside another
    /// paragraph (frame anchored in-paragraph) flattens with a line break;
    /// other block kinds sink past inline containers.
    fn push_block(&mut self, block: Block) {
        let n = self.containers.len();
        for idx in (0..n).rev() {
            match &mut self.containers[idx] {
                Container::Cell { blocks, .. } | Container::Note { blocks, .. } => {
                    blocks.push(block);
                    return;
                }
                Container::List { current, .. } => {
                    current.push(block);
                    return;
                }
                Container::Para { inlines, .. }
                | Container::Link {
                    content: inlines, ..
                } => {
                    if let Block::Paragraph(inner) = &block {
                        if !inlines.is_empty() {
                            inlines.push(Inline::LineBreak);
                        }
                        inlines.extend(inner.iter().cloned());
                        return;
                    }
                }
            }
        }
        self.blocks.push(block);
    }

    /// Route an inline into the enclosing paragraph or link; outside both, a
    /// standalone paragraph carries it.
    fn push_inline(&mut self, inline: Inline) {
        for c in self.containers.iter_mut().rev() {
            match c {
                Container::Para { inlines, .. } => {
                    inlines.push(inline);
                    return;
                }
                Container::Link { content, .. } => {
                    content.push(inline);
                    return;
                }
                _ => {}
            }
        }
        self.push_block(Block::Paragraph(vec![inline]));
    }

    fn any_inline_sink(&self) -> bool {
        self.containers
            .iter()
            .any(|c| matches!(c, Container::Para { .. } | Container::Link { .. }))
    }

    /// Effective run style: innermost span over the enclosing paragraph style.
    fn effective(&self) -> RunProps {
        for c in self.containers.iter().rev() {
            if let Container::Para { props, spans, .. } = c {
                return spans.last().copied().unwrap_or(*props);
            }
        }
        RunProps::default()
    }

    fn span_push(&mut self, name: Option<&str>) {
        let resolved = name
            .map(|n| resolve_style(&self.styles, n))
            .unwrap_or_default();
        for c in self.containers.iter_mut().rev() {
            if let Container::Para { props, spans, .. } = c {
                let parent = spans.last().copied().unwrap_or(*props);
                spans.push(parent.merge(resolved));
                return;
            }
        }
    }

    fn span_pop(&mut self) {
        for c in self.containers.iter_mut().rev() {
            if let Container::Para { spans, .. } = c {
                spans.pop();
                return;
            }
        }
    }

    fn gen_note_id(&mut self) -> String {
        self.note_seq += 1;
        format!("fn{}", self.note_seq)
    }

    // ---- container ends ---------------------------------------------------

    fn start_para(&mut self, heading: Option<u8>, style: Option<&str>) {
        let props = style
            .map(|s| resolve_style(&self.styles, s))
            .unwrap_or_default();
        self.containers.push(Container::Para {
            heading,
            inlines: Vec::new(),
            props,
            spans: Vec::new(),
        });
    }

    fn close_para(&mut self) {
        let Some(Container::Para {
            heading, inlines, ..
        }) = self.containers.pop()
        else {
            return;
        };
        if inlines.is_empty() {
            return;
        }
        self.paragraph_count += 1;
        match heading {
            Some(lvl) => self.push_block(Block::Heading {
                level: lvl.clamp(1, 6),
                content: inlines,
            }),
            None => self.push_block(Block::Paragraph(inlines)),
        }
    }

    fn close_item(&mut self) {
        if let Some(Container::List { items, current, .. }) = self.containers.last_mut() {
            if !current.is_empty() {
                items.push(ListItem {
                    blocks: std::mem::take(current),
                });
            }
        }
    }

    fn close_list(&mut self) {
        let Some(Container::List {
            marker,
            mut items,
            current,
        }) = self.containers.pop()
        else {
            return;
        };
        if !current.is_empty() {
            items.push(ListItem { blocks: current });
        }
        if !items.is_empty() {
            self.push_block(Block::List(List {
                marker,
                start: 1,
                items,
            }));
        }
    }

    fn close_link(&mut self) {
        // `<text:a>` without an href never opened a Link container and must
        // not pop the enclosing paragraph.
        if !matches!(self.containers.last(), Some(Container::Link { .. })) {
            return;
        }
        if let Some(Container::Link { content, target }) = self.containers.pop() {
            self.push_inline(Inline::Link { content, target });
        }
    }

    fn close_note(&mut self) {
        let Some(Container::Note {
            id,
            kind,
            blocks,
            emit_ref,
        }) = self.containers.pop()
        else {
            return;
        };
        if blocks.is_empty() {
            return;
        }
        let id = id.unwrap_or_else(|| self.gen_note_id());
        self.notes.push(Note {
            id: id.clone(),
            kind,
            blocks,
        });
        if emit_ref {
            self.push_inline(Inline::NoteRef(id));
        }
    }

    // ---- table placement --------------------------------------------------

    fn place_origin(&mut self, cell: Cell, repeat: u32) {
        let Some(t) = self.tables.last_mut() else {
            return;
        };
        if repeat > REPEAT_CAP {
            self.repeated_truncated += 1;
        }
        let n = repeat.min(REPEAT_CAP) as usize;
        for k in 0..n {
            let col = t.col + k * cell.col_span as usize;
            if cell.row_span > 1 {
                for i in 1..cell.row_span as usize {
                    for j in 0..cell.col_span as usize {
                        t.coverage
                            .insert((t.row_index + i, col + j), (t.row_index, col));
                    }
                }
            }
            t.row.push(CellSlot::Origin(cell.clone()));
        }
        t.col += n * cell.col_span as usize;
    }

    fn place_covered(&mut self, repeat: u32) {
        let Some(t) = self.tables.last_mut() else {
            return;
        };
        if repeat > REPEAT_CAP {
            self.repeated_truncated += 1;
        }
        let n = repeat.min(REPEAT_CAP) as usize;
        for k in 0..n {
            let col = t.col + k;
            let origin = t
                .coverage
                .get(&(t.row_index, col))
                .copied()
                .unwrap_or((t.row_index.saturating_sub(1), col));
            t.row.push(CellSlot::Covered {
                origin_row: origin.0,
                origin_col: origin.1,
            });
        }
        t.col += n;
    }

    fn close_row(&mut self) {
        let Some(t) = self.tables.last_mut() else {
            return;
        };
        if t.row_repeat > REPEAT_CAP {
            self.repeated_truncated += 1;
        }
        let n = t.row_repeat.min(REPEAT_CAP) as usize;
        for _ in 0..n {
            let row = t.row.clone();
            t.grid.push(row);
        }
        t.row_index += n;
        if t.in_header {
            t.header_rows += n;
        }
        t.row.clear();
        t.col = 0;
    }

    // ---- assets -----------------------------------------------------------

    fn asset_for(&mut self, part: &str) -> Option<usize> {
        if let Some(&idx) = self.asset_index.get(part) {
            return Some(idx);
        }
        let bytes = read_part(self.zip, part).ok()?;
        let media_type = guess_media_type(part);
        let idx = self.raw_assets.len();
        self.asset_index.insert(part.to_string(), idx);
        self.raw_assets.push(RawOdfAsset {
            part: part.to_string(),
            media_type,
            bytes,
        });
        Some(idx)
    }

    fn image_from_href(&mut self, href: Option<String>) {
        let Some(href) = href else {
            // No href: `office:binary-data` children may follow — capture them.
            self.binary = Some(String::new());
            return;
        };
        if href.contains("://") {
            self.push_inline(Inline::Image {
                alt: String::new(),
                source: ImageSource::External(href),
            });
            return;
        }
        let part = normalize_odf_href(&href);
        if let Some(idx) = self.asset_for(&part) {
            self.push_inline(Inline::Image {
                alt: String::new(),
                source: ImageSource::Asset(idx),
            });
        }
    }

    fn image_from_binary(&mut self, data: String) {
        use base64::Engine as _;
        let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(cleaned) else {
            return;
        };
        let part = format!("embedded/image-{}", self.raw_assets.len());
        let media_type = guess_media_type(&part);
        let idx = self.raw_assets.len();
        self.raw_assets.push(RawOdfAsset {
            part,
            media_type,
            bytes,
        });
        self.push_inline(Inline::Image {
            alt: String::new(),
            source: ImageSource::Asset(idx),
        });
    }

    // ---- event dispatch ---------------------------------------------------

    /// Element entry for both `Event::Start` and `Event::Empty`.
    fn on_element(&mut self, e: &BytesStart, start: bool) {
        let local = local_tag_name(e.name().as_ref());
        match local.as_str() {
            "p" if start => {
                let style = attr(e, "style-name");
                self.start_para(None, style.as_deref());
            }
            "h" if start => {
                let level = attr(e, "outline-level").and_then(|v| v.trim().parse::<u8>().ok());
                let style = attr(e, "style-name");
                self.start_para(Some(level.unwrap_or(1)), style.as_deref());
            }
            "span" if start => {
                let style = attr(e, "style-name");
                self.span_push(style.as_deref());
            }
            "a" if start => {
                let target = attr(e, "href").map(|h| link_target(&h));
                if let Some(target) = target {
                    self.containers.push(Container::Link {
                        content: Vec::new(),
                        target,
                    });
                }
            }
            "list" if start => {
                let marker = attr(e, "style-name")
                    .and_then(|s| self.styles.get(&s).and_then(|d| d.marker))
                    .unwrap_or(MarkerKind::Bullet);
                self.containers.push(Container::List {
                    marker,
                    items: Vec::new(),
                    current: Vec::new(),
                });
            }
            "list-item" | "list-header" if start => self.close_item(),
            "note" if start => {
                let kind = match attr(e, "note-class").as_deref() {
                    Some("endnote") => NoteKind::Endnote,
                    _ => NoteKind::Footnote,
                };
                let id = attr(e, "id").filter(|s| !s.is_empty());
                self.containers.push(Container::Note {
                    id,
                    kind,
                    blocks: Vec::new(),
                    emit_ref: true,
                });
            }
            "note" => {
                let id = self.gen_note_id();
                self.push_inline(Inline::NoteRef(id));
            }
            "note-citation" if start => self.citation = Some(String::new()),
            "notes" if start => {
                let id = format!("slide{}-notes", self.page_no);
                self.containers.push(Container::Note {
                    id: Some(id),
                    kind: NoteKind::Endnote,
                    blocks: Vec::new(),
                    emit_ref: false,
                });
            }
            "page" if start => {
                // ODP slide: every draw:page yields a "# Slide N" heading.
                self.page_no += 1;
                self.push_block(Block::Heading {
                    level: 1,
                    content: vec![Inline::Text {
                        text: format!("Slide {}", self.page_no),
                        style: Style::PLAIN,
                    }],
                });
            }
            "table" if start => self.tables.push(TableState::new()),
            "table-row" if start => {
                if let Some(t) = self.tables.last_mut() {
                    t.row.clear();
                    t.col = 0;
                    t.row_repeat = attr(e, "number-rows-repeated")
                        .and_then(|v| v.trim().parse::<u32>().ok())
                        .unwrap_or(1);
                }
            }
            "table-header-rows" if start => {
                if let Some(t) = self.tables.last_mut() {
                    t.in_header = true;
                }
            }
            "table-cell" if start => {
                let repeat = repeat_attr(e);
                let col_span = attr(e, "number-columns-spanned")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(1)
                    .max(1);
                let row_span = attr(e, "number-rows-spanned")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(1)
                    .max(1);
                self.containers.push(Container::Cell {
                    blocks: Vec::new(),
                    col_span,
                    row_span,
                    repeat,
                });
            }
            "table-cell" => {
                let repeat = repeat_attr(e);
                let col_span = attr(e, "number-columns-spanned")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(1)
                    .max(1);
                let row_span = attr(e, "number-rows-spanned")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(1)
                    .max(1);
                self.place_origin(
                    Cell {
                        blocks: Vec::new(),
                        col_span,
                        row_span,
                    },
                    repeat,
                );
            }
            "covered-table-cell" => self.place_covered(repeat_attr(e)),
            "image" => {
                let href = attr(e, "href");
                self.image_from_href(href);
            }
            "s" => {
                let count = attr(e, "c")
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(1)
                    .min(SPACE_CAP);
                self.push_inline(Inline::Text {
                    text: " ".repeat(count),
                    style: self.effective().to_style(),
                });
            }
            "tab" => self.push_inline(Inline::Text {
                text: "\t".to_string(),
                style: self.effective().to_style(),
            }),
            "line-break" => self.push_inline(Inline::LineBreak),
            _ => {}
        }
    }

    fn on_end(&mut self, local: &str) {
        match local {
            "p" | "h" => self.close_para(),
            "span" => self.span_pop(),
            "a" => self.close_link(),
            "list" => self.close_list(),
            "list-item" | "list-header" => self.close_item(),
            "note-citation" => {
                // Close the citation sink: without this the note *body* text
                // would keep appending to the citation and the body would end
                // up empty (note dropped entirely).
                self.last_citation = self.citation.take();
            }
            "note" => {
                // The citation text lands in `last_citation` during the walk;
                // an id-less note picks it up here.
                if let Some(text) = self.last_citation.take().or_else(|| self.citation.take()) {
                    let text = text.trim().to_string();
                    if !text.is_empty()
                        && let Some(Container::Note { id, .. }) = self.containers.last_mut()
                        && id.is_none()
                    {
                        *id = Some(text);
                    }
                }
                self.close_note();
            }
            "notes" => self.close_note(),
            "table-cell" => {
                if let Some(Container::Cell {
                    blocks,
                    col_span,
                    row_span,
                    repeat,
                }) = self.containers.pop()
                {
                    self.place_origin(
                        Cell {
                            blocks,
                            col_span,
                            row_span,
                        },
                        repeat,
                    );
                }
            }
            "table-row" => self.close_row(),
            "table" => {
                if let Some(t) = self.tables.pop() {
                    if !t.grid.is_empty() {
                        self.push_block(Block::Table(Table {
                            grid: t.grid,
                            header_rows: t.header_rows.max(1),
                            kind: TableKind::Data,
                        }));
                    }
                }
            }
            "table-header-rows" => {
                if let Some(t) = self.tables.last_mut() {
                    t.in_header = false;
                }
            }
            "image" => {
                if let Some(data) = self.binary.take() {
                    self.image_from_binary(data);
                }
            }
            _ => {}
        }
    }
}

/// Run the shared ODF event stream over `content.xml`.
pub(crate) fn walk_content(xml: &[u8], w: &mut OdfWalker) -> Result<(), AlchemizeError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => w.on_element(&e, true),
            Ok(Event::Empty(e)) => w.on_element(&e, false),
            Ok(Event::GeneralRef(r)) => {
                // XML entities arrive as separate events (quick-xml 0.42);
                // decode them into the same sinks as literal text.
                let text = crate::infrastructure::document::decode_entity(r.as_ref());
                if let Some(b) = w.citation.as_mut() {
                    b.push_str(&text);
                } else if let Some(b) = w.binary.as_mut() {
                    b.push_str(&text);
                } else if w.any_inline_sink() {
                    let style = w.effective().to_style();
                    w.push_inline(Inline::Text { text, style });
                }
            }
            Ok(Event::Text(t)) => {
                let raw: &str = t.as_ref();
                if let Some(b) = w.citation.as_mut() {
                    b.push_str(raw);
                } else if let Some(b) = w.binary.as_mut() {
                    b.push_str(raw);
                } else if w.any_inline_sink() {
                    let text = unescape_text(raw);
                    if !text.is_empty() {
                        let style = w.effective().to_style();
                        w.push_inline(Inline::Text { text, style });
                    }
                }
            }
            Ok(Event::End(e)) => w.on_end(local_tag_name(e.name().as_ref()).as_str()),
            Ok(Event::Eof) => break,
            Err(e) => return Err(AlchemizeError::Xml(format!("content.xml parse error: {e}"))),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Parse an ODF package (`content.xml` + optional `styles.xml`) into the
/// unified model. Shared by the ODT and ODP converters.
pub fn parse(bytes: &[u8]) -> Result<ParsedOdf, AlchemizeError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::Zip(format!("failed to open ODF package: {e}")))?;
    let content = read_part(&mut zip, "content.xml")?;
    let styles_xml = read_part(&mut zip, "styles.xml").ok();

    let mut styles: HashMap<String, OdfStyleDef> = HashMap::new();
    if let Some(xml) = styles_xml.as_deref() {
        styles.extend(parse_odf_styles(xml));
    }
    // content.xml automatic styles win over styles.xml named styles.
    styles.extend(parse_odf_styles(&content));

    let mut w = OdfWalker::new(&mut zip, styles);
    walk_content(&content, &mut w)?;

    let document = Document {
        blocks: std::mem::take(&mut w.blocks),
        notes: std::mem::take(&mut w.notes),
        assets: w
            .raw_assets
            .iter()
            .map(|a| crate::domain::document::Asset {
                media_type: a.media_type.clone(),
                origin_part: a.part.clone(),
                bytes: a.bytes.clone(),
            })
            .collect(),
    };
    Ok(ParsedOdf {
        document,
        raw_assets: w.raw_assets,
        paragraph_count: w.paragraph_count,
        repeated_truncated: w.repeated_truncated,
    })
}

/// Rewrite `ImageSource::Asset` references into sha8-named file references
/// (`images/{seq:04}-{sha8}.{ext}`), returning the assets in first-reference
/// order. Identical bytes reuse the first reference; the CLI writes each
/// asset at exactly that filename under the output directory.
pub(crate) fn rewrite_asset_refs(doc: &mut Document, raw: &[RawOdfAsset]) -> Vec<Asset> {
    struct RwCtx<'a> {
        raw: &'a [RawOdfAsset],
        by_idx: HashMap<usize, String>,
        by_sha: HashMap<String, String>,
        out: Vec<Asset>,
        seq: usize,
    }
    impl RwCtx<'_> {
        fn ref_for(&mut self, idx: usize) -> Option<String> {
            if let Some(r) = self.by_idx.get(&idx) {
                return Some(r.clone());
            }
            let a = self.raw.get(idx)?;
            let sha8 = sha8_hex(&a.bytes);
            let reference = if let Some(existing) = self.by_sha.get(&sha8) {
                existing.clone()
            } else {
                let ext = asset_ext(&a.part, &a.media_type);
                let r = format!("images/{:04}-{sha8}.{ext}", self.seq);
                self.seq += 1;
                self.out.push(Asset::new(r.clone(), a.bytes.clone()));
                self.by_sha.insert(sha8, r.clone());
                r
            };
            self.by_idx.insert(idx, reference.clone());
            Some(reference)
        }
    }

    fn walk_blocks(blocks: &mut [Block], ctx: &mut RwCtx) {
        for b in blocks {
            match b {
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
                _ => {}
            }
        }
    }

    fn walk_inlines(inlines: &mut [Inline], ctx: &mut RwCtx) {
        for i in inlines {
            match i {
                Inline::Image { source, .. } => {
                    if let ImageSource::Asset(idx) = source
                        && let Some(r) = ctx.ref_for(*idx)
                    {
                        *source = ImageSource::External(r);
                    }
                }
                Inline::Link { content, .. } => walk_inlines(content, ctx),
                _ => {}
            }
        }
    }

    let mut ctx = RwCtx {
        raw,
        by_idx: HashMap::new(),
        by_sha: HashMap::new(),
        out: Vec::new(),
        seq: 0,
    };
    for note in &mut doc.notes {
        walk_blocks(&mut note.blocks, &mut ctx);
    }
    walk_blocks(&mut doc.blocks, &mut ctx);
    ctx.out
}

/// Shared converter tail for ODT and ODP: rewrite asset refs, render, and
/// assemble the [`ConvertResult`].
pub(crate) fn build_result(
    parsed: ParsedOdf,
    source_format: &str,
) -> Result<ConvertResult, AlchemizeError> {
    let mut doc = parsed.document;
    let assets = rewrite_asset_refs(&mut doc, &parsed.raw_assets);
    let markdown = crate::infrastructure::document::gfm_render::render_markdown(&doc);
    let metadata = serde_json::json!({
        "source_format": source_format,
        "paragraph_count": parsed.paragraph_count,
        "asset_count": assets.len(),
        "repeated_truncated": parsed.repeated_truncated,
    });
    let mut out = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
        .with_quality(ConversionQuality::LocalFull);
    for a in assets {
        out = out.with_asset(a);
    }
    Ok(out)
}

/// Deep ODT converter: OpenDocument text → the unified document model.
pub struct OdtConverter;

impl OdtConverter {
    /// Create a new deep ODT converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for OdtConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for OdtConverter {
    fn name(&self) -> &'static str {
        "odt-deep"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read ODT input: {e}")))?;
        let parsed = tokio::task::spawn_blocking(move || parse(&bytes))
            .await
            .map_err(|e| AlchemizeError::other(format!("odt-deep task panicked: {e}")))??;
        build_result(parsed, "odt")
    }

    fn supported_extensions(&self) -> &[&str] {
        &["odt"]
    }

    fn priority(&self) -> u8 {
        18
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

// ---- XML helpers ----------------------------------------------------------

/// Read an attribute by local name (namespace-prefix agnostic).
fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| attr_local(a.key.as_ref()) == name)
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

fn attr_local(key: &str) -> &str {
    match key.rsplit_once(':') {
        Some((_, local)) => local,
        None => key,
    }
}

fn repeat_attr(e: &BytesStart) -> u32 {
    attr(e, "number-columns-repeated")
        .or_else(|| attr(e, "number-rows-repeated"))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .max(1)
}

fn unescape_text(raw: &str) -> String {
    quick_xml::escape::unescape(raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| raw.to_string())
}

fn parse_pt(v: &str) -> Option<f32> {
    let t = v.trim();
    let num = t.strip_suffix("pt").unwrap_or(t);
    num.trim().parse::<f32>().ok().filter(|f| f.is_finite())
}

fn num_marker(fmt: Option<&str>) -> MarkerKind {
    match fmt {
        Some("a") => MarkerKind::LowerAlpha,
        Some("A") => MarkerKind::UpperAlpha,
        Some("i") => MarkerKind::LowerRoman,
        Some("I") => MarkerKind::UpperRoman,
        _ => MarkerKind::Decimal,
    }
}

/// Classify an `xlink:href` from `text:a` into external vs relative.
fn link_target(href: &str) -> LinkTarget {
    const SCHEMES: [&str; 5] = ["http://", "https://", "mailto:", "ftp://", "file://"];
    if SCHEMES.iter().any(|s| href.starts_with(s)) {
        LinkTarget::External(href.to_string())
    } else {
        LinkTarget::Relative(href.to_string())
    }
}

/// Resolve a package path from a draw:image href ("../Pictures/x" → "Pictures/x").
fn normalize_odf_href(href: &str) -> String {
    let mut s = href;
    while let Some(rest) = s.strip_prefix("../") {
        s = rest;
    }
    s.strip_prefix("./")
        .or_else(|| s.strip_prefix('/'))
        .unwrap_or(s)
        .to_string()
}

/// Extension for the sha8-named file: the package part's extension first,
/// media-type subtype second.
fn asset_ext(part: &str, media_type: &str) -> String {
    if let Some((_, ext)) = part.rsplit_once('.') {
        if !ext.is_empty() && !ext.contains('/') {
            return ext.to_lowercase();
        }
    }
    let sub = media_type.rsplit('/').next().unwrap_or("bin");
    if sub.is_empty() || sub == "octet-stream" {
        "bin".to_string()
    } else {
        sub.replace('+', "-")
    }
}

/// Leading 8 hex chars of the SHA-256 digest.
fn sha8_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Resolve a style through its `style:parent-style-name` chain (root first,
/// most-derived last), yielding the merged run properties.
fn resolve_style(styles: &HashMap<String, OdfStyleDef>, name: &str) -> RunProps {
    let mut chain: Vec<&OdfStyleDef> = Vec::new();
    let mut visited: Vec<String> = Vec::new();
    let mut cur = Some(name.to_string());
    while let Some(n) = cur {
        if visited.iter().any(|v| v == &n) || chain.len() > 64 {
            break; // cycle guard
        }
        visited.push(n.clone());
        let Some(def) = styles.get(&n) else {
            break;
        };
        chain.push(def);
        cur = def.parent.clone();
    }
    let mut props = RunProps::default();
    for def in chain.iter().rev() {
        props = props.merge(RunProps {
            bold: def.bold,
            italic: def.italic,
            size_pt: def.size_pt,
        });
    }
    props
}

/// Extract every `style:style` / `text:list-style` declaration from an ODF
/// XML part (works for content.xml automatic styles and styles.xml alike).
pub(crate) fn parse_odf_styles(xml: &[u8]) -> HashMap<String, OdfStyleDef> {
    let mut out: HashMap<String, OdfStyleDef> = HashMap::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut current: Option<(String, OdfStyleDef)> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "style" | "list-style" => {
                        let name = attr(&e, "name").unwrap_or_default();
                        let mut def = OdfStyleDef::default();
                        def.parent = attr(&e, "parent-style-name");
                        if !name.is_empty() {
                            current = Some((name, def));
                        }
                    }
                    "text-properties" => {
                        if let Some((_, def)) = &mut current {
                            match attr(&e, "font-weight").as_deref() {
                                Some("bold") => def.bold = Some(true),
                                Some("normal") => def.bold = Some(false),
                                _ => {}
                            }
                            match attr(&e, "font-style").as_deref() {
                                Some("italic") => def.italic = Some(true),
                                Some("normal") => def.italic = Some(false),
                                _ => {}
                            }
                            if let Some(sz) = attr(&e, "font-size") {
                                def.size_pt = parse_pt(&sz);
                            }
                        }
                    }
                    "list-level-style-number" => {
                        if let Some((_, def)) = &mut current {
                            def.marker = Some(num_marker(attr(&e, "num-format").as_deref()));
                        }
                    }
                    "list-level-style-bullet" => {
                        if let Some((_, def)) = &mut current {
                            def.marker = Some(MarkerKind::Bullet);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let local = local_tag_name(e.name().as_ref());
                let style_done = local == "style" || local == "list-style";
                if style_done && let Some((name, def)) = current.take() {
                    out.insert(name, def);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_parts(parts: &[(&str, &[u8])]) -> Vec<u8> {
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

    fn odt_with(content: &str) -> Vec<u8> {
        zip_parts(&[("content.xml", content.as_bytes())])
    }

    async fn convert(bytes: Vec<u8>) -> ConvertResult {
        OdtConverter::new()
            .convert(Input::bytes(bytes, "t.odt"))
            .await
            .unwrap()
    }

    const NS: &str = r#"xmlns:office="o" xmlns:text="t" xmlns:style="s" xmlns:fo="f" xmlns:table="tb" xmlns:draw="d" xmlns:xlink="x""#;

    #[test]
    fn odf_style_parser_covers_negative_toggles_and_markers() {
        let xml = r#"<?xml version="1.0"?><office:document-styles xmlns:office="o" xmlns:style="s" xmlns:text="t" xmlns:fo="f">
<style:style style:name="Base"><style:text-properties fo:font-weight="bold" fo:font-size="12pt"/></style:style>
<style:style style:name="Derived" style:parent-style-name="Base"><style:text-properties fo:font-weight="normal" fo:font-style="normal" fo:font-size="bogus"/></style:style>
<style:style style:name="CycleA" style:parent-style-name="CycleB"/>
<style:style style:name="CycleB" style:parent-style-name="CycleA"/>
<style:list-style style:name="LUpper"><text:list-level-style-number text:num-format="I"/></style:list-style>
<style:list-style style:name="LAlpha"><text:list-level-style-number text:num-format="a"/></style:list-style>
<style:list-style style:name="LBullet"><text:list-level-style-bullet/></style:list-style>
<style:list-style style:name="LDefault"><text:list-level-style-number/></style:list-style>
</office:document-styles>"#;
        let styles = parse_odf_styles(xml.as_bytes());
        let derived = resolve_style(&styles, "Derived");
        assert_eq!(
            derived.bold,
            Some(false),
            "explicit normal clears parent bold"
        );
        assert_eq!(derived.italic, Some(false));
        assert_eq!(
            derived.size_pt,
            Some(12.0),
            "an unparsable own font-size inherits the parent value"
        );
        assert_eq!(
            styles.get("LUpper").and_then(|d| d.marker),
            Some(MarkerKind::UpperRoman)
        );
        assert_eq!(
            styles.get("LAlpha").and_then(|d| d.marker),
            Some(MarkerKind::LowerAlpha)
        );
        assert_eq!(
            styles.get("LBullet").and_then(|d| d.marker),
            Some(MarkerKind::Bullet)
        );
        assert_eq!(
            styles.get("LDefault").and_then(|d| d.marker),
            Some(MarkerKind::Decimal)
        );
        // Cycle guard: resolving terminates and fabricates no flags.
        let cycle = resolve_style(&styles, "CycleA");
        assert_eq!(cycle.bold, None);
    }

    #[tokio::test]
    async fn empty_elements_binary_images_and_normalized_hrefs() {
        let content = format!(
            r#"<?xml version="1.0"?><office:document-content {NS}><office:body><office:text>
<text:h style-name="P2">No level heading</text:h>
<text:p style-name="P1">normal run</text:p>
<text:p><text:a>no href</text:a><text:s text:c="oops"/><text:s text:c="99999"/>end</text:p>
<text:p><text:note/></text:p>
<table:table><table:table-row>
<table:table-cell/><table:covered-table-cell/><table:table-cell><text:p>kept</text:p></table:table-cell>
</table:table-row></table:table>
<text:p><draw:image href="http://x.test/a.png"/></text:p>
<text:p><draw:image href="../Pictures/b.bin"/></text:p>
<text:p><draw:image><office:binary-data>aGVsbG8=</office:binary-data></draw:image></text:p>
<text:p><draw:image><office:binary-data>!!!notbase64!!!</office:binary-data></draw:image></text:p>
</office:text></office:body></office:document-content>"#
        );
        let styles = format!(
            r#"<?xml version="1.0"?><office:document-styles {NS}><office:automatic-styles>
<style:style style:name="P1"><style:text-properties fo:font-weight="normal" fo:font-style="normal"/></style:style>
<style:style style:name="P2" style:parent-style-name="P1"><style:text-properties fo:font-weight="bold"/></style:style>
</office:automatic-styles></office:document-styles>"#
        );
        let bytes = zip_parts(&[
            ("content.xml", content.as_bytes()),
            ("styles.xml", styles.as_bytes()),
            ("Pictures/b.bin", b"binary-part"),
        ]);
        let result = convert(bytes).await;
        let md = &result.markdown;
        // h without outline-level defaults to level 1.
        assert!(md.contains("# **No level heading**"), "md: {md}");
        // Explicit normal toggles clear inherited property state.
        assert!(md.contains("normal run"), "md: {md}");
        assert!(!md.contains("**normal run**"), "md: {md}");
        assert!(md.contains("kept"), "empty cells keep the row: {md}");
        assert!(
            md.contains("](http://x.test/a.png)"),
            "external image: {md}"
        );
        assert!(
            md.contains("no href"),
            "anchor without href keeps text: {md:?}"
        );
        // Real part + base64 payload; the invalid payload adds nothing.
        assert_eq!(result.assets.len(), 2, "assets: {:?}", result.assets);
        assert_eq!(
            md.matches("](images/").count(),
            2,
            "internal image links: {md}"
        );
    }

    #[test]
    fn converter_traits() {
        let c = OdtConverter::new();
        assert_eq!(c.name(), "odt-deep");
        assert_eq!(c.supported_extensions(), &["odt"]);
        assert_eq!(c.priority(), 18);
        assert_eq!(c.availability(), Availability::Available);
    }

    #[tokio::test]
    async fn heading_level_comes_from_outline_level() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:h text:outline-level="2">Chapter Head</text:h>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("## Chapter Head"), "md: {}", r.markdown);
    }

    #[tokio::test]
    async fn span_style_resolves_bold_and_parent_chain() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}>
<office:automatic-styles>
<style:style style:name="Strong" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
<style:style style:name="TItal" style:family="text" style:parent-style-name="Strong"><style:text-properties fo:font-style="italic"/></style:style>
</office:automatic-styles>
<office:body><office:text>
<text:p><text:span text:style-name="Strong">bold run</text:span> and <text:span text:style-name="TItal">inherited italic</text:span></text:p>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("**bold run**"), "md: {}", r.markdown);
        assert!(
            r.markdown.contains("***inherited italic***"),
            "md: {}",
            r.markdown
        );
    }

    #[tokio::test]
    async fn table_spans_and_covered_cells() {
        let bytes = odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<table:table>
<table:table-row>
<table:table-cell><text:p>A</text:p></table:table-cell>
<table:table-cell table:number-columns-spanned="2"><text:p>Wide</text:p></table:table-cell>
</table:table-row>
<table:table-row>
<table:table-cell table:number-rows-spanned="2"><text:p>M</text:p></table:table-cell>
<table:table-cell><text:p>N</text:p></table:table-cell>
</table:table-row>
<table:table-row>
<table:covered-table-cell/>
<table:table-cell><text:p>O</text:p></table:table-cell>
</table:table-row>
</table:table>
</office:text></office:body></office:document-content>"#
        ));
        let r = convert(bytes.clone()).await;
        assert!(r.markdown.contains("| A | Wide |"), "md: {}", r.markdown);
        let doc = parse(&bytes).unwrap().document;
        let table = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("table block");
        let CellSlot::Origin(wide) = &table.grid[0][1] else {
            panic!("expected origin cell");
        };
        assert_eq!(wide.col_span, 2, "horizontal span");
        let CellSlot::Origin(merged) = &table.grid[1][0] else {
            panic!("expected origin cell");
        };
        assert_eq!(merged.row_span, 2, "vertical span origin");
        assert!(
            matches!(
                table.grid[2][0],
                CellSlot::Covered {
                    origin_row: 1,
                    origin_col: 0
                }
            ),
            "covered cell must point at its origin"
        );
    }

    #[tokio::test]
    async fn repeated_columns_truncate_and_count() {
        let bytes = odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<table:table><table:table-row>
<table:table-cell><text:p>Only</text:p></table:table-cell>
<table:table-cell table:number-columns-repeated="1500"/>
</table:table-row></table:table>
</office:text></office:body></office:document-content>"#
        ));
        let r = convert(bytes.clone()).await;
        assert_eq!(
            r.metadata["repeated_truncated"].as_u64(),
            Some(1),
            "meta: {}",
            r.metadata
        );
        let doc = parse(&bytes).unwrap().document;
        let table = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("table block");
        assert_eq!(table.grid[0].len(), 1 + REPEAT_CAP as usize, "capped width");
    }

    #[tokio::test]
    async fn images_get_sha8_names_and_dedupe_by_bytes() {
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let bytes = zip_parts(&[
            (
                "content.xml",
                format!(
                    r#"<office:document-content {NS}><office:body><office:text>
<text:p><draw:frame><draw:image xlink:href="Pictures/a.png"/></draw:frame></text:p>
<text:p><draw:frame><draw:image xlink:href="Pictures/b.png"/></draw:frame></text:p>
</office:text></office:body></office:document-content>"#
                )
                .as_bytes(),
            ),
            ("Pictures/a.png", png),
            ("Pictures/b.png", png),
        ]);
        let r = convert(bytes).await;
        let sha8 = Asset::new("x.png", png.to_vec()).sha8();
        let reference = format!("images/0000-{sha8}.png");
        assert_eq!(r.assets.len(), 1, "same bytes in two parts dedupe");
        assert_eq!(
            r.assets[0].filename, reference,
            "filename must match md ref"
        );
        assert_eq!(
            r.markdown.matches(&format!("![]({reference})")).count(),
            2,
            "both occurrences reuse one reference: {}",
            r.markdown
        );
        assert_eq!(r.assets[0].bytes.len(), png.len());
    }

    #[tokio::test]
    async fn footnote_body_and_inline_reference() {
        let bytes = odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p>Claimed<text:note text:note-class="footnote" text:id="ftn1"><text:note-citation>1</text:note-citation><text:note-body><text:p>note body here</text:p></text:note-body></text:note>.</text:p>
</office:text></office:body></office:document-content>"#
        ));
        let r = convert(bytes.clone()).await;
        assert!(r.markdown.contains("note body here"), "md: {}", r.markdown);
        let doc = parse(&bytes).unwrap().document;
        assert_eq!(doc.notes.len(), 1);
        assert_eq!(doc.notes[0].id, "ftn1", "text:id wins over citation text");
        assert_eq!(doc.notes[0].kind, NoteKind::Footnote);
        let has_ref = doc.blocks.iter().any(|b| match b {
            Block::Paragraph(inlines) => inlines
                .iter()
                .any(|i| matches!(i, Inline::NoteRef(id) if id == "ftn1")),
            _ => false,
        });
        assert!(has_ref, "note reference must appear inline");
    }

    #[tokio::test]
    async fn footnote_id_falls_back_to_citation_text() {
        let bytes = odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p>C<text:note text:note-class="footnote"><text:note-citation>7</text:note-citation><text:note-body><text:p>body</text:p></text:note-body></text:note>.</text:p>
</office:text></office:body></office:document-content>"#
        ));
        let doc = parse(&bytes).unwrap().document;
        assert_eq!(doc.notes.len(), 1);
        assert_eq!(doc.notes[0].id, "7", "citation text becomes the id");
    }

    #[tokio::test]
    async fn lists_resolve_bullet_and_numbered_markers() {
        let bytes = odt_with(&format!(
            r#"<office:document-content {NS}>
<office:automatic-styles>
<style:style style:name="LNum" style:family="list"><text:list-style style:name="LNum"><text:list-level-style-number text:level="1" style:num-format="1"/></text:list-style></style:style>
</office:automatic-styles>
<office:body><office:text>
<text:list><text:list-item><text:p>bullet alpha</text:p></text:list-item>
<text:list-item><text:p>bullet beta</text:p><text:list><text:list-item><text:p>nested bullet</text:p></text:list-item></text:list></text:list-item></text:list>
<text:list text:style-name="LNum"><text:list-item><text:p>numbered one</text:p></text:list-item><text:list-item><text:p>numbered two</text:p></text:list-item></text:list>
</office:text></office:body></office:document-content>"#
        ));
        let doc = parse(&bytes).unwrap().document;
        let lists: Vec<&List> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::List(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lists.len(), 2, "top-level lists");
        assert_eq!(lists[0].marker, MarkerKind::Bullet);
        assert_eq!(lists[0].items.len(), 2);
        let nested = lists[0].items[1]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::List(l) => Some(l),
                _ => None,
            })
            .expect("nested list inside item");
        assert_eq!(nested.items.len(), 1);
        assert_eq!(
            lists[1].marker,
            MarkerKind::Decimal,
            "num-format 1 → decimal"
        );
        let r = convert(bytes).await;
        assert!(r.markdown.contains("- bullet alpha"), "md: {}", r.markdown);
        assert!(r.markdown.contains("1. numbered one"), "md: {}", r.markdown);
        assert!(r.markdown.contains("2. numbered two"), "md: {}", r.markdown);
    }

    #[tokio::test]
    async fn spaces_tabs_and_line_break_reach_markdown() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p>a<text:tab/>b<text:line-break/>c<text:s text:c="3"/>d</text:p>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("a\tb"), "md: {:?}", r.markdown);
        assert!(r.markdown.contains("c   d"), "md: {:?}", r.markdown);
        assert!(
            r.markdown.contains("  \n"),
            "hard break renders: {:?}",
            r.markdown
        );
    }

    #[tokio::test]
    async fn entities_are_unescaped() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p>A &amp; B &lt;tag&gt; done</text:p>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert!(
            r.markdown.contains("A & B <tag> done"),
            "md: {}",
            r.markdown
        );
    }

    #[tokio::test]
    async fn link_targets_classify_external_and_relative() {
        let bytes = odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p><text:a xlink:href="https://example.com/x">web</text:a> and <text:a xlink:href="specs/a.odt">rel</text:a></text:p>
</office:text></office:body></office:document-content>"#
        ));
        let doc = parse(&bytes).unwrap().document;
        let mut targets = Vec::new();
        for b in &doc.blocks {
            if let Block::Paragraph(inlines) = b {
                for i in inlines {
                    if let Inline::Link { target, .. } = i {
                        targets.push(target.clone());
                    }
                }
            }
        }
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0],
            LinkTarget::External("https://example.com/x".into())
        );
        assert_eq!(targets[1], LinkTarget::Relative("specs/a.odt".into()));
    }

    #[tokio::test]
    async fn paragraph_count_counts_blocks() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:h text:outline-level="1">T</text:h>
<text:p>one</text:p>
<text:p>two</text:p>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert_eq!(
            r.metadata["paragraph_count"].as_u64(),
            Some(3),
            "headings count too: {}",
            r.metadata
        );
    }

    #[tokio::test]
    async fn image_href_resolves_and_becomes_asset() {
        let content = format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p><draw:frame><draw:image xlink:href="../Pictures/a.png"/></draw:frame></text:p>
</office:text></office:body></office:document-content>"#
        );
        let png: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        let r = convert(zip_parts(&[
            ("content.xml", content.as_bytes()),
            ("Pictures/a.png", png),
        ]))
        .await;
        assert_eq!(r.assets.len(), 1, "one picture asset: {r:?}");
        assert!(
            r.assets[0].filename.starts_with("images/"),
            "sha8-named asset: {}",
            r.assets[0].filename
        );
        assert!(r.markdown.contains("!["), "image reference: {}", r.markdown);
    }

    #[tokio::test]
    async fn list_markers_cover_alpha_and_roman() {
        for (num_format, expected) in [
            ("a", "a."),
            ("A", "A."),
            ("i", "i."),
            ("I", "I."),
            ("1", "1."),
        ] {
            let r = convert(odt_with(&format!(
                r#"<office:document-content {NS}>
<office:automatic-styles>
<text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="{num_format}"/></text:list-style>
</office:automatic-styles>
<office:body><office:text>
<text:list text:style-name="L"><text:list-item><text:p>item</text:p></text:list-item></text:list>
</office:text></office:body></office:document-content>"#
            )))
            .await;
            assert!(
                r.markdown.contains(expected),
                "num-format {num_format} should render '{expected}': {}",
                r.markdown
            );
        }
    }

    #[tokio::test]
    async fn header_rows_are_marked() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<table:table>
<table:table-header-rows>
<table:table-row><table:table-cell><text:p>H</text:p></table:table-cell></table:table-row>
</table:table-header-rows>
<table:table-row><table:table-cell><text:p>1</text:p></table:table-cell></table:table-row>
</table:table>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("| H |"), "header row: {}", r.markdown);
        assert!(r.markdown.contains("| 1 |"), "body row: {}", r.markdown);
    }

    #[tokio::test]
    async fn unknown_span_style_falls_back_to_plain() {
        let r = convert(odt_with(&format!(
            r#"<office:document-content {NS}><office:body><office:text>
<text:p><text:span text:style-name="NoSuchStyle">plain</text:span></text:p>
</office:text></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("plain"));
        assert!(!r.markdown.contains("**plain**"), "{}", r.markdown);
    }

    #[tokio::test]
    async fn odt_convert_requires_content_xml() {
        // Zip without content.xml: explicit error, not empty output.
        use std::io::Write;
        let buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("mimetype", opts).unwrap();
        w.write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();
        w.start_file("other.xml", opts).unwrap();
        w.write_all(b"<x/>").unwrap();
        let bytes = w.finish().unwrap().into_inner();
        assert!(parse(&bytes).is_err());
    }

    #[tokio::test]
    async fn real_odt_fixture_parses() {
        // 构造合法 fixture（磁盘未找到真实 ODT 样本）：断言其 content.xml
        // 中的真实关键词与资产命名。
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real/real.odt");
        let r = OdtConverter::new()
            .convert(Input::Path(path))
            .await
            .unwrap();
        assert!(
            r.markdown.contains("Quarterly Review"),
            "md: {}",
            r.markdown
        );
        assert!(r.markdown.contains("**bold run**"), "md: {}", r.markdown);
        assert!(
            r.markdown.contains("***inherited italic***"),
            "styles.xml parent chain applies: {}",
            r.markdown
        );
        assert!(
            r.markdown.contains("note body sentence"),
            "footnote reaches md: {}",
            r.markdown
        );
        assert!(r.markdown.contains("images/0000-"), "md: {}", r.markdown);
        assert_eq!(r.metadata["source_format"], "odt");
        assert!(r.metadata["paragraph_count"].as_u64().unwrap_or(0) > 0);
        assert_eq!(r.assets.len(), 1, "one picture asset: {}", r.metadata);
        let name = &r.assets[0].filename;
        assert!(
            name.starts_with("images/0000-") && name.ends_with(".png"),
            "asset filename: {name}"
        );
    }
}
