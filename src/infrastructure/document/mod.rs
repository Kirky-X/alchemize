// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Document format converters.

pub mod csv;
pub mod docx;
pub mod eml;
pub mod epub;
pub mod gfm_render;
pub mod html;
#[cfg(feature = "html-deep")]
pub mod html_deep;
pub mod ipynb;
pub mod latex_source;
pub mod office;
pub mod pandoc_generic;
pub mod pdf;
#[cfg(feature = "pdfium")]
pub mod pdf_images;
pub mod pdfium;
pub mod pptx;
pub mod shared;
pub mod text;
pub mod xlsx;
pub mod zip;

pub use csv::CsvConverter;
pub use docx::DocxConverter;
#[cfg(feature = "eml")]
pub use eml::EmlConverter;
pub use epub::EpubConverter;
pub use html::HtmlConverter;
pub use ipynb::IpynbConverter;
pub use latex_source::LatexSourceConverter;
pub use office::{DocxDeepConverter, OdpConverter, OdtConverter, PptxDeepConverter};
#[cfg(feature = "libreoffice")]
pub use office::{LegacyOfficeConverter, LegacyPptConverter};
pub use pandoc_generic::PandocGenericConverter;
pub use pdf::PdfConverter;
#[cfg(feature = "pdfium")]
pub use pdfium::PdfiumConverter;
pub use pptx::PptxConverter;
pub use shared::{decode_entity, detect_encoding, local_tag_name};
pub use text::TextConverter;
pub use xlsx::XlsxConverter;
pub use zip::ZipConverter;
