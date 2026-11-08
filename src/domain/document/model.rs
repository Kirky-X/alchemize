// SPDX-License-Identifier: Apache-2.0
//! Document model core: blocks, inlines, lists, notes, assets.

use super::table::Table;

/// A parsed document: body blocks in reading order, note bodies, assets.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Document {
    /// Body content in reading order.
    pub blocks: Vec<Block>,
    /// Note bodies (footnotes/endnotes), referenced by [`Inline::NoteRef`].
    pub notes: Vec<Note>,
    /// Every embedded binary asset, indexed by position.
    pub assets: Vec<Asset>,
}

/// An embedded binary asset (image payload etc.).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    /// MIME type, e.g. `image/png`.
    pub media_type: String,
    /// Origin part name inside the source package (provenance).
    pub origin_part: String,
    /// Payload bytes, exactly as stored.
    pub bytes: Vec<u8>,
}

/// Top-level block content.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Block {
    /// Section heading; `level` is 1-based and renderers clamp to target.
    Heading {
        /// Outline depth from the source.
        level: u8,
        /// Heading inline content.
        content: Vec<Inline>,
    },
    /// Body paragraph.
    Paragraph(Vec<Inline>),
    /// A list with resolved numbering.
    List(List),
    /// A table grid.
    Table(Table),
    /// Quoted content, nesting blocks.
    BlockQuote(Vec<Block>),
    /// Preformatted text.
    CodeBlock {
        /// Language hint when the source names one.
        lang: Option<String>,
        /// Literal text, newlines intact.
        text: String,
    },
    /// Horizontal rule.
    Rule,
    /// Display math (LaTeX body, delimiters added by the renderer).
    Math(String),
}

/// List container with marker semantics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct List {
    /// Marker family for every item in this run.
    pub marker: MarkerKind,
    /// Ordinal of the first item (source numbering).
    pub start: u64,
    /// Items in order.
    pub items: Vec<ListItem>,
}

/// One list item: its content is a block sequence (nested lists included).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ListItem {
    /// Item content.
    pub blocks: Vec<Block>,
}

/// Marker family of a list run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MarkerKind {
    /// Unordered.
    Bullet,
    /// `1, 2, 3`.
    Decimal,
    /// `a, b, c`.
    LowerAlpha,
    /// `A, B, C`.
    UpperAlpha,
    /// `i, ii, iii`.
    LowerRoman,
    /// `I, II, III`.
    UpperRoman,
}

/// Inline content within paragraphs/headings/cells.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Inline {
    /// Styled text run.
    Text {
        /// The text.
        text: String,
        /// Character style over the whole run.
        style: Style,
    },
    /// Hyperlink wrapping its own inline content.
    Link {
        /// Link label content.
        content: Vec<Inline>,
        /// Where it points.
        target: LinkTarget,
    },
    /// An image.
    Image {
        /// Alt text.
        alt: String,
        /// Where the bytes live.
        source: ImageSource,
    },
    /// Inline math (LaTeX body).
    Math(String),
    /// Task-list checkbox state.
    Checkbox(bool),
    /// Heading anchor reference target (rendered as nothing/label).
    Anchor(String),
    /// Footnote/endnote reference by note id.
    NoteRef(String),
    /// Hard line break.
    LineBreak,
}

/// Character style bitset over a text run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Style {
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Inline code.
    pub code: bool,
    /// Strikethrough.
    pub strike: bool,
}

impl Style {
    /// Plain style.
    pub const PLAIN: Style = Style {
        bold: false,
        italic: false,
        code: false,
        strike: false,
    };

    /// Bold style.
    pub const BOLD: Style = Style {
        bold: true,
        ..Style::PLAIN
    };

    /// Italic style.
    pub const ITALIC: Style = Style {
        italic: true,
        ..Style::PLAIN
    };

    /// Inline-code style.
    pub const CODE: Style = Style {
        code: true,
        ..Style::PLAIN
    };

    /// Whether any styling is active.
    pub fn is_plain(self) -> bool {
        self == Style::PLAIN
    }
}

/// Where an image's bytes live.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageSource {
    /// Absolute or relative URL with a scheme.
    External(String),
    /// Embedded asset: index into [`Document::assets`].
    Asset(usize),
    /// No usable source; only alt text remains.
    Unavailable,
}

/// Link target kinds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LinkTarget {
    /// Absolute URL with a scheme.
    External(String),
    /// Scheme-less relative reference, kept as written.
    Relative(String),
    /// Internal heading anchor id.
    Anchor(String),
}

/// A footnote or endnote body.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Note {
    /// Document-scoped id referenced by [`Inline::NoteRef`].
    pub id: String,
    /// Placement kind.
    pub kind: NoteKind,
    /// Note body blocks.
    pub blocks: Vec<Block>,
}

/// Note placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NoteKind {
    /// Page-bottom footnote.
    Footnote,
    /// Document-end endnote.
    Endnote,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        Document {
            blocks: vec![
                Block::Heading {
                    level: 1,
                    content: vec![Inline::Text {
                        text: "Title".into(),
                        style: Style::PLAIN,
                    }],
                },
                Block::Paragraph(vec![
                    Inline::Text {
                        text: "bold ".into(),
                        style: Style::BOLD,
                    },
                    Inline::Link {
                        content: vec![Inline::Text {
                            text: "site".into(),
                            style: Style::PLAIN,
                        }],
                        target: LinkTarget::External("https://x.test".into()),
                    },
                    Inline::Image {
                        alt: "pic".into(),
                        source: ImageSource::Asset(0),
                    },
                    Inline::NoteRef("n1".into()),
                ]),
                Block::List(List {
                    marker: MarkerKind::Bullet,
                    start: 1,
                    items: vec![ListItem {
                        blocks: vec![Block::Paragraph(vec![Inline::Text {
                            text: "item".into(),
                            style: Style::PLAIN,
                        }])],
                    }],
                }),
                Block::CodeBlock {
                    lang: Some("rust".into()),
                    text: "fn x() {}".into(),
                },
                Block::Math("c^2 = a^2 + b^2".into()),
                Block::Rule,
            ],
            notes: vec![Note {
                id: "n1".into(),
                kind: NoteKind::Footnote,
                blocks: vec![Block::Paragraph(vec![Inline::Text {
                    text: "note body".into(),
                    style: Style::PLAIN,
                }])],
            }],
            assets: vec![Asset {
                media_type: "image/png".into(),
                origin_part: "word/media/image1.png".into(),
                bytes: vec![1, 2, 3],
            }],
        }
    }

    #[test]
    fn serde_roundtrip_full_document() {
        let doc = sample();
        let json = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back.blocks.len(), doc.blocks.len());
        assert_eq!(back.notes.len(), 1);
        assert_eq!(back.assets[0].bytes, vec![1, 2, 3]);
        assert!(matches!(back.blocks[0], Block::Heading { level: 1, .. }));
    }

    #[test]
    fn style_flags() {
        assert!(Style::PLAIN.is_plain());
        assert!(!Style::BOLD.is_plain());
        assert_ne!(Style::CODE, Style::ITALIC);
    }

    #[test]
    fn image_source_variants_serde() {
        for source in [
            ImageSource::External("https://a/b.png".into()),
            ImageSource::Asset(3),
            ImageSource::Unavailable,
        ] {
            let json = serde_json::to_string(&source).unwrap();
            let back: ImageSource = serde_json::from_str(&json).unwrap();
            assert_eq!(back, source);
        }
    }

    #[test]
    fn empty_document_default() {
        let doc = Document::default();
        assert!(doc.blocks.is_empty());
        assert!(doc.notes.is_empty());
        assert!(doc.assets.is_empty());
    }
}
