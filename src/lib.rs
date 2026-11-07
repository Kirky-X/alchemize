//! Alchemize — transmute documents into Markdown.

pub mod domain;

pub use domain::error::AlchemizeError;
pub use domain::input::{Input, InputFormat, detect_format};

/// Crate version (mirrors Cargo.toml via workspace inheritance).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
