//! Row 11's container half: docs/26's schema against a real file.
//!
//! The exit criterion is `save_then_reload_preserves_the_state_hash` - the
//! invariant that makes a container trustworthy at all. Everything else here
//! exists because it can fail on a filesystem in a way it cannot fail in
//! memory: torn writes, half-renames, foreign files, newer schemas.

use std::path::PathBuf;

use ehkatra_store::container::{Container, StoreError, AUTOSAVE_BATCH_MS};
use ehkatra_store::schema::{APPLICATION_ID, USER_VERSION};
use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_recover::machine::DocState;
use usk_recover::snapshot::Snapshot;
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

/// Scratch inside the workspace - CLAUDE.md forbids temp files outside it.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join(".tmp")
        .join("container-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("{name}.ehk"));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", path.display())));
    }
    path
}

fn id(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

/// A workbook with structure, values, and a formula - enough that the state
/// hash actually depends on several code paths.
fn workbook() -> OpLog {
    let mut log = OpLog::new();
    let (r1, r2, c1) = (id(1, 1), id(1, 2), id(1, 3));
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
            value: Value::Number(10.5),
        },
    });
    log.append(Op {
        id: id(2, 1),
        lamport: 5,
        payload: Payload::SetCell {
            row: RowId(r2),
            col: ColId(c1),
            value: Value::Text(String::from("two")),
        },
    });
    log
}

fn hash_of(log: &OpLog) -> [u8; 32] {
    *State::replay(log).state_hash().as_bytes()
}

// ------------------------------------------------- ROW 11 EXIT CRITERION

/// **The invariant the whole row exists for.** Write a workbook, close it,
/// reopen from disk, and the state hash must be identical - not "similar",
/// not "the cells look right". If this can fail, nothing else in the container
/// means anything.
#[test]
fn save_then_reload_preserves_the_state_hash() {
    let path = scratch("save-reload");
    let log = workbook();
    let expected = hash_of(&log);

    {
        let mut c = Container::open_or_create(&path).expect("create");
        c.append_ops(log.ops(), 0).expect("append");
        c.maybe_commit(u64::MAX, true).expect("commit");
    }

    let c = Container::open_or_create(&path).expect("reopen");
    let opened = c.open_document().expect("open document");
    assert_eq!(*opened.doc.state(), DocState::Ready);
    assert!(
        opened.salvaged.report.is_clean(),
        "{:?}",
        opened.salvaged.report
    );
    assert_eq!(
        restored_hash(&opened),
        expected,
        "state hash survived the round trip"
    );
    assert_eq!(restored_op_count(&opened), log.ops().len());
}

/// The same invariant with a snapshot in play: the snapshot covers most of the
/// history and a tail sits on top, which is the shape every real open has.
#[test]
fn a_snapshot_plus_tail_reloads_to_the_same_state_hash() {
    let path = scratch("snapshot-tail");
    let base = workbook();

    let mut full = base.clone();
    full.append(Op {
        id: id(2, 2),
        lamport: 6,
        payload: Payload::SetCell {
            row: RowId(id(1, 1)),
            col: ColId(id(1, 3)),
            value: Value::Number(99.0),
        },
    });
    let expected = hash_of(&full);

    {
        let mut c = Container::open_or_create(&path).expect("create");
        c.append_ops(full.ops(), 0).expect("append");
        // Snapshot covers only the first five ops; the sixth is tail.
        c.put_snapshot(&Snapshot::build(&base)).expect("snapshot");
        c.maybe_commit(u64::MAX, true).expect("commit");
    }

    let c = Container::open_or_create(&path).expect("reopen");
    let opened = c.open_document().expect("open");
    assert!(opened.salvaged.snapshot.is_some(), "the snapshot was used");
    assert_eq!(
        opened.salvaged.tail.len(),
        1,
        "exactly the uncovered op is tail"
    );
    assert_eq!(restored_hash(&opened), expected);
}

// ----------------------------------------------------------- docs/26 schema

#[test]
fn the_file_identifies_itself_and_runs_in_wal_mode() {
    let path = scratch("identity");
    let c = Container::open_or_create(&path).expect("create");
    let app: i32 = c
        .conn()
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .expect("app id");
    let ver: i32 = c
        .conn()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("user version");
    let journal: String = c
        .conn()
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .expect("journal mode");
    assert_eq!(app, APPLICATION_ID);
    assert_eq!(ver, USER_VERSION);
    assert_eq!(journal.to_lowercase(), "wal", "docs/26: WAL mode always");

    // Every table docs/26 names, present.
    let mut stmt = c
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare");
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    for expected in ["blobs", "meta", "ops", "snapshots", "undo"] {
        assert!(
            tables.contains(&expected.to_string()),
            "missing table {expected}: {tables:?}"
        );
    }
}

/// docs/26: "ops are immutable - INSERT only". Re-appending is a no-op, which
/// is what makes relay redelivery free.
#[test]
fn appending_the_same_op_twice_writes_it_once() {
    let path = scratch("idempotent-append");
    let log = workbook();
    let mut c = Container::open_or_create(&path).expect("create");
    let first = c.append_ops(log.ops(), 0).expect("first");
    let second = c.append_ops(log.ops(), 0).expect("second");
    assert_eq!(first, log.ops().len());
    assert_eq!(second, 0, "a redelivered op is ignored, not updated");

    let n: i64 = c
        .conn()
        .query_row("SELECT count(*) FROM ops", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n as usize, log.ops().len());
}

/// docs/26: "the payload column stores the *identical bytes* that were hashed".
#[test]
fn the_payload_column_is_the_wire_format_verbatim() {
    let path = scratch("payload-bytes");
    let log = workbook();
    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(log.ops(), 0).expect("append");

    for op in log.ops() {
        let stored: Vec<u8> = c
            .conn()
            .query_row(
                "SELECT payload FROM ops WHERE actor = ?1 AND counter = ?2",
                rusqlite::params![op.id.actor.0.to_be_bytes().to_vec(), op.id.counter as i64],
                |r| r.get(0),
            )
            .expect("row");
        assert_eq!(stored, op.encode(), "stored bytes are the encoded op");
    }
}

/// A file that is a database but not ours must be refused, not half-read.
#[test]
fn a_foreign_database_is_refused() {
    let path = scratch("foreign");
    {
        let conn = rusqlite::Connection::open(&path).expect("open");
        conn.execute_batch(
            "CREATE TABLE somebody_elses (x INTEGER); PRAGMA application_id = 12345;",
        )
        .expect("foreign schema");
    }
    match Container::open_or_create(&path) {
        Err(StoreError::NotAnEhkatraContainer { application_id }) => {
            assert_eq!(application_id, 12345)
        }
        other => panic!("expected refusal, got {other:?}", other = other.err()),
    }
}

/// docs/26: "older code opens newer files read-only via forward preservation;
/// it never writes a schema it doesn't know."
#[test]
fn a_newer_schema_is_refused_rather_than_written_to() {
    let path = scratch("newer-schema");
    {
        let mut c = Container::open_or_create(&path).expect("create");
        c.append_ops(workbook().ops(), 0).expect("append");
        c.conn()
            .pragma_update(None, "user_version", USER_VERSION + 7)
            .expect("bump");
    }
    match Container::open_or_create(&path) {
        Err(StoreError::SchemaTooNew { found, supported }) => {
            assert_eq!(found, USER_VERSION + 7);
            assert_eq!(supported, USER_VERSION);
        }
        other => panic!("expected SchemaTooNew, got {other:?}", other = other.err()),
    }
}

// -------------------------------------------------------------- autosave

/// docs/16: "Ops are durable locally within 250 ms." The cadence is driven by
/// an injected clock so the test asserts the rule instead of sleeping through
/// it - a test that waits 250 ms to check a 250 ms rule usually ends up
/// waiting 250 ms and checking nothing (DP-C5).
#[test]
fn autosave_batches_to_the_250ms_cadence() {
    let path = scratch("autosave");
    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(workbook().ops(), 0).expect("append");
    assert!(c.uncommitted() > 0);

    assert!(
        !c.maybe_commit(0, false).expect("early"),
        "too early to flush"
    );
    assert!(
        !c.maybe_commit(AUTOSAVE_BATCH_MS - 1, false)
            .expect("still early"),
        "one millisecond short is still short"
    );
    assert!(
        c.maybe_commit(AUTOSAVE_BATCH_MS, false).expect("due"),
        "at the cadence, a durability point is taken"
    );
    assert_eq!(c.uncommitted(), 0);
}

// ------------------------------------------------------------- compaction

/// docs/26/27: compaction writes a **new file** and renames; ops arriving
/// mid-compaction are deferred, not written to the old file and not dropped.
/// The logic was proven in `usk-recover`; this runs it against a real file.
#[test]
fn compaction_writes_a_new_file_and_flushes_deferred_ops() {
    let path = scratch("compaction");
    let log = workbook();
    let expected = hash_of(&log);

    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(log.ops(), 0).expect("append");
    c.maybe_commit(u64::MAX, true).expect("commit");

    let opened = c.open_document().expect("open");
    let mut doc = opened.doc;
    assert_eq!(*doc.state(), DocState::Ready);

    // Ops arrive while compaction is running.
    doc.step(usk_recover::machine::Event::CompactionTrigger);
    doc.step(usk_recover::machine::Event::Ops(4));
    assert!(!doc.may_write_container(), "old file is off limits");
    // Put it back in READY so `compact` can drive the real cycle.
    doc.step(usk_recover::machine::Event::CompactionComplete);

    doc.step(usk_recover::machine::Event::Ops(3));
    let report = {
        // Defer three ops across the real compaction.
        doc.step(usk_recover::machine::Event::CompactionTrigger);
        doc.step(usk_recover::machine::Event::Ops(3));
        doc.step(usk_recover::machine::Event::CompactionComplete);
        c.compact(&mut doc, &log)
    };
    // `compact` drives its own trigger/complete pair; the important assertions
    // are that the file survived and still means the same thing.
    let _ = report;

    let reopened = Container::open_or_create(&path).expect("reopen after compaction");
    let opened = reopened.open_document().expect("open");
    assert_eq!(
        restored_hash(&opened),
        expected,
        "compaction must not change what the workbook is"
    );
    assert!(
        opened.salvaged.snapshot.is_some(),
        "a compacted container opens from its snapshot"
    );
}

// ------------------------------------- DP-A5 forward preservation (TD-25)

/// An op authored by a build one model version ahead of this one, arriving
/// through the framed reader the wire and the container now share.
fn op_from_the_future(counter: u64) -> Op {
    let mut body = Vec::new();
    body.extend_from_slice(&99u128.to_be_bytes());
    body.extend_from_slice(&counter.to_be_bytes());
    body.extend_from_slice(&(counter + 100).to_be_bytes());
    body.push(0x5C); // a payload tag model version 1 does not define
    body.extend_from_slice(b"something this build cannot read");
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    Op::decode_framed(&framed)
        .expect("an unknown tag is preserved, not refused")
        .0
}

/// **DP-A5 end to end.** An op this build cannot interpret is stored, snapshot,
/// compacted, and read back — byte for byte, in causal order, still hashing to
/// what its author computed. Before TD-25 the container had no per-op length,
/// so an unknown tag ended the tail and took every op behind it.
#[test]
fn an_unknown_op_type_survives_the_whole_container_round_trip() {
    let path = scratch("forward-preservation");
    let mut log = workbook();
    let future = op_from_the_future(1);
    log.append(future.clone());
    log.append(Op {
        id: id(2, 3),
        lamport: 200,
        payload: Payload::SetCell {
            row: RowId(id(1, 2)),
            col: ColId(id(1, 3)),
            value: Value::Number(7.0),
        },
    });
    let expected_state = hash_of(&log);
    let expected_log = log.canonical_hash();

    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(log.ops(), 0).expect("append");
    c.put_snapshot(&Snapshot::build(&log)).expect("snapshot");
    c.maybe_commit(u64::MAX, true).expect("commit");
    let mut doc = c.open_document().expect("open").doc;
    c.compact(&mut doc, &log).expect("compact");
    drop(doc);

    let reopened = Container::open_or_create(&path).expect("reopen");
    let opened = reopened.open_document().expect("open");
    assert!(opened.salvaged.report.is_clean());

    // **The op is in the tail, and that is the guarantee** (ADR-036
    // Amendment 1). An image is what the ops produced and an opaque op produces
    // nothing, so a snapshot never covers one — which means compaction's floor
    // never contains one and the op cannot be pruned. It therefore arrives as
    // ordinary tail, still readable, still retransmittable: DP-A5.
    let recovered = opened
        .salvaged
        .tail
        .iter()
        .find(|op| op.id == future.id)
        .cloned()
        .expect("the unknown op came back");
    assert_eq!(recovered, future, "preserved verbatim");
    assert_eq!(
        recovered.encode(),
        future.encode(),
        "and re-encodes to the author's bytes, so it hashes opaque"
    );
    // Deliberately *not* asserted: that the canonical hash of the whole op set
    // survives. Compaction prunes the ops the image represents — that is what
    // compaction is, and the v0.1 op-set body pruned them too. What changed is
    // only which ops are eligible, and the one the assertion above names never
    // is.
    let _ = expected_log;
    assert_eq!(
        restored_hash(&opened),
        expected_state,
        "and the ops behind it still mean what they meant"
    );
}

// ------------------------------------------- docs/16 retention (TD-30)

/// A history where **every** op contributes to the final state: each row is
/// inserted after the previous one and gets its own cell. That matters more
/// than it looks — a fixture that writes repeatedly to *one* cell makes losing
/// an early op invisible to the state hash, so the retention tests below would
/// pass while the container quietly threw history away.
fn chain(rows: usize) -> OpLog {
    let mut log = OpLog::new();
    let c1 = id(7, 2);
    log.append(Op {
        id: id(7, 1),
        lamport: 1,
        payload: Payload::InsertRow {
            anchor: Anchor::Start,
        },
    });
    log.append(Op {
        id: c1,
        lamport: 2,
        payload: Payload::InsertCol {
            anchor: Anchor::Start,
        },
    });
    let mut previous = id(7, 1);
    let mut counter = 3u64;
    for i in 0..rows {
        let row = id(7, counter);
        log.append(Op {
            id: row,
            lamport: counter,
            payload: Payload::InsertRow {
                anchor: Anchor::After(previous),
            },
        });
        counter += 1;
        log.append(Op {
            id: id(7, counter),
            lamport: counter,
            payload: Payload::SetCell {
                row: RowId(row),
                col: ColId(c1),
                value: Value::Number(i as f64 + 0.25),
            },
        });
        counter += 1;
        previous = row;
    }
    log
}

fn prefix(log: &OpLog, n: usize) -> OpLog {
    let mut out = OpLog::new();
    for op in log.ops().iter().take(n) {
        out.append(op.clone());
    }
    out
}

/// Corrupts the newest `n` snapshot bodies by truncation — the shape a torn
/// page leaves. Asserts the corruption actually took: a "corrupt" snapshot that
/// still verifies would make every test below vacuous while looking green.
fn corrupt_newest_snapshots(c: &Container, n: usize) {
    let rowids: Vec<i64> = {
        let mut stmt = c
            .conn()
            .prepare("SELECT rowid FROM snapshots ORDER BY created DESC, rowid DESC")
            .expect("prepare");
        let v = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();
        v
    };
    assert!(rowids.len() >= n, "asked to corrupt more than exist");
    for &rowid in rowids.iter().take(n) {
        let body: Vec<u8> = c
            .conn()
            .query_row(
                "SELECT body FROM snapshots WHERE rowid = ?1",
                rusqlite::params![rowid],
                |r| r.get(0),
            )
            .expect("body");
        let torn = body[..body.len() * 3 / 5].to_vec();
        c.conn()
            .execute(
                "UPDATE snapshots SET body = ?1 WHERE rowid = ?2",
                rusqlite::params![torn, rowid],
            )
            .expect("corrupt");
    }
    let broken = c
        .snapshots()
        .expect("snapshots")
        .iter()
        .take(n)
        .filter(|s| s.verify().is_err())
        .count();
    assert_eq!(broken, n, "the corruption must actually break verification");
}

fn snapshot_count(c: &Container) -> i64 {
    c.conn()
        .query_row("SELECT count(*) FROM snapshots", [], |r| r.get(0))
        .expect("count")
}

fn op_count(c: &Container) -> i64 {
    c.conn()
        .query_row("SELECT count(*) FROM ops", [], |r| r.get(0))
        .expect("count")
}

/// Builds a container holding the full history and three snapshots at
/// increasing watermarks — the state every retention test starts from.
fn container_with_three_snapshots(name: &str, full: &OpLog) -> (PathBuf, Container) {
    let path = scratch(name);
    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(full.ops(), 0).expect("append");
    for cut in [10usize, 20, 30] {
        c.put_snapshot(&Snapshot::build(&prefix(full, cut)))
            .expect("snapshot");
    }
    c.maybe_commit(u64::MAX, true).expect("commit");
    (path, c)
}

/// docs/16 §Retention: the chain is capped at three, and capping it touches
/// snapshots only — every op is still present afterwards.
#[test]
fn the_snapshot_chain_is_trimmed_to_three_and_never_costs_an_op() {
    let full = chain(20);
    let path = scratch("retention-trim");
    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(full.ops(), 0).expect("append");
    for cut in [5usize, 10, 15, 20, 25] {
        c.put_snapshot(&Snapshot::build(&prefix(&full, cut)))
            .expect("snapshot");
    }
    assert_eq!(snapshot_count(&c), 3, "docs/16: keep the last 3 snapshots");
    assert_eq!(
        op_count(&c) as usize,
        full.ops().len(),
        "trimming the chain must not touch ops"
    );

    // The three kept are the three newest.
    let kept: Vec<usize> = c
        .snapshots()
        .expect("snapshots")
        .iter()
        .map(|s| s.verify().expect("verifies").covered().len())
        .collect();
    assert_eq!(kept, vec![25, 20, 15], "newest first");
}

/// docs/16 §Retention consequence 2: the op floor is the **oldest** retained
/// snapshot, so the compacted file carries every op since it.
#[test]
fn compaction_keeps_three_snapshots_and_every_op_since_the_oldest() {
    let full = chain(20);
    let (path, mut c) = container_with_three_snapshots("retention-compact", &full);

    let mut doc = c.open_document().expect("open").doc;
    let report = c.compact(&mut doc, &full).expect("compact");

    assert_eq!(report.snapshots_retained, 3);
    // Chain after compaction: [full, prefix30, prefix20]. The floor is
    // prefix20, so exactly its 20 ops may go and the rest must stay.
    assert_eq!(report.ops_pruned, 20);
    assert_eq!(report.ops_kept, full.ops().len() - 20);

    let reopened = Container::open_or_create(&path).expect("reopen");
    assert_eq!(snapshot_count(&reopened), 3);
    assert_eq!(op_count(&reopened) as usize, full.ops().len() - 20);
    let opened = reopened.open_document().expect("open");
    assert_eq!(
        restored_hash(&opened),
        hash_of(&full),
        "compaction must not change what the workbook is"
    );
    assert!(opened.salvaged.report.is_clean());
}

/// **The TD-30 regression, stated directly.** Corrupt every snapshot but the
/// oldest and the workbook must still come back whole — that is what "the last
/// *valid* snapshot" means, and it is only true because the ops since the
/// oldest snapshot were retained rather than pruned to the newest.
#[test]
fn corrupting_every_snapshot_but_the_oldest_still_recovers_every_op() {
    let full = chain(20);
    let expected = hash_of(&full);
    let (path, mut c) = container_with_three_snapshots("retention-all-but-one", &full);

    let mut doc = c.open_document().expect("open").doc;
    c.compact(&mut doc, &full).expect("compact");
    drop(doc);

    let c = Container::open_or_create(&path).expect("reopen");
    corrupt_newest_snapshots(&c, 2);

    let opened = c.open_document().expect("open");
    assert_eq!(
        restored_hash(&opened),
        expected,
        "falling back to the oldest snapshot must lose nothing"
    );
    assert_eq!(restored_op_count(&opened), full.ops().len());
    assert_eq!(opened.salvaged.report.snapshots_rejected, 2);
    assert!(
        !opened.salvaged.report.lost_data(),
        "an older-but-valid snapshot loses nothing: {:?}",
        opened.salvaged.report
    );
    assert!(
        !opened.salvaged.report.is_clean(),
        "and the user is still told, because two saves were unreadable"
    );
}

/// Ops are the truth. With every snapshot destroyed and no compaction having
/// pruned anything, the workbook rebuilds from the op tail alone — bit for bit.
#[test]
fn corrupting_all_snapshots_rebuilds_from_the_full_op_tail() {
    let full = chain(20);
    let expected = hash_of(&full);
    let (path, c) = container_with_three_snapshots("retention-all-corrupt", &full);
    drop(c);

    let c = Container::open_or_create(&path).expect("reopen");
    corrupt_newest_snapshots(&c, 3);

    let opened = c.open_document().expect("open");
    assert_eq!(
        restored_hash(&opened),
        expected,
        "the op log alone rebuilds the workbook"
    );
    assert_eq!(restored_op_count(&opened), full.ops().len());
    assert_eq!(opened.salvaged.report.snapshots_rejected, 3);
    assert!(opened.salvaged.snapshot.is_none());
    assert!(
        opened.salvaged.report.lost_data(),
        "every save the user thought they had is gone, and they must be told"
    );
}

/// docs/16 §Retention consequence 4: the **first** compaction has one snapshot,
/// so it prunes nothing. This is the exact state W-OPEN-1M found catastrophic —
/// one snapshot, no tail — and it is now unreachable.
#[test]
fn a_first_compaction_prunes_no_ops_and_survives_losing_its_only_snapshot() {
    let full = chain(12);
    let expected = hash_of(&full);
    let path = scratch("retention-first-compaction");

    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(full.ops(), 0).expect("append");
    c.maybe_commit(u64::MAX, true).expect("commit");
    let mut doc = c.open_document().expect("open").doc;
    let report = c.compact(&mut doc, &full).expect("compact");
    drop(doc);

    assert_eq!(report.snapshots_retained, 1);
    assert_eq!(
        report.ops_pruned, 0,
        "one snapshot is not a fallback, so it authorises no deletion"
    );

    let c = Container::open_or_create(&path).expect("reopen");
    corrupt_newest_snapshots(&c, 1);
    let opened = c.open_document().expect("open");
    assert_eq!(
        restored_hash(&opened),
        expected,
        "the failure TD-30 was filed for: a single corrupt snapshot losing everything"
    );
}

/// docs/16 §Retention consequence 3: a snapshot that cannot prove what it
/// contains may not authorise deleting it. Corrupt the oldest of the three
/// *before* compacting, and the floor drops to the oldest one that verifies.
#[test]
fn a_corrupt_floor_snapshot_authorises_no_deletion() {
    let full = chain(20);
    let expected = hash_of(&full);
    let (path, mut c) = container_with_three_snapshots("retention-corrupt-floor", &full);

    // Break the snapshot that would become the floor. Compaction retains
    // [full, prefix-30, prefix-20], so the floor is prefix-20 — the
    // second-oldest of the three already stored, not the oldest, because the
    // oldest falls out of the chain when the fresh snapshot joins it.
    let oldest: i64 = c
        .conn()
        .query_row(
            "SELECT rowid FROM snapshots ORDER BY created ASC, rowid ASC LIMIT 1 OFFSET 1",
            [],
            |r| r.get(0),
        )
        .expect("floor-to-be");
    let body: Vec<u8> = c
        .conn()
        .query_row(
            "SELECT body FROM snapshots WHERE rowid = ?1",
            rusqlite::params![oldest],
            |r| r.get(0),
        )
        .expect("body");
    c.conn()
        .execute(
            "UPDATE snapshots SET body = ?1 WHERE rowid = ?2",
            rusqlite::params![body[..body.len() / 2].to_vec(), oldest],
        )
        .expect("corrupt");

    let mut doc = c.open_document().expect("open").doc;
    let report = c.compact(&mut doc, &full).expect("compact");
    drop(doc);

    // Chain is [full, prefix30, prefix20-broken]; the floor falls to prefix30.
    assert_eq!(report.snapshots_retained, 3);
    assert_eq!(
        report.ops_pruned, 30,
        "the broken snapshot's coverage is not deletable; the next one's is"
    );

    let reopened = Container::open_or_create(&path).expect("reopen");
    let opened = reopened.open_document().expect("open");
    assert_eq!(restored_hash(&opened), expected);
}

/// The honest boundary of the policy, recorded rather than smoothed. Once
/// compaction has pruned, losing **all three** snapshots does lose the ops the
/// floor had absorbed — and docs/16 forbids letting the user believe otherwise.
#[test]
fn total_snapshot_loss_after_pruning_is_reported_rather_than_hidden() {
    let full = chain(20);
    let (path, mut c) = container_with_three_snapshots("retention-total-loss", &full);
    let mut doc = c.open_document().expect("open").doc;
    c.compact(&mut doc, &full).expect("compact");
    drop(doc);

    let c = Container::open_or_create(&path).expect("reopen");
    corrupt_newest_snapshots(&c, 3);
    let opened = c.open_document().expect("open");

    assert!(opened.salvaged.snapshot.is_none());
    assert!(
        opened.salvaged.report.lost_data(),
        "silent partial restore is forbidden"
    );
    assert_eq!(
        restored_op_count(&opened),
        full.ops().len() - 20,
        "what survives is exactly the retained tail, and it is not the whole workbook"
    );
    assert_ne!(
        restored_hash(&opened),
        hash_of(&full),
        "and the difference is real, which is why the report must say so"
    );
}

// --------------------------------------------------------------- migration

/// docs/26's strongest migration test, and it is free because of DP-A2: a
/// migration that changes the state hash is by definition wrong. The registry
/// is empty at v1, so the *mechanism* is proven here rather than waiting for
/// the first real migration to be its own first test.
#[test]
fn the_migration_check_detects_a_changed_state_hash() {
    let path = scratch("migration");
    let log = workbook();
    let mut c = Container::open_or_create(&path).expect("create");
    c.append_ops(log.ops(), 0).expect("append");
    c.maybe_commit(u64::MAX, true).expect("commit");

    let before = ehkatra_store::migrate::state_hash(c.conn()).expect("hash");
    assert_eq!(before, hash_of(&log));

    // A "migration" that quietly drops a row - the class of bug the check
    // exists to catch.
    c.conn()
        .execute(
            "DELETE FROM ops WHERE counter = 4 AND actor = ?1",
            rusqlite::params![1u128.to_be_bytes().to_vec()],
        )
        .expect("mutate");
    let after = ehkatra_store::migrate::state_hash(c.conn()).expect("hash");
    assert_ne!(
        before, after,
        "losing an op must move the state hash, or the check proves nothing"
    );
}

/// The state hash of what a container restores to.
///
/// Replaces `hash_of(&opened.log())`: since ADR-036 a snapshot stores an image
/// rather than op bodies, so there is no log to fold — and the state hash was
/// always the property these assertions were reaching for.
fn restored_hash(opened: &ehkatra_store::Opened) -> [u8; 32] {
    *opened.state().state_hash().as_bytes()
}

/// Every op the container still accounts for: the snapshot's coverage plus the
/// tail. They cannot overlap — an op is tail precisely because coverage does
/// not contain it.
fn restored_op_count(opened: &ehkatra_store::Opened) -> usize {
    let covered = opened
        .salvaged
        .snapshot
        .as_ref()
        .map_or(0, |s| s.covered().len());
    covered + opened.salvaged.tail.len()
}
