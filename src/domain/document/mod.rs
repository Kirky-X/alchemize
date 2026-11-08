//! Unified document model (self-hosted; shape informed by the retired
//! anydoc::model design, zero code copied).

pub mod model;
pub mod table;

pub use model::{
    Asset, Block, Document, ImageSource, Inline, LinkTarget, List, ListItem, MarkerKind, Note,
    NoteKind, Style,
};
pub use table::{Cell, CellSlot, Table, TableKind};
