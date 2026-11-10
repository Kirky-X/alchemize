// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Coverage supplements: public-API paths that the fixture suite does not
//! reach. Each test asserts a behaviour, not merely that a line ran.

use alchemize::application::builder::{AlchemizeBuilder, AssetMode};
use alchemize::domain::asset::Asset as SinkAsset;
use alchemize::domain::document::{
    Asset as ModelAsset, Block, CellSlot, Document, ImageSource, Inline, LinkTarget, List,
    ListItem, MarkerKind, Note, NoteKind, Style, Table, TableKind,
};
use alchemize::domain::input::Input;
use alchemize::infrastructure::document::gfm_render::render_markdown;

/// Build a docx package from (name, bytes) parts.
fn zip_bytes(parts: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(buf);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, body) in parts {
        w.start_file(*name, opts).unwrap();
        w.write_all(body).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[tokio::test]
async fn docx_body_branches_hyperlink_tab_rstyle_notes_table() {
    let docx = zip_bytes(&[
        (
            "word/document.xml",
            br#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p>
  <w:pPr><w:pStyle w:val="H2"/></w:pPr>
  <w:r><w:t>Section</w:t></w:r>
</w:p>
<w:p>
  <w:hyperlink r:id="rId1"><w:r><w:t>External link</w:t></w:r></w:hyperlink>
  <w:r><w:t xml:space="preserve"> after</w:t></w:r>
  <w:r><w:tab/></w:r>
  <w:r><w:br/></w:r>
  <w:r><w:rPr><w:rStyle w:val="Strong"/></w:rPr><w:t>strong run</w:t></w:r>
</w:p>
<w:p>
  <w:r><w:t>Note refs</w:t></w:r>
  <w:r><w:footnoteReference w:id="1"/></w:r>
  <w:r><w:endnoteReference w:id="5"/></w:r>
</w:p>
<w:tbl>
  <w:tr>
    <w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>M</w:t></w:r></w:p></w:tc>
    <w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc>
  </w:tr>
  <w:tr>
    <w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t>covered</w:t></w:r></w:p></w:tc>
    <w:tc><w:p><w:r><w:t>c1</w:t></w:r></w:p></w:tc>
    <w:tc><w:p><w:r><w:t>c2</w:t></w:r></w:p></w:tc>
  </w:tr>
</w:tbl>
<w:p></w:p>
</w:body></w:document>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Target="https://example.test/x" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"/>
</Relationships>"#,
        ),
        (
            "word/styles.xml",
            br#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:docDefaults><w:rPrDefault><w:rPr><w:b/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="H2"><w:name w:val="heading 2"/></w:style>
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:i/></w:rPr></w:style>
</w:styles>"#,
        ),
        (
            "word/footnotes.xml",
            br#"<?xml version="1.0"?><w:footnotes xmlns:w="w"><w:footnote w:id="1"><w:p><w:r><w:t>foot body</w:t></w:r></w:p></w:footnote></w:footnotes>"#,
        ),
        (
            "word/endnotes.xml",
            br#"<?xml version="1.0"?><w:endnotes xmlns:w="w"><w:endnote w:id="5"><w:p><w:r><w:t>end body</w:t></w:r></w:p></w:endnote></w:endnotes>"#,
        ),
    ]);

    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::bytes(docx, "branches.docx"))
        .await
        .unwrap();
    let md = &result.markdown;
    assert!(
        md.contains("## ") && md.contains("Section"),
        "heading from style name: {md}"
    );
    // The label inherits docDefaults bold, so match the URL + the label text.
    assert!(
        md.contains("(https://example.test/x)"),
        "external url: {md}"
    );
    assert!(md.contains("External link"), "link label: {md}");
    assert!(md.contains("strong run"), "{md}");
    assert!(md.contains("foot body"), "footnote body: {md}");
    assert!(md.contains("end body"), "endnote body: {md}");
    assert_eq!(
        result.metadata["footnote_count"].as_u64(),
        Some(2),
        "both notes counted: {}",
        result.metadata
    );
    assert!(md.contains("wide"), "table with spans: {md}");
    // docDefaults bold applies to every run.
    assert!(md.contains("**"), "docDefaults bold reaches output: {md}");
}

#[tokio::test]
async fn docx_broken_package_reports_error() {
    let alch = AlchemizeBuilder::new().build();
    // Valid zip, missing word/document.xml.
    let docx = zip_bytes(&[("other.txt", b"hello")]);
    let err = alch
        .convert(&Input::bytes(docx, "broken.docx"))
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("document.xml"), "err: {msg}");
}

#[test]
fn gfm_render_covers_every_block_and_inline() {
    let doc = Document {
        blocks: vec![
            Block::Heading {
                level: 9,
                content: vec![Inline::Text {
                    text: "Clamped".into(),
                    style: Style::BOLD,
                }],
            },
            Block::Paragraph(vec![
                Inline::Text {
                    text: "plain ".into(),
                    style: Style::PLAIN,
                },
                Inline::Text {
                    text: "bold italic".into(),
                    style: Style {
                        bold: true,
                        italic: true,
                        code: false,
                        strike: false,
                        size_pt: Some(12.0),
                    },
                },
                Inline::Link {
                    content: vec![Inline::Text {
                        text: "anchor".into(),
                        style: Style::PLAIN,
                    }],
                    target: LinkTarget::Anchor("sec-1".into()),
                },
                Inline::Image {
                    alt: "remote".into(),
                    source: ImageSource::External("https://x.test/a.png".into()),
                },
                Inline::Image {
                    alt: "unavailable".into(),
                    source: ImageSource::Unavailable,
                },
                Inline::Math("x^2".into()),
                Inline::Checkbox(true),
                Inline::NoteRef("n1".into()),
                Inline::LineBreak,
                Inline::Text {
                    text: "tail".into(),
                    style: Style::PLAIN,
                },
            ]),
            Block::List(List {
                marker: MarkerKind::Decimal,
                start: 3,
                items: vec![
                    ListItem {
                        blocks: vec![Block::Paragraph(vec![Inline::Text {
                            text: "first".into(),
                            style: Style::PLAIN,
                        }])],
                    },
                    ListItem {
                        blocks: vec![
                            Block::Paragraph(vec![Inline::Text {
                                text: "second".into(),
                                style: Style::PLAIN,
                            }]),
                            Block::CodeBlock {
                                lang: Some("rust".into()),
                                text: "fn x() {}".into(),
                            },
                        ],
                    },
                ],
            }),
            Block::BlockQuote(vec![Block::Paragraph(vec![Inline::Text {
                text: "quoted".into(),
                style: Style::PLAIN,
            }])]),
            Block::CodeBlock {
                lang: None,
                text: "plain code".into(),
            },
            Block::Rule,
            Block::Math("\\frac{a}{b}".into()),
            Block::Table(Table {
                grid: vec![
                    vec![
                        CellSlot::Origin(alchemize::domain::document::Cell::text("H1")),
                        CellSlot::Origin(alchemize::domain::document::Cell::text("H2")),
                    ],
                    vec![
                        CellSlot::Origin(alchemize::domain::document::Cell::text("a|b")),
                        CellSlot::Covered {
                            origin_row: 0,
                            origin_col: 1,
                        },
                    ],
                ],
                header_rows: 1,
                kind: TableKind::Layout,
            }),
        ],
        notes: vec![
            Note {
                id: "n1".into(),
                kind: NoteKind::Footnote,
                blocks: vec![Block::Paragraph(vec![Inline::Text {
                    text: "note text".into(),
                    style: Style::PLAIN,
                }])],
            },
            Note {
                id: "n2".into(),
                kind: NoteKind::Endnote,
                blocks: vec![Block::Paragraph(vec![Inline::Text {
                    text: "endnote text".into(),
                    style: Style::PLAIN,
                }])],
            },
        ],
        assets: vec![ModelAsset {
            media_type: "image/png".into(),
            origin_part: "word/media/a.png".into(),
            bytes: vec![1, 2, 3],
        }],
    };
    let md = render_markdown(&doc);
    assert!(
        md.contains("###### ") && md.contains("Clamped"),
        "level clamped to 6: {md}"
    );
    assert!(md.contains("***bold italic***"), "bold+italic: {md}");
    assert!(md.contains("[anchor](#sec-1)"), "anchor link: {md}");
    assert!(md.contains("![remote](https://x.test/a.png)"), "{md}");
    assert!(md.contains("$x^2$"), "inline math: {md}");
    assert!(md.contains("[x]"), "checkbox: {md}");
    assert!(md.contains("3. first"), "list starts at 3: {md}");
    assert!(md.contains("> quoted"), "blockquote: {md}");
    assert!(md.contains("```\nplain code\n```"), "no-lang fence: {md}");
    assert!(md.contains("---"), "rule: {md}");
    assert!(md.contains("$$\\frac{a}{b}$$"), "display math: {md}");
    assert!(md.contains("a\\|b"), "cell pipe escaped: {md}");
    assert!(md.contains("note text"), "footnote body: {md}");
    assert!(md.contains("endnote text"), "endnote body: {md}");
    // Layout tables emit no header separator.
    assert!(
        !md.contains("| --- |"),
        "layout table must not synthesize a header: {md}"
    );
}

#[test]
fn builder_surface_availability_and_engines() {
    let alch = AlchemizeBuilder::new()
        .with_asset_mode(AssetMode::Inline)
        .with_max_file_size(1024)
        .build();
    assert!(alch.engine_count() > 5);
    let names = alch.engines();
    for expected in ["text", "docx-deep", "pdf", "image", "audio", "subtitle"] {
        assert!(
            names.contains(&expected),
            "engine {expected} missing from {names:?}"
        );
    }
    let rows = alch.availability();
    assert!(rows.iter().any(|(name, _, _)| name == "zip"));
    // Inline asset mode: the sink emits a data URL.
    let mut sink = alch.asset_sink_for(std::path::Path::new("/tmp/x"));
    let reference = sink.put(SinkAsset::new("a.png", vec![1, 2])).unwrap();
    assert!(
        reference.starts_with("data:image/png;base64,"),
        "{reference}"
    );
}

#[tokio::test]
async fn zip_and_cbz_route_through_the_facade() {
    let alch = AlchemizeBuilder::new().build();
    let inner = zip_bytes(&[("a.txt", b"inner text")]);
    // Nested zip: recursion depth limits reported, not crashed.
    let outer = zip_bytes(&[("bundle.zip", &inner)]);
    let result = alch
        .convert(&Input::bytes(outer, "outer.zip"))
        .await
        .unwrap();
    assert!(
        result.markdown.contains("bundle.zip"),
        "{}",
        result.markdown
    );

    // cbz goes through the same facade.
    let cbz = zip_bytes(&[
        ("page1.txt", b"comic page one"),
        ("page2.txt", b"comic page two"),
    ]);
    let result = alch.convert(&Input::bytes(cbz, "comic.cbz")).await.unwrap();
    assert!(
        result.markdown.contains("comic page one"),
        "{}",
        result.markdown
    );
    assert!(
        result.markdown.contains("comic page two"),
        "{}",
        result.markdown
    );
}

#[tokio::test]
async fn eml_converter_via_registry() {
    // Without the `eml` feature this asserts the explicit unsupported error;
    // with it, the headers/body render.
    let alch = AlchemizeBuilder::new().build();
    let eml = b"From: a@b.c\nTo: d@e.f\nSubject: Hi\n\nBody text\n";
    match alch.convert(&Input::bytes(eml.to_vec(), "m.eml")).await {
        Ok(result) => {
            assert!(result.markdown.contains("Hi"));
            assert!(result.markdown.contains("Body text"));
        }
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("eml"),
                "unsupported-or-error must name eml: {msg}"
            );
        }
    }
}

/// One package that exercises the remaining body arms: self-closing run
/// properties, outline levels, mailto/ftp/relative links, every image media
/// type, asset reuse, missing/ghost relationships, and an orphan vertical
/// merge in the first row.
#[tokio::test]
async fn docx_body_self_closing_props_links_and_media_types() {
    const DOC: &[u8] =
        br#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:a="a" xmlns:r="r"><w:body>
<w:p><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:r><w:t>Outline two</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:b/><w:i/><w:sz w:val="28"/></w:rPr><w:t>styled run</w:t></w:r></w:p>
<w:p>
  <w:hyperlink r:id="rMail"><w:r><w:t>mail me</w:t></w:r></w:hyperlink>
  <w:hyperlink r:id="rFtp"><w:r><w:t>drop box</w:t></w:r></w:hyperlink>
  <w:hyperlink r:id="rRel"><w:r><w:t>internal</w:t></w:r></w:hyperlink>
</w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgPng"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgPng"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgSvg"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgWebp"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgEmf"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgTif"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgGif"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="imgBmp"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="missingRel"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="ghostPart"/></w:drawing></w:r></w:p>
<w:tbl>
  <w:tr>
    <w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t>orphan merge</w:t></w:r></w:p></w:tc>
    <w:tc><w:p><w:r><w:t>r0c1</w:t></w:r></w:p></w:tc>
  </w:tr>
  <w:tr>
    <w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>start</w:t></w:r></w:p></w:tc>
    <w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc>
  </w:tr>
</w:tbl>
<w:p><w:r><w:t>a &amp; b</w:t></w:r></w:p>
</w:body></w:document>"#;
    const RELS: &[u8] = br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rMail" Target="mailto:a@b.c" Type="hyperlink"/>
<Relationship Id="rFtp" Target="ftp://x.test/f" Type="hyperlink"/>
<Relationship Id="rRel" Target="../customXml/x.xml" Type="hyperlink"/>
<Relationship Id="imgPng" Target="media/a.png" Type="image"/>
<Relationship Id="imgSvg" Target="media/b.svg" Type="image"/>
<Relationship Id="imgWebp" Target="media/c.webp" Type="image"/>
<Relationship Id="imgEmf" Target="media/d.emf" Type="image"/>
<Relationship Id="imgTif" Target="media/e.tif" Type="image"/>
<Relationship Id="imgGif" Target="media/f.gif" Type="image"/>
<Relationship Id="imgBmp" Target="media/g.bmp" Type="image"/>
<Relationship Id="ghostPart" Target="media/ghost.png" Type="image"/>
</Relationships>"#;
    let docx = zip_bytes(&[
        ("word/document.xml", DOC),
        ("word/_rels/document.xml.rels", RELS),
        ("word/media/a.png", b"\x89PNG-a"),
        ("word/media/b.svg", b"<svg/>"),
        ("word/media/c.webp", b"RIFF-webp"),
        ("word/media/d.emf", b"emf"),
        ("word/media/e.tif", b"II*tiff"),
        ("word/media/f.gif", b"GIF89a"),
        ("word/media/g.bmp", b"BM"),
    ]);
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::bytes(docx, "kitchen.docx"))
        .await
        .unwrap();
    let md = &result.markdown;
    assert!(md.contains("## Outline two"), "outlineLvl → h2: {md}");
    assert!(
        md.contains("***styled run***"),
        "b+i from self-closing rPr: {md}"
    );
    assert!(md.contains("(mailto:a@b.c)"), "mailto link: {md}");
    assert!(md.contains("(ftp://x.test/f)"), "ftp link: {md}");
    assert!(md.contains("customXml/x.xml"), "relative link: {md}");
    assert!(md.contains("a & b"), "entity decoded: {md}");
    // Seven real parts; the ghost part and the missing relationship add none.
    assert_eq!(result.assets.len(), 7, "assets: {}", result.assets.len());
    // The png is referenced twice but stored once: 8 image links, 7 assets.
    assert_eq!(
        md.matches("](images/").count(),
        8,
        "image links: {}\nFULL: {md}",
        md.matches("](images/").count()
    );
    assert!(md.contains("start"), "table survives: {md}");
    assert!(md.contains("orphan merge"), "orphan merge cell kept: {md}");
}
