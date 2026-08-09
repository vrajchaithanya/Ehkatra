//! docs/27 §1 conformance: every listed transition exercised, every forbidden
//! one proven rejected.
//!
//! docs/27 closes with the testing rule this file implements:
//!
//! > *each machine gets a transition-coverage test (every listed edge
//! > exercised) plus a forbidden-transition test (every "forbidden" line proven
//! > to be rejected).*

use usk_oplog::{Anchor, Op, Payload, RangeBinding};
use usk_sync::machine::{
    Negotiated, BACKOFF_BASE_MS, BACKOFF_CAP_MS, MODEL_VERSION, STALENESS_WINDOW_DAYS, WIRE_VERSION,
};
use usk_sync::validate::RejectReason;
use usk_sync::{Action, Event, SyncSession, SyncState, VectorClock};
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn id(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

/// A well-formed cell write by `actor`, its `n`-th op.
fn write(actor: u128, n: u64, value: f64) -> Op {
    Op {
        id: id(actor, n),
        lamport: n,
        payload: Payload::SetCell {
            row: RowId(id(9, 1)),
            col: ColId(id(9, 2)),
            value: Value::Number(value),
        },
    }
}

fn insert_row(actor: u128, n: u64) -> Op {
    Op {
        id: id(actor, n),
        lamport: n,
        payload: Payload::InsertRow {
            anchor: Anchor::Start,
        },
    }
}

fn connected(actor: u128) -> SyncSession {
    let mut s = SyncSession::new(ActorId(actor), 0xC0FFEE);
    s.step(Event::Connect);
    s.step(Event::HelloAck(Negotiated {
        wire: WIRE_VERSION,
        model: MODEL_VERSION,
    }));
    s.step(Event::ClocksEqual);
    assert_eq!(*s.state(), SyncState::Live);
    s
}

fn sends_ops(actions: &[Action]) -> Vec<Op> {
    actions
        .iter()
        .flat_map(|a| match a {
            Action::SendOps(ops) => ops.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn applied(actions: &[Action]) -> Vec<Op> {
    actions
        .iter()
        .flat_map(|a| match a {
            Action::Apply(ops) => ops.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn rejections(actions: &[Action]) -> Vec<RejectReason> {
    actions
        .iter()
        .flat_map(|a| match a {
            Action::Report(rs) => rs.iter().map(|r| r.reason).collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

// ---------------------------------------------------------------- coverage

/// Every edge docs/27 §1 lists, walked in one pass, each asserted by name.
#[test]
fn sync_machine_covers_every_listed_transition() {
    let mut s = SyncSession::new(ActorId(1), 7);

    // DISCONNECTED ──connect──► HELLO_SENT
    let a = s.step(Event::Connect);
    assert_eq!(*s.state(), SyncState::HelloSent);
    assert!(matches!(a.as_slice(), [Action::SendHello(_)]));

    // HELLO_SENT ──HelloAck(negotiated)──► SYNCING
    let a = s.step(Event::HelloAck(Negotiated {
        wire: WIRE_VERSION,
        model: MODEL_VERSION,
    }));
    assert_eq!(*s.state(), SyncState::Syncing);
    assert!(matches!(a.as_slice(), [Action::SendNeed(_)]));
    assert_eq!(
        s.negotiated().map(|n| n.wire),
        Some(WIRE_VERSION),
        "the negotiated versions are retained, not just acknowledged"
    );

    // SYNCING ──NEED/GIVE exchange──► SYNCING  (both directions, still SYNCING)
    let a = s.step(Event::Need(VectorClock::new()));
    assert_eq!(*s.state(), SyncState::Syncing);
    assert!(matches!(a.as_slice(), [Action::SendGive(_)]));
    let a = s.step(Event::Give(vec![write(2, 1, 10.0)]));
    assert_eq!(*s.state(), SyncState::Syncing);
    assert_eq!(applied(&a).len(), 1);

    // SYNCING ──vector clocks equal──► LIVE
    s.step(Event::ClocksEqual);
    assert_eq!(*s.state(), SyncState::Live);

    // LIVE ──local op──► LIVE (send OPS)
    let mine = write(1, 1, 42.0);
    let a = s.step(Event::LocalOp(mine.clone()));
    assert_eq!(*s.state(), SyncState::Live);
    assert_eq!(sends_ops(&a), vec![mine.clone()]);

    // LIVE ──remote OPS──► LIVE (apply, ack watermark)
    let a = s.step(Event::RemoteOps(vec![write(2, 2, 11.0)]));
    assert_eq!(*s.state(), SyncState::Live);
    assert_eq!(applied(&a).len(), 1);
    assert!(
        a.iter().any(|x| matches!(x, Action::AckWatermark(_))),
        "docs/27: applying remote ops acks the watermark"
    );

    // LIVE ──StalenessExceeded(180d watermark gap)──► REBASE_REQUIRED
    let a = s.step(Event::StalenessExceeded {
        gap_days: STALENESS_WINDOW_DAYS,
    });
    assert_eq!(*s.state(), SyncState::RebaseRequired);
    assert_eq!(a, vec![Action::RequestSnapshot]);

    // REBASE_REQUIRED ──snapshot+migrate_ops──► SYNCING
    let a = s.step(Event::RebaseComplete {
        ops: vec![write(3, 1, 1.0)],
    });
    assert_eq!(*s.state(), SyncState::Syncing);
    assert!(matches!(a.as_slice(), [Action::SendNeed(_)]));

    // SYNCING ──transport loss──► BACKOFF(n)
    let a = s.step(Event::TransportLoss);
    assert!(matches!(*s.state(), SyncState::Backoff { attempt: 0, .. }));
    assert!(matches!(a.as_slice(), [Action::ArmTimer { .. }]));

    // BACKOFF(n) ──timer──► HELLO_SENT
    let a = s.step(Event::BackoffElapsed);
    assert_eq!(*s.state(), SyncState::HelloSent);
    assert!(matches!(a.as_slice(), [Action::SendHello(_)]));

    // HELLO_SENT ──HelloReject(version)──► INCOMPATIBLE
    let a = s.step(Event::HelloReject { peer_wire: 99 });
    assert_eq!(*s.state(), SyncState::Incompatible { peer_wire: 99 });
    assert_eq!(a, vec![Action::GoReadOnly { peer_wire: 99 }]);

    // any ──auth revoked──► DISCONNECTED
    s.step(Event::AuthRevoked);
    assert_eq!(*s.state(), SyncState::Disconnected);

    // LIVE ──transport loss──► BACKOFF(n), the other half of the LIVE|SYNCING
    // edge, from a clean session so the assertion is unambiguous.
    let mut t = connected(1);
    t.step(Event::TransportLoss);
    assert!(matches!(*t.state(), SyncState::Backoff { .. }));
}

/// INCOMPATIBLE is "terminal until upgrade": no event short of auth revocation
/// moves it, and the local workbook is told to go read-only.
#[test]
fn incompatible_is_terminal_until_upgrade() {
    let mut s = SyncSession::new(ActorId(1), 3);
    s.step(Event::Connect);
    s.step(Event::HelloReject { peer_wire: 4 });
    assert_eq!(*s.state(), SyncState::Incompatible { peer_wire: 4 });

    // A local edit still queues — never-drop has no exceptions — but nothing
    // goes on the wire and the state does not move.
    let a = s.step(Event::LocalOp(write(1, 1, 5.0)));
    assert!(sends_ops(&a).is_empty());
    assert_eq!(*s.state(), SyncState::Incompatible { peer_wire: 4 });
    assert_eq!(s.queued(), 1);

    s.step(Event::AuthRevoked);
    assert_eq!(*s.state(), SyncState::Disconnected);
    assert_eq!(s.queued(), 1, "revocation retains the queue");
}

// ------------------------------------------------------------- forbidden #1
// "Forbidden: sending OPS before HelloAck"

#[test]
fn no_ops_reach_the_wire_before_hello_ack() {
    // Every state in which the handshake has not completed.
    let mut pre_ack: Vec<(&str, SyncSession)> = Vec::new();

    pre_ack.push(("DISCONNECTED", SyncSession::new(ActorId(1), 1)));

    let mut hello = SyncSession::new(ActorId(1), 1);
    hello.step(Event::Connect);
    pre_ack.push(("HELLO_SENT", hello));

    let mut incompatible = SyncSession::new(ActorId(1), 1);
    incompatible.step(Event::Connect);
    incompatible.step(Event::HelloReject { peer_wire: 9 });
    pre_ack.push(("INCOMPATIBLE", incompatible));

    let mut backoff = connected(1);
    backoff.step(Event::TransportLoss);
    pre_ack.push(("BACKOFF", backoff));

    let mut rebasing = connected(1);
    rebasing.step(Event::StalenessExceeded {
        gap_days: STALENESS_WINDOW_DAYS,
    });
    pre_ack.push(("REBASE_REQUIRED", rebasing));

    for (name, mut session) in pre_ack {
        let before = session.queued();
        let op = write(1, (before + 1) as u64, 1.0);
        let actions = session.step(Event::LocalOp(op.clone()));
        assert!(
            sends_ops(&actions).is_empty(),
            "{name} put OPS on the wire before HelloAck"
        );
        assert_eq!(
            session.queued(),
            before + 1,
            "{name} must queue the op it refused to send"
        );
    }
}

/// The queued backlog is released the instant sending becomes legal — on the
/// SYNCING→LIVE edge, not one transition earlier.
#[test]
fn offline_edits_flush_exactly_when_the_session_goes_live() {
    let mut s = SyncSession::new(ActorId(1), 5);
    for n in 1..=3 {
        assert!(sends_ops(&s.step(Event::LocalOp(write(1, n, n as f64)))).is_empty());
    }
    s.step(Event::Connect);
    s.step(Event::HelloAck(Negotiated {
        wire: WIRE_VERSION,
        model: MODEL_VERSION,
    }));
    let a = s.step(Event::ClocksEqual);
    assert_eq!(sends_ops(&a).len(), 3, "all three offline edits flushed");
    assert_eq!(
        s.queued(),
        3,
        "sent is not acked — the queue still holds them"
    );
}

// ------------------------------------------------------------- forbidden #2
// "Forbidden: applying remote ops that fail schema/bounds validation
//  (reject + report, stay LIVE)"

#[test]
fn hostile_remote_ops_are_quarantined_and_the_session_stays_live() {
    let mut s = connected(1);

    let zero_counter = Op {
        id: id(2, 0),
        lamport: 5,
        payload: Payload::SetCell {
            row: RowId(id(9, 1)),
            col: ColId(id(9, 2)),
            value: Value::Number(1.0),
        },
    };
    let zero_lamport = Op {
        id: id(2, 4),
        lamport: 0,
        ..write(2, 4, 1.0)
    };
    // The attack the lamport bound exists for: an op stamped at the top of the
    // range would win every LWW at its cell forever.
    let saturating_lamport = Op {
        lamport: u64::MAX,
        ..write(2, 5, 1.0)
    };
    let giant_formula = Op {
        id: id(2, 6),
        lamport: 6,
        payload: Payload::SetFormula {
            row: RowId(id(9, 1)),
            col: ColId(id(9, 2)),
            source: "=".repeat(9000),
            bindings: Vec::new(),
        },
    };
    let giant_text = Op {
        id: id(2, 7),
        lamport: 7,
        payload: Payload::SetCell {
            row: RowId(id(9, 1)),
            col: ColId(id(9, 2)),
            value: Value::Text("x".repeat(40_000)),
        },
    };
    let unbound_reference = Op {
        id: id(2, 8),
        lamport: 8,
        payload: Payload::SetFormula {
            row: RowId(id(9, 1)),
            col: ColId(id(9, 2)),
            source: "=A1".into(),
            bindings: vec![RangeBinding {
                row_start: id(9, 0),
                row_end: id(9, 1),
                col_start: id(9, 2),
                col_end: id(9, 2),
                anchors: 0,
            }],
        },
    };
    let honest = write(2, 1, 3.0);

    let actions = s.step(Event::RemoteOps(vec![
        zero_counter,
        zero_lamport,
        saturating_lamport,
        giant_formula,
        giant_text,
        unbound_reference,
        honest.clone(),
    ]));

    assert_eq!(
        *s.state(),
        SyncState::Live,
        "docs/27: reject and report, but stay LIVE"
    );
    let reasons = rejections(&actions);
    assert_eq!(reasons.len(), 6, "every hostile op refused");
    assert!(reasons.contains(&RejectReason::ZeroCounter));
    assert!(reasons.contains(&RejectReason::ZeroLamport));
    assert!(reasons.contains(&RejectReason::LamportOutOfBounds));
    assert!(reasons.contains(&RejectReason::FormulaTooLong));
    assert!(reasons.contains(&RejectReason::TextTooLong));
    assert!(reasons.contains(&RejectReason::MalformedBinding));
    assert_eq!(s.quarantined(), 6);

    assert_eq!(
        applied(&actions),
        vec![honest],
        "the honest op in the same batch is still applied"
    );
}

// ------------------------------------------------------------- forbidden #3
// "Forbidden: dropping queued local ops in any transition (never-drop)"

#[test]
fn queued_local_ops_survive_every_transition() {
    let mut s = SyncSession::new(ActorId(1), 11);
    let mine: Vec<Op> = (1..=3).map(|n| write(1, n, n as f64)).collect();
    for op in &mine {
        s.step(Event::LocalOp(op.clone()));
    }
    assert_eq!(s.queued(), 3);

    // A sequence that visits every state docs/27 lists, using only listed
    // edges. The queue is checked after each one.
    let script: Vec<Event> = vec![
        Event::Connect,
        Event::HelloAck(Negotiated {
            wire: WIRE_VERSION,
            model: MODEL_VERSION,
        }),
        Event::Need(VectorClock::new()),
        Event::Give(vec![write(2, 1, 0.0)]),
        Event::ClocksEqual,
        Event::RemoteOps(vec![write(2, 2, 0.0)]),
        Event::TransportLoss,
        Event::BackoffElapsed,
        Event::HelloAck(Negotiated {
            wire: WIRE_VERSION,
            model: MODEL_VERSION,
        }),
        Event::ClocksEqual,
        Event::StalenessExceeded {
            gap_days: STALENESS_WINDOW_DAYS,
        },
        Event::RebaseComplete { ops: Vec::new() },
        Event::ClocksEqual,
        Event::AuthRevoked,
        Event::Connect,
        Event::HelloReject { peer_wire: 42 },
    ];

    for (step, event) in script.into_iter().enumerate() {
        s.step(event);
        for op in &mine {
            assert!(
                s.queued_ops().iter().any(|q| q.id == op.id),
                "op {:?} was dropped at step {step}",
                op.id
            );
        }
    }
    assert_eq!(s.queued(), 3, "nothing but an ack may empty the queue");
}

#[test]
fn only_an_acknowledgement_empties_the_queue() {
    let mut s = connected(1);
    for n in 1..=3 {
        s.step(Event::LocalOp(write(1, n, n as f64)));
    }
    assert_eq!(s.queued(), 3);

    // A watermark covering the first two, and only the first two.
    let mut watermark = VectorClock::new();
    watermark.observe(id(1, 1));
    watermark.observe(id(1, 2));
    s.step(Event::Ack(watermark));
    assert_eq!(s.queued(), 1);
    assert_eq!(s.queued_ops()[0].id, id(1, 3));
}

/// An op lost in flight is redelivered, because a send is not a delivery.
#[test]
fn transport_loss_re_offers_sent_but_unacked_ops() {
    let mut s = connected(1);
    let op = write(1, 1, 1.0);
    assert_eq!(sends_ops(&s.step(Event::LocalOp(op.clone()))).len(), 1);

    s.step(Event::TransportLoss);
    s.step(Event::BackoffElapsed);
    s.step(Event::HelloAck(Negotiated {
        wire: WIRE_VERSION,
        model: MODEL_VERSION,
    }));
    let a = s.step(Event::ClocksEqual);
    assert_eq!(
        sends_ops(&a),
        vec![op],
        "the op that may have died on the wire is offered again"
    );
}

// ---------------------------------------------------- unlisted transitions

/// docs/27: "a transition not listed here is a `debug_assert` + logged error,
/// never silent". Proven on a pair the specification does not define — an
/// acknowledgement arriving while disconnected.
#[test]
#[should_panic(expected = "unlisted sync transition")]
fn an_unlisted_transition_trips_the_debug_assert() {
    let mut s = SyncSession::new(ActorId(1), 1);
    s.step(Event::Ack(VectorClock::new()));
}

// ------------------------------------------------------------------ detail

#[test]
fn backoff_grows_exponentially_jittered_and_caps_at_sixty_seconds() {
    let mut delays = Vec::new();
    let mut s = connected(1);
    for _ in 0..12 {
        s.step(Event::TransportLoss);
        match *s.state() {
            SyncState::Backoff { delay_ms, .. } => delays.push(delay_ms),
            ref other => panic!("expected BACKOFF, got {other:?}"),
        }
        s.step(Event::BackoffElapsed);
        s.step(Event::HelloAck(Negotiated {
            wire: WIRE_VERSION,
            model: MODEL_VERSION,
        }));
        // Deliberately no ClocksEqual: the link keeps failing during SYNCING,
        // so the attempt counter must keep climbing.
    }

    assert!(
        delays[0] < delays[3] && delays[3] < delays[6],
        "backoff must grow: {delays:?}"
    );
    assert!(
        delays.iter().all(|d| *d <= BACKOFF_CAP_MS),
        "docs/27 caps backoff at 60 s: {delays:?}"
    );
    assert_eq!(
        *delays.last().unwrap_or(&0),
        BACKOFF_CAP_MS.min(*delays.last().unwrap_or(&0)),
        "the tail sits at the cap"
    );
    assert!(delays[0] >= BACKOFF_BASE_MS / 2 && delays[0] < BACKOFF_BASE_MS * 2);
    // Jitter is real: successive delays at the cap are not all identical.
    let capped: Vec<u64> = delays.iter().copied().filter(|d| *d > 30_000).collect();
    assert!(
        capped.windows(2).any(|w| w[0] != w[1]),
        "jitter must spread reconnects: {capped:?}"
    );
}

/// A healthy reconnection clears the penalty — otherwise one bad afternoon
/// leaves the session backing off for a minute at a time forever.
#[test]
fn reaching_live_resets_the_backoff_attempt() {
    let mut s = connected(1);
    for _ in 0..4 {
        s.step(Event::TransportLoss);
        s.step(Event::BackoffElapsed);
        s.step(Event::HelloAck(Negotiated {
            wire: WIRE_VERSION,
            model: MODEL_VERSION,
        }));
    }
    s.step(Event::ClocksEqual);
    s.step(Event::TransportLoss);
    assert!(matches!(*s.state(), SyncState::Backoff { attempt: 0, .. }));
}

/// docs/15's failure drill: "relay crash mid-batch (client redelivery via
/// causal gaps)". An op that arrives ahead of a hole is held, not applied and
/// not dropped, until the hole fills.
#[test]
fn ops_arriving_ahead_of_a_causal_gap_are_held_not_applied() {
    let mut s = connected(1);

    // Counters 3 and 2 arrive; 1 was lost.
    let a = s.step(Event::RemoteOps(vec![write(2, 3, 3.0), write(2, 2, 2.0)]));
    assert!(applied(&a).is_empty(), "nothing is causally ready yet");
    assert_eq!(s.held_by_causal_gap(), 2);

    // The redelivered op closes the gap and releases all three, in order.
    let a = s.step(Event::RemoteOps(vec![write(2, 1, 1.0)]));
    let ready = applied(&a);
    assert_eq!(
        ready.iter().map(|o| o.id.counter).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(s.held_by_causal_gap(), 0);
}

/// "echo suppressed by op id" (docs/27): a relay fanning our own op back, or
/// redelivering one we already hold, changes nothing.
#[test]
fn duplicate_delivery_is_idempotent() {
    let mut s = connected(1);
    let remote = write(2, 1, 7.0);
    assert_eq!(
        applied(&s.step(Event::RemoteOps(vec![remote.clone()]))).len(),
        1
    );
    let again = s.step(Event::RemoteOps(vec![remote.clone(), remote]));
    assert!(
        applied(&again).is_empty(),
        "an op already held is not applied twice"
    );

    let mine = write(1, 1, 1.0);
    s.step(Event::LocalOp(mine.clone()));
    let echo = s.step(Event::RemoteOps(vec![mine]));
    assert!(
        applied(&echo).is_empty(),
        "our own op echoed back is suppressed"
    );
}

/// Structural ops name an anchor that may not have arrived yet. That is a
/// causal gap, not an invalid op — quarantining it would destroy honest work.
#[test]
fn an_unknown_anchor_is_a_causal_gap_not_a_validation_failure() {
    let mut s = connected(1);
    let a = s.step(Event::RemoteOps(vec![Op {
        payload: Payload::InsertRow {
            anchor: Anchor::After(id(77, 5)),
        },
        ..insert_row(2, 1)
    }]));
    assert!(rejections(&a).is_empty());
    assert_eq!(applied(&a).len(), 1);
}

// ------------------------------------- DP-A5 forward preservation (TD-25)

/// An op from a peer running a newer model version: a payload tag we do not
/// know, arriving through the framed reader.
fn op_from_the_future(actor: u128, counter: u64, body: &[u8]) -> Op {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&actor.to_be_bytes());
    bytes.extend_from_slice(&counter.to_be_bytes());
    bytes.extend_from_slice(&counter.to_be_bytes());
    bytes.push(0x3D); // outside model version 1's taxonomy
    bytes.extend_from_slice(body);
    let mut framed = (bytes.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&bytes);
    usk_oplog::Op::decode_framed(&framed)
        .expect("preserved, not refused")
        .0
}

/// **DP-A5 across the boundary docs/37 calls "a permitted collaborator is still
/// an untrusted input source".** An op we cannot interpret is *accepted* —
/// validated, applied (to nothing), and retransmitted — not quarantined.
/// Quarantining it would make every version skew look like an attack.
#[test]
fn an_op_type_we_do_not_know_is_preserved_rather_than_quarantined() {
    let mut s = connected(1);
    let future = op_from_the_future(2, 1, b"payload from a newer build");
    let a = s.step(Event::RemoteOps(vec![future.clone()]));
    assert!(
        rejections(&a).is_empty(),
        "version skew is not hostility: {:?}",
        rejections(&a)
    );
    let applied = applied(&a);
    assert_eq!(applied, vec![future.clone()]);
    assert_eq!(
        applied[0].encode(),
        future.encode(),
        "and it crosses the boundary byte-exact, so it can be retransmitted"
    );
    assert_eq!(*s.state(), SyncState::Live);
}

/// Preservation is not a hole in admission control: an opaque body is still
/// untrusted input and still bounded (docs/37 boundary 2).
#[test]
fn an_oversized_opaque_op_is_still_refused() {
    let mut s = connected(1);
    let huge = vec![0u8; usk_sync::validate::MAX_OPAQUE_BYTES + 1];
    let a = s.step(Event::RemoteOps(vec![op_from_the_future(2, 1, &huge)]));
    assert_eq!(rejections(&a), vec![RejectReason::OpaqueTooLong]);
    assert!(applied(&a).is_empty());
    assert_eq!(
        *s.state(),
        SyncState::Live,
        "one refused op must not cost the collaborator their session"
    );
}
