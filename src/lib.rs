//! Alchemize — transmute documents into Markdown.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Input formats the converter recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Html,
    Epub,
    Audio,
    Video,
}
