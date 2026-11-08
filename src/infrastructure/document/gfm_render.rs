// SPDX-License-Identifier: Apache-2.0

//! GFM renderer for the self-hosted document model
//! ([`crate::domain::document`]) — the single rendering layer for every
//! self-hosted parser. Emits GitHub-Flavored Markdown with images rendered
//! as `![alt](images/...)` once asset references are rewritten.

use crate::domain::document::{
    Block, CellSlot, Document, ImageSource, Inline, LinkTarget, List, MarkerKind, NoteKind,
    TableKind,
};

/// Render a document (already asset-rewritten) to GFM markdown.
pub fn render_markdown(doc: &Document) -> String {
    let mut out = String::new();
    render_blocks(&doc.blocks, &mut out);
    if !doc.notes.is_empty() {
        out.push_str("\n---\n");
        for note in &doc.notes {
            let label = match note.kind {
                NoteKind::Footnote => "footnote",
                NoteKind::Endnote => "endnote",
            };
            out.push_str(&format!("\n[^{}]: ", label));
            let mut body = String::new();
            render_blocks(&note.blocks, &mut body);
            out.push_str(body.trim());
            out.push('\n');
        }
    }
    out
}

fn render_blocks(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { level, content } => {
                let depth = (*level).clamp(1, 6);
                out.push('\n');
                out.push_str(&"#".repeat(depth as usize));
                out.push(' ');
                out.push_str(&render_inlines(content).trim().to_string());
                out.push_str("\n\n");
            }
            Block::Paragraph(inlines) => {
                let text = render_inlines(inlines);
                if !text.trim().is_empty() {
                    out.push_str(text.trim_end());
                    out.push_str("\n\n");
                }
            }
            Block::List(list) => render_list(list, 0, out),
            Block::Table(table) => render_table(table, out),
            Block::BlockQuote(nested) => {
                let mut inner = String::new();
                render_blocks(nested, &mut inner);
                for line in inner.trim_end().split('\n') {
                    out.push_str("> ");
                    out.push_str(line);
                    out.push('\n');
                }
                out.push('\n');
            }
            Block::CodeBlock { lang, text } => {
                let lang = lang.as_deref().unwrap_or("");
                out.push_str(&format!("```{lang}\n{text}\n```\n\n"));
            }
            Block::Rule => out.push_str("---\n\n"),
            Block::Math(tex) => {
                out.push_str(&format!("$${tex}$$\n\n"));
            }
        }
    }
}

fn render_list(list: &List, depth: usize, out: &mut String) {
    let mut ordinal = list.start;
    for item in &list.items {
        let marker = match list.marker {
            MarkerKind::Bullet => "- ".to_string(),
            _ => format!("{ordinal}. "),
        };
        out.push_str(&"  ".repeat(depth));
        out.push_str(&marker);
        let mut body = String::new();
        render_blocks(&item.blocks, &mut body);
        // First paragraph joins the marker line; subsequent blocks keep their
        // own formatting with the list indent.
        let mut lines = body.trim_end().splitn(2, '\n');
        if let Some(first) = lines.next() {
            out.push_str(first.trim());
            out.push('\n');
        }
        if let Some(rest) = lines.next() {
            for line in rest.split('\n') {
                if line.trim().is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&"  ".repeat(depth + 1));
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        ordinal += 1;
    }
    out.push('\n');
}

fn render_table(table: &crate::domain::document::Table, out: &mut String) {
    let cols = table.grid.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in &table.grid {
        let mut cells = Vec::with_capacity(cols);
        for slot in row {
            match slot {
                CellSlot::Origin(cell) => {
                    let mut body = String::new();
                    render_blocks(&cell.blocks, &mut body);
                    cells.push(escape_cell(body.trim()));
                }
                CellSlot::Covered { .. } => cells.push(String::new()),
            }
        }
        cells.resize(cols, String::new());
        rows.push(cells);
    }
    let header_rows = match table.kind {
        TableKind::Data => table.header_rows.max(1),
        TableKind::Layout => 0,
    };
    let header: Vec<String> = rows.first().cloned().unwrap_or_default();
    if header_rows > 0 {
        rows.remove(0);
    }
    out.push_str("| ");
    out.push_str(&header.join(" | "));
    out.push_str(" |\n|");
    for c in 0..cols {
        out.push_str(if c + 1 < cols { "---|" } else { "---|" });
    }
    out.push_str("\n");
    for row in rows {
        out.push_str("| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |\n");
    }
    out.push('\n');
}

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn render_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text, .. } => out.push_str(text),
            Inline::Link { content, target } => {
                let label = render_inlines(content);
                let url = match target {
                    LinkTarget::External(u) | LinkTarget::Relative(u) => u.clone(),
                    LinkTarget::Anchor(id) => format!("#{id}"),
                };
                out.push_str(&format!("[{label}]({url})"));
            }
            Inline::Image { alt, source } => match source {
                ImageSource::External(url) => {
                    out.push_str(&format!("![{alt}]({url})"));
                }
                ImageSource::Asset(_) | ImageSource::Unavailable => {
                    out.push_str(alt);
                }
            },
            Inline::Math(tex) => out.push_str(&format!("${tex}$")),
            Inline::Checkbox(checked) => {
                out.push_str(if *checked { "[x] " } else { "[ ] " });
            }
            Inline::Anchor(_) | Inline::NoteRef(_) => {}
            Inline::LineBreak => out.push_str("  \n"),
        }
    }
    out
}
