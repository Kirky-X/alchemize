#![cfg(any(feature = "parquet", feature = "sqlite", feature = "bibtex"))]

//! Structured data converters (parquet/sqlite/bib) — T024/T025/T026.
//!
//! Per-format features (capability-named): each converter compiles only when
//! its own feature is enabled.

#[cfg(feature = "bibtex")]
pub mod bib;
#[cfg(feature = "parquet")]
pub mod parquet;
#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "bibtex")]
pub use bib::BibConverter;
#[cfg(feature = "parquet")]
pub use parquet::ParquetConverter;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteConverter;
