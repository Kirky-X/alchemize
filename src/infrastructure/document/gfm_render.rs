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
                out.push_str(render_inlines(content).trim());
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
        // GFM only distinguishes bullets from ordered lists, but the ordinal
        // itself can still reflect the source family (a/b/c, i/ii/iii).
        let marker = match list.marker {
            MarkerKind::Bullet => "- ".to_string(),
            MarkerKind::Decimal => format!("{ordinal}. "),
            MarkerKind::LowerAlpha => format!("{}. ", alpha_marker(ordinal, false)),
            MarkerKind::UpperAlpha => format!("{}. ", alpha_marker(ordinal, true)),
            MarkerKind::LowerRoman => format!("{}. ", roman_marker(ordinal).to_lowercase()),
            MarkerKind::UpperRoman => format!("{}. ", roman_marker(ordinal)),
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

/// Alpha ordinal: 1 → a/A, 27 → aa/AA (spreadsheet-column style).
fn alpha_marker(mut n: u64, upper: bool) -> String {
    if n == 0 {
        n = 1;
    }
    let mut out = String::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        let ch = (b'a' + rem) as char;
        out.insert(0, if upper { ch.to_ascii_uppercase() } else { ch });
        n = (n - 1) / 26;
    }
    out
}

/// Roman numeral for the ordinals a list realistically reaches.
fn roman_marker(mut n: u64) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    const TABLE: &[(u64, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, glyph) in TABLE {
        while n >= *value {
            out.push_str(glyph);
            n -= value;
        }
    }
    out
}

#[cfg(test)]
mod marker_tests {
    use super::*;
    use crate::domain::document::{ListItem, Style};

    #[test]
    fn alpha_markers_follow_spreadsheet_columns() {
        assert_eq!(alpha_marker(1, false), "a");
        assert_eq!(alpha_marker(26, false), "z");
        assert_eq!(alpha_marker(27, false), "aa");
        assert_eq!(alpha_marker(2, true), "B");
        assert_eq!(alpha_marker(0, false), "a");
    }

    #[test]
    fn roman_markers_render_and_fall_back() {
        assert_eq!(roman_marker(1), "I");
        assert_eq!(roman_marker(4), "IV");
        assert_eq!(roman_marker(1994), "MCMXCIV");
        assert_eq!(roman_marker(4000), "4000");
        assert_eq!(roman_marker(0), "0");
    }

    #[test]
    fn lists_use_their_marker_family() {
        let list = |marker| List {
            marker,
            start: 1,
            items: (1..=4)
                .map(|i| ListItem {
                    blocks: vec![Block::Paragraph(vec![Inline::Text {
                        text: format!("i{i}"),
                        style: Style::PLAIN,
                    }])],
                })
                .collect(),
        };
        let render = |m| {
            let mut out = String::new();
            render_list(&list(m), 0, &mut out);
            out
        };
        assert!(render(MarkerKind::Bullet).contains("- i1"));
        assert!(render(MarkerKind::Decimal).contains("3. i3"));
        assert!(render(MarkerKind::LowerAlpha).contains("c. i3"));
        assert!(render(MarkerKind::UpperAlpha).contains("D. i4"));
        assert!(render(MarkerKind::LowerRoman).contains("iv. i4"));
        assert!(render(MarkerKind::UpperRoman).contains("II. i2"));
    }
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
    for _ in 0..cols {
        out.push_str("---|");
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
    // First, coalesce adjacent text runs with identical styles (Word emits
    // one run per toggle; GFM wants one span per styled region).
    let mut coalesced: Vec<Coalesced> = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text { text, style } => match coalesced.last_mut() {
                Some(Coalesced::Text {
                    text: prev,
                    style: prev_style,
                }) if prev_style == style => {
                    prev.push_str(text);
                }
                _ => coalesced.push(Coalesced::Text {
                    text: text.clone(),
                    style: *style,
                }),
            },
            other => coalesced.push(Coalesced::Other(other)),
        }
    }

    let mut out = String::new();
    for item in &coalesced {
        match item {
            Coalesced::Text { text, style } => {
                let (open, close) = style_markers(style);
                // Trim inner edge whitespace out of the emphasis markers so
                // `**bold **` (invalid GFM emphasis) becomes `**bold** `.
                let trimmed = text.trim_matches(' ');
                let lead = text.len() - text.trim_start_matches(' ').len();
                let trail = text.len() - text.trim_end_matches(' ').len();
                if open.is_empty() {
                    out.push_str(text);
                } else {
                    out.push_str(&" ".repeat(lead));
                    out.push_str(open);
                    out.push_str(trimmed);
                    out.push_str(close);
                    out.push_str(&" ".repeat(trail));
                }
            }
            Coalesced::Other(inline) => render_inline_other(inline, &mut out),
        }
    }
    out
}

/// A coalesced inline: either a maximal same-style text run or any other
/// inline element, passed through untouched.
enum Coalesced<'a> {
    Text {
        text: String,
        style: crate::domain::document::Style,
    },
    Other(&'a Inline),
}

fn render_inline_other(inline: &Inline, out: &mut String) {
    match inline {
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
        Inline::Text { text, style } => {
            let (open, close) = style_markers(style);
            out.push_str(open);
            out.push_str(text);
            out.push_str(close);
        }
    }
}

/// Opening/closing markers for a text style (`**` bold, `*` italic).
fn style_markers(style: &crate::domain::document::Style) -> (&'static str, &'static str) {
    match (style.bold, style.italic) {
        (true, true) => ("***", "***"),
        (true, false) => ("**", "**"),
        (false, true) => ("*", "*"),
        (false, false) => ("", ""),
    }
}
