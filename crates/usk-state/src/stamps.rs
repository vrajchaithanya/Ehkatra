//! Per-cell **winner stamps**, reconstructed from the op log (ADR-036, TD-46).
//!
//! # Why this file exists
//! ADR-005 says a summary tile carries no per-cell CRDT metadata, and TD-09
//! measured why: per-cell stamps took a 10M-cell workbook from 8.4 to 74.5
//! B/cell. A `State` therefore has nothing to serialise when an image is
//! written — which is not an oversight, it is the memory model working.
//!
//! But an image that carries no stamps cannot be *adopted and continued*. Apply
//! a tail op to a cell the image holds and there is no way to tell whether the
//! tail wins, and no way to keep the loser if it does — and ADR-006 and DP-A8
//! promise the loser is kept. That is the whole of TD-46.
//!
//! ADR-036 resolves it by reconstructing the stamps **at snapshot-write time**,
//! from the log, rather than keeping them resident. The reconstruction is
//! cheap because of where it happens: `Snapshot::build` already holds the whole
//! `OpLog` and already folds it once for the state hash, so this is a second
//! fold over ops already in hand — no new I/O, and nothing kept in memory
//! afterwards.
//!
//! # What a stamp is
//! The `(lamport, OpId)` of the write that **won** a cell. That is exactly the
//! pair `Meta::Mixed` stores for a contested cell, so an adopted image can
//! promote a cell retroactively and get the same answer a full replay would.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_oplog::{OpLog, Payload};
use usk_types::{ColId, Lamport, OpId, RowId};

/// The winning `(lamport, op id)` for one cell.
pub type Stamp = (Lamport, OpId);

/// Winner stamps for every cell an op log writes, keyed by cell **identity**
/// rather than by position — the same key the tile store interns, so an image
/// and its stamps cannot disagree about which cell they mean even after a row
/// is inserted above it (DP-A6).
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct WinnerStamps {
    by_cell: BTreeMap<(RowId, ColId), Stamp>,
    /// The greatest canonical key in the log this was built from.
    ///
    /// ADR-036 requires the image to record it so a tail op that is canonically
    /// *earlier* than the image is **refused rather than misplaced** — the
    /// ordering half of D-101, which D-102 explicitly left open. Applying such
    /// an op to a summary tile would silently overwrite a newer value, because
    /// the summary path trusts arrival order instead of comparing stamps.
    greatest: Option<Stamp>,
}

impl WinnerStamps {
    /// Folds a log into winner stamps.
    ///
    /// Order-independent by construction: it keeps the greatest
    /// `(lamport, id)` per cell rather than the last one seen, so it does not
    /// inherit `State::replay_sorted`'s precondition that its caller sorted the
    /// input (TD-11). A snapshot writer that fed ops in the wrong order would
    /// otherwise record stamps that disagree with the state beside them.
    pub fn from_log(log: &OpLog) -> WinnerStamps {
        let mut out = WinnerStamps::default();
        for op in log.ops() {
            let cell = match &op.payload {
                Payload::SetCell { row, col, .. }
                | Payload::ClearCell { row, col }
                | Payload::SetFormula { row, col, .. } => (*row, *col),
                _ => {
                    out.observe(op.lamport, op.id);
                    continue;
                }
            };
            out.observe(op.lamport, op.id);
            let stamp = (op.lamport, op.id);
            out.by_cell
                .entry(cell)
                .and_modify(|winner| {
                    if stamp > *winner {
                        *winner = stamp;
                    }
                })
                .or_insert(stamp);
        }
        out
    }

    fn observe(&mut self, lamport: Lamport, id: OpId) {
        let stamp = (lamport, id);
        if self.greatest.is_none_or(|g| stamp > g) {
            self.greatest = Some(stamp);
        }
    }

    /// The stamp of the cell's winning write, if the log wrote that cell.
    pub fn get(&self, row: RowId, col: ColId) -> Option<Stamp> {
        self.by_cell.get(&(row, col)).copied()
    }

    /// The greatest canonical key covered. `None` for an empty log.
    pub fn greatest(&self) -> Option<Stamp> {
        self.greatest
    }

    pub fn len(&self) -> usize {
        self.by_cell.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_cell.is_empty()
    }

    pub(crate) fn insert(&mut self, row: RowId, col: ColId, stamp: Stamp) {
        self.by_cell.insert((row, col), stamp);
    }

    pub(crate) fn set_greatest(&mut self, greatest: Option<Stamp>) {
        self.greatest = greatest;
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&(RowId, ColId), &Stamp)> {
        self.by_cell.iter()
    }
}

// ------------------------------------------------------------------ varint

/// LEB128, unsigned. The encoding D-102 measured at **3.10 B/cell**: within a
/// tile a bulk write assigns lamports and counters that ascend almost in
/// lockstep, so each delta is a single byte. The naive fixed-width layout the
/// TD-46 entry originally assumed measured 32 B/cell and *failed* A-001 by 7%.
pub(crate) fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads a varint, refusing an over-long encoding rather than wrapping.
///
/// An image is an untrusted input the moment a container is a file somebody can
/// hand you (docs/37), and a ten-byte run with the continuation bit set is the
/// cheapest way to make a decoder loop or overflow.
pub(crate) fn read_varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in 0..10 {
        let byte = *b.get(*at)?;
        *at += 1;
        v |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// Zig-zag, so a delta that goes backwards costs one byte rather than ten.
pub(crate) fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

pub(crate) fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}
