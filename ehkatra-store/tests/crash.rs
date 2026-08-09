//! Crash injection and corruption - BOOTSTRAP row 11's "kill -9 mid-write test
//! recovers", and docs/16's SALVAGE path against a real damaged file.
//!
//! These are the tests the logic half could not write. `usk-recover` proved
//! what recovery *decides* given bytes; only a killed process and a corrupted
//! file prove the bytes are what we thought.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use ehkatra_store::{crash_corpus, Container};
use usk_oplog::OpLog;
use usk_recover::machine::DocState;
use usk_recover::salvage::SalvageReason;
use usk_recover::snapshot::Snapshot;
use usk_state::State;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = workspace_root().join(".tmp").join("crash-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("{name}.ehk"));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", path.display())));
    }
    path
}

/// The crash-writer binary cargo built alongside this test.
fn crash_writer_bin() -> PathBuf {
    // The test executable lives in target/<profile>/deps/; the binary is one
    // level up.
    let mut dir = std::env::current_exe().expect("test exe");
    dir.pop();
    if dir.ends_with("deps") {
        dir.pop();
    }
    let exe = dir.join(if cfg!(windows) {
        "crash-writer.exe"
    } else {
        "crash-writer"
    });
    assert!(
        exe.exists(),
        "crash-writer not built at {exe:?} - it is a [[bin]] of this crate, so \
         `cargo test -p ehkatra-store` should have produced it"
    );
    exe
}

fn hash_of(log: &OpLog) -> [u8; 32] {
    *State::replay(log).state_hash().as_bytes()
}

fn log_of(ops: Vec<usk_oplog::Op>) -> OpLog {
    let mut log = OpLog::new();
    for op in ops {
        log.append(op);
    }
    log
}

/// **BOOTSTRAP row 11's proof.** A real process writes, is killed with no
/// chance to clean up, and the container still holds every op it acknowledged.
///
/// The promise being tested is docs/16's, exactly: *ops are durable within
/// 250 ms*, so everything before the last `COMMITTED` line must survive. Ops
/// after it are explicitly not claimed, and the test does not pretend
/// otherwise: a durability test that asserts more than the contract is a test
/// that will eventually fail for the wrong reason.
#[test]
fn a_killed_process_keeps_every_op_it_acknowledged() {
    let path = scratch("kill-9");
    let mut child = Command::new(crash_writer_bin())
        .arg(&path)
        .arg("4000")
        .arg("50")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn crash-writer");

    // Read acknowledgements until the writer is well underway, then kill it
    // mid-flight.
    let stdout = child.stdout.take().expect("stdout");
    let mut acknowledged = 0usize;
    {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(n) = line.strip_prefix("COMMITTED ") {
                acknowledged = n.trim().parse().unwrap_or(acknowledged);
                if acknowledged >= 1_000 {
                    break;
                }
            }
        }
    }
    assert!(
        acknowledged >= 1_000,
        "writer never acknowledged enough ops (got {acknowledged})"
    );

    // KILL. No unwinding, no destructors, no checkpoint.
    child.kill().expect("kill");
    let _ = child.wait();

    // Reopen from whatever is on disk.
    let container = Container::open_or_create(&path).expect("reopen after kill");
    let opened = container.open_document().expect("open");

    // No snapshot here by construction: `crash-writer` only appends. Asserted
    // rather than assumed, because the check below reads the tail directly and
    // would quietly weaken if a snapshot ever appeared.
    assert!(
        opened.salvaged.snapshot.is_none(),
        "this corpus is append-only; a snapshot would change what `tail` means"
    );
    let recovered = opened.salvaged.tail.clone();
    assert!(
        recovered.len() >= acknowledged,
        "container lost acknowledged work: {} recovered, {acknowledged} acknowledged",
        recovered.len()
    );

    // Everything acknowledged is present, and is the *same* op.
    let expected = crash_corpus(acknowledged);
    for op in &expected {
        assert!(
            recovered.iter().any(|o| o.id == op.id && o == op),
            "op {:?} was acknowledged and then lost",
            op.id
        );
    }

    // And the workbook those ops describe is intact: replaying the recovered
    // prefix reproduces the hash the writer would have had at that point.
    let prefix = log_of(recovered.iter().take(acknowledged).cloned().collect());
    assert_eq!(
        hash_of(&prefix),
        hash_of(&log_of(expected)),
        "the recovered prefix is not the workbook that was acknowledged"
    );
    assert_eq!(*opened.doc.state(), DocState::Ready);
}

/// docs/16: a corrupted container opens through SALVAGE with an honest report -
/// last valid snapshot, readable tail, quarantined remainder - and never
/// silently.
#[test]
fn a_corrupted_snapshot_opens_through_salvage_and_reports_it() {
    let path = scratch("corrupt-snapshot");
    let ops = crash_corpus(120);
    let full = log_of(ops.clone());
    let expected = hash_of(&full);

    {
        let mut c = Container::open_or_create(&path).expect("create");
        c.append_ops(&ops, 0).expect("append");
        c.put_snapshot(&Snapshot::build(&full)).expect("snapshot");
        c.maybe_commit(u64::MAX, true).expect("commit");
    }

    // Corrupt the snapshot body in place - the "corrupted final page" docs/38
    // names for W-OPEN-1M.
    {
        let c = Container::open_or_create(&path).expect("open");
        c.conn()
            .execute("UPDATE snapshots SET body = randomblob(length(body))", [])
            .expect("corrupt");
    }

    let c = Container::open_or_create(&path).expect("reopen");
    let opened = c.open_document().expect("open");

    assert!(
        matches!(opened.doc.state(), DocState::Salvage(_)),
        "a corrupt snapshot must open through SALVAGE, got {:?}",
        opened.doc.state()
    );
    assert!(opened.needs_acknowledgement(), "the user must be told");
    assert!(
        opened.salvaged.report.reasons.iter().any(|r| matches!(
            r,
            SalvageReason::SnapshotFaulty(_)
        ) || matches!(
            r,
            SalvageReason::NoValidSnapshot
        )),
        "the report must name the fault: {:?}",
        opened.salvaged.report
    );

    // Ops are the truth: the workbook is still recoverable in full from them.
    assert_eq!(
        *opened.state().state_hash().as_bytes(),
        expected,
        "the op log rebuilt the workbook the corrupt snapshot could not"
    );
}

/// A container whose op tail is truncated mid-op - the torn final write a power
/// cut produces - recovers everything before the tear and quarantines the rest.
#[test]
fn a_torn_op_in_the_container_is_quarantined_not_fatal() {
    let path = scratch("torn-op");
    let ops = crash_corpus(40);

    {
        let mut c = Container::open_or_create(&path).expect("create");
        c.append_ops(&ops, 0).expect("append");
        c.maybe_commit(u64::MAX, true).expect("commit");
    }

    // Truncate the payload of the last op, leaving a partial encoding - what a
    // write interrupted between bytes leaves behind.
    {
        let c = Container::open_or_create(&path).expect("open");
        c.conn()
            .execute(
                "UPDATE ops SET payload = substr(payload, 1, length(payload) - 5) \
                 WHERE counter = (SELECT max(counter) FROM ops)",
                [],
            )
            .expect("truncate payload");
    }

    let c = Container::open_or_create(&path).expect("reopen");
    let opened = c.open_document().expect("open");

    assert!(
        matches!(opened.doc.state(), DocState::Salvage(_)),
        "a torn op is a salvage, not a clean open"
    );
    assert_eq!(
        opened.salvaged.tail.len(),
        ops.len() - 1,
        "everything before the tear is recovered"
    );
    assert!(
        opened.salvaged.report.quarantined_bytes > 0,
        "the unreadable remainder is held, not deleted"
    );
    assert!(opened.salvaged.report.lost_data());
}
