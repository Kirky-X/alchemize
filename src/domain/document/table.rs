// SPDX-License-Identifier: Apache-2.0
//! Table model: grid of cell slots with span-aware coverage.

use super::model::Block;

/// A table: rows of slots; spanned positions point back at their origin.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Table {
    /// Rows of slots; rows may differ in length when the source is ragged.
    pub grid: Vec<Vec<CellSlot>>,
    /// Number of leading header rows (0 = none).
    pub header_rows: usize,
    /// Whether the source used the table for data or layout.
    pub kind: TableKind,
}

impl Table {
    /// Column count = widest row.
    pub fn width(&self) -> usize {
        self.grid.iter().map(|r| r.len()).max().unwrap_or(0)
    }

    /// Whether the grid has any content at all.
    pub fn is_empty(&self) -> bool {
        self.grid.is_empty() || self.width() == 0
    }
}

/// What a grid position holds.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CellSlot {
    /// The cell itself with content and span extents.
    Origin(Cell),
    /// A position swallowed by a span from an earlier origin.
    Covered {
        /// Row of the covering origin.
        origin_row: usize,
        /// Column of the covering origin.
        origin_col: usize,
    },
}

/// One table cell.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    /// Cell content blocks.
    pub blocks: Vec<Block>,
    /// Columns covered, at least 1.
    pub col_span: u32,
    /// Rows covered, at least 1.
    pub row_span: u32,
}

impl Cell {
    /// A single-position cell with plain paragraph text.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            blocks: vec![Block::Paragraph(vec![super::Inline::Text {
                text: content.into(),
                style: super::model::Style::PLAIN,
            }])],
            col_span: 1,
            row_span: 1,
        }
    }
}

/// Source table intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TableKind {
    /// Data table (gets a markdown header row).
    #[default]
    Data,
    /// Layout table (no header inference).
    Layout,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Table {
        Table {
            grid: vec![
                vec![
                    CellSlot::Origin(Cell::text("A")),
                    CellSlot::Origin(Cell::text("B")),
                ],
                vec![
                    CellSlot::Origin(Cell {
                        blocks: vec![],
                        col_span: 2,
                        row_span: 1,
                    }),
                    CellSlot::Covered {
                        origin_row: 1,
                        origin_col: 0,
                    },
                ],
            ],
            header_rows: 1,
            kind: TableKind::Data,
        }
    }

    #[test]
    fn width_and_emptiness() {
        let t = grid();
        assert_eq!(t.width(), 2);
        assert!(!t.is_empty());
        assert!(Table::default().is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let t = grid();
        let json = serde_json::to_string(&t).unwrap();
        let back: Table = serde_json::from_str(&json).unwrap();
        assert_eq!(back.width(), 2);
        assert_eq!(back.header_rows, 1);
        assert!(matches!(back.grid[1][1], CellSlot::Covered { .. }));
    }

    #[test]
    fn cell_text_helper() {
        let c = Cell::text("hi");
        assert_eq!(c.col_span, 1);
        assert_eq!(c.row_span, 1);
        assert_eq!(c.blocks.len(), 1);
    }
}
