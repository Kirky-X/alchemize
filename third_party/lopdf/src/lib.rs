#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(clippy::all)]

pub mod content;
pub mod encryption;
pub mod filters;
pub mod xobject;
pub mod xref;

#[macro_use]
mod object;
mod document;
mod incremental_document;

mod bookmarks;
mod cmap_section;
mod common_data_structures;
mod creator;
mod datetime;
mod destinations;
mod encodings;
mod error;
mod outlines;
mod processor;
mod toc;
mod writer;

mod load_options;
mod object_stream;
mod parser;
mod parser_aux;
mod reader;
mod save_options;

mod font;

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fork addition (alchemize): per-stream decompressed-output ceiling in bytes.
///
/// A few kilobytes of compressed FlateDecode/LZWDecode data can inflate to
/// gigabytes (decompression bomb). Every stream decompression — including
/// ObjStm/XRefStm during [`Document::parse`] — is bounded by this value.
/// Defaults to 64 MiB (aligned with alchemize's container per-entry cap);
/// `0` disables the cap.
static MAX_STREAM_DECOMPRESSED_BYTES: AtomicUsize = AtomicUsize::new(64 * 1024 * 1024);

/// Set the per-stream decompressed-output ceiling in bytes (`0` = unlimited).
///
/// Returns the previous limit. Applies to every subsequent
/// `Stream::decompressed_content()` call in this process.
pub fn set_max_stream_decompressed_bytes(limit: usize) -> usize {
    MAX_STREAM_DECOMPRESSED_BYTES.swap(limit, Ordering::Relaxed)
}

/// Current per-stream decompressed-output ceiling (`0` = unlimited).
pub fn max_stream_decompressed_bytes() -> usize {
    MAX_STREAM_DECOMPRESSED_BYTES.load(Ordering::Relaxed)
}

thread_local! {
    /// Fork addition (alchemize): set when a stream decompression on this
    /// thread was refused by the cap. Hosts read it after a parse call —
    /// some consumers swallow per-stream errors and would otherwise turn a
    /// bomb into silent garbage output.
    static STREAM_DECOMPRESSION_LIMIT_HIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether any stream decompression on the current thread hit the ceiling
/// since the last [`reset_stream_decompression_limit_hit`].
pub fn stream_decompression_limit_hit() -> bool {
    STREAM_DECOMPRESSION_LIMIT_HIT.with(Cell::get)
}

/// Clear the current thread's decompression-ceiling flag.
pub fn reset_stream_decompression_limit_hit() {
    STREAM_DECOMPRESSION_LIMIT_HIT.with(|c| c.set(false));
}

/// Internal: record a refused decompression on the current thread.
pub(crate) fn note_stream_decompression_limit_hit() {
    STREAM_DECOMPRESSION_LIMIT_HIT.with(|c| c.set(true));
}

pub use document::Document;
pub use object::{Dictionary, Object, ObjectId, Stream, StringFormat};

pub use bookmarks::Bookmark;
pub use common_data_structures::{decode_text_string, text_string};
pub use destinations::Destination;
pub use encodings::{Encoding, encode_utf8, encode_utf16_be};
pub use encryption::{EncryptionState, EncryptionVersion, Permissions};
pub use error::{Error, Result};
pub use incremental_document::IncrementalDocument;
pub use load_options::{FilterFunc, LoadOptions};
pub use object_stream::{ObjectStream, ObjectStreamBuilder, ObjectStreamConfig};
pub use outlines::Outline;
pub use reader::{PdfMetadata, Reader};
pub use save_options::{SaveOptions, SaveOptionsBuilder};
pub use toc::Toc;

pub use parser_aux::substr;
pub use parser_aux::substring;

pub use font::FontData;
