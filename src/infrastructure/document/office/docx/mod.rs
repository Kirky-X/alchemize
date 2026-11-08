// SPDX-License-Identifier: Apache-2.0
//! DOCX deep parser (self-hosted): WordprocessingML event-stream parsing into
//! the unified document model with style inheritance, footnotes, merged
//! table cells, drawing assets, and OMML math.
//!
//! Submodules:
//! - [`styles`] — the style cascade (`docDefaults` → `basedOn` chain → own)
//! - [`notes`] — footnotes and endnotes
//! - [`body`] — the `word/document.xml` event stream
//! - [`converter`] — the `Converter` implementation and package plumbing

pub mod body;
pub mod converter;
pub mod notes;
pub mod styles;

pub use converter::{parse, DocxDeepConverter, Warnings};
pub use styles::{parse_styles, RunProps, StyleTable};
