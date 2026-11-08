// SPDX-License-Identifier: Apache-2.0
//! DOCX notes parts: `word/footnotes.xml` and `word/endnotes.xml`.
//!
//! A note body is a sequence of paragraphs. The auto-number marker
//! (`w:footnoteRef` / `w:endnoteRef`) is skipped so it does not leak into
//! the extracted text; separator entries (id `-1` / `0`) are dropped.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::domain::document::{Block, Inline, Note, NoteKind, Style};

/// Parse a notes part into note bodies.
pub fn parse_notes(xml: &[u8], kind: NoteKind) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    let mut current_id: Option<String> = None;
    let mut current_blocks: Vec<Block> = Vec::new();
    let mut current_text = String::new();
    let mut in_paragraph = false;
    let mut skip_marker = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "footnote" | "endnote" => {
                        let id = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == "w:id")
                            .map(|a| a.value.clone().into_owned())
                            .unwrap_or_default();
                        current_id = Some(id);
                        current_blocks.clear();
                    }
                    "p" if current_id.is_some() => {
                        in_paragraph = true;
                        current_text.clear();
                    }
                    "footnoteRef" | "endnoteRef" => skip_marker = true,
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if local == "footnoteRef" || local == "endnoteRef" {
                    skip_marker = true;
                }
            }
            Ok(Event::Text(t)) => {
                if in_paragraph && !skip_marker {
                    current_text.push_str(&String::from_utf8_lossy(t.as_bytes()));
                }
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "p" if current_id.is_some() => {
                        in_paragraph = false;
                        let text = current_text.trim().to_string();
                        if !text.is_empty() {
                            current_blocks.push(Block::Paragraph(vec![Inline::Text {
                                text,
                                style: Style::PLAIN,
                            }]));
                        }
                        current_text.clear();
                    }
                    "footnote" | "endnote" => {
                        if let Some(id) = current_id.take() {
                            // Separator/continuation markers carry id -1 or 0.
                            if id != "0" && id != "-1" && !current_blocks.is_empty() {
                                notes.push(Note {
                                    id,
                                    kind,
                                    blocks: std::mem::take(&mut current_blocks),
                                });
                            }
                            current_blocks.clear();
                        }
                    }
                    _ => {}
                }
                skip_marker = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_bodies_and_skips_markers() {
        let xml = br#"<?xml version="1.0"?><w:footnotes xmlns:w="w">
<w:footnote w:id="-1"><w:p><w:r><w:t>sep</w:t></w:r></w:p></w:footnote>
<w:footnote w:id="0"><w:p><w:r><w:t>cont</w:t></w:r></w:p></w:footnote>
<w:footnote w:id="2"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t>Note body</w:t></w:r></w:p></w:footnote>
</w:footnotes>"#;
        let notes = parse_notes(xml, NoteKind::Footnote);
        assert_eq!(notes.len(), 1, "separators must be dropped");
        assert_eq!(notes[0].id, "2");
        assert_eq!(notes[0].kind, NoteKind::Footnote);
        let text = match &notes[0].blocks[0] {
            Block::Paragraph(inlines) => match &inlines[0] {
                Inline::Text { text, .. } => text.clone(),
                _ => panic!("expected text"),
            },
            _ => panic!("expected paragraph"),
        };
        assert_eq!(text, "Note body", "auto-number marker must not leak");
    }

    #[test]
    fn endnotes_use_endnote_kind() {
        let xml = br#"<?xml version="1.0"?><w:endnotes xmlns:w="w">
<w:endnote w:id="3"><w:p><w:r><w:t>End</w:t></w:r></w:p></w:endnote>
</w:endnotes>"#;
        let notes = parse_notes(xml, NoteKind::Endnote);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, NoteKind::Endnote);
    }
}
