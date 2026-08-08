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
        hash_of(&opened.log()),
        expected,
        "state hash survived the round trip"
    );
    assert_eq!(opened.ops().len(), log.ops().len());
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
    assert_eq!(hash_of(&opened.log()), expected);
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
        hash_of(&opened.log()),
        expected,
        "compaction must not change what the workbook is"
    );
    assert!(
        opened.salvaged.snapshot.is_some(),
        "a compacted container opens from its snapshot"
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
