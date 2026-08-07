//! usk-oplog — the op algebra, canonical encoding, and hashing.
//!
//! Ops are the ONLY mutation path (docs/03). This crate defines what an op
//! is, its single valid byte encoding, and the BLAKE3 hashing used for the
//! determinism gate (identical logs ⇒ identical hash on every platform).

#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use usk_types::{ColId, Lamport, OpId, RowId, Value};

/// Where a new row/col is placed relative to existing identities.
/// v0.1 uses neighbor-anchored insertion (Fugue-style origin); the order
/// CRDT in usk-state resolves concurrent inserts deterministically.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// Insert at the very start of the axis.
    Start,
    /// Insert immediately after the row/col created by this op id.
    After(OpId),
}

/// The closed op payload taxonomy for model version 1 (docs/10).
/// Adding behavior later = adding a variant, never changing one.
#[derive(Clone, PartialEq, Debug)]
pub enum Payload {
    InsertRow {
        anchor: Anchor,
    },
    DeleteRow {
        row: RowId,
    },
    InsertCol {
        anchor: Anchor,
    },
    DeleteCol {
        col: ColId,
    },
    SetCell {
        row: RowId,
        col: ColId,
        value: Value,
    },
    ClearCell {
        row: RowId,
        col: ColId,
    },
}

/// One immutable operation.
#[derive(Clone, PartialEq, Debug)]
pub struct Op {
    pub id: OpId,
    pub lamport: Lamport,
    pub payload: Payload,
}

fn encode_opid(id: &OpId, out: &mut Vec<u8>) {
    out.extend_from_slice(&id.actor.0.to_be_bytes());
    out.extend_from_slice(&id.counter.to_be_bytes());
}

fn encode_anchor(a: &Anchor, out: &mut Vec<u8>) {
    match a {
        Anchor::Start => out.push(0x00),
        Anchor::After(id) => {
            out.push(0x01);
            encode_opid(id, out);
        }
    }
}

impl Op {
    /// The single canonical encoding of this op (docs/10: one valid
    /// encoding per op, so hashing is well-defined). Deterministic by
    /// construction: fixed field order, big-endian integers, no maps.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        encode_opid(&self.id, &mut out);
        out.extend_from_slice(&self.lamport.to_be_bytes());
        match &self.payload {
            Payload::InsertRow { anchor } => {
                out.push(0x10);
                encode_anchor(anchor, &mut out);
            }
            Payload::DeleteRow { row } => {
                out.push(0x11);
                encode_opid(&row.0, &mut out);
            }
            Payload::InsertCol { anchor } => {
                out.push(0x12);
                encode_anchor(anchor, &mut out);
            }
            Payload::DeleteCol { col } => {
                out.push(0x13);
                encode_opid(&col.0, &mut out);
            }
            Payload::SetCell { row, col, value } => {
                out.push(0x14);
                encode_opid(&row.0, &mut out);
                encode_opid(&col.0, &mut out);
                value.encode_into(&mut out);
            }
            Payload::ClearCell { row, col } => {
                out.push(0x15);
                encode_opid(&row.0, &mut out);
                encode_opid(&col.0, &mut out);
            }
        }
        out
    }

    /// Content hash of a single op.
    pub fn hash(&self) -> blake3::Hash {
        blake3::hash(&self.encode())
    }
}

/// A causally-ordered set of ops. v0.1 keeps the log as a simple grow-only
/// vec per replica; `merged_hash` defines the canonical hash over a set of
/// ops *independent of arrival order* — the determinism-gate primitive.
#[derive(Default, Clone)]
pub struct OpLog {
    ops: Vec<Op>,
}

impl OpLog {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn append(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn merge_from(&mut self, other: &OpLog) {
        for op in &other.ops {
            if !self.ops.iter().any(|o| o.id == op.id) {
                self.ops.push(op.clone());
            }
        }
    }

    /// Canonical hash over the op *set*: ops are sorted by the total order
    /// (lamport, actor, counter) and chain-hashed. Two replicas holding the
    /// same set in any arrival order produce the same hash.
    pub fn canonical_hash(&self) -> blake3::Hash {
        let mut sorted: Vec<&Op> = self.ops.iter().collect();
        sorted.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
        let mut hasher = blake3::Hasher::new();
        for op in sorted {
            hasher.update(op.hash().as_bytes());
        }
        hasher.finalize()
    }
}
