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
use usk_oplog::{DecodeError, Op, OpLog};
use usk_state::State;

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
    /// The tail held **cell writes** canonically at or before the snapshot
    /// image, which an adopted image cannot place (ADR-036).
    ///
    /// Almost always duplicates the image already folded, in which case nothing
    /// is lost. It can also be a genuinely late remote op, which *is* a loss —
    /// so it is reported either way rather than judged silently, because
    /// docs/16 forbids a silent partial restore and this path cannot tell the
    /// two apart.
    ///
    /// Note what this is *not*: an op the image could not represent. Those are
    /// never covered and never pruned (ADR-036 Amendment 1), so they arrive as
    /// ordinary tail and apply normally.
    TailPredatesSnapshot { ops: usize },
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
#[derive(Clone)]
pub struct Salvaged {
    pub snapshot: Option<VerifiedSnapshot>,
    pub tail: Vec<Op>,
    /// Held verbatim, never deleted — docs/16's "quarantined remainder". A user
    /// who takes the file to support must still have the bytes.
    pub quarantine: Vec<u8>,
    pub report: SalvageReport,
}

impl Salvaged {
    /// The state this recovery restores.
    ///
    /// With a snapshot: **decode the image and apply the tail** (ADR-036),
    /// where v0.1 replayed the entire history. Without one: fold the tail
    /// alone, which is ops-as-truth doing its job.
    ///
    /// Cell writes the image already covers are dropped rather than applied.
    /// `State::apply_tail` refuses them by contract and is right to: the
    /// summary path trusts arrival order, so replaying one would overwrite a
    /// newer value with an older one. The drop is not silent — `recover` counts
    /// them into `TailPredatesSnapshot` first.
    pub fn into_state(self) -> State {
        let Some(snapshot) = self.snapshot else {
            let mut log = OpLog::new();
            for op in self.tail {
                log.append(op);
            }
            return State::replay(&log);
        };
        let stamps = snapshot.stamps().clone();
        let greatest = stamps.greatest();
        let mut state = snapshot.into_state();
        let fresh: Vec<Op> = self
            .tail
            .into_iter()
            .filter(|op| !predates(op, greatest))
            .collect();
        // Cannot fail: every op left is outside the only condition
        // `apply_tail` refuses on.
        let _ = state.apply_tail(&stamps, &fresh);
        state
    }
}

/// Whether an op is a cell write the image has already folded.
///
/// Only cell writes can predate an image in a way that matters — the refusal
/// exists because a summary tile trusts arrival order, and nothing else has
/// one. An axis op resolves independently of arrival order and an opaque op
/// applies to nothing, so both are ordinary tail however old they are.
fn predates(op: &Op, greatest: Option<(u64, usk_types::OpId)>) -> bool {
    let Some(greatest) = greatest else {
        return false;
    };
    let writes_cell = matches!(
        op.payload,
        usk_oplog::Payload::SetCell { .. }
            | usk_oplog::Payload::ClearCell { .. }
            | usk_oplog::Payload::SetFormula { .. }
    );
    writes_cell && (op.lamport, op.id) <= greatest
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
        // Framed (TD-25), matching the snapshot body and the wire. Before the
        // frame existed, an unknown tag ended the tail — a peer one version
        // ahead could truncate a recovery.
        match Op::decode_framed(&bytes[at..]) {
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

    // Counted before the report is built, so a tail the image already covers is
    // named rather than discovered later by a user wondering where an edit went.
    let greatest = chosen.as_ref().and_then(|s| s.stamps().greatest());
    let predating = tail.iter().filter(|op| predates(op, greatest)).count();
    if predating > 0 {
        reasons.push(SalvageReason::TailPredatesSnapshot { ops: predating });
    }

    let report = SalvageReport {
        snapshot_used: chosen.as_ref().map(|s| s.watermark().clone()),
        snapshots_rejected,
        tail_ops_recovered: tail.len() - predating,
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
