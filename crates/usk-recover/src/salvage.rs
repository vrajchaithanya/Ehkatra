//! The SALVAGE path (docs/16, docs/27 §2).
//!
//! > *Corrupted container: salvage path = last valid snapshot + readable tail +
//! > quarantined remainder, with an honest user report — **silent partial
//! > restore is forbidden**.*
//!
//! That last clause is the design constraint. Every function here returns what
//! it recovered *and* what it could not, and the lifecycle machine will not
//! leave SALVAGE without the user acknowledging the report. A recovery path
//! that quietly returns less than the user had is worse than one that fails
//! loudly, because the user goes on trusting it.

use alloc::vec::Vec;
use usk_oplog::{DecodeError, Op};

use crate::snapshot::{Snapshot, SnapshotFault, VerifiedSnapshot, Watermark};

/// Why salvage ran at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SalvageReason {
    /// The newest snapshot failed verification; an older one was used.
    SnapshotFaulty(SnapshotFault),
    /// Snapshots existed and **none** of them verified — recovery fell all the
    /// way back to the op log.
    ///
    /// Note what this is *not*: a container that has simply never been
    /// snapshotted. A young workbook opens from its ops alone and that is an
    /// ordinary clean open, not a salvage. Conflating the two was a real defect
    /// — every test in the logic half supplied a snapshot, so the distinction
    /// only appeared when a real container was opened after its first save.
    NoValidSnapshot,
    /// Snapshots were fine; the op tail was truncated or corrupt.
    TailCorrupt { at_offset: usize, err: DecodeError },
}

/// What the user is told. Every field exists so the report can be specific:
/// "we recovered X, we lost Y, here is where it stopped".
#[derive(Clone, PartialEq, Debug)]
pub struct SalvageReport {
    pub reasons: Vec<SalvageReason>,
    /// Watermark of the snapshot actually used, if any.
    pub snapshot_used: Option<Watermark>,
    /// How many snapshots were rejected before one verified.
    pub snapshots_rejected: usize,
    /// Ops recovered from the tail, on top of the snapshot.
    pub tail_ops_recovered: usize,
    /// Bytes of tail that could not be read and are held, not deleted.
    pub quarantined_bytes: usize,
}

impl SalvageReport {
    /// True when nothing was lost — the caller can go straight to READY
    /// without an acknowledgement, because there is nothing to acknowledge.
    pub fn is_clean(&self) -> bool {
        self.reasons.is_empty() && self.quarantined_bytes == 0
    }

    /// Whether any user data is known to be unrecoverable. Distinct from
    /// `is_clean`: an older-but-valid snapshot with a fully readable tail loses
    /// nothing, yet is still not a clean open.
    pub fn lost_data(&self) -> bool {
        self.quarantined_bytes > 0
            || self
                .reasons
                .iter()
                .any(|r| matches!(r, SalvageReason::NoValidSnapshot))
    }
}

/// Everything recovery produced: the ops to rebuild from, the bytes that could
/// not be read, and the report that must be shown before the document opens.
#[derive(Clone, Debug)]
pub struct Salvaged {
    pub snapshot: Option<VerifiedSnapshot>,
    pub tail: Vec<Op>,
    /// Held verbatim, never deleted — docs/16's "quarantined remainder". A user
    /// who takes the file to support must still have the bytes.
    pub quarantine: Vec<u8>,
    pub report: SalvageReport,
}

impl Salvaged {
    /// Every op the recovery believes in, snapshot first then tail, ready to
    /// fold into a `State`.
    pub fn ops(&self) -> Vec<Op> {
        let mut out = match &self.snapshot {
            Some(s) => s.ops().to_vec(),
            None => Vec::new(),
        };
        out.extend(self.tail.iter().cloned());
        out
    }
}

/// Reads the op tail, stopping at the first byte it cannot decode.
///
/// A torn final write is the *expected* case after a crash, not an exotic one:
/// the process died between appending bytes and committing them. So a
/// truncated last op is recovered as "everything before it, plus a quarantine",
/// never as a failure to open.
pub fn read_tail(bytes: &[u8]) -> (Vec<Op>, usize, Option<SalvageReason>) {
    let mut ops = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        match Op::decode(&bytes[at..]) {
            Ok((op, used)) => {
                at += used;
                ops.push(op);
            }
            Err(err) => {
                return (
                    ops,
                    at,
                    Some(SalvageReason::TailCorrupt { at_offset: at, err }),
                )
            }
        }
    }
    (ops, at, None)
}

/// The recovery decision, over a snapshot chain (newest first) and an op tail.
///
/// Snapshots are tried newest to oldest and the **first one that proves itself**
/// wins — docs/16's "last valid snapshot". Rejected ones are counted and their
/// faults named, because "your newest save was corrupt, we opened the one
/// before it" is information the user needs and a silent fallback withholds.
pub fn recover(snapshots: &[Snapshot], tail_bytes: &[u8]) -> Salvaged {
    let mut reasons = Vec::new();
    let mut snapshots_rejected = 0usize;
    let mut chosen: Option<VerifiedSnapshot> = None;

    for snapshot in snapshots {
        match snapshot.verify() {
            Ok(verified) => {
                chosen = Some(verified);
                break;
            }
            Err(fault) => {
                snapshots_rejected += 1;
                reasons.push(SalvageReason::SnapshotFaulty(fault));
            }
        }
    }
    if chosen.is_none() && !snapshots.is_empty() {
        // Every snapshot present failed. Not fatal — a workbook can still be
        // rebuilt from its op tail, which is the point of ops-as-truth — but
        // the user must be told the saves they thought they had are gone.
        //
        // Guarded on `!snapshots.is_empty()` because a container that has never
        // been snapshotted is not in trouble; it is new.
        reasons.push(SalvageReason::NoValidSnapshot);
    }

    let (tail, consumed, tail_fault) = read_tail(tail_bytes);
    if let Some(fault) = tail_fault {
        reasons.push(fault);
    }
    let quarantine = tail_bytes[consumed..].to_vec();

    let report = SalvageReport {
        snapshot_used: chosen.as_ref().map(|s| s.watermark().clone()),
        snapshots_rejected,
        tail_ops_recovered: tail.len(),
        quarantined_bytes: quarantine.len(),
        reasons,
    };

    Salvaged {
        snapshot: chosen,
        tail,
        quarantine,
        report,
    }
}
