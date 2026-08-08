//! docs/27 §2 conformance and docs/16's salvage contract.
//!
//! Same obligation docs/27 closes with, applied to the second machine: every
//! listed edge exercised, every forbidden line proven rejected.

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_recover::machine::{Action, DocState, Document, Event};
use usk_recover::salvage::{recover, SalvageReason};
use usk_recover::snapshot::{Snapshot, SnapshotFault, Watermark};
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn id(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

/// A small workbook: two rows, one column, values in both cells.
fn workbook() -> OpLog {
    let mut log = OpLog::new();
    let r1 = id(1, 1);
    let r2 = id(1, 2);
    let c1 = id(1, 3);
    log.append(Op {
        id: r1,
        lamport: 1,
        payload: Payload::InsertRow {
            anchor: Anchor::Start,
        },
    });
    log.append(Op {
        id: r2,
        lamport: 2,
        payload: Payload::InsertRow {
            anchor: Anchor::After(r1),
        },
    });
    log.append(Op {
        id: c1,
        lamport: 3,
        payload: Payload::InsertCol {
            anchor: Anchor::Start,
        },
    });
    log.append(Op {
        id: id(1, 4),
        lamport: 4,
        payload: Payload::SetCell {
            row: RowId(r1),
            col: ColId(c1),
            value: Value::Number(10.0),
        },
    });
    log.append(Op {
        id: id(1, 5),
        lamport: 5,
        payload: Payload::SetCell {
            row: RowId(r2),
            col: ColId(c1),
            value: Value::Text(String::from("two")),
        },
    });
    log
}

/// Ops appended after the snapshot — the WAL tail a crash catches mid-write.
fn tail_ops() -> Vec<Op> {
    vec![
        Op {
            id: id(2, 1),
            lamport: 6,
            payload: Payload::SetCell {
                row: RowId(id(1, 1)),
                col: ColId(id(1, 3)),
                value: Value::Number(99.0),
            },
        },
        Op {
            id: id(2, 2),
            lamport: 7,
            payload: Payload::InsertRow {
                anchor: Anchor::Start,
            },
        },
    ]
}

fn tail_bytes(ops: &[Op]) -> Vec<u8> {
    let mut out = Vec::new();
    for op in ops {
        out.extend_from_slice(&op.encode());
    }
    out
}

// ------------------------------------------------------------------ snapshot

#[test]
fn a_snapshot_proves_itself_by_replay_not_by_checksum() {
    let snapshot = Snapshot::build(&workbook());
    let verified = snapshot
        .verify()
        .expect("a freshly built snapshot verifies");
    assert_eq!(verified.ops().len(), 5);
    assert_eq!(verified.watermark(), &Watermark::of(verified.ops()));

    // Content addressing is over the body, and is stable.
    assert_eq!(
        snapshot.content_hash(),
        Snapshot::build(&workbook()).content_hash()
    );
}

/// The check docs/26 asks for: a snapshot whose bytes survived but whose
/// *meaning* changed must not load. Flipping a value byte keeps the body
/// decodable and breaks the state hash — the case a checksum would also catch,
/// but for the wrong reason.
#[test]
fn a_tampered_body_fails_the_state_hash_check() {
    let mut snapshot = Snapshot::build(&workbook());
    let last = snapshot.body.len() - 1;
    snapshot.body[last] ^= 0xFF;
    match snapshot.verify() {
        Err(SnapshotFault::StateHashMismatch) | Err(SnapshotFault::Undecodable { .. }) => {}
        other => panic!("tampered snapshot must not verify: {other:?}"),
    }
}

/// A snapshot cannot be told what it hashes to. This is the property that makes
/// `VerifiedSnapshot` mean something.
#[test]
fn a_snapshot_with_a_forged_state_hash_is_refused() {
    let mut snapshot = Snapshot::build(&workbook());
    snapshot.state_hash = [0u8; 32];
    assert_eq!(
        snapshot.verify().err(),
        Some(SnapshotFault::StateHashMismatch)
    );
}

#[test]
fn a_snapshot_whose_watermark_lies_is_refused() {
    let mut snapshot = Snapshot::build(&workbook());
    snapshot.watermark = Watermark::of(&tail_ops());
    assert_eq!(
        snapshot.verify().err(),
        Some(SnapshotFault::WatermarkMismatch)
    );
}

#[test]
fn a_truncated_snapshot_body_names_the_offset_it_died_at() {
    let good = Snapshot::build(&workbook());
    let mut snapshot = good.clone();
    snapshot.body.truncate(good.body.len() - 3);
    match snapshot.verify() {
        Err(SnapshotFault::Undecodable { at_offset, .. }) => {
            assert!(at_offset < good.body.len());
        }
        other => panic!("expected an offset-bearing fault, got {other:?}"),
    }
}

// ------------------------------------------------------------------- salvage

#[test]
fn a_clean_container_needs_no_salvage() {
    let snapshot = Snapshot::build(&workbook());
    let salvaged = recover(&[snapshot], &tail_bytes(&tail_ops()));
    assert!(salvaged.report.is_clean(), "{:?}", salvaged.report);
    assert_eq!(salvaged.tail.len(), 2);
    assert_eq!(salvaged.quarantine.len(), 0);
    assert_eq!(salvaged.ops().len(), 7);
}

/// docs/16: "last valid snapshot". The newest is corrupt, the one before it is
/// used, and the user is told which — a silent fallback would hide that the
/// most recent save is gone.
#[test]
fn a_corrupt_newest_snapshot_falls_back_and_says_so() {
    let older = Snapshot::build(&workbook());
    let mut newer = Snapshot::build(&workbook());
    newer.state_hash = [7u8; 32];

    let salvaged = recover(&[newer, older.clone()], &tail_bytes(&tail_ops()));
    assert_eq!(salvaged.report.snapshots_rejected, 1);
    assert_eq!(
        salvaged.report.snapshot_used.as_ref(),
        Some(&older.watermark)
    );
    assert!(salvaged.report.reasons.iter().any(|r| matches!(
        r,
        SalvageReason::SnapshotFaulty(SnapshotFault::StateHashMismatch)
    )));
    assert!(!salvaged.report.is_clean(), "the user must be told");
    assert!(
        !salvaged.report.lost_data(),
        "but nothing was actually lost"
    );
}

/// A torn final write is the expected crash shape, not an exotic one: recover
/// everything before it, quarantine the rest, and never delete the bytes.
#[test]
fn a_torn_final_write_is_recovered_up_to_the_tear() {
    let snapshot = Snapshot::build(&workbook());
    let ops = tail_ops();
    let mut bytes = tail_bytes(&ops);
    let full = bytes.len();
    bytes.truncate(full - 4); // the process died mid-append

    let salvaged = recover(&[snapshot], &bytes);
    assert_eq!(salvaged.tail.len(), 1, "the intact op is recovered");
    assert!(salvaged.report.quarantined_bytes > 0);
    assert_eq!(
        salvaged.quarantine.len(),
        salvaged.report.quarantined_bytes,
        "the quarantined bytes are held, not deleted"
    );
    assert!(matches!(
        salvaged.report.reasons.as_slice(),
        [SalvageReason::TailCorrupt { .. }]
    ));
    assert!(salvaged.report.lost_data());
}

/// Ops are the truth: a workbook whose every snapshot is corrupt still opens
/// from its tail. Losing all snapshots must not lose the document.
#[test]
fn a_workbook_with_no_valid_snapshot_rebuilds_from_ops_alone() {
    let mut broken = Snapshot::build(&workbook());
    broken.state_hash = [1u8; 32];
    let all_ops: Vec<Op> = workbook().ops().to_vec();

    let salvaged = recover(&[broken], &tail_bytes(&all_ops));
    assert!(salvaged.snapshot.is_none());
    assert!(salvaged
        .report
        .reasons
        .contains(&SalvageReason::NoValidSnapshot));
    assert_eq!(salvaged.tail.len(), 5, "every op was still readable");
    assert!(salvaged.report.lost_data());
}

// --------------------------------------------------------- lifecycle machine

#[test]
fn document_machine_covers_every_listed_transition() {
    let mut doc = Document::new();
    assert_eq!(*doc.state(), DocState::Closed);

    // CLOSED ──open──► RECOVERING
    assert_eq!(doc.step(Event::Open), vec![Action::BeginRecovery]);
    assert_eq!(*doc.state(), DocState::Recovering);

    // RECOVERING ──snapshot ok + tail replay──► READY
    let verified = Snapshot::build(&workbook()).verify().expect("verify");
    let restored = verified.ops().len();
    assert!(doc
        .step(Event::Recovered {
            snapshot: Some(verified),
            tail_ops: 2,
        })
        .is_empty());
    assert_eq!(*doc.state(), DocState::Ready);
    assert_eq!(doc.acked_ops(), restored + 2);
    assert!(doc.restored_from().is_some());

    // READY ──ops──► READY
    assert_eq!(doc.step(Event::Ops(3)), vec![Action::AppendWal { ops: 3 }]);
    assert_eq!(*doc.state(), DocState::Ready);

    // READY ──compaction trigger──► COMPACTING
    assert_eq!(
        doc.step(Event::CompactionTrigger),
        vec![Action::WriteCompactedFile]
    );
    assert_eq!(*doc.state(), DocState::Compacting);

    // COMPACTING ──► READY
    assert_eq!(
        doc.step(Event::CompactionComplete),
        vec![Action::AtomicRename]
    );
    assert_eq!(*doc.state(), DocState::Ready);

    // READY ──close──► CLOSED (final fsync)
    assert_eq!(doc.step(Event::Close), vec![Action::Fsync]);
    assert_eq!(*doc.state(), DocState::Closed);

    // RECOVERING ──snapshot hash mismatch──► SALVAGE ──user ack──► READY
    let mut broken = Snapshot::build(&workbook());
    broken.state_hash = [3u8; 32];
    let salvaged = recover(&[broken], &tail_bytes(&tail_ops()));
    let mut doc = Document::new();
    doc.step(Event::Open);
    let actions = doc.step(Event::Salvaged {
        report: salvaged.report.clone(),
        snapshot: salvaged.snapshot,
        tail_ops: salvaged.tail.len(),
    });
    assert_eq!(
        actions,
        vec![Action::ReportSalvage(salvaged.report.clone())]
    );
    assert_eq!(*doc.state(), DocState::Salvage(salvaged.report));
    assert!(doc.step(Event::UserAck).is_empty());
    assert_eq!(*doc.state(), DocState::Ready);
}

// --- forbidden #1: "writing to the old file during COMPACTING"

#[test]
fn no_container_write_is_permitted_during_compaction() {
    let mut doc = ready_document();
    doc.step(Event::CompactionTrigger);
    assert!(
        !doc.may_write_container(),
        "the old file must not be written during COMPACTING"
    );

    let before = doc.acked_ops();
    let actions = doc.step(Event::Ops(5));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::AppendWal { .. })),
        "ops during COMPACTING must not reach the old file: {actions:?}"
    );
    // ...and equally must not vanish.
    assert_eq!(doc.acked_ops(), before + 5, "acked ops are still counted");
    assert_eq!(doc.deferred_ops(), 5, "they are deferred, not dropped");

    let actions = doc.step(Event::CompactionComplete);
    assert_eq!(
        actions,
        vec![Action::AtomicRename, Action::FlushDeferred { ops: 5 }],
        "the deferred ops land on the new file after the rename"
    );
    assert_eq!(doc.deferred_ops(), 0);
    assert!(doc.may_write_container());
}

// --- forbidden #2: "opening READY without hash-verifying the loaded snapshot"

/// This one cannot be tested by attempting it, because it cannot be expressed.
/// `Event::Recovered` carries a `VerifiedSnapshot`, and the only constructor is
/// `Snapshot::verify`, which replays the body and compares the state hash. The
/// test therefore proves the *property that makes it unreachable*: no unverified
/// snapshot yields a value that could be handed to the machine.
#[test]
fn an_unverified_snapshot_cannot_reach_the_ready_transition() {
    let mut forged = Snapshot::build(&workbook());
    forged.state_hash = [0xAB; 32];
    assert!(
        forged.verify().is_err(),
        "no VerifiedSnapshot exists for a snapshot that does not verify, \
         so Event::Recovered cannot be constructed with one"
    );

    // The one door that *is* open — SALVAGE — carries the report with it, and
    // the machine will not leave SALVAGE without an explicit acknowledgement.
    let salvaged = recover(&[forged], &[]);
    let mut doc = Document::new();
    doc.step(Event::Open);
    doc.step(Event::Salvaged {
        report: salvaged.report,
        snapshot: salvaged.snapshot,
        tail_ops: 0,
    });
    assert!(
        matches!(doc.state(), DocState::Salvage(_)),
        "a failed verification opens SALVAGE, never READY"
    );
}

// --- forbidden #3: "any transition that loses acked ops (RPO contract)"

#[test]
fn no_transition_loses_an_acked_op() {
    let mut doc = ready_document();
    doc.step(Event::Ops(4));
    let high_water = doc.acked_ops();
    assert!(high_water > 0);

    let script = vec![
        Event::Ops(2),
        Event::CompactionTrigger,
        Event::Ops(3),
        Event::CompactionComplete,
        Event::Ops(1),
        Event::Close,
        Event::Open,
    ];
    let mut seen = high_water;
    for (step, event) in script.into_iter().enumerate() {
        doc.step(event);
        assert!(
            doc.acked_ops() >= seen,
            "acked ops fell from {seen} to {} at step {step}",
            doc.acked_ops()
        );
        seen = doc.acked_ops();
    }
    assert_eq!(doc.acked_ops(), high_water + 6);
}

/// docs/27: "a transition not listed here is a `debug_assert` + logged error".
#[test]
#[should_panic(expected = "unlisted document transition")]
fn an_unlisted_document_transition_trips_the_debug_assert() {
    let mut doc = Document::new();
    // Closing a document that was never opened is not a listed edge.
    doc.step(Event::Close);
}

/// docs/27: "READY ──close──► CLOSED (final fsync; **no other work permitted**)".
#[test]
fn closing_emits_a_final_fsync_and_nothing_else() {
    let mut doc = ready_document();
    doc.step(Event::Ops(2));
    let actions = doc.step(Event::Close);
    assert_eq!(actions, vec![Action::Fsync], "no other work at close");
    assert_eq!(*doc.state(), DocState::Closed);
}

fn ready_document() -> Document {
    let mut doc = Document::new();
    doc.step(Event::Open);
    let verified = Snapshot::build(&workbook()).verify().expect("verify");
    doc.step(Event::Recovered {
        snapshot: Some(verified),
        tail_ops: 0,
    });
    doc
}
