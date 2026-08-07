//! replay-check — the DP-A2 determinism gate.
//! Builds a fixed, deterministic op corpus (LCG-driven, no ambient entropy),
//! replays it, and prints the canonical state hash. CI runs this binary on
//! native AND wasm32; the printed hashes MUST be identical.

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn main() {
    let mut log = OpLog::new();
    let mut rows: Vec<OpId> = Vec::new();
    let mut cols: Vec<OpId> = Vec::new();
    let mut seed: u64 = 0xDEADBEEFCAFEBABE;
    let mut rand = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };

    // 3 actors, 5000 ops: structural + cell writes + deletes, fully seeded.
    for i in 0..5000u64 {
        // Per-op counter is 1-based and advances in lockstep with the loop, so
        // the corpus is a pure function of the seed (DP-A2).
        let counter = i + 1;
        let actor = ActorId(1 + (rand() % 3) as u128);
        let id = OpId { actor, counter };
        let lamport = i + 1;
        let payload = match rand() % 10 {
            0 => {
                let anchor = if rows.is_empty() {
                    Anchor::Start
                } else {
                    Anchor::After(rows[(rand() as usize) % rows.len()])
                };
                rows.push(id);
                Payload::InsertRow { anchor }
            }
            1 => {
                let anchor = if cols.is_empty() {
                    Anchor::Start
                } else {
                    Anchor::After(cols[(rand() as usize) % cols.len()])
                };
                cols.push(id);
                Payload::InsertCol { anchor }
            }
            2 if rows.len() > 4 => {
                let r = rows[(rand() as usize) % rows.len()];
                Payload::DeleteRow { row: RowId(r) }
            }
            _ => {
                if rows.is_empty() || cols.is_empty() {
                    rows.push(id);
                    Payload::InsertRow {
                        anchor: Anchor::Start,
                    }
                } else {
                    let r = rows[(rand() as usize) % rows.len()];
                    let c = cols[(rand() as usize) % cols.len()];
                    let v = match rand() % 4 {
                        0 => Value::Number((rand() % 100000) as f64 / 100.0),
                        1 => Value::Bool(rand() % 2 == 0),
                        2 => Value::Text(alloc_text(rand())),
                        _ => Value::Blank,
                    };
                    Payload::SetCell {
                        row: RowId(r),
                        col: ColId(c),
                        value: v,
                    }
                }
            }
        };
        log.append(Op {
            id,
            lamport,
            payload,
        });
    }

    let state = State::replay(&log);
    println!("oplog:{}", log.canonical_hash());
    println!("state:{}", state.state_hash());
}

fn alloc_text(n: u64) -> String {
    format!("cell-{n}")
}
