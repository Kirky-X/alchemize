#![cfg(feature = "data")]

//! Structured data converters (parquet/sqlite/bib) — T024/T025/T026.

pub mod bib;
pub mod parquet;
pub mod sqlite;

pub use bib::BibConverter;
pub use parquet::ParquetConverter;
pub use sqlite::SqliteConverter;
