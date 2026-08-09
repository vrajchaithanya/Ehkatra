//! image-bench — **W-IMAGE-STAMPS**: what a stamp-carrying tile image costs,
//! measured against A-001 (docs/38, TD-46).
//!
//! # The question this answers
//! `usk_state::image` produces a tile image that round-trips to the same state
//! hash, but it cannot yet *be* the snapshot body: adopting an image and
//! applying a tail loses the identity of a retained loser at any cell first
//! written inside the image, because a summary tile carries no per-cell stamps
//! (D-101). The fix is to put the stamps in the image — and the reason that is
//! a decision rather than a patch is that per-cell metadata is exactly what
//! ADR-005 exists to avoid, and TD-09 measured what it costs.
//!
//! So: how many bytes per cell, and does the load-time footprint still fit
//! A-001's 400 MB bar at 10M cells?
//!
//! # What is measured, and what is arithmetic
//! The image size and the encoded stamp sizes are **measured** — the stamps are
//! the real winners from the real corpus, encoded by the real encoders below.
//! The 10M projection is stated as a projection, because holding a winner map
//! for 10M cells in the harness would itself distort the RSS being discussed.
//!
//! Usage: `image-bench [rows] [cols]` — defaults to a grid small enough that
//! the harness's own winner map is not the dominant cost.

use std::collections::BTreeMap;

use usk_oplog::{Anchor, Op, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

/// The W-TILE-10M patterns, so the numbers here are comparable with A-001's.
#[derive(Clone, Copy)]
enum Pattern {
    Import,
    Collab,
    Adversarial,
}

impl Pattern {
    fn name(self) -> &'static str {
        match self {
            Pattern::Import => "import (1 actor)",
            Pattern::Collab => "collab (3 actors, 1%)",
            Pattern::Adversarial => "adversarial (3 actors, 50%)",
        }
    }

    fn overlap_in_n(self) -> usize {
        match self {
            Pattern::Import => 0,
            Pattern::Collab => 100,
            Pattern::Adversarial => 2,
        }
    }
}

/// The winning `(lamport, actor, counter)` for one cell — what a summary tile
/// would have to start carrying.
type Stamp = (u64, u128, u64);

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
    let cells = rows * cols;
    let authored = (0..cells).map(move |k| {
        let (r, c) = (k / cols, k % cols);
        Op {
            id: OpId {
                actor: ActorId(1),
                counter: base + k as u64,
            },
            lamport: base + k as u64,
            payload: Payload::SetCell {
                row: RowId(OpId {
                    actor: ActorId(1),
                    counter: r as u64,
                }),
                col: ColId(OpId {
                    actor: ActorId(1),
                    counter: (rows + c) as u64,
                }),
                value: Value::Number(k as f64),
            },
        }
    });

    let storm_base = base + cells as u64;
    let step = pattern.overlap_in_n();
    let storm = (0..cells)
        .filter(move |k| step != 0 && k % step == 0)
        .flat_map(move |k| {
            let (r, c) = (k / cols, k % cols);
            [2u128, 3].into_iter().map(move |actor| Op {
                id: OpId {
                    actor: ActorId(actor),
                    counter: storm_base + k as u64,
                },
                lamport: storm_base + k as u64 * 2 + actor as u64,
                payload: Payload::SetCell {
                    row: RowId(OpId {
                        actor: ActorId(1),
                        counter: r as u64,
                    }),
                    col: ColId(OpId {
                        actor: ActorId(1),
                        counter: (rows + c) as u64,
                    }),
                    value: Value::Number((k + actor as usize) as f64),
                },
            })
        });

    structural.chain(authored).chain(storm)
}

/// The winner per cell, taken from the same ordered stream `replay_sorted`
/// consumes — so these are the stamps the tile store *would* have kept.
fn winners(rows: usize, cols: usize, pattern: Pattern) -> BTreeMap<(u64, u64), Stamp> {
    let mut out: BTreeMap<(u64, u64), Stamp> = BTreeMap::new();
    for op in corpus(rows, cols, pattern) {
        if let Payload::SetCell { row, col, .. } = op.payload {
            // Canonical order, so the last write to a cell is its winner —
            // the same argument that lets a summary tile keep no stamps at all.
            out.insert(
                (row.0.counter, col.0.counter),
                (op.lamport, op.id.actor.0, op.id.counter),
            );
        }
    }
    out
}

// ------------------------------------------------------------- encodings

/// (a) The obvious one: the stamp as it sits in memory.
fn naive_bytes(stamps: &BTreeMap<(u64, u64), Stamp>) -> usize {
    stamps.len() * (8 + 16 + 8)
}

/// (b) A per-tile writer table. A *summary* tile has one author per cell and
/// few authors overall, so the 16-byte actor id becomes a one-byte index.
fn indexed_bytes(stamps: &BTreeMap<(u64, u64), Stamp>) -> usize {
    stamps.len() * (1 + 8 + 8)
}

/// (c) Writer index plus **delta-varint** lamport and counter.
///
/// The encoding the format would actually use: within a tile, cells are visited
/// in index order and a bulk write assigns lamports and counters that ascend
/// almost in lockstep, so the deltas are tiny and the varints are one byte. This
/// is measured rather than assumed — a scattered edit history would not delta
/// this well, and the adversarial pattern is here to show how much worse it
/// gets.
fn delta_varint_bytes(stamps: &BTreeMap<(u64, u64), Stamp>) -> usize {
    let mut bytes = 0usize;
    let mut last: Option<Stamp> = None;
    for stamp in stamps.values() {
        bytes += 1; // writer index
        let (lamport, _, counter) = *stamp;
        let (dl, dc) = match last {
            Some((pl, _, pc)) => (
                zigzag(lamport as i64 - pl as i64),
                zigzag(counter as i64 - pc as i64),
            ),
            None => (lamport, counter),
        };
        bytes += varint_len(dl) + varint_len(dc);
        last = Some(*stamp);
    }
    bytes
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn varint_len(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 0x80 {
        v >>= 7;
        n += 1;
    }
    n
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let cols: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let cells = rows * cols;

    println!("== W-IMAGE-STAMPS (TD-46) — the cost of a stamp-carrying tile image ==");
    println!("grid           : {rows} x {cols} = {cells} numeric cells");
    println!("A-001 bar      : collab-pattern RSS <= 400 MB at 10M cells (docs/38)\n");

    println!(
        "{:<28} {:>10} {:>11} {:>10} {:>10} {:>10} {:>10}",
        "pattern", "state B/c", "image B/c", "naive B/c", "index B/c", "delta B/c", "promoted"
    );

    let mut delta_per_cell_collab = 0.0f64;
    for pattern in [Pattern::Import, Pattern::Collab, Pattern::Adversarial] {
        let state = State::replay_sorted(|| corpus(rows, cols, pattern));
        let image = state.write_image();
        let stamps = winners(rows, cols, pattern);
        let promoted = state.promotion_stats().promoted_cell_fraction() * 100.0;

        let per = |b: usize| b as f64 / cells as f64;
        let delta = per(delta_varint_bytes(&stamps));
        if matches!(pattern, Pattern::Collab) {
            delta_per_cell_collab = delta;
        }
        println!(
            "{:<28} {:>10.2} {:>11.2} {:>10.2} {:>10.2} {:>10.2} {:>9.2}%",
            pattern.name(),
            state.cell_heap_bytes() as f64 / cells as f64,
            per(image.len()),
            per(naive_bytes(&stamps)),
            per(indexed_bytes(&stamps)),
            delta,
            promoted
        );
    }

    // The projection to 10M, stated as one. The collab pattern is the one
    // A-001's bar is written against.
    let ten_m = 10_000_000usize;
    let state_collab_mb = 123.6; // measured, W-TILE-10M, MEASUREMENTS.md
    println!("\n-- projection to 10M cells, collab pattern --");
    println!("  measured state RSS                {state_collab_mb:>8.1} MB   (W-TILE-10M)");
    for (label, per_cell) in [
        ("naive stamps (32 B/cell)", 32.0),
        ("writer index + u64s (17 B/cell)", 17.0),
        ("delta-varint (measured above)", delta_per_cell_collab),
    ] {
        let sidecar = per_cell * ten_m as f64 / (1024.0 * 1024.0);
        let total = state_collab_mb + sidecar;
        println!(
            "  {label:<33} +{sidecar:>7.1} MB = {total:>7.1} MB  {}",
            if total <= 400.0 { "PASS" } else { "FAIL" }
        );
    }
    println!(
        "\nThe sidecar only costs RSS if it is *decoded* at load. Kept encoded and\n\
         decoded per tile as the tail touches it, the load cost is the image's own\n\
         bytes and the decode is paid for the few tiles a tail actually reaches."
    );
    let _ = mb(0);
}
