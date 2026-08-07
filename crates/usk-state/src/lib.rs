//! usk-state — CRDT state: axis order + cell registers, fold over the op log.
//!
//! State is DERIVED: `State::replay(log)` is the only constructor from data,
//! and all mutators are private to the applier (docs/03 invariant I3).
//! Convergence contract: same op set (any order) ⇒ same state ⇒ same hash.

#![no_std]
extern crate alloc;

pub mod tile;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use tile::{PromotionStats, TileStore};
use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_types::{ColId, Lamport, OpId, RowId, Value};

/// One axis (rows or columns) as a neighbor-anchored ordered sequence with
/// tombstones. Deterministic concurrent-insert resolution: children of the
/// same anchor order by (lamport, actor, counter) DESCENDING so later
/// same-place inserts land nearer their anchor (RGA-style, interleaving-safe
/// for the v0.1 single-level case; full Fugue tree arrives with usk-state v2).
#[derive(Default, Clone)]
struct AxisSeq {
    /// Insertion tree: anchor op-id -> ordered child op ids.
    children: BTreeMap<Option<OpId>, Vec<(u64, OpId)>>,
    tombstones: BTreeMap<OpId, ()>,
}

impl AxisSeq {
    fn insert(&mut self, anchor: &Anchor, id: OpId, lamport: u64) {
        let key = match anchor {
            Anchor::Start => None,
            Anchor::After(a) => Some(*a),
        };
        let kids = self.children.entry(key).or_default();
        // Deterministic position independent of arrival order.
        let pos = kids
            .binary_search_by(|(l, k)| (lamport, id).cmp(&(*l, *k)))
            .unwrap_or_else(|e| e);
        kids.insert(pos, (lamport, id));
    }

    fn delete(&mut self, id: OpId) {
        self.tombstones.insert(id, ());
    }

    /// Depth-first walk producing the live visual order.
    fn live_order(&self) -> Vec<OpId> {
        let mut out = Vec::new();
        self.walk(None, &mut out);
        out
    }

    fn walk(&self, key: Option<OpId>, out: &mut Vec<OpId>) {
        if let Some(kids) = self.children.get(&key) {
            for (_, id) in kids {
                if !self.tombstones.contains_key(id) {
                    out.push(*id);
                }
                self.walk(Some(*id), out);
            }
        }
    }

    fn is_live(&self, id: &OpId) -> bool {
        !self.tombstones.contains_key(id)
    }
}

/// The workbook state (single sheet in v0.1).
///
/// Cells live in the tile store (docs/14, ADR-005), not in a flat per-cell map:
/// CRDT metadata is a per-tile causal summary until concurrency forces
/// promotion, which is what makes a 10M-cell workbook fit its memory budget.
#[derive(Default, Clone)]
pub struct State {
    rows: AxisSeq,
    cols: AxisSeq,
    cells: TileStore,
}

impl State {
    /// Fold a log into state. The ONLY public constructor from data.
    ///
    /// Two passes over the canonical order. The first assigns identity slots
    /// and decides which tiles must start promoted; the second applies. The
    /// split exists because promotion has to be decided *before* the first
    /// write lands: a summary tile records no per-cell stamps, so it could not
    /// reconstruct them if it were asked to promote later (see `tile::Meta`).
    pub fn replay(log: &OpLog) -> Self {
        let mut ops: Vec<&Op> = log.ops().iter().collect();
        // Total order: applying in this order is equivalent to any causal
        // order because every apply_* below is commutative for concurrent ops.
        ops.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
        let mut s = State {
            cells: TileStore::from_plan(tile::plan_promotions(ops.iter().copied())),
            ..Default::default()
        };
        for op in ops {
            s.apply(op);
        }
        s
    }

    /// Replays ops that are **already** in canonical total order, without ever
    /// materializing an `OpLog`.
    ///
    /// `source` is called twice — once for the promotion pre-pass, once to
    /// apply — and must yield the identical sequence both times, ordered by
    /// `(lamport, actor, counter)`. The caller owns that precondition, which is
    /// exactly why `replay` (which sorts, and cannot be misused) stays the
    /// default constructor.
    ///
    /// This exists because some inputs are larger than the state they build: a
    /// 10M-cell import is ~1.2 GB of ops producing ~80 MB of tiles. Row 11
    /// (snapshot + op-tail recovery) and Row 12 (import) need the same shape,
    /// and the A-001 memory harness cannot be run at all without it.
    pub fn replay_sorted<F, I>(mut source: F) -> Self
    where
        F: FnMut() -> I,
        I: Iterator<Item = Op>,
    {
        let mut s = State {
            cells: TileStore::from_plan(tile::plan_promotions(source())),
            ..Default::default()
        };
        for op in source() {
            s.apply(&op);
        }
        s
    }

    fn apply(&mut self, op: &Op) {
        match &op.payload {
            Payload::InsertRow { anchor } => self.rows.insert(anchor, op.id, op.lamport),
            Payload::DeleteRow { row } => self.rows.delete(row.0),
            Payload::InsertCol { anchor } => self.cols.insert(anchor, op.id, op.lamport),
            Payload::DeleteCol { col } => self.cols.delete(col.0),
            Payload::SetCell { row, col, value } => {
                self.cells
                    .write(row.0, col.0, op.lamport, op.id, value.clone())
            }
            // A clear is a write of Blank, not an erasure: the cell keeps its
            // identity and its place in the causal history (DP-A1).
            Payload::ClearCell { row, col } => {
                self.cells
                    .write(row.0, col.0, op.lamport, op.id, Value::Blank)
            }
        }
    }

    /// Live row identities in display order.
    pub fn row_order(&self) -> Vec<RowId> {
        self.rows.live_order().into_iter().map(RowId).collect()
    }

    /// Live column identities in display order.
    pub fn col_order(&self) -> Vec<ColId> {
        self.cols.live_order().into_iter().map(ColId).collect()
    }

    /// Current winning value of a cell, if any (Blank clears count as values).
    ///
    /// Returns an owned `Value` rather than a reference: a packed numeric tile
    /// stores bare `f64`s, so there is no `Value` in memory to borrow.
    pub fn cell(&self, row: RowId, col: ColId) -> Option<Value> {
        self.cells.get(&row.0, &col.0)
    }

    /// Retained concurrent losers for conflict surfacing (ADR-006).
    pub fn conflicts(&self, row: RowId, col: ColId) -> &[(Lamport, OpId, Value)] {
        self.cells.losers(&row.0, &col.0)
    }

    /// Deterministic state hash: the determinism-gate primitive (docs/10).
    /// Hashes live axis order + winning cell values of live cells.
    ///
    /// Cells fold in tile-major order — the tile-Merkle direction docs/10
    /// specifies — which is deterministic because slot assignment is a pure
    /// function of the op set. Identities, never slots, go into the hash.
    pub fn state_hash(&self) -> blake3::Hash {
        let mut h = blake3::Hasher::new();
        let mut buf = Vec::new();
        for r in self.rows.live_order() {
            h.update(&r.actor.0.to_be_bytes());
            h.update(&r.counter.to_be_bytes());
        }
        h.update(b"|cols|");
        for c in self.cols.live_order() {
            h.update(&c.actor.0.to_be_bytes());
            h.update(&c.counter.to_be_bytes());
        }
        h.update(b"|cells|");
        let (rows, cols) = (&self.rows, &self.cols);
        self.cells.for_each(|r, c, value| {
            if rows.is_live(&r) && cols.is_live(&c) {
                h.update(&r.actor.0.to_be_bytes());
                h.update(&r.counter.to_be_bytes());
                h.update(&c.actor.0.to_be_bytes());
                h.update(&c.counter.to_be_bytes());
                buf.clear();
                value.encode_into(&mut buf);
                h.update(&buf);
            }
        });
        h.finalize()
    }

    /// The causal summary `(max lamport, sole writer)` of the tile holding this
    /// cell, or `None` if that tile is promoted (docs/14 §CRDT metadata).
    pub fn cell_summary(&self, row: RowId, col: ColId) -> Option<(Lamport, usk_types::ActorId)> {
        self.cells.causal_summary(&row.0, &col.0)
    }

    /// Promotion accounting for assumption A-002 (docs/42) — the number that
    /// decides whether ADR-005's tile granularity holds up.
    pub fn promotion_stats(&self) -> PromotionStats {
        self.cells.promotion_stats()
    }

    /// Structural heap bytes held by the cell store, for the A-001 harness.
    pub fn cell_heap_bytes(&self) -> usize {
        self.cells.heap_bytes()
    }

    /// Number of live tiles backing the cell store.
    pub fn tile_count(&self) -> usize {
        self.cells.tile_count()
    }
}
