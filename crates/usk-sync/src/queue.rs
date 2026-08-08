//! The never-drop outbound queue (docs/15 §Offline, docs/27 §1).
//!
//! > *local unsynced ops are **never silently dropped***
//!
//! That is a published contract, so it is enforced structurally rather than by
//! care: the only method that can remove an op is [`Queue::ack`], and it removes
//! exactly the ops a peer's watermark says it has durably taken. Every other
//! event — a send, a transport loss, an auth revocation, a rebase — can at most
//! move an op back to *unsent*. There is deliberately no `clear`, no `drain`,
//! and no `retain`.
//!
//! `queued_ops_survive_every_transition` in the machine's test suite drives the
//! whole event alphabet against this type and asserts the invariant directly.

use alloc::vec::Vec;
use usk_oplog::Op;

use crate::clock::VectorClock;

/// A local op awaiting acknowledgement.
#[derive(Clone)]
struct Entry {
    op: Op,
    /// Sent at least once. Not "delivered" — only an ack proves delivery.
    sent: bool,
}

/// Local ops that have not yet been acknowledged by the relay.
#[derive(Default, Clone)]
pub struct Queue {
    entries: Vec<Entry>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a locally authored op. Called before any transport exists, which
    /// is what makes the editor work offline (docs/15).
    pub fn enqueue(&mut self, op: Op) {
        if self.entries.iter().any(|e| e.op.id == op.id) {
            return;
        }
        self.entries.push(Entry { op, sent: false });
    }

    /// Ops not yet put on the wire, marked sent. Returns them in authoring
    /// order, which keeps each actor's run causally contiguous (docs/15).
    pub fn take_unsent(&mut self) -> Vec<Op> {
        let mut out = Vec::new();
        for entry in self.entries.iter_mut().filter(|e| !e.sent) {
            entry.sent = true;
            out.push(entry.op.clone());
        }
        out
    }

    /// Everything still unacknowledged, whether or not it has been sent —
    /// what a fresh session must re-offer after a reconnect.
    pub fn all(&self) -> Vec<Op> {
        self.entries.iter().map(|e| e.op.clone()).collect()
    }

    /// Marks every entry unsent, so a reconnect re-offers them.
    ///
    /// This is the transport-loss path: an op that was on the wire when the
    /// socket died may or may not have arrived, and the safe assumption is that
    /// it did not. Redelivery is free — merge is idempotent (DP-A8).
    pub fn mark_all_unsent(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.sent = false;
        }
    }

    /// Removes the ops a peer's watermark proves it holds. **The only removal
    /// path in this type.**
    pub fn ack(&mut self, watermark: &VectorClock) -> usize {
        let before = self.entries.len();
        let mut kept = Vec::with_capacity(before);
        for entry in self.entries.drain(..) {
            if !watermark.covers(entry.op.id) {
                kept.push(entry);
            }
        }
        self.entries = kept;
        before - self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when every queued op has been put on the wire at least once.
    pub fn all_sent(&self) -> bool {
        self.entries.iter().all(|e| e.sent)
    }

    pub fn contains(&self, op: &Op) -> bool {
        self.entries.iter().any(|e| e.op.id == op.id)
    }
}
