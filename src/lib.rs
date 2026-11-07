//! Alchemize — transmute documents into Markdown.

pub mod application;
pub mod domain;
pub mod infrastructure;

pub use application::builder::{Alchemize, AlchemizeBuilder, MineruConfig};
pub use application::registry::ConverterRegistry;
pub use domain::error::AlchemizeError;
pub use domain::input::{Input, InputFormat, detect_format};

/// Crate version (mirrors Cargo.toml via workspace inheritance).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
