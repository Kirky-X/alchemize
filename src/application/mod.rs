//! Application layer — orchestration, use cases, and ports.

pub mod ports;
pub mod registry;

pub use ports::{
    Availability, Converter, ExternalTool, Transcript, TranscriptSegment, TranscribeProvider,
    VisionProvider,
};
pub use registry::ConverterRegistry;
