// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Document format converters.

pub mod gfm_render;
pub mod csv;
pub mod docx;
pub mod epub;
pub mod office;
pub mod html;
#[cfg(feature = "moli")]
pub mod html_moli;
pub mod ipynb;
pub mod latex_source;
pub mod pdf;
pub mod pdfium;
pub mod pptx;
pub mod shared;
pub mod text;
pub mod xlsx;
pub mod zip;

pub use csv::CsvConverter;
pub use docx::DocxConverter;
pub use epub::EpubConverter;
pub use office::{
    DocxDeepConverter, OdpConverter, OdtConverter, PptxDeepConverter,
};
#[cfg(feature = "libreoffice")]
pub use office::{LegacyOfficeConverter, LegacyPptConverter};
pub use html::HtmlConverter;
pub use ipynb::IpynbConverter;
pub use latex_source::LatexSourceConverter;
pub use pdf::PdfConverter;
#[cfg(feature = "pdfium")]
pub use pdfium::PdfiumConverter;
pub use pptx::PptxConverter;
pub use shared::{decode_entity, detect_encoding, local_tag_name};
pub use text::TextConverter;
pub use xlsx::XlsxConverter;
pub use zip::ZipConverter;
