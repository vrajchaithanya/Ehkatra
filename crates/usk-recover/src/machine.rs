//! The document lifecycle machine — **docs/27 §2, transcribed**.
//!
//! ```text
//! CLOSED ──open──► RECOVERING ──snapshot ok + tail replay──► READY
//! RECOVERING ──snapshot hash mismatch──► SALVAGE (last valid snapshot + readable tail
//!              + quarantined remainder + honest user report) ──user ack──► READY
//! READY ──ops──► READY (WAL append ≤250 ms batched fsync)
//! READY ──compaction trigger──► COMPACTING (new file, atomic rename) ──► READY
//! READY ──close──► CLOSED (final fsync; no other work permitted)
//! ```
//!
//! Forbidden, each proven rejected in `tests/recovery.rs`:
//! * writing to the old file during COMPACTING;
//! * opening READY without hash-verifying the loaded snapshot;
//! * any transition that loses acked ops (the RPO contract, docs/16).
//!
//! Same shape as the sync machine: a pure transition function, no I/O, no
//! clock. Durability is something the shell performs when it sees
//! [`Action::Fsync`]; this machine only decides *that* it must happen and
//! refuses to move on as if it had.

use alloc::vec;
use alloc::vec::Vec;

use crate::salvage::SalvageReport;
use crate::snapshot::VerifiedSnapshot;

/// docs/16's desktop RPO: ops are durable within 250 ms of typing.
pub const WAL_BATCH_MS: u64 = 250;

/// States, exactly as docs/27 §2 names them.
#[derive(Clone, PartialEq, Debug)]
pub enum DocState {
    Closed,
    Recovering,
    /// Holding the honest report until the user acknowledges it. docs/16:
    /// "silent partial restore is forbidden", so this state cannot be skipped.
    Salvage(SalvageReport),
    Ready,
    Compacting,
}

/// Inbound events.
pub enum Event {
    Open,
    /// Recovery finished cleanly: the snapshot verified and the tail replayed.
    /// Carrying a [`VerifiedSnapshot`] is what makes "opening READY without
    /// hash-verifying" unrepresentable — there is no other way to build one.
    Recovered {
        snapshot: Option<VerifiedSnapshot>,
        tail_ops: usize,
    },
    /// Recovery finished, but something was wrong and the user must be told.
    Salvaged {
        report: SalvageReport,
        snapshot: Option<VerifiedSnapshot>,
        tail_ops: usize,
    },
    UserAck,
    /// Acked ops arriving from the editor.
    Ops(usize),
    CompactionTrigger,
    /// The new file is written and atomically renamed into place.
    CompactionComplete,
    Close,
}

/// What the shell must do.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    BeginRecovery,
    /// Append to the WAL; the shell batches the fsync within `WAL_BATCH_MS`.
    AppendWal {
        ops: usize,
    },
    /// Show the salvage report. The document does not open until acknowledged.
    ReportSalvage(SalvageReport),
    /// Write the compacted image to a **new** file (docs/26: never in place).
    WriteCompactedFile,
    /// Atomically rename it over the old one.
    AtomicRename,
    /// Ops that arrived during COMPACTING, replayed onto the new file once the
    /// rename lands. They were never written to the old file, and they were
    /// never dropped either.
    FlushDeferred {
        ops: usize,
    },
    Fsync,
    LogUnexpected,
}

/// One open document.
pub struct Document {
    state: DocState,
    /// Acked ops the document is responsible for. **Monotonic** — the RPO
    /// contract in one field. No transition may lower it.
    acked_ops: usize,
    /// Ops that arrived while COMPACTING and are held, not written to the old
    /// file. This is how "no writing to the old file during COMPACTING" and
    /// "no transition loses acked ops" are satisfied at the same time; either
    /// one alone is easy, and the pair is the actual requirement.
    deferred_ops: usize,
    restored_from: Option<Vec<u8>>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Document {
        Document {
            state: DocState::Closed,
            acked_ops: 0,
            deferred_ops: 0,
            restored_from: None,
        }
    }

    pub fn state(&self) -> &DocState {
        &self.state
    }

    /// Every op this document has acknowledged. Never decreases.
    pub fn acked_ops(&self) -> usize {
        self.acked_ops
    }

    pub fn deferred_ops(&self) -> usize {
        self.deferred_ops
    }

    /// The watermark of the snapshot the current session was restored from,
    /// encoded — `None` when the document was rebuilt from ops alone.
    pub fn restored_from(&self) -> Option<&[u8]> {
        self.restored_from.as_deref()
    }

    /// True only in states where the old container file may be written.
    /// COMPACTING is deliberately absent (docs/27 forbidden line 1).
    pub fn may_write_container(&self) -> bool {
        matches!(self.state, DocState::Ready)
    }

    pub fn step(&mut self, event: Event) -> Vec<Action> {
        match (&self.state, event) {
            // CLOSED ──open──► RECOVERING
            (DocState::Closed, Event::Open) => {
                self.state = DocState::Recovering;
                vec![Action::BeginRecovery]
            }

            // RECOVERING ──snapshot ok + tail replay──► READY
            (DocState::Recovering, Event::Recovered { snapshot, tail_ops }) => {
                self.adopt(snapshot, tail_ops);
                self.state = DocState::Ready;
                Vec::new()
            }

            // RECOVERING ──snapshot hash mismatch──► SALVAGE
            (
                DocState::Recovering,
                Event::Salvaged {
                    report,
                    snapshot,
                    tail_ops,
                },
            ) => {
                self.adopt(snapshot, tail_ops);
                self.state = DocState::Salvage(report.clone());
                vec![Action::ReportSalvage(report)]
            }

            // SALVAGE ──user ack──► READY
            (DocState::Salvage(_), Event::UserAck) => {
                self.state = DocState::Ready;
                Vec::new()
            }

            // READY ──ops──► READY (WAL append <=250 ms batched fsync)
            (DocState::Ready, Event::Ops(n)) => {
                self.acked_ops += n;
                vec![Action::AppendWal { ops: n }]
            }

            // Ops during COMPACTING: **not** written to the old file, and
            // **not** dropped. Held until the rename lands. Both forbidden
            // lines are live at once here, which is why this arm exists rather
            // than an error return.
            (DocState::Compacting, Event::Ops(n)) => {
                self.acked_ops += n;
                self.deferred_ops += n;
                Vec::new()
            }

            // READY ──compaction trigger──► COMPACTING (new file, atomic rename)
            (DocState::Ready, Event::CompactionTrigger) => {
                self.state = DocState::Compacting;
                vec![Action::WriteCompactedFile]
            }

            // COMPACTING ──► READY
            (DocState::Compacting, Event::CompactionComplete) => {
                self.state = DocState::Ready;
                let deferred = core::mem::take(&mut self.deferred_ops);
                let mut actions = vec![Action::AtomicRename];
                if deferred > 0 {
                    actions.push(Action::FlushDeferred { ops: deferred });
                }
                actions
            }

            // READY ──close──► CLOSED (final fsync; no other work permitted)
            (DocState::Ready, Event::Close) => {
                self.state = DocState::Closed;
                vec![Action::Fsync]
            }

            (_, _) => {
                debug_assert!(
                    false,
                    "unlisted document transition — docs/27 §2 does not define it"
                );
                vec![Action::LogUnexpected]
            }
        }
    }

    fn adopt(&mut self, snapshot: Option<VerifiedSnapshot>, tail_ops: usize) {
        // Only a `VerifiedSnapshot` can get here, and it can only exist if
        // `Snapshot::verify` replayed the body and matched the recorded state
        // hash. docs/27's "opening READY without hash-verifying the loaded
        // snapshot" is therefore not a check that could be forgotten — it is a
        // state the type system does not let the caller construct.
        let from_snapshot = snapshot.as_ref().map(|s| s.ops().len()).unwrap_or(0);
        self.restored_from = snapshot.map(|s| s.watermark().encode());
        self.acked_ops += from_snapshot + tail_ops;
    }
}
