//! End-to-end sync: replicas + relay + a lossy link, proven by state hash.
//!
//! BOOTSTRAP row 10's proof obligation is "divergence test = hash equality",
//! and these run it through the real composition — reducer, op log, CRDT state,
//! sync machine, admission control and framing — rather than a mock of it.

use ehkatra_relay::bus::Bus;
use ehkatra_relay::frame::Message;
use ehkatra_relay::replica::Replica;
use usk_reduce::Command;
use usk_types::{ActorId, Value};

fn grid(bus: &mut Bus, rows: u32, cols: u32) {
    for _ in 0..rows {
        bus.edit(0, Command::InsertRow { before: 0 });
    }
    for _ in 0..cols {
        bus.edit(0, Command::InsertCol { before: 0 });
    }
    bus.settle(400);
}

/// The headline: two replicas, concurrent edits, agreeing hashes.
#[test]
fn two_replicas_converge_through_a_relay() {
    let mut bus = Bus::new(2, 0xA11CE, 5, 0);
    bus.connect_all();
    assert!(
        bus.replicas.iter().all(|r| r.is_live()),
        "handshake completed"
    );

    grid(&mut bus, 6, 3);

    // Both write, into overlapping and disjoint cells, without waiting.
    for r in 0..6 {
        bus.edit(
            0,
            Command::SetValue {
                row: r,
                col: 0,
                value: Value::Number(r as f64),
            },
        );
        bus.edit(
            1,
            Command::SetValue {
                row: r,
                col: 1,
                value: Value::Number(100.0 + r as f64),
            },
        );
    }
    // The contested cell: both replicas write A1 concurrently.
    bus.edit(
        0,
        Command::SetValue {
            row: 0,
            col: 0,
            value: Value::Text(String::from("alice")),
        },
    );
    bus.edit(
        1,
        Command::SetValue {
            row: 0,
            col: 0,
            value: Value::Text(String::from("bob")),
        },
    );

    bus.settle(600);
    assert!(bus.converged(), "replicas diverged: {:?}", bus.hashes());
    assert!(
        bus.replicas.iter().all(|r| r.unacked().is_empty()),
        "every op was acknowledged"
    );
}

/// The canonical CRDT case (docs/15), now over the wire: Alice inserts a row
/// inside the span Bob is writing into, concurrently.
#[test]
fn a_concurrent_row_insert_against_a_sum_converges_over_the_wire() {
    let mut bus = Bus::new(2, 0xB0B, 5, 0);
    bus.connect_all();
    grid(&mut bus, 5, 3);
    for r in 0..5 {
        bus.edit(
            0,
            Command::SetValue {
                row: r,
                col: 0,
                value: Value::Number((r as f64 + 1.0) * 10.0),
            },
        );
    }
    bus.settle(400);
    bus.edit(
        1,
        Command::SetFormula {
            row: 0,
            col: 2,
            source: String::from("=SUM(A1:A5)"),
        },
    );
    bus.settle(400);

    // Concurrently: Alice inserts inside the span, Bob overwrites a cell in it.
    bus.edit(0, Command::InsertRow { before: 2 });
    bus.edit(
        1,
        Command::SetValue {
            row: 3,
            col: 0,
            value: Value::Number(999.0),
        },
    );
    bus.settle(600);

    assert!(bus.converged(), "diverged: {:?}", bus.hashes());
    let rows0 = bus.replicas[0].doc.state().row_order();
    let rows1 = bus.replicas[1].doc.state().row_order();
    assert_eq!(rows0, rows1, "both replicas see the same axis order");
    assert_eq!(rows0.len(), 6, "the inserted row is present exactly once");

    // The formula still resolves, and to the same answer on both sides.
    let cols = bus.replicas[0].doc.state().col_order();
    let a = bus.replicas[0].value(rows0[0], cols[2]);
    let b = bus.replicas[1].value(rows1[0], cols[2]);
    assert_eq!(a, b, "the formula agrees across replicas");
    assert!(
        matches!(a, Some(Value::Number(_)) | Some(Value::Decimal(_))),
        "the sum is a computed number, not an error: {a:?}"
    );
}

/// docs/38's W-SYNC-RELAY loss condition, as a correctness test: 1% of frames
/// die, taking their connection with them, and the session still converges.
#[test]
fn convergence_survives_packet_loss_and_reconnection() {
    let mut bus = Bus::new(3, 0x105551, 5, 10); // 10‰ = 1%
    bus.connect_all();
    grid(&mut bus, 8, 4);

    for round in 0..12u32 {
        for replica in 0..3u32 {
            bus.edit(
                replica as usize,
                Command::SetValue {
                    row: round % 8,
                    col: replica,
                    value: Value::Number((round * 10 + replica) as f64),
                },
            );
        }
        bus.settle(200);
    }
    bus.settle(4000);

    assert!(bus.dropped > 0, "the test must actually lose frames");
    assert!(bus.reconnects > 0, "and must actually reconnect");
    let (drops, reconnects) = (bus.dropped, bus.reconnects);
    let hashes = bus.hashes();
    assert!(
        bus.converged(),
        "diverged after {drops} drops / {reconnects} reconnects: {hashes:?}"
    );
    for (i, r) in bus.replicas.iter().enumerate() {
        assert!(
            r.unacked().is_empty(),
            "replica {i} still holds unacked ops"
        );
        assert_eq!(r.unexpected, 0, "replica {i} took an unlisted transition");
    }
}

/// Never-drop across a process death: a replica is killed with unacknowledged
/// ops, rebuilt from its durable log, and every op still arrives.
///
/// The "durable log" here is the in-memory op vector, because Row 11 owns the
/// container file and its fsync contract. What this proves is the *protocol*
/// half — that recovery re-offers unacknowledged work rather than forgetting
/// it. The storage half is Row 11's to prove, and is not claimed here.
#[test]
fn a_killed_replica_loses_no_queued_op() {
    let mut bus = Bus::new(2, 0xDEAD, 5, 0);
    bus.connect_all();
    grid(&mut bus, 4, 2);

    // Partition replica 1 by dropping every frame it would exchange: it edits
    // while unable to reach the relay.
    bus.replicas[1].transport_loss();
    for r in 0..4u32 {
        bus.replicas[1]
            .edit(Command::SetValue {
                row: r,
                col: 1,
                value: Value::Number(500.0 + r as f64),
            })
            .expect("offline edit");
    }
    let queued = bus.replicas[1].unacked();
    assert_eq!(queued.len(), 4, "offline edits queued, not sent");

    // KILL: the process dies. All that survives is the durable log.
    let durable_log = bus.replicas[1].log();
    let durable_queue = bus.replicas[1].unacked();
    let actor = bus.replicas[1].actor();
    bus.replicas[1] = Replica::recover(actor, 7, &durable_log, &durable_queue);
    assert_eq!(
        bus.replicas[1].unacked().len(),
        4,
        "recovery must re-queue every unacknowledged op"
    );

    // Reconnect and let anti-entropy do its work.
    bus.reconnect(1);
    bus.settle(1200);

    assert!(
        bus.replicas[1].unacked().is_empty(),
        "the survivor's queue drained after reconnect"
    );
    let peer_log = bus.replicas[0].log();
    for op in &queued {
        assert!(
            peer_log.iter().any(|o| o.id == op.id),
            "op {:?} authored before the kill never reached the peer",
            op.id
        );
    }
    assert!(
        bus.converged(),
        "diverged after recovery: {:?}",
        bus.hashes()
    );
}

/// The relay refuses ops that claim someone else's identity, and keeps serving
/// the connection that sent them (docs/37 boundary 2).
#[test]
fn the_relay_refuses_spoofed_and_hostile_ops() {
    let mut bus = Bus::new(2, 1, 5, 0);
    bus.connect_all();
    grid(&mut bus, 2, 2);

    // Replica 0's socket submits an op signed by replica 1.
    let forged = {
        let mut r = Replica::new(ActorId(2), 1);
        r.doc.apply(Command::InsertRow { before: 0 }).expect("op");
        r.log()[0].clone()
    };
    let out = bus
        .relay
        .handle(bus.replicas[0].actor(), Message::Ops(vec![forged]), 0);
    assert!(
        out.to_others.is_empty(),
        "a spoofed op must not be fanned out"
    );
    assert_eq!(bus.relay.stats.spoofed, 1);

    // The connection is still usable.
    bus.edit(
        0,
        Command::SetValue {
            row: 0,
            col: 0,
            value: Value::Number(1.0),
        },
    );
    bus.settle(400);
    assert!(bus.converged());
}

/// A peer announcing an unsupported wire version is told so, and goes
/// read-only locally rather than silently mis-parsing (docs/27 INCOMPATIBLE).
#[test]
fn an_unsupported_wire_version_is_rejected_into_read_only() {
    let mut bus = Bus::new(1, 1, 5, 0);
    let actor = bus.replicas[0].actor();
    let hello = match bus.replicas[0].connect().into_iter().next() {
        Some(Message::Hello(mut h)) => {
            h.wire = 9;
            Message::Hello(h)
        }
        other => panic!("expected HELLO, got {other:?}"),
    };
    let out = bus.relay.handle(actor, hello, 0);
    assert_eq!(
        out.to_sender,
        vec![Message::HelloReject { peer_wire: 1 }],
        "the relay names the version it does speak"
    );
    bus.replicas[0].receive(Message::HelloReject { peer_wire: 1 });
    assert!(bus.replicas[0].read_only, "INCOMPATIBLE is read-only local");
    assert!(bus.replicas[0]
        .edit(Command::InsertRow { before: 0 })
        .is_err());
}

/// Regression for the wedge W-SYNC-RELAY found at 50 replicas.
///
/// docs/27 §1 defines no transition for transport loss during HELLO_SENT
/// (D-064). The shell's first answer was to do nothing, which left a replica
/// that lost its link mid-handshake stuck there permanently — no timer, no
/// event, no way out. The whole test suite passed; only a run with thousands of
/// dropped frames exposed it. This is that run, compressed to the one moment
/// that matters.
#[test]
fn a_replica_that_loses_its_link_mid_handshake_recovers() {
    let mut bus = Bus::new(2, 0x11AD, 5, 0);
    bus.connect_all();
    grid(&mut bus, 3, 2);

    // Walk the victim to HELLO_SENT, then kill the link exactly there.
    bus.replicas[1].transport_loss();
    assert!(matches!(
        bus.replicas[1].sync.state(),
        usk_sync::SyncState::Backoff { .. }
    ));
    let _hello_lost_in_the_void = bus.replicas[1].resume();
    assert!(matches!(
        bus.replicas[1].sync.state(),
        usk_sync::SyncState::HelloSent
    ));
    bus.replicas[1].transport_loss();

    assert_eq!(
        bus.replicas[1].resets, 1,
        "the session must be torn down and rebuilt, not left wedged"
    );
    assert!(
        bus.replicas[1].retry_at_ms.is_some(),
        "and it must be armed to reconnect — the wedge was the missing timer"
    );

    // Work authored while wedged must still arrive.
    bus.replicas[1]
        .edit(Command::SetValue {
            row: 0,
            col: 1,
            value: Value::Number(7.0),
        })
        .expect("edit while disconnected");
    assert_eq!(bus.replicas[1].unacked().len(), 1);

    bus.settle(3000);
    assert!(bus.replicas[1].is_live(), "the replica reconnected");
    assert!(
        bus.replicas[1].unacked().is_empty(),
        "the op authored while wedged was delivered and acknowledged"
    );
    assert!(bus.converged(), "diverged: {:?}", bus.hashes());
}

/// A partition is not a dropped frame, and the difference is measurable.
///
/// The mid-run-kill harness first modelled "offline" as one transport loss plus
/// a long retry timer. Under 1% loss that is not offline at all: the next
/// dropped frame runs the teardown-and-reconnect path, arms a fresh 500 ms
/// timer, and the replica rejoins. At 50 replicas it rejoined and drained its
/// queue *before* the kill, so W-SYNC-RELAY reported 2 queued ops where it
/// should have reported ~30 — a durability measurement that silently measured
/// nothing. This pins the semantics the harness now relies on.
#[test]
fn a_partitioned_replica_stays_offline_and_keeps_its_queue() {
    let mut bus = Bus::new(2, 0xDEADBEEF, 5, 50); // 5% loss, to provoke the old bug
    bus.connect_all();
    grid(&mut bus, 4, 2);

    bus.partition(1);
    assert!(bus.is_partitioned(1));

    for r in 0..6u32 {
        bus.replicas[1]
            .edit(Command::SetValue {
                row: r % 4,
                col: 1,
                value: Value::Number(r as f64),
            })
            .expect("offline edits are still edits");
        bus.advance(100);
    }

    assert!(
        !bus.replicas[1].is_live(),
        "a partitioned replica must not rejoin"
    );
    assert_eq!(
        bus.replicas[1].unacked().len(),
        6,
        "and must still be holding every op it authored"
    );
    // The peer cannot have seen any of it.
    let peer_log = bus.replicas[0].log();
    for op in bus.replicas[1].unacked() {
        assert!(!peer_log.iter().any(|o| o.id == op.id));
    }

    bus.heal(1);
    bus.settle(4000);
    assert!(bus.replicas[1].unacked().is_empty(), "the backlog drained");
    assert!(
        bus.converged(),
        "diverged after healing: {:?}",
        bus.hashes()
    );
}

/// Fanout to many replicas: fifty peers, one relay, all converge.
#[test]
fn fifty_replicas_converge() {
    let mut bus = Bus::new(50, 0x50, 2, 0);
    bus.connect_all();
    grid(&mut bus, 4, 2);
    for i in 0..50usize {
        bus.edit(
            i,
            Command::SetValue {
                row: (i % 4) as u32,
                col: (i % 2) as u32,
                value: Value::Number(i as f64),
            },
        );
    }
    bus.settle(3000);
    assert!(bus.converged(), "50-replica divergence");
    assert_eq!(bus.relay.stats.spoofed, 0);
    assert_eq!(bus.relay.stats.invalid, 0);
}
