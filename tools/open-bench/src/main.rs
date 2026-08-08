//! W-OPEN-1M (docs/38) - Row 11's acceptance workload.
//!
//! > *container with 1M-cell workbook + 100k-op tail. Measures: cold open to
//! > READY (snapshot decode + replay); SALVAGE path time with a corrupted
//! > final page.*
//!
//! Cold open means from a file this process did not write: the container is
//! built, dropped, and reopened, so the numbers include SQLite's own read path
//! and not a warm page cache of our own making. The OS page cache is still
//! warm, and that is stated rather than worked around - a cold-cache figure
//! would need a machine this session cannot reboot.
//!
//! Seeded and pure (D-052): the corpus is a function of its shape.

use std::path::PathBuf;
use std::time::Instant;

use ehkatra_store::Container;
use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_recover::snapshot::Snapshot;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

const CELLS: usize = 1_000_000;
const TAIL: usize = 100_000;
/// 1M cells as 1000 x 1000, which is the shape a real sheet has - a 1M x 1
/// column would exercise the axis CRDT and nothing else.
const COLS: usize = 1_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn build_corpus() -> (OpLog, usize) {
    let actor = ActorId(1);
    let mut log = OpLog::new();
    let mut counter = 0u64;
    fn push(log: &mut OpLog, actor: ActorId, counter: &mut u64, payload: Payload) {
        *counter += 1;
        log.append(Op {
            id: OpId {
                actor,
                counter: *counter,
            },
            lamport: *counter,
            payload,
        });
    }

    let rows = CELLS / COLS;
    let mut row_ids = Vec::with_capacity(rows);
    let mut col_ids = Vec::with_capacity(COLS);
    let mut prev: Option<OpId> = None;
    for _ in 0..rows {
        let anchor = prev.map_or(Anchor::Start, Anchor::After);
        counter += 1;
        let id = OpId { actor, counter };
        log.append(Op {
            id,
            lamport: counter,
            payload: Payload::InsertRow { anchor },
        });
        row_ids.push(RowId(id));
        prev = Some(id);
    }
    prev = None;
    for _ in 0..COLS {
        let anchor = prev.map_or(Anchor::Start, Anchor::After);
        counter += 1;
        let id = OpId { actor, counter };
        log.append(Op {
            id,
            lamport: counter,
            payload: Payload::InsertCol { anchor },
        });
        col_ids.push(ColId(id));
        prev = Some(id);
    }
    for (r, row) in row_ids.iter().enumerate() {
        for (c, col) in col_ids.iter().enumerate() {
            push(
                &mut log,
                actor,
                &mut counter,
                Payload::SetCell {
                    row: *row,
                    col: *col,
                    value: Value::Number((r * COLS + c) as f64),
                },
            );
        }
    }
    let snapshot_upto = log.ops().len();

    // The 100k-op tail: edits made after the last snapshot.
    for i in 0..TAIL {
        push(
            &mut log,
            actor,
            &mut counter,
            Payload::SetCell {
                row: row_ids[i % rows],
                col: col_ids[i % COLS],
                value: Value::Number(1_000_000.0 + i as f64),
            },
        );
    }
    (log, snapshot_upto)
}

fn main() {
    let dir = workspace_root().join(".tmp").join("open-bench");
    std::fs::create_dir_all(&dir).expect("scratch");
    let path = dir.join("w-open-1m.ehk");
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", path.display())));
    }

    println!("W-OPEN-1M - {CELLS} cells + {TAIL}-op tail");

    let t = Instant::now();
    let (log, snapshot_upto) = build_corpus();
    println!(
        "  corpus built            {:>8} ops in {:?}",
        log.ops().len(),
        t.elapsed()
    );

    // The snapshot covers everything except the tail.
    let mut snap_log = OpLog::new();
    for op in log.ops().iter().take(snapshot_upto) {
        snap_log.append(op.clone());
    }
    let t = Instant::now();
    let snapshot = Snapshot::build(&snap_log);
    println!(
        "  snapshot built          {:>8} B body in {:?}",
        snapshot.body.len(),
        t.elapsed()
    );

    let t = Instant::now();
    {
        let mut c = Container::open_or_create(&path).expect("create");
        c.put_snapshot(&snapshot).expect("put snapshot");
        // Only the tail needs to be in `ops`; the snapshot carries the rest.
        c.append_ops(&log.ops()[snapshot_upto..], 0)
            .expect("append tail");
        c.maybe_commit(u64::MAX, true).expect("commit");
    }
    let write = t.elapsed();
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("  container written       {:>8} B in {:?}", bytes, write);

    // --- cold open to READY ---
    let t = Instant::now();
    let c = Container::open_or_create(&path).expect("reopen");
    let opened = c.open_document().expect("open");
    let cold_open = t.elapsed();
    println!(
        "  COLD OPEN to READY      {:?}  (snapshot {} ops + tail {} ops, clean={})",
        cold_open,
        opened
            .salvaged
            .snapshot
            .as_ref()
            .map_or(0, |s| s.ops().len()),
        opened.salvaged.tail.len(),
        opened.salvaged.report.is_clean()
    );
    drop(opened);
    drop(c);

    // --- SALVAGE path with a corrupted final page ---
    {
        let c = Container::open_or_create(&path).expect("open");
        c.conn()
            .execute(
                // CAST is load-bearing: SQLite's `||` yields TEXT even when
                // both operands are blobs, and a TEXT body would fail to read
                // back as a different error than the corruption being tested.
                "UPDATE snapshots SET body = CAST(substr(body, 1, length(body) - 4096) \
                 || randomblob(4096) AS BLOB)",
                [],
            )
            .expect("corrupt final page");
    }
    let t = Instant::now();
    let c = Container::open_or_create(&path).expect("reopen");
    let opened = c.open_document().expect("open");
    let salvage = t.elapsed();
    println!(
        "  SALVAGE (corrupt page)  {:?}  (snapshots rejected {}, tail {} ops, quarantined {} B)",
        salvage,
        opened.salvaged.report.snapshots_rejected,
        opened.salvaged.tail.len(),
        opened.salvaged.report.quarantined_bytes
    );
    println!(
        "  salvage clean={} lost_data={}",
        opened.salvaged.report.is_clean(),
        opened.salvaged.report.lost_data()
    );
}
