//! tile-bench — the Row 4 evidence harness for assumptions A-001 and A-002.
//!
//! * **A-001** (docs/42): a 10M-cell workbook fits well inside 400 MB. Reported
//!   as structural bytes per cell from the tile store itself; the caller
//!   cross-checks against OS peak working set (`tools/gates.ps1` does not run
//!   this — it is minutes of work, so it runs on demand and lands in
//!   MEASUREMENTS.md).
//! * **A-002** (docs/42, ADR-005): CRDT promotion stays rare under realistic
//!   multi-author load. Reported as the fraction of cells sitting in promoted
//!   tiles, under three collaboration patterns.
//!
//! Nothing here uses ambient time or randomness: every corpus is a pure
//! function of its seed, so a number in MEASUREMENTS.md can be reproduced
//! exactly (DP-A2, DP-B1).

use usk_oplog::{Anchor, Op, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, Decimal, OpId, RowId, Value};

/// How a second author's edits land on top of a sheet someone else authored.
///
/// This is the variable A-002 is actually sensitive to. Merely *co-locating*
/// authors does not promote anything — only a cell two actors both wrote does —
/// so the question is how contested cells are distributed, and how much of the
/// sheet each one drags into promotion with it.
#[derive(Clone, Copy)]
enum Pattern {
    /// One author, no contention. The single-user baseline.
    Solo,
    /// A second author re-writes cells in one contiguous block — the review /
    /// hand-off shape, where edits cluster.
    ClusteredContention,
    /// A second author re-writes cells scattered evenly across the sheet — the
    /// shape that punishes tile-granularity promotion hardest.
    ScatteredContention,
}

impl Pattern {
    fn name(self) -> &'static str {
        match self {
            Pattern::Solo => "solo, no contested cells",
            Pattern::ClusteredContention => "contention clustered in one block",
            Pattern::ScatteredContention => "contention scattered evenly",
        }
    }
}

/// Which value type fills the grid — one per packed tile layout (docs/14), so
/// the cost of each layout is measured rather than assumed (Row 5).
#[derive(Clone, Copy)]
enum Stored {
    /// `CellPack::Numbers` — packed f64.
    Number,
    /// `CellPack::Decimals` — packed exact base-10, the currency column.
    Decimal,
    /// `CellPack::Tagged` — the mixed-type fallback.
    Text,
}

impl Stored {
    fn make(self, v: f64) -> Value {
        match self {
            Stored::Number => Value::Number(v),
            // Cents, which is what a currency column actually holds.
            Stored::Decimal => Value::Decimal(Decimal::new((v * 100.0) as i128, -2)),
            Stored::Text => Value::Text(alloc_text(v)),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Stored::Number => "Number  (packed f64)",
            Stored::Decimal => "Decimal (packed exact base-10)",
            Stored::Text => "Text    (tagged union)",
        }
    }
}

fn alloc_text(v: f64) -> String {
    format!("{v}")
}

/// Yields a fully-ordered op stream for a `rows`x`cols` grid: actor 1 authors
/// every cell, then a second actor re-writes `contested_in_n` of them according
/// to `pattern`. Called twice by `State::replay_sorted`, and deterministic, so
/// both passes agree.
fn corpus(
    rows: usize,
    cols: usize,
    contested_in_n: usize,
    pattern: Pattern,
    stored: Stored,
) -> impl Iterator<Item = Op> {
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

    // Row-major writes: the order a bulk load actually arrives in, and the
    // order the tile append fast path is built for.
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
            value: stored.make(v),
        },
    };

    let authored = (0..rows).flat_map(move |r| {
        (0..cols).map(move |c| {
            let n = (r * cols + c) as u64;
            cell_op(1, base + n, base + n, r, c, n as f64 / 8.0)
        })
    });

    // The second author's pass, after everything actor 1 wrote.
    let rewrite_base = base + (rows * cols) as u64;
    let total = rows * cols;
    let contested = (0..total).filter_map(move |n| {
        let take = match pattern {
            Pattern::Solo => false,
            // A contiguous prefix: the same number of contested cells, packed
            // into as few tiles as possible.
            Pattern::ClusteredContention => contested_in_n != 0 && n * contested_in_n < total,
            Pattern::ScatteredContention => contested_in_n != 0 && n.is_multiple_of(contested_in_n),
        };
        if !take {
            return None;
        }
        let (r, c) = (n / cols, n % cols);
        Some(cell_op(
            2,
            rewrite_base + n as u64,
            rewrite_base + n as u64,
            r,
            c,
            -(n as f64),
        ))
    });

    structural.chain(authored).chain(contested)
}

fn build(rows: usize, cols: usize, contested_in_n: usize, pattern: Pattern) -> State {
    State::replay_sorted(|| corpus(rows, cols, contested_in_n, pattern, Stored::Number))
}

fn build_typed(rows: usize, cols: usize, stored: Stored) -> State {
    State::replay_sorted(|| corpus(rows, cols, 0, Pattern::Solo, stored))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Default is the honest A-001 shape: 10M cells. Overridable so the harness
    // is usable on a smaller machine without editing code.
    let rows: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let cols: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1_000);

    println!("== A-001: memory, {rows} x {cols} numeric cells ==");
    let state = build(rows, cols, 0, Pattern::Solo);
    let cells = rows * cols;
    let bytes = state.cell_heap_bytes();
    println!("cells          : {cells}");
    println!("tiles          : {}", state.tile_count());
    println!("structural heap: {bytes} B ({:.1} MB)", bytes as f64 / 1e6);
    println!("bytes per cell : {:.3}", bytes as f64 / cells as f64);
    println!("state hash     : {}", state.state_hash().to_hex());
    drop(state);

    // Row 5 added a second packed layout and grew `Value`. Measure all three
    // layouts on the same grid rather than reasoning about struct sizes.
    println!("\n== Row 5: cost per cell by stored type (1024 x 256) ==");
    println!("size_of::<Value>() = {} B", size_of::<Value>());
    println!(
        "{:<34} {:>12} {:>10}",
        "stored type", "heap bytes", "B/cell"
    );
    for stored in [Stored::Number, Stored::Decimal, Stored::Text] {
        let s = build_typed(1_024, 256, stored);
        let n = 1_024 * 256;
        println!(
            "{:<34} {:>12} {:>10.1}",
            stored.name(),
            s.cell_heap_bytes(),
            s.cell_heap_bytes() as f64 / n as f64
        );
    }

    // A-002 asks for <1% of cells promoted. Promotion is per tile, so the
    // number that matters is not how many cells are contested but how many
    // *tiles* the contested cells touch. The sweep varies both.
    println!("\n== A-002: promotion vs contention shape (1024 x 256 = 262,144 cells) ==");
    println!(
        "{:<36} {:>10} {:>8} {:>10} {:>16} {:>10}",
        "pattern", "contested", "tiles", "promoted", "cells promoted", "B/cell"
    );
    for pattern in [
        Pattern::Solo,
        Pattern::ClusteredContention,
        Pattern::ScatteredContention,
    ] {
        for contested_in_n in [0usize, 1000, 100] {
            if matches!(pattern, Pattern::Solo) && contested_in_n != 0 {
                continue;
            }
            if !matches!(pattern, Pattern::Solo) && contested_in_n == 0 {
                continue;
            }
            let s = build(1_024, 256, contested_in_n, pattern);
            let st = s.promotion_stats();
            let contested_pct = if contested_in_n == 0 {
                0.0
            } else {
                100.0 / contested_in_n as f64
            };
            println!(
                "{:<36} {:>9.2}% {:>8} {:>10} {:>15.2}% {:>10.1}",
                pattern.name(),
                contested_pct,
                st.tiles,
                st.promoted_tiles,
                st.promoted_cell_fraction() * 100.0,
                // What promotion actually costs — the number A-001 depends on.
                s.cell_heap_bytes() as f64 / st.cells as f64
            );
        }
    }
}
