// SPDX-License-Identifier: Apache-2.0
//! ODP deep parser (self-hosted): OpenDocument presentation slides into the
//! unified document model.
//!
//! The ODF event-stream engine lives in [`super::odt`] — the shared walker
//! already handles `draw:page` slides (`# Slide N` headings in document
//! order), `presentation:notes` (Endnote, id `slide{N}-notes`), text-box
//! paragraphs, lists, tables, and images. This file carries the `Converter`
//! implementation and the ODP-specific tests.

use async_trait::async_trait;

use super::odt::{build_result, parse};
use crate::application::ports::{Availability, Converter};

use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::ConvertResult;

/// Deep ODP converter: OpenDocument presentation → the unified model.
pub struct OdpConverter;

impl OdpConverter {
    /// Create a new deep ODP converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for OdpConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for OdpConverter {
    fn name(&self) -> &'static str {
        "odp-deep"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read ODP input: {e}")))?;
        let parsed = tokio::task::spawn_blocking(move || parse(&bytes))
            .await
            .map_err(|e| AlchemizeError::other(format!("odp-deep task panicked: {e}")))??;
        build_result(parsed, "odp")
    }

    fn supported_extensions(&self) -> &[&str] {
        &["odp"]
    }

    fn priority(&self) -> u8 {
        18
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{Block, Inline, NoteKind};
    use crate::domain::document::Document;
    use std::io::Cursor;

    const NS: &str = r#"xmlns:office="o" xmlns:text="t" xmlns:style="s" xmlns:fo="f" xmlns:draw="d" xmlns:presentation="p" xmlns:xlink="x""#;

    fn odp_with(content: &str) -> Vec<u8> {
        use std::io::Write;
        let buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("content.xml", opts).unwrap();
        w.write_all(content.as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    }

    async fn convert(bytes: Vec<u8>) -> ConvertResult {
        OdpConverter::new()
            .convert(Input::bytes(bytes, "t.odp"))
            .await
            .unwrap()
    }

    #[test]
    fn converter_traits() {
        let c = OdpConverter::new();
        assert_eq!(c.name(), "odp-deep");
        assert_eq!(c.supported_extensions(), &["odp"]);
        assert_eq!(c.priority(), 18);
        assert_eq!(c.availability(), Availability::Available);
    }

    #[tokio::test]
    async fn every_page_yields_a_slide_heading_in_order() {
        let r = convert(odp_with(&format!(
            r#"<office:document-content {NS}><office:body><office:presentation>
<draw:page draw:name="a"><draw:frame><draw:text-box><text:p>alpha text</text:p></draw:text-box></draw:frame></draw:page>
<draw:page draw:name="b"><draw:frame><draw:text-box><text:p>beta text</text:p></draw:text-box></draw:frame></draw:page>
</office:presentation></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("# Slide 1"), "md: {}", r.markdown);
        assert!(r.markdown.contains("# Slide 2"), "md: {}", r.markdown);
        assert!(r.markdown.contains("alpha text"), "md: {}", r.markdown);
        assert!(r.markdown.contains("beta text"), "md: {}", r.markdown);
        // Slide 1's heading precedes its content; Slide 2 follows.
        let s1 = r.markdown.find("# Slide 1").unwrap();
        let s2 = r.markdown.find("# Slide 2").unwrap();
        assert!(s1 < s2, "pages must keep document order");
        assert_eq!(r.metadata["source_format"], "odp");
        assert_eq!(
            r.metadata["paragraph_count"].as_u64(),
            Some(2),
            "meta: {}",
            r.metadata
        );
    }

    #[tokio::test]
    async fn slide_notes_become_endnotes_with_slide_ids() {
        let bytes = odp_with(&format!(
            r#"<office:document-content {NS}><office:body><office:presentation>
<draw:page draw:name="a"><draw:frame><draw:text-box><text:p>body text</text:p></draw:text-box></draw:frame>
<presentation:notes><draw:frame><draw:text-box><text:p>speaker note sentence</text:p></draw:text-box></draw:frame></presentation:notes>
</draw:page>
</office:presentation></office:body></office:document-content>"#
        ));
        let doc: Document = parse(&bytes).unwrap().document;
        assert_eq!(doc.notes.len(), 1);
        assert_eq!(doc.notes[0].id, "slide1-notes");
        assert_eq!(doc.notes[0].kind, NoteKind::Endnote);
        let note_text = match &doc.notes[0].blocks[0] {
            Block::Paragraph(inlines) => match &inlines[0] {
                Inline::Text { text, .. } => text.clone(),
                _ => panic!("expected text"),
            },
            _ => panic!("expected paragraph"),
        };
        assert_eq!(note_text, "speaker note sentence");
        let r = convert(bytes).await;
        assert!(
            r.markdown.contains("speaker note sentence"),
            "md: {}",
            r.markdown
        );
    }

    #[tokio::test]
    async fn slide_images_become_sha8_named_assets() {
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
        w.start_file(
            "content.xml",
            opts,
        )
        .unwrap();
        w.write_all(
            format!(
                r#"<office:document-content {NS}><office:body><office:presentation>
<draw:page><draw:frame><draw:image xlink:href="Pictures/pic.png"/></draw:frame></draw:page>
</office:presentation></office:body></office:document-content>"#
            )
            .as_bytes(),
        )
        .unwrap();
        w.start_file("Pictures/pic.png", opts).unwrap();
        w.write_all(png).unwrap();
        let bytes = w.finish().unwrap().into_inner();

        let r = convert(bytes).await;
        let sha8 = crate::domain::asset::Asset::new("x.png", png.to_vec()).sha8();
        let reference = format!("images/0000-{sha8}.png");
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].filename, reference);
        assert!(
            r.markdown.contains(&format!("![]({reference})")),
            "md: {}",
            r.markdown
        );
    }

    #[tokio::test]
    async fn slide_span_styles_apply() {
        let r = convert(odp_with(&format!(
            r#"<office:document-content {NS}>
<office:automatic-styles>
<style:style style:name="TBold" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
</office:automatic-styles>
<office:body><office:presentation>
<draw:page><draw:frame><draw:text-box><text:p><text:span text:style-name="TBold">loud words</text:span></text:p></draw:text-box></draw:frame></draw:page>
</office:presentation></office:body></office:document-content>"#
        )))
        .await;
        assert!(r.markdown.contains("**loud words**"), "md: {}", r.markdown);
    }

    #[tokio::test]
    async fn real_odp_fixture_parses() {
        // 构造合法 fixture（磁盘未找到真实 ODP 样本）：断言其 content.xml
        // 中的真实关键词、页序与讲者备注。
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/real/real.odp");
        let r = OdpConverter::new().convert(Input::Path(path)).await.unwrap();
        assert!(r.markdown.contains("# Slide 1"), "md: {}", r.markdown);
        assert!(r.markdown.contains("# Slide 2"), "md: {}", r.markdown);
        assert!(r.markdown.contains("Slide one headline"), "md: {}", r.markdown);
        assert!(r.markdown.contains("Slide two headline"), "md: {}", r.markdown);
        assert!(
            r.markdown.contains("speaker notes sentence for slide two"),
            "notes reach md: {}",
            r.markdown
        );
        assert!(r.markdown.contains("images/0000-"), "md: {}", r.markdown);
        assert_eq!(r.metadata["source_format"], "odp");
        assert_eq!(r.assets.len(), 1, "meta: {}", r.metadata);
        let s1 = r.markdown.find("# Slide 1").unwrap();
        let s2 = r.markdown.find("# Slide 2").unwrap();
        assert!(s1 < s2, "document page order");
    }
}
