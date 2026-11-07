//! Domain layer — pure data types and rules, zero IO.

pub mod asset;
pub mod error;
pub mod input;
pub mod latex_validate;
pub mod result;

pub use asset::{Asset, AssetRole, AssetSink, DirSink, InlineSink};
pub use error::AlchemizeError;
pub use input::{Input, InputFormat, detect_format, ext_to_format};
pub use latex_validate::validate_markdown_math;
pub use result::{
    ContentType, ConversionQuality, ConvertResult, DetailLevel, VideoProcessingMode,
};
