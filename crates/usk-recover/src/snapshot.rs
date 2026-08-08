//! Snapshots: content-addressed, and **verified semantically on every load**
//! (docs/16, docs/26).
//!
//! # What a v0.1 snapshot is
//! The compacted op set, in canonical total order, concatenated in the same
//! canonical encoding the wire and the container use — plus the state hash that
//! op set must produce. Nothing new is invented: docs/26 says "the payload
//! column stores the *identical bytes* that were hashed — the file IS the wire
//! format at rest", and this takes that literally for the snapshot body too.
//!
//! A tile image (docs/16's "structurally shared via tile Merkle identity") is
//! the Row-12+ body format. Swapping it in changes [`Snapshot::body`] and
//! nothing else, because every caller here goes through `verify`.
//!
//! # Why verification is a replay, not a checksum
//! docs/26 requires `state_hash` to be checked on load, and docs/16 forbids
//! silent partial restore. A checksum over the bytes only proves the bytes
//! survived; replaying them and comparing `State::state_hash()` proves the
//! bytes still *mean* what they meant. That is the stronger check, and DP-A2
//! makes it free — the same op set produces the same hash on every platform
//! forever. It is also what makes docs/26's migration rule executable: a
//! migration that changes the state hash is by definition wrong.

use alloc::vec::Vec;
use usk_oplog::{DecodeError, Op, OpLog};
use usk_state::State;
use usk_types::{ActorId, Counter};

/// A vector-clock watermark: sorted `(actor, max counter)` pairs, canonical
/// encoding (docs/26 §Identity encodings).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Watermark {
    entries: Vec<(ActorId, Counter)>,
}

impl Watermark {
    /// The watermark covering exactly this op set.
    pub fn of(ops: &[Op]) -> Watermark {
        let mut entries: Vec<(ActorId, Counter)> = Vec::new();
        for op in ops {
            match entries.iter_mut().find(|(a, _)| *a == op.id.actor) {
                Some(slot) => slot.1 = slot.1.max(op.id.counter),
                None => entries.push((op.id.actor, op.id.counter)),
            }
        }
        // Sorted, because "canonical encoding" means one byte string per value.
        entries.sort_unstable();
        Watermark { entries }
    }

    /// Rebuilds a watermark from `(actor, max counter)` pairs — the inverse of
    /// [`Watermark::encode`], for a container reading one back off disk.
    /// Canonicalises (sorts, keeps the greater counter per actor) so a stored
    /// watermark cannot smuggle a second spelling of one value past DP-A4.
    pub fn from_pairs<I: IntoIterator<Item = (ActorId, Counter)>>(pairs: I) -> Watermark {
        let mut entries: Vec<(ActorId, Counter)> = Vec::new();
        for (actor, counter) in pairs {
            match entries.iter_mut().find(|(a, _)| *a == actor) {
                Some(slot) => slot.1 = slot.1.max(counter),
                None => entries.push((actor, counter)),
            }
        }
        entries.sort_unstable();
        Watermark { entries }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.entries.len() * 24);
        for (actor, counter) in &self.entries {
            out.extend_from_slice(&actor.0.to_be_bytes());
            out.extend_from_slice(&counter.to_be_bytes());
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[(ActorId, Counter)] {
        &self.entries
    }
}

/// A stored snapshot, exactly the columns docs/26 gives it.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    /// `snapshots.watermark` — the op set this image covers.
    pub watermark: Watermark,
    /// `snapshots.state_hash` — BLAKE3, verified on load.
    pub state_hash: [u8; 32],
    /// `snapshots.body` — canonical op bytes (zstd is a container concern).
    pub body: Vec<u8>,
}

/// Why a snapshot could not be trusted. Named, because docs/16 forbids a
/// silent partial restore and a user cannot be told "it broke".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapshotFault {
    /// The body does not decode as a run of canonical ops.
    Undecodable { at_offset: usize, err: DecodeError },
    /// It decodes, but replaying it does not produce the recorded state hash —
    /// the bytes survived and their meaning did not.
    StateHashMismatch,
    /// It decodes and hashes correctly but covers a different op set than its
    /// watermark claims.
    WatermarkMismatch,
}

impl Snapshot {
    /// Builds a snapshot from a log. The state hash is computed, never copied
    /// from a caller — a snapshot that could be told what it hashes to would
    /// verify against nothing.
    pub fn build(log: &OpLog) -> Snapshot {
        let mut ops: Vec<&Op> = log.ops().iter().collect();
        ops.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
        let mut body = Vec::new();
        for op in &ops {
            body.extend_from_slice(&op.encode());
        }
        let owned: Vec<Op> = ops.into_iter().cloned().collect();
        Snapshot {
            watermark: Watermark::of(&owned),
            state_hash: *State::replay(log).state_hash().as_bytes(),
            body,
        }
    }

    /// The content address of the body — what "content-addressed" means in
    /// docs/16, and what a container keys structural sharing on.
    pub fn content_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.body).as_bytes()
    }

    /// Decodes the body back into ops. Total: a malformed body names the byte
    /// offset it failed at, so SALVAGE can quarantine the remainder rather than
    /// discard the whole file (DP-A10).
    pub fn decode_body(&self) -> Result<Vec<Op>, SnapshotFault> {
        let mut ops = Vec::new();
        let mut at = 0usize;
        while at < self.body.len() {
            match Op::decode(&self.body[at..]) {
                Ok((op, used)) => {
                    at += used;
                    ops.push(op);
                }
                Err(err) => return Err(SnapshotFault::Undecodable { at_offset: at, err }),
            }
        }
        Ok(ops)
    }

    /// Loads the snapshot **only if it proves itself**: the body decodes, its
    /// replay reproduces the recorded state hash, and it covers the op set its
    /// watermark claims.
    ///
    /// The returned [`VerifiedSnapshot`] is the *only* way to reach READY in
    /// the lifecycle machine, so docs/27's forbidden "opening READY without
    /// hash-verifying the loaded snapshot" is structurally unreachable rather
    /// than checked at runtime — the same technique the undo machine's
    /// forbidden transitions use (D-060).
    pub fn verify(&self) -> Result<VerifiedSnapshot, SnapshotFault> {
        let ops = self.decode_body()?;
        let mut log = OpLog::new();
        for op in &ops {
            log.append(op.clone());
        }
        let state = State::replay(&log);
        if state.state_hash().as_bytes() != &self.state_hash {
            return Err(SnapshotFault::StateHashMismatch);
        }
        if Watermark::of(&ops) != self.watermark {
            return Err(SnapshotFault::WatermarkMismatch);
        }
        Ok(VerifiedSnapshot {
            watermark: self.watermark.clone(),
            ops,
        })
    }
}

/// Proof that a snapshot verified. Cannot be constructed except by
/// [`Snapshot::verify`], which is the point.
#[derive(Clone, Debug)]
pub struct VerifiedSnapshot {
    watermark: Watermark,
    ops: Vec<Op>,
}

impl VerifiedSnapshot {
    pub fn watermark(&self) -> &Watermark {
        &self.watermark
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// The state this snapshot restores, before any tail is replayed onto it.
    pub fn into_state(self) -> State {
        let mut log = OpLog::new();
        for op in self.ops {
            log.append(op);
        }
        State::replay(&log)
    }
}
