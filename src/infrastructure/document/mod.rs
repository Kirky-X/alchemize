//! Document format converters.

pub mod anydoc;
pub mod anydoc_render;
pub mod csv;
pub mod epub;
pub mod xlsx;
pub mod html;
pub mod ipynb;
pub mod latex_source;
pub mod pdf;
pub mod shared;
pub mod docx;
pub mod pptx;
pub mod text;
pub mod zip;

#[cfg(feature = "anydoc")]
pub use anydoc::AnydocConverter;
pub use csv::CsvConverter;
pub use epub::EpubConverter;
pub use xlsx::XlsxConverter;
pub use html::HtmlConverter;
pub use ipynb::IpynbConverter;
pub use latex_source::LatexSourceConverter;
pub use pdf::PdfConverter;
pub use shared::{detect_encoding, local_tag_name};
pub use docx::DocxConverter;
pub use pptx::PptxConverter;
pub use text::TextConverter;
pub use zip::ZipConverter;
