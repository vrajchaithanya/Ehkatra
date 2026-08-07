//! The sheet the calc engine evaluates over, and the geometry it reasons with.
//!
//! This is a *view-ordinal* model: cells are addressed by `(row, col)` position.
//! DP-A6 says references are identity intervals and A1 is a computed view, and
//! that is what Row 8 installs; until then the engine speaks ordinals and the
//! seam is deliberately narrow — `Rect` and `CellRef` are the only two types
//! that would change.

use alloc::string::String;
use alloc::vec::Vec;
use usk_types::Value;

/// A cell address in view ordinals, 0-based.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

/// An inclusive rectangle of cells. The unit of dependency: groups read rects
/// and write rects, never individual cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub r0: u32,
    pub r1: u32,
    pub c0: u32,
    pub c1: u32,
}

impl Rect {
    pub fn single(cell: CellRef) -> Rect {
        Rect {
            r0: cell.row,
            r1: cell.row,
            c0: cell.col,
            c1: cell.col,
        }
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.r0 <= other.r1 && other.r0 <= self.r1 && self.c0 <= other.c1 && other.c0 <= self.c1
    }

    pub fn contains(&self, cell: CellRef) -> bool {
        cell.row >= self.r0 && cell.row <= self.r1 && cell.col >= self.c0 && cell.col <= self.c1
    }

    /// Smallest rectangle covering both.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            r0: self.r0.min(other.r0),
            r1: self.r1.max(other.r1),
            c0: self.c0.min(other.c0),
            c1: self.c1.max(other.c1),
        }
    }

    pub fn cell_count(&self) -> u64 {
        (self.r1 - self.r0 + 1) as u64 * (self.c1 - self.c0 + 1) as u64
    }
}

/// What occupies a cell.
#[derive(Clone, Debug)]
pub enum Cell {
    Empty,
    /// A value written directly.
    Literal(Value),
    /// A formula belonging to group `group`, plus its last computed value.
    ///
    /// The cached value is a fold over the log with a watermark, not
    /// independently mutable state (DP-A9): every write to it goes through
    /// `Engine::recalc`.
    Formula {
        group: u32,
        cached: Value,
    },
}

/// A rectangular sheet of cells.
pub struct Sheet {
    rows: u32,
    cols: u32,
    cells: Vec<Cell>,
    /// Formula source per cell, kept so groups can be rebuilt and so the CST is
    /// recoverable for explanation (DP-A11).
    sources: Vec<Option<String>>,
}

impl Sheet {
    pub fn new(rows: u32, cols: u32) -> Sheet {
        let n = (rows as usize) * (cols as usize);
        let mut cells = Vec::new();
        cells.resize(n, Cell::Empty);
        let mut sources = Vec::new();
        sources.resize(n, None);
        Sheet {
            rows,
            cols,
            cells,
            sources,
        }
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    fn index(&self, cell: CellRef) -> Option<usize> {
        if cell.row >= self.rows || cell.col >= self.cols {
            return None;
        }
        Some((cell.row as usize) * (self.cols as usize) + cell.col as usize)
    }

    pub fn cell(&self, cell: CellRef) -> Option<&Cell> {
        self.cells.get(self.index(cell)?)
    }

    /// The value a reader sees: a literal, a formula's cached result, or blank.
    pub fn value(&self, cell: CellRef) -> Option<Value> {
        match self.cell(cell)? {
            Cell::Empty => Some(Value::Blank),
            Cell::Literal(v) => Some(v.clone()),
            Cell::Formula { cached, .. } => Some(cached.clone()),
        }
    }

    pub fn set_literal(&mut self, cell: CellRef, value: Value) {
        if let Some(i) = self.index(cell) {
            self.cells[i] = Cell::Literal(value);
            self.sources[i] = None;
        }
    }

    pub fn set_formula(&mut self, cell: CellRef, source: &str) {
        if let Some(i) = self.index(cell) {
            self.cells[i] = Cell::Formula {
                group: u32::MAX,
                cached: Value::Blank,
            };
            self.sources[i] = Some(String::from(source));
        }
    }

    pub fn formula_source(&self, cell: CellRef) -> Option<&str> {
        self.sources.get(self.index(cell)?)?.as_deref()
    }

    pub(crate) fn assign_group(&mut self, cell: CellRef, group: u32) {
        if let Some(i) = self.index(cell) {
            if let Cell::Formula { group: g, .. } = &mut self.cells[i] {
                *g = group;
            }
        }
    }

    pub(crate) fn store_result(&mut self, cell: CellRef, value: Value) {
        if let Some(i) = self.index(cell) {
            if let Cell::Formula { cached, .. } = &mut self.cells[i] {
                *cached = value;
            }
        }
    }

    /// Every cell holding a formula, in row-major identity order. Deterministic
    /// traversal is a convergence requirement, not a convenience (docs/13).
    pub(crate) fn formula_cells(&self) -> Vec<CellRef> {
        let mut out = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let c = CellRef { row, col };
                if matches!(self.cell(c), Some(Cell::Formula { .. })) {
                    out.push(c);
                }
            }
        }
        out
    }
}

/// Reads the sheet's *cached* values, which is what a formula sees.
impl usk_formula::eval::Grid for Sheet {
    fn read(&self, row: u32, col: u32) -> Option<Value> {
        self.value(CellRef { row, col })
    }

    fn extent(&self) -> (u32, u32) {
        (self.rows, self.cols)
    }
}
