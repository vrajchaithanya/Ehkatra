//! tile-bench — **W-TILE-10M** (docs/38), the evidence harness for A-001/A-002.
//!
//! W-TILE-10M as specified: 10M numeric cells written by 1 actor (import
//! pattern), then a 3-actor concurrent edit storm at 1% cell overlap (collab
//! pattern), then 50% overlap (adversarial). Measures RSS at load, bytes/cell,
//! **promotion rate** per pattern, and compaction ratio.
//!
//! A-002's pass bar is <1% promotion at the collab pattern.
//!
//! Nothing here uses ambient time or randomness: every corpus is a pure
//! function of its shape, so a number in MEASUREMENTS.md is reproducible
//! exactly (DP-A2, DP-B1).

extern crate alloc;

use usk_oplog::{Anchor, Op, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

/// The three W-TILE-10M patterns.
#[derive(Clone, Copy)]
enum Pattern {
    /// One actor writes every cell — the import shape.
    Import,
    /// Three actors, overlapping on 1% of cells.
    Collab,
    /// Three actors, overlapping on 50% of cells.
    Adversarial,
}

impl Pattern {
    fn name(self) -> &'static str {
        match self {
            Pattern::Import => "import (1 actor, 0% overlap)",
            Pattern::Collab => "collab (3 actors, 1% overlap)",
            Pattern::Adversarial => "adversarial (3 actors, 50% overlap)",
        }
    }

    /// One cell in `overlap_in_n` is also written by actors 2 and 3.
    fn overlap_in_n(self) -> usize {
        match self {
            Pattern::Import => 0,
            Pattern::Collab => 100,
            Pattern::Adversarial => 2,
        }
    }
}

/// Yields the fully-ordered op stream. Called twice by `State::replay_sorted`
/// (pre-pass then apply), and deterministic, so both passes agree.
fn corpus(rows: usize, cols: usize, pattern: Pattern) -> impl Iterator<Item = Op> {
    let structural = (0..rows)
        .map(move |i| Op {
            id: OpId {
                actor: ActorId(1),
                counter: i as u64,
            },
            lamport: i as u64,
            payload: Payload::InsertRow {
                anchor: if i == 0 {
                    Anchor::Start
                } else {
                    Anchor::After(OpId {
                        actor: ActorId(1),
                        counter: i as u64 - 1,
                    })
                },
            },
        })
        .chain((0..cols).map(move |j| Op {
            id: OpId {
                actor: ActorId(1),
                counter: (rows + j) as u64,
            },
            lamport: (rows + j) as u64,
            payload: Payload::InsertCol {
                anchor: if j == 0 {
                    Anchor::Start
                } else {
                    Anchor::After(OpId {
                        actor: ActorId(1),
                        counter: (rows + j) as u64 - 1,
                    })
                },
            },
        }));

    let base = (rows + cols) as u64;
    let cell_op = move |actor: u128, counter: u64, lamport: u64, r: usize, c: usize, v: f64| Op {
        id: OpId {
            actor: ActorId(actor),
            counter,
        },
        lamport,
        payload: Payload::SetCell {
            row: RowId(OpId {
                actor: ActorId(1),
                counter: r as u64,
            }),
            col: ColId(OpId {
                actor: ActorId(1),
                counter: (rows + c) as u64,
            }),
            value: Value::Number(v),
        },
    };

    // Actor 1 authors the whole sheet, row-major — the import shape, and the
    // order the tile append fast path is built for.
    let authored = (0..rows).flat_map(move |r| {
        (0..cols).map(move |c| {
            let n = (r * cols + c) as u64;
            cell_op(1, base + n, base + n, r, c, n as f64 / 8.0)
        })
    });

    // Then actors 2 and 3 re-write an evenly scattered share. Scattered, not
    // clustered: it is the shape that punished tile-granularity promotion
    // hardest, so it is the shape TD-09 has to survive.
    let total = rows * cols;
    let storm_base = base + total as u64;
    let every = pattern.overlap_in_n();
    let storm = (0..total).flat_map(move |n| {
        let hit = every != 0 && n % every == 0;
        let (r, c) = (n / cols, n % cols);
        [2u128, 3u128]
            .into_iter()
            .enumerate()
            .filter_map(move |(k, actor)| {
                if !hit {
                    return None;
                }
                let counter = storm_base + (n as u64) * 2 + k as u64;
                Some(cell_op(
                    actor,
                    counter,
                    counter,
                    r,
                    c,
                    -(n as f64) - k as f64,
                ))
            })
    });

    structural.chain(authored).chain(storm)
}

fn build(rows: usize, cols: usize, pattern: Pattern) -> State {
    State::replay_sorted(|| corpus(rows, cols, pattern))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let cols: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let cells = rows * cols;

    println!("== W-TILE-10M (docs/38) — A-001 / A-002 ==");
    println!("grid           : {rows} x {cols} = {cells} numeric cells");
    println!("size_of::<Value>() = {} B", size_of::<Value>());
    println!();
    println!(
        "{:<38} {:>8} {:>13} {:>9} {:>16}",
        "pattern", "tiles", "heap bytes", "B/cell", "promoted cells"
    );

    // An optional 3rd arg runs one pattern alone, so OS peak RSS can be
    // attributed per pattern rather than to whichever ran largest.
    let only = args.get(3).map(|s| s.as_str());
    let selected: Vec<Pattern> = match only {
        Some("import") => alloc::vec![Pattern::Import],
        Some("collab") => alloc::vec![Pattern::Collab],
        Some("adversarial") => alloc::vec![Pattern::Adversarial],
        _ => alloc::vec![Pattern::Import, Pattern::Collab, Pattern::Adversarial],
    };
    for pattern in selected {
        let state = build(rows, cols, pattern);
        let stats = state.promotion_stats();
        let bytes = state.cell_heap_bytes();
        println!(
            "{:<38} {:>8} {:>13} {:>9.2} {:>15.3}%",
            pattern.name(),
            state.tile_count(),
            bytes,
            bytes as f64 / cells as f64,
            stats.promoted_cell_fraction() * 100.0
        );
    }

    println!("\nA-002 pass bar (docs/38): < 1% promotion at the collab pattern.");
    println!(
        "Promotion is per contested CELL since TD-09, so promoted == contested:\n\
         the amplification factor is 1, which is the floor for any implementation\n\
         that must retain a concurrent loser (ADR-006)."
    );
}
