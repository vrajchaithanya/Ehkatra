//! Snapshots: content-addressed, and **verified semantically on every load**
//! (docs/16, docs/26).
//!
//! # What a snapshot body is
//! docs/16 and docs/26 both specify a **tile image**, and as of ADR-036 that is
//! what this writes. The body is two sections:
//!
//! 1. the **image** — a materialised `State` plus the winner-stamp sidecar a
//!    tail needs (`usk_state::image`);
//! 2. the **covered op ids** — exactly which ops the image folded.
//!
//! Section 2 is here rather than inside the image because it is not state: an
//! image is what the ops *produced*, and the container separately needs to know
//! *which* ops those were, to tell a stored op that is already folded from one
//! that is still tail. Keeping it in the body rather than in a new column means
//! no schema change and no `user_version` bump — and D-101 records what happened
//! the last time a stored encoding was asked to carry more than it had room for.
//!
//! Before ADR-036 the body was the compacted op set and verification replayed
//! it, which is why opening a 1M-cell workbook cost 7.86 s (TD-45) and three
//! retained snapshots cost three copies of the history (TD-31).
//!
//! # Why verification is still semantic, not a checksum
//! docs/26 requires `state_hash` to be checked on load, and docs/16 forbids
//! silent partial restore. A checksum over the bytes proves only that the bytes
//! survived; **decoding** them into a `State` and recomputing
//! `State::state_hash()` proves the bytes still *mean* what they meant. That
//! property is unchanged by ADR-036 — what changed is that proving it is now a
//! decode instead of a replay. DP-A2 makes it free: the same state produces the
//! same hash on every platform forever, so docs/26's migration rule stays
//! executable — a migration that changes the state hash is by definition
//! wrong.

use alloc::vec::Vec;
use usk_oplog::{DecodeError, Op, OpLog};
use usk_state::image::ImageError;
use usk_state::{image_represents, State, WinnerStamps};
use usk_types::{ActorId, Counter, OpId};

/// A vector-clock watermark: sorted `(actor, max counter)` pairs, canonical
/// encoding (docs/26 §Identity encodings).
///
/// # What it deliberately does not answer
/// "Does this snapshot contain op X?" — because `counter <= max` is exact only
/// while an actor's counters are dense, and a replica mid-sync legitimately
/// holds `{A:1, A:3}` with `A:2` still in the causal buffer. Callers that need
/// membership use the snapshot's own op ids. Recording the gaps would make the
/// watermark exact, and was tried and reverted with the tile-image wiring
/// (D-101): the stored encoding has no room for them, so a watermark read back
/// off disk would never match one built in memory.
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
    /// `snapshots.body` — the tile image plus the covered op ids (ADR-036).
    pub body: Vec<u8>,
}

/// Why a snapshot could not be trusted. Named, because docs/16 forbids a
/// silent partial restore and a user cannot be told "it broke".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapshotFault {
    /// The body is not a well-formed image-plus-ids body.
    Undecodable { at_offset: usize, err: DecodeError },
    /// The image section is not an image this build can load.
    ImageFault(ImageError),
    /// It decodes, but the state it produces does not have the recorded state
    /// hash — the bytes survived and their meaning did not.
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
        let state = State::replay(log);
        // The stamps come from the same log the state came from, folded a
        // second time (ADR-036). No new I/O: the log is already in hand, which
        // is the reason reconstruction was chosen over keeping stamps resident.
        let stamps = WinnerStamps::from_log(log);
        // **Coverage is what the image represents** (ADR-036 Amendment 1), not
        // what the snapshot was built from. Compaction prunes coverage, so an
        // op outside it is one compaction can never delete — which is how
        // DP-A5 survives an image body. `Payload::Opaque` produces no state, so
        // an image has nowhere to put it and it is never covered.
        let mut covered: Vec<OpId> = log
            .ops()
            .iter()
            .filter(|o| image_represents(&o.payload))
            .map(|o| o.id)
            .collect();
        covered.sort_unstable();

        let image = state.write_image_with(&stamps);
        let mut body = Vec::with_capacity(image.len() + covered.len() * 24 + 8);
        body.extend_from_slice(&(image.len() as u64).to_le_bytes());
        body.extend_from_slice(&image);
        body.extend_from_slice(&(covered.len() as u64).to_le_bytes());
        for id in &covered {
            body.extend_from_slice(&id.actor.0.to_le_bytes());
            body.extend_from_slice(&id.counter.to_le_bytes());
        }

        // The watermark is built from the *covered* set for the same reason —
        // a watermark claiming ops the image cannot restore would be a promise
        // the body cannot keep, and `verify` compares the two.
        let owned: Vec<Op> = log
            .ops()
            .iter()
            .filter(|o| image_represents(&o.payload))
            .cloned()
            .collect();
        Snapshot {
            watermark: Watermark::of(&owned),
            state_hash: *state.state_hash().as_bytes(),
            body,
        }
    }

    /// Splits the body into its image bytes and its covered op ids.
    ///
    /// Total: a malformed body names the byte offset it failed at, so SALVAGE
    /// can quarantine rather than discard (DP-A10).
    fn split_body(&self) -> Result<(&[u8], Vec<OpId>), SnapshotFault> {
        let bad = |at: usize| SnapshotFault::Undecodable {
            at_offset: at,
            err: DecodeError::Truncated,
        };
        let b = &self.body;
        let take_u64 = |at: usize| -> Result<u64, SnapshotFault> {
            let end = at.checked_add(8).ok_or_else(|| bad(at))?;
            let slice = b.get(at..end).ok_or_else(|| bad(at))?;
            let mut a = [0u8; 8];
            a.copy_from_slice(slice);
            Ok(u64::from_le_bytes(a))
        };

        let image_len = take_u64(0)? as usize;
        let image_end = 8usize.checked_add(image_len).ok_or_else(|| bad(0))?;
        let image = b.get(8..image_end).ok_or_else(|| bad(8))?;

        let count = take_u64(image_end)? as usize;
        let mut at = image_end + 8;
        // Bounded before allocating: a corrupted count is an arbitrary u64, and
        // reserving on it is the cheapest way to turn a bad file into an OOM
        // (docs/37). 24 bytes per id is the floor, so the byte length is the
        // bound.
        if count > b.len().saturating_sub(at) / 24 {
            return Err(bad(image_end));
        }
        let mut covered = Vec::with_capacity(count);
        for _ in 0..count {
            let end = at + 24;
            let slice = b.get(at..end).ok_or_else(|| bad(at))?;
            let mut actor = [0u8; 16];
            actor.copy_from_slice(&slice[..16]);
            let mut counter = [0u8; 8];
            counter.copy_from_slice(&slice[16..]);
            covered.push(OpId {
                actor: ActorId(u128::from_le_bytes(actor)),
                counter: u64::from_le_bytes(counter),
            });
            at = end;
        }
        if at != b.len() {
            return Err(bad(at));
        }
        Ok((image, covered))
    }

    /// The content address of the body — what "content-addressed" means in
    /// docs/16, and what a container keys structural sharing on.
    pub fn content_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.body).as_bytes()
    }

    /// The op ids this snapshot folded, without decoding the image.
    pub fn covered_ids(&self) -> Result<Vec<OpId>, SnapshotFault> {
        self.split_body().map(|(_, ids)| ids)
    }

    /// Loads the snapshot **only if it proves itself**: the body decodes, its
    /// replay reproduces the recorded state hash, and it covers the op set its
    /// watermark claims.
    ///
    /// The returned [`VerifiedSnapshot`] is the *only* way to reach READY in
    /// the lifecycle machine, so docs/27's forbidden "opening READY without
    /// hash-verifying the loaded snapshot" is structurally unreachable rather
    /// than checked at runtime (D-060).
    pub fn verify(&self) -> Result<VerifiedSnapshot, SnapshotFault> {
        let (image, covered) = self.split_body()?;
        let (state, stamps) =
            State::from_image_with_stamps(image).map_err(SnapshotFault::ImageFault)?;
        if state.state_hash().as_bytes() != &self.state_hash {
            return Err(SnapshotFault::StateHashMismatch);
        }
        // The watermark is rebuilt from the covered ids rather than trusted, so
        // a body and a watermark column that disagree are caught rather than
        // averaged.
        let rebuilt = Watermark::from_pairs(covered.iter().map(|id| (id.actor, id.counter)));
        if rebuilt != self.watermark {
            return Err(SnapshotFault::WatermarkMismatch);
        }
        Ok(VerifiedSnapshot {
            watermark: self.watermark.clone(),
            state,
            stamps,
            covered,
        })
    }
}

/// Proof that a snapshot verified. Cannot be constructed except by
/// [`Snapshot::verify`], which is the point.
#[derive(Clone)]
pub struct VerifiedSnapshot {
    watermark: Watermark,
    state: State,
    stamps: WinnerStamps,
    covered: Vec<OpId>,
}

/// Hand-written, not derived: a `VerifiedSnapshot` now carries a whole
/// materialised `State`, and deriving `Debug` would print a workbook into a
/// test failure or a log line. `Event` derives `Debug`, so it needs one.
impl core::fmt::Debug for VerifiedSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VerifiedSnapshot")
            .field("watermark", &self.watermark)
            .field("covered", &self.covered.len())
            .field("stamps", &self.stamps.len())
            .finish()
    }
}

impl VerifiedSnapshot {
    pub fn watermark(&self) -> &Watermark {
        &self.watermark
    }

    /// The op ids this snapshot folded — sorted, so a container can binary
    /// search it to separate stored ops into "already in the image" and "tail".
    ///
    /// This is what the op *bodies* used to be needed for, and it is all they
    /// were needed for.
    pub fn covered(&self) -> &[OpId] {
        &self.covered
    }

    /// The winner stamps the image carries, which a tail needs (ADR-036).
    pub fn stamps(&self) -> &WinnerStamps {
        &self.stamps
    }

    /// The state this snapshot restores, before any tail is applied.
    ///
    /// A decode, not a replay — the whole point of ADR-036.
    pub fn into_state(self) -> State {
        self.state
    }

    pub fn state(&self) -> &State {
        &self.state
    }
}
