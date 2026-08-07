//! Identity references — the payoff for DP-A6 (docs/11, docs/04 invariant 3).
//!
//! A1 text is a *view*. A reference binds to permanent row and column
//! identities at edit time, and evaluation never performs address arithmetic.
//! Excel's insert/delete shift semantics then fall out structurally rather than
//! being implemented: nothing rewrites a reference when a row is inserted,
//! because the reference never named a position in the first place.
//!
//! # What "interval" means here
//! An interval is a pair of *endpoint identities*, and it resolves to whatever
//! currently lies between them in the axis order. That single rule produces
//! every behaviour docs/11 lists:
//!
//! * insert **above** the interval → the endpoints are unmoved, so the span is
//!   unchanged;
//! * insert **inside** it → the new row is between the endpoints, so it joins
//!   the range, which is what a spreadsheet user expects;
//! * delete inside → the range shrinks;
//! * delete an **endpoint** → re-anchor inward to the nearest surviving
//!   identity;
//! * every identity gone → empty interval → `#REF!`.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_state::State;
use usk_types::{CellError, ColId, ErrorKind, Origin, RowId, Value};

/// Whether an endpoint is rewritten when a formula is copied or filled.
///
/// Resolution ignores this — an anchored and an unanchored reference to the
/// same identity resolve identically. It matters to the reducer, which rewrites
/// relative endpoints on copy/fill (docs/11). Carried here so a bound reference
/// keeps the information rather than losing it at bind time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnchorMode {
    Relative,
    Absolute,
}

/// A reference bound to permanent identities.
///
/// Note there is no position anywhere in this type. That is the point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IdRange {
    pub row_start: RowId,
    pub row_end: RowId,
    pub col_start: ColId,
    pub col_end: ColId,
    pub row_anchor: AnchorMode,
    pub col_anchor: AnchorMode,
}

/// A resolved reference: the live cells an interval currently covers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resolved {
    pub rows: Vec<RowId>,
    pub cols: Vec<ColId>,
}

impl Resolved {
    pub fn cell_count(&self) -> usize {
        self.rows.len() * self.cols.len()
    }
}

/// One axis, in full order **including tombstones**, with an identity index.
///
/// docs/11 specifies an order-statistic tree. A sorted map over the order gives
/// the same complexity for the two queries that matter here — id → position and
/// position → id — without the rebalancing machinery; the tree earns its keep
/// when pixel offsets join the same index, which is a renderer concern.
///
/// Tombstones are retained in the order because they are what makes
/// "re-anchor inward" answerable: a deleted endpoint still marks the place the
/// interval used to reach, and the live order alone has forgotten it.
pub struct Axis<Id: Ord + Copy> {
    /// Every identity ever inserted on this axis, in order, tagged live.
    full: Vec<(Id, bool)>,
    /// identity → index into `full`.
    index: BTreeMap<Id, usize>,
    /// Live identities only — what A1 ordinals count.
    live: Vec<Id>,
    /// identity → live ordinal, for identities that still exist.
    live_index: BTreeMap<Id, usize>,
}

impl<Id: Ord + Copy> Axis<Id> {
    pub fn new(full: Vec<(Id, bool)>) -> Axis<Id> {
        let index = full
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();
        let live: Vec<Id> = full
            .iter()
            .filter(|(_, alive)| *alive)
            .map(|(id, _)| *id)
            .collect();
        let live_index = live.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        Axis {
            full,
            index,
            live,
            live_index,
        }
    }

    /// Number of live entries — the axis length a user sees.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// The identity currently at live `ordinal`, i.e. what A1 text names.
    pub fn at(&self, ordinal: usize) -> Option<Id> {
        self.live.get(ordinal).copied()
    }

    /// Where an identity currently renders. `None` once it is deleted.
    pub fn position_of(&self, id: &Id) -> Option<usize> {
        self.live_index.get(id).copied()
    }

    pub fn order(&self) -> &[Id] {
        &self.live
    }

    /// Resolves an endpoint pair to the live identities currently between them.
    ///
    /// Every documented insert/delete behaviour falls out of this one walk:
    /// endpoints are located in the *full* order, then each is moved inward to
    /// the nearest surviving identity. Anything inserted between them is
    /// included because it is, literally, between them; anything deleted
    /// disappears because it is no longer live. Nothing rewrites the reference.
    ///
    /// Returns empty when the endpoints have crossed — every identity in the
    /// interval is gone — which the caller reports as `#REF!`.
    pub fn resolve(&self, start: &Id, end: &Id) -> Vec<Id> {
        let (Some(&a), Some(&b)) = (self.index.get(start), self.index.get(end)) else {
            return Vec::new();
        };
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };

        // Move the low endpoint forward and the high endpoint back until both
        // land on something live. This is "re-anchor inward".
        let mut low = lo;
        while low <= hi && !self.full[low].1 {
            low += 1;
        }
        if low > hi {
            return Vec::new();
        }
        let mut high = hi;
        while high > low && !self.full[high].1 {
            high -= 1;
        }
        if !self.full[high].1 {
            return Vec::new();
        }

        self.full[low..=high]
            .iter()
            .filter(|(_, alive)| *alive)
            .map(|(id, _)| *id)
            .collect()
    }
}

/// Binds A1 view ordinals to identities against the current order.
///
/// This is the "at edit time" half of docs/04 invariant 3: text becomes
/// identities once, when the formula is authored, and never again.
pub struct Binder {
    pub rows: Axis<RowId>,
    pub cols: Axis<ColId>,
}

impl Binder {
    pub fn from_state(state: &State) -> Binder {
        Binder {
            rows: Axis::new(state.full_row_order()),
            cols: Axis::new(state.full_col_order()),
        }
    }

    /// Binds `A1:B2`-style ordinals. `None` when an ordinal is off the grid,
    /// which the caller reports as `#REF!`.
    pub fn bind(
        &self,
        row_start: usize,
        row_end: usize,
        col_start: usize,
        col_end: usize,
        row_anchor: AnchorMode,
        col_anchor: AnchorMode,
    ) -> Option<IdRange> {
        Some(IdRange {
            row_start: self.rows.at(row_start)?,
            row_end: self.rows.at(row_end)?,
            col_start: self.cols.at(col_start)?,
            col_end: self.cols.at(col_end)?,
            row_anchor,
            col_anchor,
        })
    }

    /// Binds a single cell.
    pub fn bind_cell(&self, row: usize, col: usize) -> Option<IdRange> {
        self.bind(
            row,
            row,
            col,
            col,
            AnchorMode::Relative,
            AnchorMode::Relative,
        )
    }
}

impl IdRange {
    /// Resolves against a (possibly much later) axis order.
    pub fn resolve(&self, binder: &Binder) -> Resolved {
        Resolved {
            rows: binder.rows.resolve(&self.row_start, &self.row_end),
            cols: binder.cols.resolve(&self.col_start, &self.col_end),
        }
    }

    /// Reads every live cell the reference covers, in identity order.
    ///
    /// An interval whose identities are all deleted is `#REF!` — docs/11's
    /// empty-interval rule — rather than an empty sum, because a range that
    /// lost its target is a broken formula, not a range of nothing.
    pub fn read(&self, state: &State, binder: &Binder) -> Result<Vec<Value>, CellError> {
        let resolved = self.resolve(binder);
        if resolved.rows.is_empty() || resolved.cols.is_empty() {
            return Err(CellError::new(ErrorKind::Ref, Origin::Propagated));
        }
        let mut out = Vec::with_capacity(resolved.cell_count());
        for row in &resolved.rows {
            for col in &resolved.cols {
                out.push(state.cell(*row, *col).unwrap_or(Value::Blank));
            }
        }
        Ok(out)
    }
}

/// Reads a `State` through the formula engine's `Grid` port, translating view
/// ordinals to identities on the way in.
///
/// This is where the calc engine stops speaking positions: the formula text
/// still says `A1`, the grid still has an extent, but every read goes
/// identity-first.
pub struct StateGrid<'a> {
    pub state: &'a State,
    pub binder: Binder,
}

impl<'a> StateGrid<'a> {
    pub fn new(state: &'a State) -> StateGrid<'a> {
        StateGrid {
            state,
            binder: Binder::from_state(state),
        }
    }
}

impl usk_formula::eval::Grid for StateGrid<'_> {
    fn read(&self, row: u32, col: u32) -> Option<Value> {
        let r = self.binder.rows.at(row as usize)?;
        let c = self.binder.cols.at(col as usize)?;
        Some(self.state.cell(r, c).unwrap_or(Value::Blank))
    }

    fn extent(&self) -> (u32, u32) {
        (self.binder.rows.len() as u32, self.binder.cols.len() as u32)
    }
}
