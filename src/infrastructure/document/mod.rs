// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Document format converters.
//!
//! Every converter family is gated by its own capability feature (see
//! `Cargo.toml`); modules with no optional dependencies stay unconditional.

#[cfg(feature = "csv")]
pub mod csv;
#[cfg(feature = "docx")]
pub mod docx;
#[cfg(feature = "email")]
pub mod eml;
#[cfg(feature = "epub")]
pub mod epub;
pub mod gfm_render;
#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "html")]
pub mod html_deep;
#[cfg(feature = "ipynb")]
pub mod ipynb;
#[cfg(feature = "latex")]
pub mod latex_source;
#[cfg(any(
    feature = "docx",
    feature = "pptx",
    feature = "odt",
    feature = "odp",
    feature = "legacy-office"
))]
pub mod office;
#[cfg(feature = "pandoc")]
pub mod pandoc_generic;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "pdf-local")]
pub mod pdf_images;
#[cfg(feature = "pdf-local")]
pub mod pdfium;
#[cfg(feature = "pptx")]
pub mod pptx;
pub mod shared;
#[cfg(feature = "text")]
pub mod text;
#[cfg(feature = "zip")]
pub mod zip;

#[cfg(feature = "csv")]
pub use csv::CsvConverter;
#[cfg(feature = "docx")]
pub use docx::DocxConverter;
#[cfg(feature = "email")]
pub use eml::EmlConverter;
#[cfg(feature = "epub")]
pub use epub::EpubConverter;
#[cfg(feature = "html")]
pub use html::HtmlConverter;
#[cfg(feature = "ipynb")]
pub use ipynb::IpynbConverter;
#[cfg(feature = "latex")]
pub use latex_source::LatexSourceConverter;
#[cfg(feature = "docx")]
pub use office::DocxDeepConverter;
#[cfg(feature = "odp")]
pub use office::OdpConverter;
#[cfg(feature = "odt")]
pub use office::OdtConverter;
#[cfg(feature = "pptx")]
pub use office::PptxDeepConverter;
#[cfg(feature = "legacy-office")]
pub use office::{LegacyOfficeConverter, LegacyPptConverter};
#[cfg(feature = "pandoc")]
pub use pandoc_generic::PandocGenericConverter;
#[cfg(feature = "pdf")]
pub use pdf::PdfConverter;
#[cfg(feature = "pdf-local")]
pub use pdfium::PdfiumConverter;
#[cfg(feature = "pptx")]
pub use pptx::PptxConverter;
#[cfg(any(
    feature = "text",
    feature = "csv",
    feature = "pandoc",
    feature = "media"
))]
pub use shared::detect_encoding;
pub use shared::{decode_entity, local_tag_name};
#[cfg(feature = "text")]
pub use text::TextConverter;
#[cfg(feature = "zip")]
pub use zip::ZipConverter;
