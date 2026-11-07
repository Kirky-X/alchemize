//! Document format converters.

pub mod csv;
pub mod shared;
pub mod text;
pub mod zip;

pub use csv::CsvConverter;
pub use shared::{detect_encoding, local_tag_name};
pub use text::TextConverter;
pub use zip::ZipConverter;
