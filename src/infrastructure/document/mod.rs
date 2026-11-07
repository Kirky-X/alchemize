//! Document format converters.

pub mod csv;
pub mod epub;
pub mod html;
pub mod shared;
pub mod text;
pub mod zip;

pub use csv::CsvConverter;
pub use epub::EpubConverter;
pub use html::HtmlConverter;
pub use shared::{detect_encoding, local_tag_name};
pub use text::TextConverter;
pub use zip::ZipConverter;
