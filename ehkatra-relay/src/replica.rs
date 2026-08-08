//! One replica = an editing session (`usk-reduce`) + a sync session
//! (`usk-sync`), plus the glue that turns [`Action`]s into wire messages.
//!
//! This is the shell layer the kernel crates deliberately do not contain: the
//! machine says *what* to send, this says *how*, and the two are separable
//! precisely because the machine performs no I/O (DP-A3).

use usk_oplog::Op;
use usk_reduce::{Command, CommandError, Session};
use usk_sync::machine::{Action, Event, SyncSession, SyncState};
use usk_sync::Rejection;
use usk_types::{ActorId, ColId, RowId, Value};

use crate::frame::Message;

/// A replica: its workbook, its connection, and what it has refused.
pub struct Replica {
    pub doc: Session,
    pub sync: SyncSession,
    /// Ops quarantined this session (DP-E4). Surfaced, never silently dropped.
    pub rejected: Vec<Rejection>,
    /// Set when the peer's wire version is unsupported (docs/27: INCOMPATIBLE
    /// is "read-only local").
    pub read_only: bool,
    /// Backoff deadline the shell must honour before reconnecting.
    pub retry_at_ms: Option<u64>,
    /// Unlisted transitions observed — a defect counter, expected to stay 0.
    pub unexpected: usize,
    /// Sessions torn down and rebuilt after a loss outside LIVE/SYNCING.
    pub resets: usize,
    /// Seed for reset backoff jitter. Injected, never ambient (DP-A2).
    reset_seed: u32,
}

/// How long a torn-down session waits before reconnecting.
const RESET_BACKOFF_MS: u64 = 500;

impl Replica {
    pub fn new(actor: ActorId, jitter_seed: u32) -> Replica {
        Replica {
            doc: Session::new(actor),
            sync: SyncSession::new(actor, jitter_seed),
            rejected: Vec::new(),
            read_only: false,
            retry_at_ms: None,
            unexpected: 0,
            resets: 0,
            reset_seed: jitter_seed | 1,
        }
    }

    /// Rebuilds a replica from its durable op log — the recovery path a restart
    /// takes. Every op is re-declared to the sync session, so an op that was
    /// authored but never acknowledged is still queued afterwards.
    pub fn recover(actor: ActorId, jitter_seed: u32, log: &[Op], unacked: &[Op]) -> Replica {
        let mut r = Replica::new(actor, jitter_seed);
        r.doc.integrate_batch(log.to_vec());
        r.sync.observe_local_history(log);
        for op in unacked {
            // Re-queues without re-sending: the session is DISCONNECTED, so the
            // "no OPS before HelloAck" rule holds the ops until the handshake.
            r.sync.step(Event::LocalOp(op.clone()));
        }
        r
    }

    pub fn actor(&self) -> ActorId {
        self.doc.actor()
    }

    /// Takes `&mut self` because reading state may owe a fold (TD-24): a
    /// replica that has been absorbing remote batches has a log ahead of its
    /// materialised state until someone asks.
    pub fn state_hash(&mut self) -> blake3::Hash {
        self.doc.state().state_hash()
    }

    pub fn value(&mut self, row: RowId, col: ColId) -> Option<Value> {
        self.doc.value(row, col)
    }

    /// Ops authored here that no peer has acknowledged — what a crash must not
    /// lose.
    pub fn unacked(&self) -> Vec<Op> {
        self.sync.queued_ops()
    }

    pub fn log(&self) -> Vec<Op> {
        self.doc.log.ops().to_vec()
    }

    /// Applies a local command, then offers the ops it produced to the sync
    /// session. Returns whatever must go on the wire — nothing at all when the
    /// session has not completed its handshake.
    pub fn edit(&mut self, cmd: Command) -> Result<Vec<Message>, CommandError> {
        if self.read_only {
            return Err(CommandError::OutOfRange);
        }
        let before = self.doc.log.ops().len();
        self.doc.apply(cmd)?;
        let fresh: Vec<Op> = self.doc.log.ops()[before..].to_vec();
        let mut out = Vec::new();
        for op in fresh {
            let actions = self.sync.step(Event::LocalOp(op));
            out.extend(self.pump(actions));
        }
        Ok(out)
    }

    /// Feeds one inbound message to the machine.
    ///
    /// # Why this is a *match on state*, not a plain message → event map
    /// A real link reorders, duplicates and delays. docs/27 §1 defines the
    /// happy transitions and says any pair it does not list is a defect — so
    /// the shell's job is to never hand the machine a pair the specification
    /// has not defined, rather than to widen the machine until nothing is
    /// unlisted. Concretely: a second `InSync` after the session already went
    /// LIVE is a duplicate and is dropped; ops offered during anti-entropy and
    /// ops pushed during LIVE are the same payload and are routed to whichever
    /// of `Give`/`RemoteOps` the current state defines. A message arriving in a
    /// state that has no use for it is discarded, which is safe because the
    /// next anti-entropy round re-derives whatever it carried — and because a
    /// *local* op is never what gets discarded (that is the queue's job).
    ///
    /// The adaptation lives here, in the transport shell, exactly where docs/20
    /// puts transport concerns. The gap it papers over is filed as D-064.
    pub fn receive(&mut self, msg: Message) -> Vec<Message> {
        use SyncState::*;
        let state = self.sync.state().clone();
        let event = match (msg, &state) {
            (Message::HelloAck(n), HelloSent) => Event::HelloAck(n),
            (Message::HelloReject { peer_wire }, HelloSent) => Event::HelloReject { peer_wire },
            (Message::Need(c), Syncing) => Event::Need(c),
            // One payload, two names: the state decides which edge it is.
            (Message::Give(ops), Syncing) | (Message::Ops(ops), Syncing) => Event::Give(ops),
            (Message::Give(ops), Live) | (Message::Ops(ops), Live) => Event::RemoteOps(ops),
            (Message::Ack(c), Live) | (Message::Ack(c), Syncing) => Event::Ack(c),
            (Message::InSync, Syncing) => Event::ClocksEqual,
            // Late, duplicated, or addressed to a state that cannot use it —
            // and HELLO, which only a relay ever receives.
            _ => return Vec::new(),
        };
        let actions = self.sync.step(event);
        self.pump(actions)
    }

    pub fn connect(&mut self) -> Vec<Message> {
        let a = self.sync.step(Event::Connect);
        self.pump(a)
    }

    /// The transport died.
    ///
    /// From LIVE or SYNCING this is docs/27's `──transport loss──► BACKOFF(n)`.
    /// From HELLO_SENT or BACKOFF the specification defines nothing (D-064), and
    /// the shell's answer is the one documented there: **tear the session down
    /// and build a fresh one from DISCONNECTED**, which needs no undefined
    /// transition because a new session legitimately starts there.
    ///
    /// The first implementation of this branch did nothing at all, on the theory
    /// that an existing backoff timer would drive the next attempt. There is no
    /// such timer after a retry has already fired, so a replica that lost its
    /// link mid-handshake wedged in HELLO_SENT forever. W-SYNC-RELAY at 50
    /// replicas found it: 4,051 dropped frames, and the run diverged.
    pub fn transport_loss(&mut self) -> Vec<Message> {
        if matches!(self.sync.state(), SyncState::Live | SyncState::Syncing) {
            let a = self.sync.step(Event::TransportLoss);
            return self.pump(a);
        }
        self.hard_reset();
        Vec::new()
    }

    /// Rebuilds the sync session from DISCONNECTED, carrying the durable log
    /// and **every unacknowledged op** across. Never-drop survives a teardown
    /// for the same reason it survives a process death: the queue is rebuilt
    /// from what the workbook durably holds, not from session memory.
    fn hard_reset(&mut self) {
        let log = self.doc.log.ops().to_vec();
        let unacked = self.sync.queued_ops();
        // Vary the seed so fifty replicas resetting on the same tick do not
        // then reconnect on the same tick (DP-A2: injected, still deterministic).
        self.reset_seed = self
            .reset_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.sync = SyncSession::new(self.doc.actor(), self.reset_seed | 1);
        self.sync.observe_local_history(&log);
        for op in unacked {
            self.sync.step(Event::LocalOp(op));
        }
        // A fresh session is DISCONNECTED, so `resume` will `connect` it. The
        // delay keeps a flapping link from reconnecting in a tight loop.
        self.retry_at_ms = Some(RESET_BACKOFF_MS + u64::from(self.reset_seed % 500));
        self.resets += 1;
    }

    /// The retry timer fired: reconnect by whichever route the current state
    /// defines. A session that was torn down is DISCONNECTED and wants
    /// `Connect`; one that took docs/27's backoff edge wants `BackoffElapsed`.
    /// Asking the state rather than assuming is what keeps every step a
    /// transition the specification actually lists.
    pub fn resume(&mut self) -> Vec<Message> {
        self.retry_at_ms = None;
        match self.sync.state() {
            SyncState::Disconnected => {
                let a = self.sync.step(Event::Connect);
                self.pump(a)
            }
            SyncState::Backoff { .. } => {
                let a = self.sync.step(Event::BackoffElapsed);
                self.pump(a)
            }
            // Already recovered by another path; nothing to do.
            _ => Vec::new(),
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self.sync.state(), SyncState::Live)
    }

    fn pump(&mut self, actions: Vec<Action>) -> Vec<Message> {
        let mut out = Vec::new();
        for action in actions {
            match action {
                Action::SendHello(h) => out.push(Message::Hello(h)),
                Action::SendOps(ops) => out.push(Message::Ops(ops)),
                Action::SendNeed(c) => out.push(Message::Need(c)),
                Action::SendGive(ops) => out.push(Message::Give(ops)),
                Action::AckWatermark(c) => out.push(Message::Ack(c)),
                Action::Apply(ops) => self.doc.integrate_batch(ops),
                Action::Report(rs) => self.rejected.extend(rs),
                Action::ArmTimer { delay_ms } => self.retry_at_ms = Some(delay_ms),
                Action::GoReadOnly { .. } => self.read_only = true,
                // Row 11 owns snapshots; until then the shell has nothing to
                // fetch, and saying so is better than pretending.
                Action::RequestSnapshot => {}
                Action::LogUnexpected => self.unexpected += 1,
            }
        }
        out
    }
}
