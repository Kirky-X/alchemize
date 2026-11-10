#![cfg(feature = "containers")]

//! Recursive archive container converters (tar/tgz/gz/xz/zst/7z).
//!
//! Shared skeleton: [`ContainerBase`] holds the [`ConverterRegistry`] used to
//! convert extracted entries, enforces decompression-bomb safety limits, and
//! renders the per-entry markdown with a graceful fallback (fenced code block
//! plus attachment list) for entries no converter supports.
//!
//! This module declares the shared surface; implementations live in `base.rs`.

pub mod sevenz;
pub mod single;
pub mod tar;

mod base;

pub(crate) use base::{
    acquire_depth, charge_chain_bytes, check_entry_size, check_total_bytes, fallback_entry,
    is_metadata_entry, read_limited, with_depth_scope,
};
pub use sevenz::SevenZConverter;
pub use single::{GzConverter, XzConverter, ZstConverter};
pub use tar::{TarConverter, TarGzConverter};

use std::sync::{Arc, OnceLock};

use crate::application::registry::ConverterRegistry;
use crate::domain::error::AlchemizeError;

/// Maximum size of a single entry inside a container.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum total decompressed size of a container (bomb protection).
///
/// Deliberately lower than the zip facade's 1 GiB aggregate limit: archive
/// containers keep every extracted entry alive at once while rendering, so
/// the in-memory ceiling is stricter there.
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum decompressed size for a single-file compressed stream (gz/xz/zst).
pub const MAX_SINGLE_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum container nesting depth when recursively converting archives.
pub const MAX_DEPTH: u32 = 3;

/// Maximum cumulative decompressed bytes across one nesting chain.
///
/// Per-layer limits are not enough: three nested layers can each hold
/// `MAX_TOTAL_BYTES` while the parent entries stay alive.
pub(crate) const MAX_CHAIN_BYTES: u64 = 512 * 1024 * 1024;

/// Upper bound for inlining fallback text into a fenced code block.
pub(crate) const FALLBACK_TEXT_BYTES: usize = 8 * 1024;

/// One extracted file from a container archive.
#[derive(Debug)]
pub struct ContainerEntry {
    /// Entry path as stored in the archive.
    pub name: String,
    /// Entry content.
    pub bytes: Vec<u8>,
}

/// A container format that can be enumerated into flat entries.
///
/// Implemented by per-archive view structs (e.g. [`tar::TarArchive`]); the
/// corresponding `Converter` impls construct the view from the input bytes
/// and hand the entries to [`ContainerBase::convert_container`].
pub trait RecursiveContainer {
    /// Stable format identifier (recorded as `metadata.source_format`).
    fn format_name(&self) -> &'static str;

    /// Extract all file entries. Directories, symlinks and pax/long-name
    /// headers are skipped; per-entry and total size limits are enforced.
    fn entries(&self) -> Result<Vec<ContainerEntry>, AlchemizeError>;
}

/// Shared skeleton for recursive container converters.
///
/// The registry sits behind a deferred slot because container converters are
/// registered *into* the registry they dispatch nested entries through; see
/// [`RegistrySlot`]. Nesting depth is tracked per conversion chain by the
/// crate-internal `with_depth_scope`/`acquire_depth` primitives, not on this
/// struct. Third-party container converters built outside this crate must
/// enforce their own nesting bound: `RecursiveContainer` only enumerates
/// entries. Implementation lives in `base.rs`.
pub struct ContainerBase {
    pub(crate) registry: RegistrySlot,
    /// Strong reference used only by the test-only `with_arc` constructor;
    /// production wiring relies on the weak slot so the registry never
    /// references itself through its converters (no Arc cycle).
    #[cfg(test)]
    pub(crate) owned: Option<Arc<ConverterRegistry>>,
}

/// Deferred registry handle (weak: the registry holds the converters that
/// hold this slot, so a strong reference would form a cycle).
///
/// The final `Arc<ConverterRegistry>` cannot exist at construction time:
/// the converter must be registered into that very registry. The builder
/// creates the slot, hands it to every container converter, assembles the
/// registry, then fills the slot — the same post-assembly wiring the zip
/// facade uses. If a caller leaves the slot empty, entries degrade to
/// attachments instead of failing the whole archive.
pub type RegistrySlot = Arc<OnceLock<std::sync::Weak<ConverterRegistry>>>;
