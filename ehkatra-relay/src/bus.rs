//! A deterministic in-process transport, so the failure drills docs/15 asks for
//! (partition, relay crash mid-batch, packet loss, reordering) are ordinary
//! tests rather than flaky network experiments.
//!
//! Every source of nondeterminism is a seeded LCG (D-052): loss decisions,
//! delivery jitter and edit scheduling all come from one seed, so a failing run
//! is reproducible from its seed alone.
//!
//! # How a lost frame is modelled
//! Replicas speak to the relay over a stream transport, so a *frame* does not
//! quietly vanish — the connection carrying it breaks. A drop therefore raises
//! `TransportLoss` on the affected replica, which is exactly docs/27's
//! `LIVE|SYNCING ──transport loss──► BACKOFF(n) ──timer──► HELLO_SENT` recovery
//! path. Anti-entropy on reconnect is what actually repairs the gap, and the
//! never-drop queue is what makes the repair complete. Modelling loss as a
//! silent frame disappearance would have quietly skipped both.

use std::collections::VecDeque;

use usk_reduce::Command;
use usk_types::ActorId;

use crate::endpoint::RelayEndpoint;
use crate::frame::Message;
use crate::replica::Replica;

/// Deterministic pseudo-randomness (D-052: seeded LCG, not `proptest`).
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Lcg {
        Lcg(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

/// Where a frame is going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum To {
    Relay(usize),
    Replica(usize),
}

struct InFlight {
    to: To,
    msg: Message,
    at_ms: u64,
}

/// Per-frame propagation record, for the W-SYNC-RELAY percentiles.
#[derive(Clone, Copy, Debug)]
pub struct Propagation {
    pub from: usize,
    pub to: usize,
    pub authored_ms: u64,
    pub arrived_ms: u64,
}

impl Propagation {
    pub fn latency_ms(&self) -> u64 {
        self.arrived_ms.saturating_sub(self.authored_ms)
    }
}

/// One relay, N replicas, a link with latency and loss.
pub struct Bus {
    pub relay: RelayEndpoint,
    pub replicas: Vec<Replica>,
    queue: VecDeque<InFlight>,
    lcg: Lcg,
    pub now_ms: u64,
    /// One-way link latency.
    pub latency_ms: u64,
    /// Loss probability, in parts per thousand.
    pub loss_permille: u64,
    /// Author time of each op, for propagation measurement.
    authored_ms: std::collections::BTreeMap<(u128, u64), u64>,
    /// Per replica, the ops it has already been credited with receiving.
    /// Harness bookkeeping — the replicas track this themselves in their own
    /// vector clocks, but reading it from them would mean asking each replica
    /// to settle its fold on every delivered frame.
    seen: Vec<std::collections::BTreeSet<(u128, u64)>>,
    pub propagations: Vec<Propagation>,
    pub delivered: usize,
    pub dropped: usize,
    pub reconnects: usize,
    /// Replicas whose link is held down.
    ///
    /// A *partition* is not the same failure as a lost frame, and conflating
    /// them cost a measurement: the mid-run-kill harness used to take its victim
    /// offline with a single `transport_loss` and a long retry timer, but any
    /// subsequent dropped frame ran the teardown-and-reconnect path and armed a
    /// fresh 500 ms timer, so the victim rejoined and drained its queue before
    /// the kill landed. A partitioned replica exchanges nothing and reconnects
    /// never, until `heal`.
    partitioned: Vec<bool>,
}

impl Bus {
    pub fn new(replicas: usize, seed: u64, latency_ms: u64, loss_permille: u64) -> Bus {
        Bus {
            relay: RelayEndpoint::new(),
            replicas: (0..replicas)
                .map(|i| {
                    // Distinct jitter seeds, so fifty replicas do not all
                    // reconnect on the same millisecond.
                    let seed = (i as u32 + 1).wrapping_mul(2_654_435_761);
                    Replica::new(ActorId(i as u128 + 1), seed)
                })
                .collect(),
            queue: VecDeque::new(),
            lcg: Lcg::new(seed),
            now_ms: 0,
            latency_ms,
            loss_permille,
            authored_ms: std::collections::BTreeMap::new(),
            seen: vec![std::collections::BTreeSet::new(); replicas],
            propagations: Vec::new(),
            delivered: 0,
            dropped: 0,
            reconnects: 0,
            partitioned: vec![false; replicas],
        }
    }

    /// Cuts a replica off: its link goes down, nothing crosses it in either
    /// direction, and no timer will bring it back. Editing continues locally
    /// and queues, which is docs/15's offline-first case.
    pub fn partition(&mut self, replica: usize) {
        self.replicas[replica].transport_loss();
        self.replicas[replica].retry_at_ms = None;
        self.partitioned[replica] = true;
    }

    /// Restores the link and reconnects. Anti-entropy does the rest.
    ///
    /// Goes through `resume`, not `connect`: a partitioned replica may be in
    /// BACKOFF (the partition began with a transport loss) or DISCONNECTED (it
    /// was rebuilt after a kill), and `Connect` is only a listed transition from
    /// the latter. Asking the state is the same discipline `advance` uses — and
    /// the machine's `debug_assert` caught this exact mistake the first time
    /// `heal` was written, which is what that assert is for.
    pub fn heal(&mut self, replica: usize) {
        self.partitioned[replica] = false;
        let msgs = self.replicas[replica].resume();
        self.send_to_relay(replica, msgs);
    }

    pub fn is_partitioned(&self, replica: usize) -> bool {
        self.partitioned[replica]
    }

    /// Brings every replica through the handshake to LIVE.
    pub fn connect_all(&mut self) {
        for i in 0..self.replicas.len() {
            let msgs = self.replicas[i].connect();
            self.send_to_relay(i, msgs);
        }
        self.settle(200);
    }

    /// Re-opens a replica's connection — a restart, or a manual reconnect.
    pub fn reconnect(&mut self, replica: usize) {
        let msgs = self.replicas[replica].connect();
        self.send_to_relay(replica, msgs);
    }

    pub fn edit(&mut self, replica: usize, cmd: Command) {
        // Borrow the log, never `Replica::log()` — that clones every op, and at
        // 30,000 ops it made the harness's own instrumentation cost more than
        // the system under test.
        let before = self.replicas[replica].doc.log.ops().len();
        let msgs = match self.replicas[replica].edit(cmd) {
            Ok(m) => m,
            Err(_) => return,
        };
        let now = self.now_ms;
        let authored: Vec<(u128, u64)> = self.replicas[replica].doc.log.ops()[before..]
            .iter()
            .map(|op| (op.id.actor.0, op.id.counter))
            .collect();
        for key in authored {
            self.authored_ms.insert(key, now);
            self.seen[replica].insert(key);
        }
        self.send_to_relay(replica, msgs);
    }

    fn send_to_relay(&mut self, from: usize, msgs: Vec<Message>) {
        for msg in msgs {
            self.enqueue(To::Relay(from), msg, from);
        }
    }

    fn enqueue(&mut self, to: To, msg: Message, owner: usize) {
        // A partitioned link swallows the frame and tells nobody: the replica
        // already knows its transport is down, and re-notifying it would arm the
        // reconnect this partition exists to prevent.
        if self.partitioned[owner] {
            self.dropped += 1;
            return;
        }
        if self.lcg.below(1000) < self.loss_permille {
            self.dropped += 1;
            // The link carrying this frame is gone; both endpoints of a stream
            // transport learn that. Only the replica has a machine to tell.
            let msgs = self.replicas[owner].transport_loss();
            if !msgs.is_empty() {
                for m in msgs {
                    self.queue.push_back(InFlight {
                        to: To::Relay(owner),
                        msg: m,
                        at_ms: self.now_ms + self.latency_ms,
                    });
                }
            }
            return;
        }
        self.queue.push_back(InFlight {
            to,
            msg,
            at_ms: self.now_ms + self.latency_ms,
        });
    }

    /// Advances the clock, delivering everything due and firing backoff timers.
    /// Returns the number of frames delivered.
    pub fn advance(&mut self, ms: u64) -> usize {
        self.now_ms += ms;
        let mut delivered = 0usize;

        // Backoff timers first: a replica that has been waiting reconnects
        // before this tick's frames land.
        for i in 0..self.replicas.len() {
            if self.partitioned[i] {
                continue;
            }
            if let Some(delay) = self.replicas[i].retry_at_ms {
                if delay <= ms + self.latency_ms {
                    // `resume`, not `retry`: a session torn down after a loss
                    // outside LIVE/SYNCING is DISCONNECTED and needs `Connect`,
                    // while one that took docs/27's backoff edge needs
                    // `BackoffElapsed`. Asking the state is what keeps every
                    // step a transition the specification lists.
                    let msgs = self.replicas[i].resume();
                    self.reconnects += 1;
                    self.send_to_relay(i, msgs);
                } else {
                    self.replicas[i].retry_at_ms = Some(delay - ms);
                }
            }
        }

        let due: Vec<InFlight> = {
            let mut keep = VecDeque::new();
            let mut out = Vec::new();
            while let Some(f) = self.queue.pop_front() {
                if f.at_ms <= self.now_ms {
                    out.push(f);
                } else {
                    keep.push_back(f);
                }
            }
            self.queue = keep;
            out
        };

        for frame in due {
            delivered += 1;
            self.delivered += 1;
            match frame.to {
                To::Relay(from) => {
                    let principal = self.replicas[from].actor();
                    let out = self.relay.handle(principal, frame.msg, self.now_ms);
                    for m in out.to_sender {
                        self.enqueue(To::Replica(from), m, from);
                    }
                    for m in out.to_others {
                        for i in 0..self.replicas.len() {
                            if i != from {
                                self.enqueue(To::Replica(i), m.clone(), i);
                            }
                        }
                    }
                }
                To::Replica(to) => {
                    self.record_arrivals(to, &frame.msg);
                    let replies = self.replicas[to].receive(frame.msg);
                    self.send_to_relay(to, replies);
                }
            }
        }
        delivered
    }

    fn record_arrivals(&mut self, to: usize, msg: &Message) {
        let ops = match msg {
            Message::Ops(ops) | Message::Give(ops) => ops,
            _ => return,
        };
        let mine = self.replicas[to].actor();
        let mut first_arrivals: Vec<(u128, u64)> = Vec::new();
        for op in ops {
            if op.id.actor == mine {
                continue;
            }
            let key = (op.id.actor.0, op.id.counter);
            // A per-replica seen-set, not a scan of the replica's log. The scan
            // was O(log length) per delivered op and allocated a full clone of
            // the log to do it — instrumentation that cost more than the thing
            // being instrumented.
            if self.seen[to].contains(&key) {
                continue;
            }
            first_arrivals.push(key);
        }
        for key in first_arrivals {
            self.seen[to].insert(key);
            if let Some(authored) = self.authored_ms.get(&key) {
                self.propagations.push(Propagation {
                    from: 0,
                    to,
                    authored_ms: *authored,
                    arrived_ms: self.now_ms,
                });
            }
        }
    }

    /// Runs the bus until it is idle or `max_ticks` elapse. Returns the tick
    /// count actually used — a convergence time, in bus milliseconds.
    pub fn settle(&mut self, max_ticks: u64) -> u64 {
        for tick in 0..max_ticks {
            // A partitioned replica is never live and never has a timer, by
            // design — waiting for it would spin out the whole budget.
            let live_enough = |(i, r): (usize, &Replica)| self.partitioned[i] || r.is_live();
            let pending = !self.queue.is_empty()
                || self
                    .replicas
                    .iter()
                    .enumerate()
                    .any(|(i, r)| !self.partitioned[i] && r.retry_at_ms.is_some())
                || !self.replicas.iter().enumerate().all(live_enough);
            if !pending {
                return tick * self.latency_ms.max(1);
            }
            self.advance(self.latency_ms.max(1));
        }
        max_ticks * self.latency_ms.max(1)
    }

    /// True when every replica holds the same state hash — the convergence
    /// property, checked the only way that means anything.
    /// `&mut` because taking a replica's state hash settles its outstanding
    /// fold (TD-24). This is the only place the 50-replica benchmark pays for
    /// all the folds it deferred, which is the entire point.
    pub fn converged(&mut self) -> bool {
        let hashes = self.hashes();
        hashes.windows(2).all(|w| w[0] == w[1])
    }

    pub fn hashes(&mut self) -> Vec<String> {
        self.replicas
            .iter_mut()
            .map(|r| r.state_hash().to_hex().to_string())
            .collect()
    }

    pub fn lcg(&mut self) -> &mut Lcg {
        &mut self.lcg
    }

    /// p95 of measured propagation latencies, in bus milliseconds.
    pub fn propagation_p95_ms(&self) -> u64 {
        if self.propagations.is_empty() {
            return 0;
        }
        let mut v: Vec<u64> = self.propagations.iter().map(|p| p.latency_ms()).collect();
        v.sort_unstable();
        v[(v.len() * 95 / 100).min(v.len() - 1)]
    }

    pub fn propagation_p50_ms(&self) -> u64 {
        if self.propagations.is_empty() {
            return 0;
        }
        let mut v: Vec<u64> = self.propagations.iter().map(|p| p.latency_ms()).collect();
        v.sort_unstable();
        v[v.len() / 2]
    }
}
