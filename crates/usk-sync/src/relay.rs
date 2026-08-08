//! The relay: fanout, retention and admission control — **never a merge
//! authority** (docs/15 §Protocol).
//!
//! It validates op schema and bounds, binds each connection's `ActorId` to the
//! principal that authenticated it, enforces per-actor token buckets on rate and
//! bytes, and fans out to the other subscribers. It never merges, never
//! resolves a conflict, and never rewrites an op — every replica reaches the
//! same state on its own, so a compromised relay can withhold or delay but not
//! corrupt (docs/37 boundary 2).
//!
//! This module is pure logic with no I/O, exactly like the session machine: the
//! transport shell feeds it bytes-decoded ops and performs the fanout it
//! returns.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_oplog::Op;
use usk_types::ActorId;

use crate::clock::VectorClock;
use crate::validate::validate;

/// Ops per second an actor may sustain. A human types single-digit ops/s; a
/// fill-down or paste is a burst, which the bucket depth covers.
pub const RATE_OPS_PER_SEC: u32 = 200;
/// Burst depth, in ops.
pub const RATE_BURST_OPS: u32 = 2_000;
/// Bytes per second an actor may sustain.
pub const RATE_BYTES_PER_SEC: u32 = 1 << 20;
/// Burst depth, in bytes.
pub const RATE_BURST_BYTES: u32 = 8 << 20;

/// A token bucket over one dimension. Time is injected in milliseconds — the
/// relay is a kernel-side pure function and reads no clock itself (DP-A2).
#[derive(Clone, Debug)]
struct Bucket {
    tokens: u64,
    burst: u64,
    per_sec: u64,
    last_refill_ms: u64,
}

impl Bucket {
    fn new(per_sec: u32, burst: u32) -> Self {
        Bucket {
            tokens: burst as u64,
            burst: burst as u64,
            per_sec: per_sec as u64,
            last_refill_ms: 0,
        }
    }

    fn refill(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.last_refill_ms);
        if elapsed == 0 {
            return;
        }
        self.last_refill_ms = now_ms;
        self.tokens = (self.tokens + elapsed.saturating_mul(self.per_sec) / 1000).min(self.burst);
    }

    fn take(&mut self, n: u64) -> bool {
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

/// Why the relay refused to admit an op.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdmissionError {
    /// Op authorship does not match the authenticated principal (spoofing).
    Spoofed,
    /// Failed schema/bounds validation (DP-E4).
    Invalid,
    /// Per-actor op-rate bucket exhausted.
    RateLimited,
    /// Per-actor byte bucket exhausted.
    ByteLimited,
}

/// What one submission produced.
#[derive(Default, Debug)]
pub struct Admission {
    /// Ops to fan out to every *other* subscriber.
    pub fanout: Vec<Op>,
    /// Ops refused, with why. Reported to the sender; never fanned out.
    pub refused: Vec<(Op, AdmissionError)>,
    /// How far the relay has durably taken this actor's ops — the ack the
    /// sender's never-drop queue waits for.
    pub watermark: VectorClock,
}

/// One connected actor's admission state.
struct Peer {
    rate: Bucket,
    bytes: Bucket,
}

impl Default for Peer {
    fn default() -> Self {
        Peer {
            rate: Bucket::new(RATE_OPS_PER_SEC, RATE_BURST_OPS),
            bytes: Bucket::new(RATE_BYTES_PER_SEC, RATE_BURST_BYTES),
        }
    }
}

/// The relay's state: retained history plus per-actor admission buckets.
#[derive(Default)]
pub struct Relay {
    retained: Vec<Op>,
    clock: VectorClock,
    peers: BTreeMap<ActorId, Peer>,
}

impl Relay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything the relay retains, for a joining replica's catch-up.
    pub fn retained(&self) -> &[Op] {
        &self.retained
    }

    /// Ops the peer's clock says it lacks — the GIVE half of anti-entropy.
    pub fn ops_missing_from(&self, peer: &VectorClock) -> Vec<Op> {
        self.retained
            .iter()
            .filter(|op| !peer.covers(op.id))
            .cloned()
            .collect()
    }

    pub fn clock(&self) -> &VectorClock {
        &self.clock
    }

    /// Admits a batch from the connection authenticated as `principal`.
    ///
    /// `now_ms` is injected, never read: a pure function of (state, input, time)
    /// is what makes the quota tests deterministic rather than timing-flaky.
    pub fn submit(&mut self, principal: ActorId, ops: Vec<Op>, now_ms: u64) -> Admission {
        let frontier = self.frontier();
        let peer = self.peers.entry(principal).or_default();
        peer.rate.refill(now_ms);
        peer.bytes.refill(now_ms);
        let (rate, bytes) = (&mut peer.rate, &mut peer.bytes);

        let mut out = Admission::default();
        for op in ops {
            let encoded = op.encode().len() as u64;
            // Cheapest checks first, so a flood of spoofed ops cannot spend the
            // relay's CPU on validation.
            let refusal = if op.id.actor != principal {
                Some(AdmissionError::Spoofed)
            } else if !rate.take(1) {
                Some(AdmissionError::RateLimited)
            } else if !bytes.take(encoded) {
                Some(AdmissionError::ByteLimited)
            } else if validate(&op, Some(principal), frontier).is_err() {
                Some(AdmissionError::Invalid)
            } else {
                None
            };
            match refusal {
                Some(err) => out.refused.push((op, err)),
                None => {
                    if !self.retained.iter().any(|o| o.id == op.id) {
                        self.clock.observe(op.id);
                        self.retained.push(op.clone());
                        out.fanout.push(op);
                    } else {
                        // A redelivery. Not an error — the ack below still
                        // moves the sender's queue forward.
                        out.fanout.push(op);
                    }
                }
            }
        }
        out.watermark = self.clock.clone();
        out
    }

    fn frontier(&self) -> u64 {
        self.retained.iter().map(|o| o.lamport).max().unwrap_or(0)
    }
}
