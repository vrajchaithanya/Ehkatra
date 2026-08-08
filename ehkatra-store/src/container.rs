//! The container: one workbook, one SQLite file (ADR-031, docs/26).
//!
//! This is the shell adapter under `usk-recover`'s already-proven logic. The
//! port is narrow on purpose — `Snapshot`, `recover()` and `Document` are the
//! whole interface — so what lives here is `INSERT`/`SELECT`, fsync discipline
//! and atomic rename, not new semantics. Anything that could be wrong in an
//! interesting way was proven in the kernel, without a filesystem.
//!
//! # The rules docs/26 states, and where each is enforced
//! * *"ops are immutable — INSERT only, never UPDATE/DELETE except compaction"*
//!   — [`Container::append_ops`] is the only writer, and it is `INSERT OR
//!   IGNORE`; compaction never rewrites in place, it builds a new file and
//!   renames ([`Container::compact`]).
//! * *"`state_hash` verifies on every snapshot load"* — [`Container::open`]
//!   goes through `usk_recover::recover`, which replays the body and compares.
//!   There is no code path that loads a snapshot without that check, because
//!   the only way to obtain a `VerifiedSnapshot` is to pass it.
//! * *"the payload column stores the identical bytes that were hashed"* — the
//!   `payload` column is `Op::encode()` output, byte for byte, and
//!   [`Container::open`] reads it back through `Op::decode`. The file *is* the
//!   wire format at rest.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, Transaction};
use usk_oplog::{Op, OpLog};
use usk_recover::machine::{Action, Document, Event};
use usk_recover::salvage::{recover, Salvaged};
use usk_recover::snapshot::{Snapshot, VerifiedSnapshot};
use usk_types::{ActorId, OpId};

use crate::schema::{APPLICATION_ID, SCHEMA_V1, USER_VERSION};

/// Errors that are the *container's* — docs/28's domain 2. Spreadsheet errors
/// are values and never appear here.
#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// The file is a database, but not one of ours.
    NotAnEhkatraContainer {
        application_id: i32,
    },
    /// Written by a newer build. docs/26: older code opens newer files
    /// read-only via forward preservation; it never writes a schema it does not
    /// know.
    SchemaTooNew {
        found: i32,
        supported: i32,
    },
    /// A stored op does not decode. Recovery handles this; seeing it escape
    /// means the caller bypassed `open`.
    CorruptOp {
        at: usize,
    },
    /// docs/26: "a migration that changes the state hash is by definition
    /// wrong". Rolled back, never committed.
    MigrationChangedStateHash {
        from: i32,
        to: i32,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "sqlite: {e}"),
            StoreError::Io(e) => write!(f, "io: {e}"),
            StoreError::NotAnEhkatraContainer { application_id } => write!(
                f,
                "not an Ehkatra container (application_id {application_id:#x})"
            ),
            StoreError::SchemaTooNew { found, supported } => write!(
                f,
                "container schema v{found} is newer than this build's v{supported}; opened read-only"
            ),
            StoreError::CorruptOp { at } => write!(f, "undecodable op at row {at}"),
            StoreError::MigrationChangedStateHash { from, to } => write!(
                f,
                "migration v{from}->v{to} changed the workbook state hash; refused and rolled back"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

type Result<T> = std::result::Result<T, StoreError>;

/// docs/16: "Ops are durable locally within 250 ms of typing." The container
/// batches commits to that cadence rather than committing per op.
pub const AUTOSAVE_BATCH_MS: u64 = 250;

/// docs/16 §Retention: **keep the last 3 snapshots**.
///
/// One is not a fallback. The salvage path promises "the *last valid*
/// snapshot", which presupposes more than one exists; before this constant
/// existed the container kept exactly one, and W-OPEN-1M measured what that
/// costs — corrupting it lost 1,002,000 ops with zero quarantined bytes.
pub const SNAPSHOTS_RETAINED: usize = 3;

/// docs/16 §Retention consequence 4: ops are pruned only when at least this
/// many retained snapshots **verify**.
///
/// With one snapshot the tail is empty by construction, so a single corruption
/// is total loss. With two, the guarantee is structural: snapshots are nested,
/// so whichever one is corrupt, the newest survivor covers at least the floor
/// and the retained tail covers everything after it.
pub const MIN_VERIFIED_SNAPSHOTS_TO_PRUNE_OPS: usize = 2;

/// What `open` produced: the document machine in READY, the ops to fold, and —
/// if anything was wrong — the salvage report the user must see first.
pub struct Opened {
    pub doc: Document,
    pub salvaged: Salvaged,
    pub path: PathBuf,
}

impl Opened {
    /// Every op recovery believes in, snapshot then tail.
    pub fn ops(&self) -> Vec<Op> {
        self.salvaged.ops()
    }

    /// The log this container restores to.
    pub fn log(&self) -> OpLog {
        let mut log = OpLog::new();
        for op in self.ops() {
            log.append(op);
        }
        log
    }

    /// True when the open was not clean and docs/16 requires the user to be
    /// told before the document becomes editable.
    pub fn needs_acknowledgement(&self) -> bool {
        !self.salvaged.report.is_clean()
    }
}

/// One workbook file.
pub struct Container {
    conn: Connection,
    path: PathBuf,
    /// Ops appended since the last commit, and when the batch opened.
    uncommitted: usize,
    /// Injected, never read from a clock inside the kernel — the container is
    /// shell code and may look at the OS, but the *cadence* is a parameter so
    /// the durability test can drive it deterministically.
    batch_opened_ms: Option<u64>,
}

impl Container {
    /// Creates or opens the file and brings the schema up to date.
    ///
    /// WAL mode "always" (docs/26) and `synchronous = FULL`: docs/16 promises
    /// "RPO = last local fsync <= 250 ms of typing ... including after power
    /// loss mid-write". `synchronous = NORMAL` in WAL mode explicitly does not
    /// survive power loss, only process death, so the weaker setting would make
    /// the published promise false.
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Container> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;

        let found_app: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let found_ver: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

        if found_app == 0 && found_ver == 0 && Self::is_empty(&conn)? {
            conn.execute_batch(SCHEMA_V1)?;
            conn.pragma_update(None, "application_id", APPLICATION_ID)?;
            conn.pragma_update(None, "user_version", USER_VERSION)?;
            let created = now_ms() as i64;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('created', ?1)",
                params![created.to_be_bytes().to_vec()],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('model_version', ?1)",
                params![vec![0u8, 1u8]],
            )?;
        } else if found_app != APPLICATION_ID {
            return Err(StoreError::NotAnEhkatraContainer {
                application_id: found_app,
            });
        } else if found_ver > USER_VERSION {
            return Err(StoreError::SchemaTooNew {
                found: found_ver,
                supported: USER_VERSION,
            });
        } else if found_ver < USER_VERSION {
            crate::migrate::run(&conn, found_ver, USER_VERSION)?;
        }

        Ok(Container {
            conn,
            path,
            uncommitted: 0,
            batch_opened_ms: None,
        })
    }

    fn is_empty(conn: &Connection) -> Result<bool> {
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get(0),
        )?;
        Ok(n == 0)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends ops. **INSERT only** (docs/26): an op already present is ignored
    /// rather than updated, which makes redelivery free and makes accidental
    /// mutation impossible to express.
    ///
    /// Does not commit — see [`Container::maybe_commit`]. Batching is the whole
    /// point of the 250 ms cadence.
    ///
    /// `now_ms` is supplied by the caller, not read here. The first version
    /// stamped the batch from the system clock while `maybe_commit` took an
    /// injected one, so the two ends of the same interval came from different
    /// clocks and the cadence never fired. Injected time is a kernel rule
    /// (DP-A2) and it turns out to be worth keeping in the shell too, for the
    /// duller reason that a timer compared against a different clock is
    /// always wrong.
    pub fn append_ops(&mut self, ops: &[Op], now_ms: u64) -> Result<usize> {
        if ops.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let mut written = 0usize;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO ops (actor, counter, lamport, payload) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for op in ops {
                written += stmt.execute(params![
                    op.id.actor.0.to_be_bytes().to_vec(),
                    op.id.counter as i64,
                    op.lamport as i64,
                    // The identical bytes that were hashed (docs/26).
                    op.encode(),
                ])?;
            }
        }
        tx.commit()?;
        self.uncommitted += written;
        if self.batch_opened_ms.is_none() {
            self.batch_opened_ms = Some(now_ms);
        }
        Ok(written)
    }

    /// Flushes if the batch has been open for `AUTOSAVE_BATCH_MS`, or if
    /// `force`. Returns whether a durability point was taken.
    ///
    /// `now_ms` is a parameter so the cadence can be tested without sleeping;
    /// a test that has to wait 250 ms to check a 250 ms rule tends to become a
    /// test that waits 250 ms and checks nothing (DP-C5).
    pub fn maybe_commit(&mut self, now_ms: u64, force: bool) -> Result<bool> {
        let due = self
            .batch_opened_ms
            .is_some_and(|opened| now_ms.saturating_sub(opened) >= AUTOSAVE_BATCH_MS);
        if !force && !due {
            return Ok(false);
        }
        if self.uncommitted == 0 && !force {
            return Ok(false);
        }
        // Each `append_ops` already committed its transaction; this is the
        // WAL checkpoint that makes the data durable in the main file rather
        // than only in the write-ahead log.
        self.conn
            .pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        self.uncommitted = 0;
        self.batch_opened_ms = None;
        Ok(true)
    }

    pub fn uncommitted(&self) -> usize {
        self.uncommitted
    }

    /// Stores a snapshot, then trims the chain to [`SNAPSHOTS_RETAINED`].
    /// Content-addressed by its watermark, per docs/26's primary key.
    ///
    /// Trimming here touches **snapshots only, never ops** — dropping a
    /// snapshot can never lose data while the ops it covered are still
    /// present, so this path is unconditionally safe. Ops are pruned in
    /// [`Container::compact`] and nowhere else.
    pub fn put_snapshot(&mut self, snapshot: &Snapshot) -> Result<()> {
        let created = self.next_created()?;
        self.put_snapshot_at(snapshot, created)?;
        self.trim_snapshot_chain()?;
        Ok(())
    }

    /// Stores a snapshot with an explicit `created` stamp and does *not* trim.
    ///
    /// Compaction needs both: it rewrites the chain into a fresh file and must
    /// preserve each snapshot's age, because "newest first" is what makes
    /// docs/16's "last valid snapshot" walk mean the right thing. Reusing
    /// `put_snapshot` there would stamp three snapshots with one clock reading
    /// and lose the ordering the walk depends on.
    pub fn put_snapshot_at(&mut self, snapshot: &Snapshot, created_ms: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO snapshots (watermark, state_hash, body, created) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                snapshot.watermark.encode(),
                snapshot.state_hash.to_vec(),
                snapshot.body.clone(),
                created_ms
            ],
        )?;
        Ok(())
    }

    /// A `created` stamp strictly newer than every stored snapshot. The wall
    /// clock is the usual source, but a clock that has gone backwards must not
    /// be able to reorder the chain — age ordering is load-bearing here, so it
    /// is enforced rather than assumed.
    fn next_created(&self) -> Result<i64> {
        let newest: Option<i64> =
            self.conn
                .query_row("SELECT max(created) FROM snapshots", [], |r| r.get(0))?;
        let now = now_ms() as i64;
        Ok(match newest {
            Some(prev) if prev >= now => prev + 1,
            _ => now,
        })
    }

    /// Deletes everything past the newest [`SNAPSHOTS_RETAINED`] snapshots.
    fn trim_snapshot_chain(&mut self) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM snapshots WHERE rowid NOT IN \
             (SELECT rowid FROM snapshots ORDER BY created DESC, rowid DESC LIMIT ?1)",
            params![SNAPSHOTS_RETAINED as i64],
        )?;
        Ok(removed)
    }

    /// Snapshots newest first — the order docs/16's "last valid snapshot" walk
    /// requires.
    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        Ok(self.snapshot_rows()?.into_iter().map(|(s, _)| s).collect())
    }

    /// Snapshots newest first, each with the `created` stamp that ordered it.
    fn snapshot_rows(&self) -> Result<Vec<(Snapshot, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT watermark, state_hash, body, created FROM snapshots \
             ORDER BY created DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let watermark: Vec<u8> = r.get(0)?;
            let state_hash: Vec<u8> = r.get(1)?;
            let body: Vec<u8> = r.get(2)?;
            let created: i64 = r.get(3)?;
            Ok((watermark, state_hash, body, created))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (watermark, state_hash, body, created) = row?;
            let mut hash = [0u8; 32];
            if state_hash.len() == 32 {
                hash.copy_from_slice(&state_hash);
            }
            out.push((
                Snapshot {
                    watermark: crate::decode_watermark(&watermark),
                    state_hash: hash,
                    body,
                },
                created,
            ));
        }
        Ok(out)
    }

    /// The op tail: everything the newest verified snapshot does not already
    /// cover, in canonical replay order (docs/26's `ops_total` index).
    ///
    /// Returned as raw bytes rather than ops so SALVAGE can find the exact byte
    /// a torn write stopped at — the same shape `usk_recover::read_tail`
    /// consumes.
    pub fn tail_bytes(&self, covered: &dyn Fn(OpId) -> bool) -> Result<Vec<u8>> {
        let mut stmt = self
            .conn
            .prepare("SELECT actor, counter, payload FROM ops ORDER BY lamport, actor, counter")?;
        let rows = stmt.query_map([], |r| {
            let actor: Vec<u8> = r.get(0)?;
            let counter: i64 = r.get(1)?;
            let payload: Vec<u8> = r.get(2)?;
            Ok((actor, counter, payload))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (actor, counter, payload) = row?;
            let mut a = [0u8; 16];
            if actor.len() == 16 {
                a.copy_from_slice(&actor);
            }
            let id = OpId {
                actor: ActorId(u128::from_be_bytes(a)),
                counter: counter as u64,
            };
            if covered(id) {
                continue;
            }
            out.extend_from_slice(&payload);
        }
        Ok(out)
    }

    /// Opens the document: recover, then drive `usk-recover`'s lifecycle
    /// machine to READY (or to SALVAGE, which the caller must acknowledge).
    ///
    /// This is the only path to a usable document, which is what makes
    /// docs/27's "opening READY without hash-verifying the loaded snapshot"
    /// unreachable in the shell as well as in the kernel.
    pub fn open_document(&self) -> Result<Opened> {
        let snapshots = self.snapshots()?;
        // Ops the newest *verifiable* snapshot already contains are not tail.
        let covered_ids: std::collections::BTreeSet<(u128, u64)> = snapshots
            .iter()
            .find_map(|s| s.verify().ok())
            .map(|v| {
                v.ops()
                    .iter()
                    .map(|o| (o.id.actor.0, o.id.counter))
                    .collect()
            })
            .unwrap_or_default();
        let tail = self.tail_bytes(&|id| covered_ids.contains(&(id.actor.0, id.counter)))?;

        let salvaged = recover(&snapshots, &tail);

        let mut doc = Document::new();
        doc.step(Event::Open);
        let tail_ops = salvaged.tail.len();
        if salvaged.report.is_clean() {
            doc.step(Event::Recovered {
                snapshot: salvaged.snapshot.clone(),
                tail_ops,
            });
        } else {
            doc.step(Event::Salvaged {
                report: salvaged.report.clone(),
                snapshot: salvaged.snapshot.clone(),
                tail_ops,
            });
        }

        Ok(Opened {
            doc,
            salvaged,
            path: self.path.clone(),
        })
    }

    /// Compaction: **a new file and an atomic rename**, never an in-place
    /// rewrite (docs/26, docs/27 §2), under docs/16's retention policy.
    ///
    /// Drives the caller's `Document` through COMPACTING so the deferred-ops
    /// rule proven in `usk-recover` runs against the real file: ops arriving
    /// mid-compaction are not written to the old container and are not dropped,
    /// and are flushed once the rename lands.
    ///
    /// # Retention (docs/16, TD-30)
    /// The compacted file carries the newest [`SNAPSHOTS_RETAINED`] snapshots —
    /// the one this compaction takes, plus the most recent already present —
    /// and **every op since the oldest retained snapshot**.
    ///
    /// Two guards make "a single corrupt snapshot never loses acknowledged ops"
    /// structural rather than hoped for:
    /// * the floor is the oldest retained snapshot **that verifies**, so an
    ///   unreadable snapshot can never authorise deleting the ops it claims to
    ///   contain;
    /// * fewer than [`MIN_VERIFIED_SNAPSHOTS_TO_PRUNE_OPS`] verified snapshots
    ///   prunes nothing at all, because with one snapshot the tail is empty by
    ///   construction and that one corruption is the whole workbook.
    ///
    /// Verification here is a full replay of each retained body. That is the
    /// expensive option and it is the correct one: this is the single moment
    /// where the container decides to destroy user data, and a checksum would
    /// only prove the bytes survived, not that they still mean what they meant.
    pub fn compact(&mut self, doc: &mut Document, keep: &OpLog) -> Result<CompactReport> {
        let actions = doc.step(Event::CompactionTrigger);
        debug_assert!(actions.contains(&Action::WriteCompactedFile));

        let fresh_snapshot = Snapshot::build(keep);
        let retained = self.retention_chain(fresh_snapshot)?;
        let floor = Self::prune_floor(&retained);
        let kept_ops: Vec<Op> = keep
            .ops()
            .iter()
            .filter(|op| !floor.contains(&(op.id.actor.0, op.id.counter)))
            .cloned()
            .collect();
        let ops_pruned = keep.ops().len() - kept_ops.len();

        let tmp = self.path.with_extension("compact.tmp");
        let _ = fs::remove_file(&tmp);
        {
            let mut fresh = Container::open_or_create(&tmp)?;
            // Oldest first, so `created` ties break by rowid in age order.
            for (snapshot, created) in retained.iter().rev() {
                fresh.put_snapshot_at(snapshot, *created)?;
            }
            fresh.append_ops(&kept_ops, 0)?;
            fresh.maybe_commit(u64::MAX, true)?;
            // Drop the connection before renaming: Windows will not rename a
            // file with an open handle, and a half-renamed container is exactly
            // the corruption this path exists to avoid.
        }

        // Close ours too, for the same reason.
        let reopened_from = self.path.clone();
        self.conn
            .pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        let placeholder = Connection::open_in_memory()?;
        let old = std::mem::replace(&mut self.conn, placeholder);
        drop(old);
        for suffix in ["-wal", "-shm"] {
            let side = PathBuf::from(format!("{}{suffix}", reopened_from.display()));
            let _ = fs::remove_file(side);
        }
        fs::rename(&tmp, &reopened_from)?;
        self.conn = Connection::open(&reopened_from)?;
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "synchronous", "FULL")?;

        let actions = doc.step(Event::CompactionComplete);
        debug_assert!(actions.contains(&Action::AtomicRename));
        let flushed = actions.iter().find_map(|a| match a {
            Action::FlushDeferred { ops } => Some(*ops),
            _ => None,
        });
        Ok(CompactReport {
            ops_kept: kept_ops.len(),
            ops_pruned,
            snapshots_retained: retained.len(),
            deferred_flushed: flushed.unwrap_or(0),
        })
    }

    /// The snapshot chain the compacted file will carry: the snapshot this
    /// compaction takes, then the newest already present, newest first, capped
    /// at [`SNAPSHOTS_RETAINED`].
    ///
    /// A stored snapshot whose watermark equals the fresh one is skipped rather
    /// than kept twice — `snapshots.watermark` is docs/26's primary key, so a
    /// duplicate would collapse on insert and silently shorten the chain to two
    /// where the caller was told three.
    fn retention_chain(&self, fresh: Snapshot) -> Result<Vec<(Snapshot, i64)>> {
        let stored = self.snapshot_rows()?;
        let newest_created = stored.first().map(|(_, c)| *c).unwrap_or(i64::MIN);
        let created = (now_ms() as i64).max(newest_created.saturating_add(1));

        let mut chain = Vec::with_capacity(SNAPSHOTS_RETAINED);
        chain.push((fresh, created));
        for (snapshot, created) in stored {
            if chain.len() == SNAPSHOTS_RETAINED {
                break;
            }
            if snapshot.watermark == chain[0].0.watermark {
                continue;
            }
            chain.push((snapshot, created));
        }
        Ok(chain)
    }

    /// The op ids compaction is permitted to delete: exactly those the oldest
    /// **verified** retained snapshot proved it contains, and only once
    /// [`MIN_VERIFIED_SNAPSHOTS_TO_PRUNE_OPS`] of the chain verify.
    ///
    /// Returning an empty set means "prune nothing", which is always safe and
    /// is deliberately the default for every path that is not certain.
    fn prune_floor(chain: &[(Snapshot, i64)]) -> BTreeSet<(u128, u64)> {
        let verified: Vec<VerifiedSnapshot> =
            chain.iter().filter_map(|(s, _)| s.verify().ok()).collect();
        if verified.len() < MIN_VERIFIED_SNAPSHOTS_TO_PRUNE_OPS {
            return BTreeSet::new();
        }
        // `chain` is newest first, so the last verified entry is the oldest
        // snapshot that proved itself — docs/16 consequence 2 and 3 together.
        match verified.last() {
            Some(oldest) => oldest
                .ops()
                .iter()
                .map(|op| (op.id.actor.0, op.id.counter))
                .collect(),
            None => BTreeSet::new(),
        }
    }

    /// Final fsync and close (docs/27 §2: "no other work permitted").
    pub fn close(mut self, doc: &mut Document) -> Result<()> {
        let actions = doc.step(Event::Close);
        debug_assert_eq!(actions, vec![Action::Fsync]);
        self.maybe_commit(u64::MAX, true)?;
        Ok(())
    }

    /// Escape hatch for tests and migrations that need raw SQL. Not part of the
    /// container's contract.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        Ok(self.conn.transaction()?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CompactReport {
    /// Ops carried into the compacted file — everything since the oldest
    /// retained snapshot.
    pub ops_kept: usize,
    /// Ops the floor snapshot proved it contains, and which were therefore
    /// dropped. Zero whenever the retention guards declined to prune.
    pub ops_pruned: usize,
    /// Length of the retained snapshot chain (docs/16: up to
    /// [`SNAPSHOTS_RETAINED`]).
    pub snapshots_retained: usize,
    pub deferred_flushed: usize,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
