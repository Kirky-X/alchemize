//! Self-hosted office-format deep parsers.

pub mod docx;
pub mod odt;
pub mod odp;
pub mod numfmt;
pub mod omml;
pub mod pptx_deep;

#[cfg(feature = "libreoffice")]
pub mod legacy_office;

pub use docx::DocxDeepConverter;
pub use odp::OdpConverter;
pub use odt::OdtConverter;
pub use pptx_deep::PptxDeepConverter;
#[cfg(feature = "libreoffice")]
pub use legacy_office::{LegacyOfficeConverter, LegacyPptConverter};
