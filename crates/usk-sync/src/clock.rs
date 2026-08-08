//! Vector clocks and the causal-gap buffer (docs/15 §Protocol).
//!
//! A replica summarises what it holds as *the highest contiguous counter per
//! actor*. Contiguity is the point: `{A: 7}` means "every op A ever minted up
//! to counter 7", which is a complete answer in `actors × 12` bytes and lets
//! anti-entropy compute exactly what a peer lacks without comparing op sets.
//!
//! Ops that arrive ahead of a gap — the relay crashed mid-batch, a packet was
//! lost — are **held**, not applied and not dropped, until the gap fills. That
//! is docs/15's "client redelivery via causal gaps" failure drill, and it is why
//! `observe` refuses to advance over a hole.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_oplog::Op;
use usk_types::{ActorId, Counter, OpId};

/// Highest contiguous counter held per actor. Absent actor = nothing held.
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct VectorClock {
    entries: BTreeMap<ActorId, Counter>,
}

impl VectorClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// The highest contiguous counter held for `actor` (0 = none).
    pub fn get(&self, actor: ActorId) -> Counter {
        self.entries.get(&actor).copied().unwrap_or(0)
    }

    /// Records that the op is held. Advances only across a contiguous run, so
    /// a clock never claims coverage it does not have.
    pub fn observe(&mut self, id: OpId) {
        let slot = self.entries.entry(id.actor).or_insert(0);
        if id.counter == *slot + 1 {
            *slot = id.counter;
        }
    }

    /// True when this clock already covers the op.
    pub fn covers(&self, id: OpId) -> bool {
        self.get(id.actor) >= id.counter
    }

    /// Merges another clock, taking the greater coverage per actor. Used when a
    /// peer's watermark acknowledges our ops.
    pub fn merge_max(&mut self, other: &VectorClock) {
        for (actor, counter) in &other.entries {
            let slot = self.entries.entry(*actor).or_insert(0);
            if *counter > *slot {
                *slot = *counter;
            }
        }
    }

    pub fn actors(&self) -> impl Iterator<Item = (ActorId, Counter)> + '_ {
        self.entries.iter().map(|(a, c)| (*a, *c))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.values().all(|c| *c == 0)
    }
}

/// Holds ops that arrived ahead of a causal gap and releases them, in counter
/// order, the moment the gap closes.
///
/// The buffer is bounded (`CAUSAL_BUFFER_LIMIT`) because an actor that only ever
/// sends counter 1,000,000 would otherwise pin memory forever — DoS via
/// op amplification, docs/37 boundary 2. Ops evicted at the bound are **not
/// lost**: they were never acked, so the sender's own never-drop queue still
/// holds them and redelivers on the next anti-entropy round.
pub const CAUSAL_BUFFER_LIMIT: usize = 4096;

/// Applies ops to a clock, releasing whatever the arrivals make contiguous.
#[derive(Default, Clone)]
pub struct CausalBuffer {
    held: BTreeMap<(ActorId, Counter), Op>,
}

impl CausalBuffer {
    /// Offers ops to the buffer against `clock`, returning those now causally
    /// ready, in `(actor, counter)` order, with `clock` advanced over them.
    ///
    /// Duplicates are dropped silently: a relay redelivering an op the replica
    /// already holds is normal, not an error (idempotent merge, DP-A8).
    pub fn admit(&mut self, clock: &mut VectorClock, ops: Vec<Op>) -> Vec<Op> {
        for op in ops {
            if clock.covers(op.id) {
                continue;
            }
            if self.held.len() >= CAUSAL_BUFFER_LIMIT {
                continue;
            }
            self.held.insert((op.id.actor, op.id.counter), op);
        }

        let mut ready = Vec::new();
        // A release can unlock a further release (…, 5, 6, 7 arriving as 6,7,5),
        // so sweep until a full pass finds nothing.
        loop {
            let next: Option<(ActorId, Counter)> = self
                .held
                .keys()
                .find(|(actor, counter)| *counter == clock.get(*actor) + 1)
                .copied();
            match next {
                Some(key) => {
                    if let Some(op) = self.held.remove(&key) {
                        clock.observe(op.id);
                        ready.push(op);
                    }
                }
                None => break,
            }
        }
        ready
    }

    /// Ops still waiting on a gap.
    pub fn held(&self) -> usize {
        self.held.len()
    }
}
