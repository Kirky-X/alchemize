// SPDX-License-Identifier: Apache-2.0

//! GFM renderer for the self-hosted document model
//! ([`crate::domain::document`]) — the single rendering layer for every
//! self-hosted parser. Emits GitHub-Flavored Markdown with images rendered
//! as `![alt](images/...)` once asset references are rewritten.

use crate::domain::document::{
    Block, CellSlot, Document, ImageSource, Inline, LinkTarget, List, MarkerKind, NoteKind,
};
use crate::infrastructure::document::shared::fence_for;

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
                // Info string: ASCII alphanumerics only — the language tag
                // must not smuggle markdown through the fence line.
                let lang: String = lang
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+'))
                    .take(16)
                    .collect();
                let fence = fence_for(text);
                out.push_str(&format!("{fence}{lang}\n{text}\n{fence}\n\n"));
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
    // GFM requires a header row: the first rendered row always becomes the
    // header regardless of the source's layout/data classification —
    // otherwise a layout table would duplicate its first row in the body.
    let header: Vec<String> = if rows.is_empty() {
        Vec::new()
    } else {
        rows.remove(0)
    };
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
                // Inline code wins over emphasis markers (GFM renders no
                // emphasis inside backticks).
                if style.code {
                    let trimmed = text.trim_matches('`');
                    out.push('`');
                    out.push_str(trimmed);
                    out.push('`');
                    continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{Cell, CellSlot, ListItem, Note, NoteKind, Style, TableKind};

    fn text(s: &str) -> Inline {
        Inline::Text {
            text: s.to_string(),
            style: Style::PLAIN,
        }
    }

    fn bold(s: &str) -> Inline {
        Inline::Text {
            text: s.to_string(),
            style: Style::BOLD,
        }
    }

    fn plain_paragraph(s: &str) -> Block {
        Block::Paragraph(vec![text(s)])
    }

    fn empty_doc(blocks: Vec<Block>) -> Document {
        Document {
            blocks,
            notes: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn heading_level_clamped_to_six() {
        let doc = empty_doc(vec![Block::Heading {
            level: 9,
            content: vec![text("deep")],
        }]);
        let md = render_markdown(&doc);
        assert!(md.starts_with("\n###### deep"), "{md}");
    }

    #[test]
    fn inline_markers_and_coalescing() {
        let doc = empty_doc(vec![Block::Paragraph(vec![
            bold("a"),
            bold("b"),
            Inline::Text {
                text: " code ".to_string(),
                style: Style::CODE,
            },
            Inline::Text {
                text: "gone".to_string(),
                style: Style {
                    strike: true,
                    ..Style::PLAIN
                },
            },
        ])]);
        let md = render_markdown(&doc);
        assert!(md.contains("**ab**"), "{md}");
        assert!(md.contains("` code `"), "{md}");
        assert!(md.contains("gone"), "{md}");
    }

    #[test]
    fn links_images_anchor_and_checkbox() {
        let doc = empty_doc(vec![Block::Paragraph(vec![
            Inline::Link {
                content: vec![text("site")],
                target: LinkTarget::External("https://x.test".into()),
            },
            Inline::Link {
                content: vec![text("rel")],
                target: LinkTarget::Relative("media/a.png".into()),
            },
            Inline::Link {
                content: vec![text("sec")],
                target: LinkTarget::Anchor("heading-2".into()),
            },
            Inline::Image {
                alt: "alt".into(),
                source: ImageSource::External("images/0.png".into()),
            },
            Inline::Checkbox(true),
            Inline::LineBreak,
            text("after"),
            Inline::NoteRef("1".into()),
            Inline::Anchor("drop".into()),
            Inline::Math("x^2".into()),
        ])]);
        let md = render_markdown(&doc);
        assert!(md.contains("[site](https://x.test)"), "{md}");
        assert!(md.contains("[rel](media/a.png)"), "{md}");
        assert!(md.contains("[sec](#heading-2)"), "{md}");
        assert!(md.contains("![alt](images/0.png)"), "{md}");
        assert!(md.contains("[x]"), "{md}");
        assert!(md.contains("after"), "{md}");
        assert!(md.contains("$x^2$"), "{md}");
        // Anchor renders nothing and note refs without a note body are dropped.
        assert!(!md.contains("drop"), "{md}");
    }

    #[test]
    fn lists_render_marker_families_and_nested_blocks() {
        let doc = empty_doc(vec![
            Block::List(List {
                marker: MarkerKind::Decimal,
                start: 3,
                items: vec![
                    ListItem {
                        blocks: vec![plain_paragraph("first")],
                    },
                    ListItem {
                        blocks: vec![Block::List(List {
                            marker: MarkerKind::Bullet,
                            start: 1,
                            items: vec![ListItem {
                                blocks: vec![plain_paragraph("nested")],
                            }],
                        })],
                    },
                ],
            }),
            Block::List(List {
                marker: MarkerKind::LowerRoman,
                start: 1,
                items: vec![ListItem {
                    blocks: vec![plain_paragraph("roman")],
                }],
            }),
        ]);
        let md = render_markdown(&doc);
        assert!(md.contains("3. first"), "{md}");
        assert!(md.contains("- nested"), "{md}");
        assert!(md.contains("i. roman"), "{md}");
    }

    #[test]
    fn table_renders_header_and_blank_covered_slots() {
        let doc = empty_doc(vec![Block::Table(crate::domain::document::Table {
            grid: vec![
                vec![
                    CellSlot::Origin(Cell {
                        blocks: vec![Block::Paragraph(vec![text("h1")])],
                        col_span: 2,
                        row_span: 1,
                    }),
                    CellSlot::Covered {
                        origin_row: 0,
                        origin_col: 0,
                    },
                ],
                vec![
                    CellSlot::Origin(Cell {
                        blocks: vec![Block::Paragraph(vec![text("a")])],
                        col_span: 1,
                        row_span: 1,
                    }),
                    CellSlot::Origin(Cell {
                        blocks: vec![Block::Paragraph(vec![text("b")])],
                        col_span: 1,
                        row_span: 1,
                    }),
                ],
            ],
            header_rows: 1,
            kind: TableKind::Data,
        })]);
        let md = render_markdown(&doc);
        assert!(md.contains("| h1 |  |"), "{md}");
        assert!(md.contains("| a | b |"), "{md}");
    }

    #[test]
    fn layout_table_has_no_duplicate_body_row() {
        let doc = empty_doc(vec![Block::Table(crate::domain::document::Table {
            grid: vec![vec![CellSlot::Origin(Cell {
                blocks: vec![Block::Paragraph(vec![text("only")])],
                col_span: 1,
                row_span: 1,
            })]],
            header_rows: 1,
            kind: TableKind::Layout,
        })]);
        let md = render_markdown(&doc);
        // GFM needs a header + separator; the header row must not repeat in
        // the body.
        assert_eq!(
            md, "| only |\n|---|\n\n",
            "layout table duplicated its header row: {md}"
        );
    }

    #[test]
    fn blockquote_rule_math_and_code_render() {
        let doc = empty_doc(vec![
            Block::BlockQuote(vec![plain_paragraph("quoted"), Block::Rule]),
            Block::CodeBlock {
                lang: Some("rust".into()),
                text: "fn main() {}".into(),
            },
            Block::Math("E=mc^2".into()),
            Block::Paragraph(vec![]),
        ]);
        let md = render_markdown(&doc);
        assert!(md.contains("> quoted"), "{md}");
        assert!(md.contains("---"), "{md}");
        assert!(md.contains("```rust\nfn main() {}\n```"), "{md}");
        assert!(md.contains("$$E=mc^2$$"), "{md}");
    }

    #[test]
    fn notes_section_renders_footnote_and_endnote() {
        let mut doc = empty_doc(vec![Block::Paragraph(vec![Inline::NoteRef("n1".into())])]);
        doc.notes.push(Note {
            id: "n1".into(),
            kind: NoteKind::Footnote,
            blocks: vec![plain_paragraph("body one")],
        });
        doc.notes.push(Note {
            id: "n2".into(),
            kind: NoteKind::Endnote,
            blocks: vec![plain_paragraph("body two")],
        });
        let md = render_markdown(&doc);
        assert!(md.contains("[^footnote]: body one"), "{md}");
        assert!(md.contains("[^endnote]: body two"), "{md}");
    }
}
