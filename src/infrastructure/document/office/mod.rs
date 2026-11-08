//! Self-hosted office-format deep parsers.

pub mod docx;
pub mod odt;
pub mod odp;
pub mod numfmt;
pub mod omml;
pub mod pptx_deep;

pub use docx::DocxDeepConverter;
pub use odp::OdpConverter;
pub use odt::OdtConverter;
pub use pptx_deep::PptxDeepConverter;
