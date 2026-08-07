//! usk-state — CRDT state: axis order + cell registers, fold over the op log.
//!
//! State is DERIVED: `State::replay(log)` is the only constructor from data,
//! and all mutators are private to the applier (docs/03 invariant I3).
//! Convergence contract: same op set (any order) ⇒ same state ⇒ same hash.

#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_types::{ColId, OpId, RowId, Value};

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

/// Cell register: last-writer-wins by (lamport, actor, counter), with the
/// losing concurrent write RETAINED for conflict surfacing (ADR-006).
#[derive(Clone, Debug, PartialEq)]
pub struct CellReg {
    pub winner: (u64, OpId, Value),
    pub losers: Vec<(u64, OpId, Value)>,
}

/// The workbook state (single sheet in v0.1).
#[derive(Default, Clone)]
pub struct State {
    rows: AxisSeq,
    cols: AxisSeq,
    cells: BTreeMap<(OpId, OpId), CellReg>,
}

impl State {
    /// Fold a log into state. The ONLY public constructor from data.
    pub fn replay(log: &OpLog) -> Self {
        let mut ops: Vec<&Op> = log.ops().iter().collect();
        // Total order: applying in this order is equivalent to any causal
        // order because every apply_* below is commutative for concurrent ops.
        ops.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
        let mut s = State::default();
        for op in ops {
            s.apply(op);
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
                let key = (row.0, col.0);
                let cand = (op.lamport, op.id, value.clone());
                match self.cells.get_mut(&key) {
                    None => {
                        self.cells.insert(
                            key,
                            CellReg {
                                winner: cand,
                                losers: Vec::new(),
                            },
                        );
                    }
                    Some(reg) => {
                        if (cand.0, cand.1) > (reg.winner.0, reg.winner.1) {
                            let old = core::mem::replace(&mut reg.winner, cand);
                            reg.losers.push(old);
                        } else {
                            reg.losers.push(cand);
                        }
                        reg.losers.sort_by_key(|(l, id, _)| (*l, *id));
                    }
                }
            }
            Payload::ClearCell { row, col } => {
                let key = (row.0, col.0);
                let cand = (op.lamport, op.id, Value::Blank);
                match self.cells.get_mut(&key) {
                    None => {
                        self.cells.insert(
                            key,
                            CellReg {
                                winner: cand,
                                losers: Vec::new(),
                            },
                        );
                    }
                    Some(reg) => {
                        if (cand.0, cand.1) > (reg.winner.0, reg.winner.1) {
                            let old = core::mem::replace(&mut reg.winner, cand);
                            reg.losers.push(old);
                        } else {
                            reg.losers.push(cand);
                        }
                        reg.losers.sort_by_key(|(l, id, _)| (*l, *id));
                    }
                }
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
    pub fn cell(&self, row: RowId, col: ColId) -> Option<&Value> {
        self.cells.get(&(row.0, col.0)).map(|r| &r.winner.2)
    }

    /// Retained concurrent losers for conflict surfacing (ADR-006).
    pub fn conflicts(&self, row: RowId, col: ColId) -> &[(u64, OpId, Value)] {
        self.cells
            .get(&(row.0, col.0))
            .map(|r| r.losers.as_slice())
            .unwrap_or(&[])
    }

    /// Deterministic state hash: the determinism-gate primitive (docs/10).
    /// Hashes live axis order + winning cell values of live cells.
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
        for ((r, c), reg) in &self.cells {
            if self.rows.is_live(r) && self.cols.is_live(c) {
                h.update(&r.actor.0.to_be_bytes());
                h.update(&r.counter.to_be_bytes());
                h.update(&c.actor.0.to_be_bytes());
                h.update(&c.counter.to_be_bytes());
                buf.clear();
                reg.winner.2.encode_into(&mut buf);
                h.update(&buf);
            }
        }
        h.finalize()
    }
}
