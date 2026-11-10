// SPDX-License-Identifier: Apache-2.0
//! DOCX package plumbing: the converter, package part reading, and the
//! relationship table.

use std::collections::HashMap;
use std::io::Cursor;

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::application::ports::{Availability, Converter};
use crate::domain::asset::Asset;
use crate::domain::document::Document;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

use crate::infrastructure::document::office::{RawMedia, rewrite_image_assets};

use super::body::{self, BodyCtx};
use super::notes::parse_notes;
use super::styles::{StyleTable, parse_styles};

/// Deep DOCX converter: WordprocessingML → the unified document model.
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

        let parsed = result?;
        let mut doc = parsed.document;
        let warnings = parsed.warnings;
        // Turn internal asset indices into resolvable `images/…` references:
        // the renderer only emits links for external sources, and the CLI
        // writes exactly these filenames.
        let media: Vec<RawMedia<'_>> = parsed
            .raw_assets
            .iter()
            .map(|a| RawMedia {
                part: &a.part,
                media_type: &a.media_type,
                bytes: &a.bytes,
            })
            .collect();
        let assets = rewrite_image_assets(&mut doc, &media);
        doc.assets.clear();
        let markdown = crate::infrastructure::document::gfm_render::render_markdown(&doc);
        crate::infrastructure::document::shared::check_output_budget(markdown.len())?;

        let metadata = serde_json::json!({
            "source_format": "docx",
            "asset_count": assets.len(),
            "footnote_count": doc.notes.len(),
            "parser": "docx-deep",
            "omml_fallback": warnings.omml_fallback,
            "numfmt_fallback": 0,
        });

        let mut out = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in assets {
            out = out.with_asset(asset);
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

/// Parser warnings surfaced into metadata.
#[derive(Default)]
pub struct Warnings {
    /// Number of OMML formulas that fell back to plain text.
    pub omml_fallback: usize,
}

/// Raw zip part payload.
pub struct RawAsset {
    /// Package-relative part name.
    pub part: String,
    /// Guessed media type.
    pub media_type: String,
    /// Raw bytes.
    pub bytes: Vec<u8>,
}

/// Entry of a parsed relationships part.
struct Rel {
    id: String,
    target: String,
}

/// Parse result: the document model plus the raw media payloads.
pub struct ParsedDocx {
    /// Unified document model.
    pub document: Document,
    /// Parser warnings.
    pub warnings: Warnings,
    /// Raw media payloads in first-reference order.
    pub raw_assets: Vec<RawAsset>,
}

/// Parse DOCX bytes into the unified model.
pub fn parse(bytes: &[u8]) -> Result<ParsedDocx, AlchemizeError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::Zip(format!("failed to open DOCX: {e}")))?;

    let document_xml = body::read_part(&mut zip, "word/document.xml")?;
    let styles_xml = body::read_part(&mut zip, "word/styles.xml").ok();
    let footnotes_xml = body::read_part(&mut zip, "word/footnotes.xml").ok();
    let endnotes_xml = body::read_part(&mut zip, "word/endnotes.xml").ok();
    let rels_xml = body::read_part(&mut zip, "word/_rels/document.xml.rels").ok();

    let style_table: StyleTable = styles_xml.as_deref().map(parse_styles).unwrap_or_default();
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
        assets_bytes: 0,
        asset_index: HashMap::new(),
        zip: &mut zip,
    };

    let blocks = body::parse_body(&document_xml, &mut ctx)?;
    let mut notes = Vec::new();
    if let Some(xml) = footnotes_xml.as_deref() {
        notes.extend(parse_notes(
            xml,
            crate::domain::document::NoteKind::Footnote,
        ));
    }
    if let Some(xml) = endnotes_xml.as_deref() {
        notes.extend(parse_notes(xml, crate::domain::document::NoteKind::Endnote));
    }

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

    Ok(ParsedDocx {
        document,
        warnings,
        raw_assets,
    })
}

/// Parse relationships part.
fn parse_rels(xml: &[u8]) -> Vec<Rel> {
    let mut out = Vec::new();
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if crate::infrastructure::document::local_tag_name(e.name().as_ref())
                    == "Relationship" =>
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{Block, CellSlot, Inline};

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
        assert!(
            result.markdown.contains("# Title"),
            "md: {}",
            result.markdown
        );
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
        assert!(
            result.markdown.contains("| A | B |"),
            "md: {}",
            result.markdown
        );
    }

    #[tokio::test]
    async fn inherited_bold_reaches_markdown() {
        let doc = docx_zip(&[
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:pStyle w:val="Mid"/></w:pPr><w:r><w:t>Inherited</w:t></w:r></w:p>
</w:body></w:document>"#,
            ),
            (
                "word/styles.xml",
                r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="paragraph" w:styleId="Base"><w:name w:val="Base"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Mid"><w:name w:val="Mid"/><w:basedOn w:val="Base"/></w:style>
</w:styles>"#,
            ),
        ]);
        let result = DocxDeepConverter::new()
            .convert(Input::bytes(doc, "t.docx"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("**Inherited**"),
            "md: {}",
            result.markdown
        );
    }

    #[tokio::test]
    async fn footnote_reference_is_injected() {
        let doc = docx_zip(&[
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:r><w:t>Claim</w:t></w:r><w:r><w:footnoteReference w:id="2"/></w:r></w:p>
</w:body></w:document>"#,
            ),
            (
                "word/footnotes.xml",
                r#"<?xml version="1.0"?><w:footnotes xmlns:w="w">
<w:footnote w:id="2"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t>Note body</w:t></w:r></w:p></w:footnote>
</w:footnotes>"#,
            ),
        ]);
        let result = DocxDeepConverter::new()
            .convert(Input::bytes(doc, "t.docx"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("Note body"),
            "note body missing: {}",
            result.markdown
        );
        assert_eq!(
            result.metadata["footnote_count"].as_u64(),
            Some(1),
            "meta: {}",
            result.metadata
        );
    }

    #[tokio::test]
    async fn vertical_merge_yields_covered_slot() {
        let doc = docx_zip(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:tbl>
<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>M</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>N</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t>merged</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>O</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
</w:body></w:document>"#,
        )]);
        let doc = parse(&doc).unwrap().document;
        let table = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("table block");
        assert_eq!(table.grid.len(), 2);
        assert!(matches!(
            table.grid[1][0],
            CellSlot::Covered {
                origin_row: 0,
                origin_col: 0
            }
        ));
    }

    #[tokio::test]
    async fn drawing_image_becomes_asset_reference() {
        // A 1x1 PNG (real bytes) referenced through document rels.
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        use std::io::Write;
        let buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("word/document.xml", opts).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:r><w:drawing><a:blip r:embed="rId7"/></w:drawing></w:r></w:p>
</w:body></w:document>"#,
        )
        .unwrap();
        w.start_file("word/_rels/document.xml.rels", opts).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId7" Target="media/image1.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/>
</Relationships>"#,
        )
        .unwrap();
        w.start_file("word/media/image1.png", opts).unwrap();
        w.write_all(png).unwrap();
        let doc = w.finish().unwrap().into_inner();

        let result = DocxDeepConverter::new()
            .convert(Input::bytes(doc, "t.docx"))
            .await
            .unwrap();
        assert_eq!(result.assets.len(), 1, "one image asset expected");
        // Assets follow the shared `images/{seq:04}-{sha8}.{ext}` convention.
        let expected_name = format!(
            "images/0000-{}.png",
            Asset::new("x.png", png.to_vec()).sha8()
        );
        assert_eq!(
            result.assets[0].filename, expected_name,
            "asset filename: {}",
            result.assets[0].filename
        );
        assert!(
            result.markdown.contains(&format!("]({expected_name})")),
            "markdown must link the asset: {}",
            result.markdown
        );
        assert_eq!(
            result.assets[0].bytes.len(),
            png.len(),
            "asset bytes must equal the source media bytes"
        );
        // sha8 of the asset must match the sha8 of the source bytes.
        assert_eq!(
            result.assets[0].sha8(),
            Asset::new("x.png", png.to_vec()).sha8()
        );
    }

    #[tokio::test]
    async fn hostile_shapes_parse_without_loss() {
        // Edge shapes: skipped note ids, hyperlink without a rel, unknown
        // character style, empty table, breaks inside a cell, oMathPara,
        // text outside any paragraph, and a second table without rows.
        let doc = docx_zip(&[
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:m="m"><w:body>
<w:p><w:r><w:t>loose</w:t></w:r></w:p>
<w:p>
  <w:hyperlink><w:r><w:t>no-rel link</w:t></w:r></w:hyperlink>
  <w:r><w:rPr><w:rStyle w:val="Ghost"/></w:rPr><w:t>ghost style</w:t></w:r>
  <w:r><w:footnoteReference w:id="0"/></w:r>
  <w:r><w:footnoteReference w:id="-1"/></w:r>
  <w:r><w:endnoteReference w:id="7"/></w:r>
</w:p>
<w:p><m:oMathPara><m:oMath><m:r><m:t>m1</m:t></m:r></m:oMath></m:oMathPara></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r><w:r><w:br/></w:r><w:r><w:t>b</w:t></w:r></w:p><w:p><w:r><w:t>second</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:tbl></w:tbl>
<w:p><w:r><w:drawing><a:blip r:embed="rIdMissing"/></w:drawing></w:r><w:r><w:t>after-bad-image</w:t></w:r></w:p>
</w:body></w:document>"#,
            ),
            (
                "word/endnotes.xml",
                r#"<?xml version="1.0"?><w:endnotes xmlns:w="w"><w:endnote w:id="7"><w:p><w:r><w:t>end seven</w:t></w:r></w:p></w:endnote></w:endnotes>"#,
            ),
        ]);
        let result = DocxDeepConverter::new()
            .convert(Input::bytes(doc, "hostile.docx"))
            .await
            .unwrap();
        let md = &result.markdown;
        assert!(md.contains("loose"), "{md}");
        assert!(
            md.contains("no-rel link"),
            "text of a rel-less link survives: {md}"
        );
        assert!(md.contains("ghost style"), "{md}");
        assert!(md.contains("m1"), "oMathPara content survives: {md}");
        assert!(md.contains("a") && md.contains("b"), "{md}");
        assert!(md.contains("second"), "second cell paragraph: {md}");
        assert!(md.contains("after-bad-image"), "{md}");
        assert!(md.contains("end seven"), "{md}");
        // Note ids 0/-1 are separators: only the endnote counts.
        assert_eq!(result.metadata["footnote_count"].as_u64(), Some(1));
        assert_eq!(result.metadata["asset_count"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn real_docx_recall_smoke() {
        // Character-recall smoke: deep parse of the real 923KB wordlist must
        // recover the bulk of extractable characters.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real/real.docx");
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

    #[tokio::test]
    async fn note_reference_inline_is_present() {
        let doc = docx_zip(&[
            (
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:r><w:t>Claim</w:t></w:r><w:r><w:footnoteReference w:id="2"/></w:r></w:p>
</w:body></w:document>"#,
            ),
            (
                "word/footnotes.xml",
                r#"<?xml version="1.0"?><w:footnotes xmlns:w="w">
<w:footnote w:id="2"><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:footnote>
</w:footnotes>"#,
            ),
        ]);
        let doc = parse(&doc).unwrap().document;
        let has_ref = doc.blocks.iter().any(|b| match b {
            Block::Paragraph(inlines) => inlines
                .iter()
                .any(|i| matches!(i, Inline::NoteRef(id) if id == "2")),
            _ => false,
        });
        assert!(has_ref, "footnote reference must appear in the body");
    }
}
