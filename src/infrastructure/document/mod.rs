// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Document format converters.

pub mod anydoc;
pub mod anydoc_render;
pub mod csv;
pub mod docx;
pub mod epub;
pub mod html;
#[cfg(feature = "moli")]
pub mod html_moli;
pub mod ipynb;
pub mod latex_source;
pub mod pdf;
pub mod pptx;
pub mod shared;
pub mod text;
pub mod xlsx;
pub mod zip;

#[cfg(feature = "anydoc")]
pub use anydoc::AnydocConverter;
pub use csv::CsvConverter;
pub use docx::DocxConverter;
pub use epub::EpubConverter;
pub use html::HtmlConverter;
pub use ipynb::IpynbConverter;
pub use latex_source::LatexSourceConverter;
pub use pdf::PdfConverter;
pub use pptx::PptxConverter;
pub use shared::{detect_encoding, local_tag_name};
pub use text::TextConverter;
pub use xlsx::XlsxConverter;
pub use zip::ZipConverter;
