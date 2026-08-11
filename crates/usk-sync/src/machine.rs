//! The replica sync session machine — **docs/27 §1, transcribed**.
//!
//! ```text
//! DISCONNECTED ──connect──► HELLO_SENT ──HelloAck(negotiated)──► SYNCING
//! HELLO_SENT ──HelloReject(version)──► INCOMPATIBLE (terminal until upgrade; read-only local)
//! SYNCING ──vector clocks equal──► LIVE
//! SYNCING ──NEED/GIVE exchange──► SYNCING            (anti-entropy loop, Merkle-guided)
//! LIVE ──local op──► LIVE (send OPS)                 (echo suppressed by op id)
//! LIVE ──remote OPS──► LIVE (apply, ack watermark)
//! LIVE|SYNCING ──transport loss──► BACKOFF(n) ──timer──► HELLO_SENT   (jittered exp backoff, cap 60 s)
//! LIVE ──StalenessExceeded(180d watermark gap)──► REBASE_REQUIRED ──snapshot+migrate_ops──► SYNCING
//! any ──auth revoked──► DISCONNECTED (queued ops retained; never dropped)
//! ```
//!
//! Forbidden, and each one proven rejected in `tests/sync_machine.rs`:
//! * sending OPS before HelloAck;
//! * applying remote ops that fail schema/bounds validation (reject + report,
//!   **stay LIVE**);
//! * dropping queued local ops in any transition.
//!
//! # Shape
//! `step(&mut self, Event) -> Vec<Action>` is a pure transition function over
//! owned data: no clock, no socket, no entropy source (DP-A2). Time enters as
//! `Event::BackoffElapsed`, jitter enters as an injected seed, and I/O is
//! something the *caller* does with the returned `Action`s. That is what lets
//! the whole protocol be tested without a network, and the network shell be a
//! thin adapter (docs/20: L3 changes transport, never semantics).
//!
//! An unlisted (state, event) pair is a no-op with a `debug_assert` and an
//! `Action::LogUnexpected` — docs/27's "never silent" rule.

use alloc::vec;
use alloc::vec::Vec;
use usk_oplog::Op;
use usk_types::ActorId;

use crate::clock::{CausalBuffer, VectorClock};
use crate::queue::Queue;
use crate::validate::{partition, Rejection};

/// Wire protocol version. N−2 is supported (docs/15 §Protocol).
pub const WIRE_VERSION: u16 = 1;
/// Model (op taxonomy) version.
pub const MODEL_VERSION: u16 = 1;
/// How many versions back this build still speaks — docs/15's "N−2 support".
pub const WIRE_SUPPORT_DEPTH: u16 = 2;

/// Whether this build will negotiate with a peer announcing `peer_wire`.
///
/// Newer than us: refuse, because the peer may send op types we would have to
/// preserve opaquely and this encoding carries no length to skip over (TD-25).
/// Older by more than N−2: refuse, because that is the published support
/// window. Everything between is accepted and negotiated down.
pub fn wire_supported(peer_wire: u16) -> bool {
    peer_wire <= WIRE_VERSION && WIRE_VERSION.saturating_sub(peer_wire) <= WIRE_SUPPORT_DEPTH
}

/// First reconnect delay, doubling per attempt.
pub const BACKOFF_BASE_MS: u64 = 500;
/// docs/27's cap: "jittered exp backoff, cap 60 s".
pub const BACKOFF_CAP_MS: u64 = 60_000;

/// The published staleness window (docs/15): past it, a replica must rebase
/// from a snapshot rather than merge.
pub const STALENESS_WINDOW_DAYS: u32 = 180;

/// States, exactly as docs/27 §1 names them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SyncState {
    Disconnected,
    HelloSent,
    /// Terminal until the build is upgraded. Editing continues locally and
    /// read-only against the peer — the workbook does not become unusable
    /// because a relay is newer.
    Incompatible {
        peer_wire: u16,
    },
    Syncing,
    Live,
    Backoff {
        attempt: u32,
        delay_ms: u64,
    },
    RebaseRequired,
}

impl SyncState {
    /// Whether ops may be put on the wire. The forbidden-transition rule
    /// "no OPS before HelloAck" is this predicate, and every send path asks it.
    pub fn may_send_ops(&self) -> bool {
        matches!(self, SyncState::Syncing | SyncState::Live)
    }
}

/// What the negotiation settled on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Negotiated {
    pub wire: u16,
    pub model: u16,
}

/// The HELLO payload: auth principal, versions, and a vector-clock summary.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hello {
    pub actor: ActorId,
    pub wire: u16,
    pub model: u16,
    pub clock: VectorClock,
}

/// Inbound events. Transport and timer events are facts the shell observes;
/// everything else is a protocol message or a local edit.
///
/// `LocalOp(Op)` makes this enum as large as an `Op`, which ADR-041 grew when
/// `Payload::SetStyle` added an identity rectangle. Boxing it is the obvious
/// answer and is the wrong one here: an `Event` is constructed once per op and
/// consumed immediately by `step`, so the allocation would be paid on the hot
/// path to save stack bytes on a value that never outlives its call. The lint
/// is silenced with the reason rather than obeyed.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Event {
    Connect,
    HelloAck(Negotiated),
    HelloReject {
        peer_wire: u16,
    },
    /// Anti-entropy: the peer says what it holds.
    Need(VectorClock),
    /// Anti-entropy: the peer sends ops we lack.
    Give(Vec<Op>),
    /// The exchange converged — both sides hold the same set.
    ClocksEqual,
    /// A locally authored op (already applied to local state).
    LocalOp(Op),
    /// Ops pushed by the peer during LIVE.
    RemoteOps(Vec<Op>),
    /// The peer acknowledged up to this watermark.
    Ack(VectorClock),
    TransportLoss,
    BackoffElapsed,
    StalenessExceeded {
        gap_days: u32,
    },
    /// A snapshot + migrated op tail replaced the local history.
    RebaseComplete {
        ops: Vec<Op>,
    },
    AuthRevoked,
}

/// What the shell must do. The machine performs no I/O itself (DP-A2).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SendHello(Hello),
    SendOps(Vec<Op>),
    SendNeed(VectorClock),
    SendGive(Vec<Op>),
    /// Ops that passed validation and are causally ready — hand to the applier.
    Apply(Vec<Op>),
    /// Quarantined ops with their reasons. Reported, never applied (DP-E4).
    Report(Vec<Rejection>),
    /// Tell the peer how far we have durably taken its ops.
    AckWatermark(VectorClock),
    ArmTimer {
        delay_ms: u64,
    },
    /// Staleness past the published window: fetch a snapshot and migrate.
    RequestSnapshot,
    /// INCOMPATIBLE: docs/27 says "terminal until upgrade; read-only local".
    /// The shell stops accepting edits; the machine still refuses to drop what
    /// is already queued, because never-drop has no exceptions.
    GoReadOnly {
        peer_wire: u16,
    },
    /// An unlisted (state, event) pair. Logged, never silent (docs/27).
    LogUnexpected,
}

/// One replica's view of one connection.
pub struct SyncSession {
    actor: ActorId,
    state: SyncState,
    /// Ops authored locally and not yet acknowledged. Never dropped.
    queue: Queue,
    /// What this replica holds.
    clock: VectorClock,
    /// Arrivals waiting on a causal gap.
    buffer: CausalBuffer,
    /// Greatest lamport this replica has seen — the bound hostile lamports are
    /// checked against (DP-E4).
    frontier_lamport: u64,
    /// Deterministic jitter source. Entropy is *injected*, never ambient
    /// (DP-A2/DP-A3): the shell seeds it from the PAL, and a given seed always
    /// produces the same backoff schedule, so a reconnect storm is reproducible
    /// in a test.
    jitter: u32,
    negotiated: Option<Negotiated>,
    /// Rejections seen this session, for the health report (docs/36).
    quarantined: usize,
    /// Consecutive failed connections — the `n` in docs/27's `BACKOFF(n)`.
    /// Reset on reaching LIVE, so a healthy link never inherits an old penalty.
    backoff_attempt: u32,
}

impl SyncSession {
    pub fn new(actor: ActorId, jitter_seed: u32) -> Self {
        SyncSession {
            actor,
            state: SyncState::Disconnected,
            queue: Queue::new(),
            clock: VectorClock::new(),
            buffer: CausalBuffer::default(),
            frontier_lamport: 0,
            jitter: jitter_seed | 1,
            negotiated: None,
            quarantined: 0,
            backoff_attempt: 0,
        }
    }

    pub fn state(&self) -> &SyncState {
        &self.state
    }

    pub fn clock(&self) -> &VectorClock {
        &self.clock
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    pub fn queued_ops(&self) -> Vec<Op> {
        self.queue.all()
    }

    pub fn quarantined(&self) -> usize {
        self.quarantined
    }

    pub fn negotiated(&self) -> Option<Negotiated> {
        self.negotiated
    }

    pub fn held_by_causal_gap(&self) -> usize {
        self.buffer.held()
    }

    /// Records an op this replica already holds (its own history at startup, or
    /// a snapshot's contents) so the clock summarises it.
    pub fn observe_local_history(&mut self, ops: &[Op]) {
        for op in ops {
            self.clock.observe(op.id);
            self.frontier_lamport = self.frontier_lamport.max(op.lamport);
        }
    }

    /// The transition function. Returns the actions the shell must perform.
    pub fn step(&mut self, event: Event) -> Vec<Action> {
        // `any ──auth revoked──► DISCONNECTED (queued ops retained; never
        // dropped)` — listed for *every* state, so it is handled before the
        // per-state match rather than repeated in each arm.
        if let Event::AuthRevoked = event {
            self.state = SyncState::Disconnected;
            self.negotiated = None;
            // The queue is deliberately untouched: revocation is the case the
            // never-drop contract exists for.
            self.queue.mark_all_unsent();
            return Vec::new();
        }

        match (&self.state.clone(), event) {
            // DISCONNECTED ──connect──► HELLO_SENT
            (SyncState::Disconnected, Event::Connect) => {
                self.state = SyncState::HelloSent;
                vec![Action::SendHello(Hello {
                    actor: self.actor,
                    wire: WIRE_VERSION,
                    model: MODEL_VERSION,
                    clock: self.clock.clone(),
                })]
            }

            // BACKOFF(n) ──timer──► HELLO_SENT
            (SyncState::Backoff { .. }, Event::BackoffElapsed) => {
                self.state = SyncState::HelloSent;
                vec![Action::SendHello(Hello {
                    actor: self.actor,
                    wire: WIRE_VERSION,
                    model: MODEL_VERSION,
                    clock: self.clock.clone(),
                })]
            }

            // HELLO_SENT ──HelloAck(negotiated)──► SYNCING
            (SyncState::HelloSent, Event::HelloAck(negotiated)) => {
                self.state = SyncState::Syncing;
                self.negotiated = Some(negotiated);
                // Anti-entropy opens by declaring what we hold.
                vec![Action::SendNeed(self.clock.clone())]
            }

            // HELLO_SENT ──HelloReject(version)──► INCOMPATIBLE
            (SyncState::HelloSent, Event::HelloReject { peer_wire }) => {
                self.state = SyncState::Incompatible { peer_wire };
                vec![Action::GoReadOnly { peer_wire }]
            }

            // SYNCING ──NEED/GIVE exchange──► SYNCING
            (SyncState::Syncing, Event::Need(peer_clock)) => {
                let missing = self.ops_peer_lacks(&peer_clock);
                vec![Action::SendGive(missing)]
            }
            (SyncState::Syncing, Event::Give(ops)) => self.receive(ops),

            // SYNCING ──vector clocks equal──► LIVE
            (SyncState::Syncing, Event::ClocksEqual) => {
                self.state = SyncState::Live;
                self.backoff_attempt = 0;
                // Anything authored while offline goes out the moment sending
                // becomes legal — and not one transition earlier.
                let pending = self.queue.take_unsent();
                if pending.is_empty() {
                    Vec::new()
                } else {
                    vec![Action::SendOps(pending)]
                }
            }

            // LIVE ──local op──► LIVE (send OPS)
            // In every other state the op is *queued only*: that is the
            // "no OPS before HelloAck" rule, enforced at the one place ops can
            // reach the wire.
            (_, Event::LocalOp(op)) => {
                self.clock.observe(op.id);
                self.frontier_lamport = self.frontier_lamport.max(op.lamport);
                self.queue.enqueue(op);
                if self.state.may_send_ops() {
                    let pending = self.queue.take_unsent();
                    if pending.is_empty() {
                        Vec::new()
                    } else {
                        vec![Action::SendOps(pending)]
                    }
                } else {
                    Vec::new()
                }
            }

            // LIVE ──remote OPS──► LIVE (apply, ack watermark)
            (SyncState::Live, Event::RemoteOps(ops)) => self.receive(ops),

            // The return leg of docs/27's "apply, ack watermark": the peer
            // tells us how far it has durably taken our ops. This is the only
            // path that may empty the queue, and it changes no state — a
            // self-loop on the states where ops legally flow.
            (SyncState::Live, Event::Ack(watermark))
            | (SyncState::Syncing, Event::Ack(watermark)) => {
                self.queue.ack(&watermark);
                Vec::new()
            }

            // LIVE|SYNCING ──transport loss──► BACKOFF(n)
            //
            // Exactly those two states. A loss during HELLO_SENT or BACKOFF is
            // *not* a transition docs/27 lists, so it lands in the unlisted arm
            // below rather than being invented here; the shell tears such a
            // session down and builds a fresh one from DISCONNECTED. Filed as a
            // spec gap (D-064) rather than self-amended — same rule as D-062.
            (SyncState::Live, Event::TransportLoss)
            | (SyncState::Syncing, Event::TransportLoss) => {
                let n = self.backoff_attempt;
                self.backoff_attempt = n.saturating_add(1);
                self.enter_backoff(n)
            }

            // LIVE ──StalenessExceeded(180d watermark gap)──► REBASE_REQUIRED
            (SyncState::Live, Event::StalenessExceeded { gap_days })
                if gap_days >= STALENESS_WINDOW_DAYS =>
            {
                self.state = SyncState::RebaseRequired;
                vec![Action::RequestSnapshot]
            }

            // REBASE_REQUIRED ──snapshot+migrate_ops──► SYNCING
            (SyncState::RebaseRequired, Event::RebaseComplete { ops }) => {
                self.state = SyncState::Syncing;
                self.observe_local_history(&ops);
                // Unrebasable local ops are *not* discarded here: the queue is
                // untouched, so they are re-offered against the new baseline
                // and, if they still cannot merge, surface in the unmerged-
                // changes ledger the shell owns (docs/15 §Offline).
                vec![Action::SendNeed(self.clock.clone())]
            }

            // Everything else is unlisted: log it, change nothing (docs/27).
            (_, _other) => {
                debug_assert!(
                    false,
                    "unlisted sync transition — docs/27 §1 does not define it"
                );
                vec![Action::LogUnexpected]
            }
        }
    }

    /// Validate → quarantine → causal-gap buffer → apply → ack.
    ///
    /// The order matters. Validation comes first so a malformed op never
    /// reaches the buffer, and the session **stays in its current state**
    /// throughout: docs/27 forbids applying an invalid op, not staying
    /// connected to the collaborator who sent one.
    fn receive(&mut self, ops: Vec<Op>) -> Vec<Action> {
        // The principal check belongs at the relay, which knows which socket
        // authenticated as whom. A replica receives a fan-out of many actors'
        // ops, so it validates everything except authorship.
        let (accepted, rejected) = partition(ops, None, self.frontier_lamport);
        let mut actions = Vec::new();
        if !rejected.is_empty() {
            self.quarantined += rejected.len();
            actions.push(Action::Report(rejected));
        }
        let ready = self.buffer.admit(&mut self.clock, accepted);
        if !ready.is_empty() {
            for op in &ready {
                self.frontier_lamport = self.frontier_lamport.max(op.lamport);
            }
            actions.push(Action::Apply(ready));
            actions.push(Action::AckWatermark(self.clock.clone()));
        }
        actions
    }

    /// Ops we hold that the peer's clock says it lacks.
    ///
    /// v0.1 answers from the never-drop queue, which holds every op this
    /// replica authored and has not had acknowledged. That is exact for the
    /// two-replica and relay-fanout cases Row 10 proves. Localising divergence
    /// inside a *large* shared history is what docs/15's tile-Merkle comparison
    /// is for; it needs Row 11's snapshot Merkle and is filed as TD-26.
    fn ops_peer_lacks(&self, peer: &VectorClock) -> Vec<Op> {
        self.queue
            .all()
            .into_iter()
            .filter(|op| !peer.covers(op.id))
            .collect()
    }

    /// `BACKOFF(n) ──timer──► HELLO_SENT`, with docs/27's jittered exponential
    /// delay capped at 60 s. Jitter is ±25%, derived from the injected seed by
    /// an xorshift step, so the schedule is deterministic and replayable.
    fn enter_backoff(&mut self, attempt: u32) -> Vec<Action> {
        // Losing the socket does not lose the ops: everything sent-but-unacked
        // becomes unsent again and is redelivered on reconnect.
        self.queue.mark_all_unsent();
        let base = BACKOFF_BASE_MS
            .saturating_mul(1u64 << attempt.min(20))
            .min(BACKOFF_CAP_MS);
        self.jitter ^= self.jitter << 13;
        self.jitter ^= self.jitter >> 17;
        self.jitter ^= self.jitter << 5;
        // ±25% of base, in [0.75×, 1.25×).
        let spread = base / 2;
        let delay_ms =
            (base - spread / 2 + (self.jitter as u64 % spread.max(1))).min(BACKOFF_CAP_MS);
        self.state = SyncState::Backoff { attempt, delay_ms };
        vec![Action::ArmTimer { delay_ms }]
    }
}
