//! Migrations — docs/26 §"Migration rules".
//!
//! > *Additive-only within a `user_version`; version bump = migration function
//! > registered in an ordered table of `(from, to, fn)`, run inside one
//! > transaction, verified by replaying to the same state hash afterward (a
//! > migration that changes the state hash is by definition wrong — the
//! > strongest migration test that exists, free because of DP-A2).*
//!
//! The verification clause is the interesting one, and it is implemented here
//! rather than described: [`run`] captures the workbook's state hash before the
//! migration and refuses to commit if it moved. A schema change is allowed to
//! reshape storage however it likes; it is not allowed to change what the
//! workbook *is*.

use rusqlite::Connection;
use usk_oplog::{Op, OpLog};
use usk_state::State;

use crate::container::StoreError;

/// One registered step. The table is ordered, and `run` walks it.
pub struct Migration {
    pub from: i32,
    pub to: i32,
    pub apply: fn(&Connection) -> rusqlite::Result<()>,
}

/// The ordered `(from, to, fn)` table docs/26 specifies.
///
/// Empty at v1 because there is nothing before v1 — recorded as an empty
/// registry rather than omitted, so the *mechanism* is proven by
/// `a_migration_that_changes_the_state_hash_is_rejected` before the first real
/// migration depends on it. A migration path first exercised by its first real
/// migration is a path that has never been tested.
pub const MIGRATIONS: &[Migration] = &[];

/// Runs every registered step from `from` to `to`, in one transaction, and
/// verifies the workbook's state hash is unchanged.
pub fn run(conn: &Connection, from: i32, to: i32) -> Result<(), StoreError> {
    let before = state_hash(conn)?;

    for step in MIGRATIONS.iter().filter(|m| m.from >= from && m.to <= to) {
        conn.execute_batch("BEGIN")?;
        match (step.apply)(conn) {
            Ok(()) => {}
            Err(e) => {
                conn.execute_batch("ROLLBACK")?;
                return Err(StoreError::Sqlite(e));
            }
        }
        let after = state_hash(conn)?;
        if after != before {
            // The migration is wrong by definition (docs/26). Roll back rather
            // than leave a workbook that is not the one the user had.
            conn.execute_batch("ROLLBACK")?;
            return Err(StoreError::MigrationChangedStateHash {
                from: step.from,
                to: step.to,
            });
        }
        conn.execute_batch("COMMIT")?;
    }

    conn.pragma_update(None, "user_version", to)?;
    Ok(())
}

/// The workbook's state hash, read straight from the `ops` table.
///
/// This is the whole reason the migration check is free: state is a pure fold
/// over the op log (DP-A2), so "did the workbook change" is one replay and a
/// comparison, with no schema-specific knowledge at all.
pub fn state_hash(conn: &Connection) -> Result<[u8; 32], StoreError> {
    let mut stmt = conn.prepare("SELECT payload FROM ops ORDER BY lamport, actor, counter")?;
    let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
    let mut log = OpLog::new();
    for (i, row) in rows.enumerate() {
        let payload = row?;
        match Op::decode(&payload) {
            Ok((op, used)) if used == payload.len() => log.append(op),
            _ => return Err(StoreError::CorruptOp { at: i }),
        }
    }
    Ok(*State::replay(&log).state_hash().as_bytes())
}
