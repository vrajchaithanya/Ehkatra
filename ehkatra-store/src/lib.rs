//! ehkatra-store — the container: one workbook, one SQLite file (ADR-031,
//! docs/26, BOOTSTRAP row 11).
//!
//! `std` by construction: this is the shell adapter beneath `usk-recover`,
//! which owns everything that can be wrong in an interesting way — snapshot
//! verification, the docs/27 §2 lifecycle, the SALVAGE decision — and owns it
//! without a filesystem. What is left here is `INSERT`/`SELECT`, fsync
//! discipline and atomic rename.
//!
//! That split is deliberate and is the reason this crate is small. The
//! kernel's `no_std` gate still proves SQLite never reaches it.

pub mod container;
pub mod migrate;
pub mod schema;

pub use container::{Container, Opened, StoreError};
pub use schema::{APPLICATION_ID, SCHEMA_V1, USER_VERSION};

use usk_oplog::{Anchor, Op, Payload};
use usk_recover::snapshot::Watermark;
use usk_types::{ActorId, ColId, Counter, OpId, RowId, Value};

/// A deterministic op corpus, shared by the crash-injection writer and the test
/// that judges it.
///
/// Both sides must agree on exactly what op number *n* is, or "the container
/// kept everything it acknowledged" is unfalsifiable — the test would be
/// comparing against whatever it happened to find. Seeded and pure, so op *n*
/// is the same op in the killed process and in the process that reopens the
/// file (DP-A2 applied to a test fixture).
pub fn crash_corpus(n: usize) -> Vec<Op> {
    let actor = ActorId(0xC0FFEE);
    let row_anchor = OpId { actor, counter: 1 };
    let col_anchor = OpId { actor, counter: 2 };
    let mut ops = Vec::with_capacity(n);
    for i in 0..n {
        let counter = i as u64 + 1;
        let id = OpId { actor, counter };
        let payload = match i {
            0 => Payload::InsertRow {
                anchor: Anchor::Start,
            },
            1 => Payload::InsertCol {
                anchor: Anchor::Start,
            },
            _ => Payload::SetCell {
                row: RowId(row_anchor),
                col: ColId(col_anchor),
                // Distinct per op, so a lost write changes the state hash.
                value: Value::Number(i as f64 * 1.5),
            },
        };
        ops.push(Op {
            id,
            lamport: counter,
            payload,
        });
    }
    ops
}

/// Rebuilds a watermark from its canonical encoding (docs/26: sorted
/// `(actor, max-counter)` pairs, 24 bytes each).
///
/// The run is *replayed* rather than the counter trusted, matching how the
/// wire decoder treats a peer's clock: a stored watermark claiming coverage it
/// cannot substantiate should not be able to assert it into existence.
pub fn decode_watermark(bytes: &[u8]) -> Watermark {
    let mut ops = Vec::new();
    for chunk in bytes.chunks_exact(24) {
        let mut a = [0u8; 16];
        a.copy_from_slice(&chunk[..16]);
        let mut c = [0u8; 8];
        c.copy_from_slice(&chunk[16..24]);
        let actor = ActorId(u128::from_be_bytes(a));
        let counter = Counter::from_be_bytes(c);
        ops.push(OpId { actor, counter });
    }
    Watermark::from_pairs(ops.into_iter().map(|id| (id.actor, id.counter)))
}
