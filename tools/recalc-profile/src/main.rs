//! W-RECALC-PROFILE (TD-23) — where the time in a full recalculation goes.
//!
//! # Why this exists rather than a profiler
//! No sampling profiler runs on this host without elevation (DP-S5: no admin),
//! and the kernel is `no_std` so it cannot time itself. What is available is
//! the method that found TD-66's quadratic: **vary one thing and measure**.
//!
//! Three experiments, discriminating three different suspects.
//!
//! **1. Shape.** Reads per formula at a fixed formula count, plus a *gapped*
//! variant. Every cell a formula reads costs one `results` lookup; every cell it
//! writes costs one lookup and one insert. So `narrow` (1 read) and `wide` (12)
//! pay the same write cost and 12× the read cost, and fitting a line through
//! them separates the per-formula cost from the per-read cost. The gapped
//! variant varies neither — it changes only whether a group's read rectangles
//! merge.
//!
//! **2. The data structure on its own.** The same population, the same access
//! pattern, outside the engine — which turns "the map is slow" into a figure in
//! nanoseconds and prices the alternatives before one is built.
//!
//! **3. `State::cell`.** What a read costs when the results miss, which is every
//! read of a plain value and therefore most reads.
//!
//! # What they found (session 26)
//! The three costs are separable and have three different owners:
//!
//! * **per formula, 1.554 → 0.383 µs** — the results map's lookup-and-insert per
//!   member. That was TD-23 and it is fixed.
//! * **per read, ~300 ns, of which ~195–255 ns is `State::cell`** — three
//!   `BTreeMap` lookups inside the tile store per cell read. **Flat** in sheet
//!   size, so it is a constant and not a scaling problem, but it is the largest
//!   remaining term on read-heavy sheets.
//! * **the stab over a group's read rectangles** — invisible when the
//!   rectangles merge to one, and growing with the sheet when they do not. At
//!   500,000 rows the gapped corpus has **33% fewer formulas and takes 15%
//!   longer**; at 100,000 the two are equal. That is TD-20.
//!
//! This lives here rather than in `calc-bench` because `calc-bench` measures the
//! *budget* (A-003, docs/31) and must keep meaning the same thing run to run.
//! This one is a microscope and its shapes will change as the questions do.

use std::collections::BTreeMap;
use std::time::Instant;

use usk_calc::Engine;
use usk_oplog::{Anchor, Op, OpLog, Payload, RangeBinding};
use usk_state::State;
use usk_types::coerce::Profile;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn main() {
    let rows: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);

    println!("== W-RECALC-PROFILE (TD-23) ==\n");
    shape_experiment(rows);
    println!();
    structure_experiment(rows);
    println!();
    read_experiment(rows);
    println!();
    locate_experiment(rows);
}

// ------------------------------------- 4. what, inside a read, is the lookups

/// Decomposes `State::cell` (TD-71). The register claims the cost is
/// `TileStore::locate`'s three `BTreeMap` lookups — row identity → slot,
/// column identity → slot, tile key → tile. Three-for-three says: check.
///
/// The three maps are rebuilt here with the same content, the same key types
/// and the same probe pattern as experiment 3, so the difference between
/// "the three lookups alone" and "the whole of `State::cell`" is the residue —
/// `Presence::rank`'s popcount walk plus the payload fetch.
fn locate_experiment(rows: usize) {
    println!("-- experiment 4: State::cell decomposed (TD-71) --");
    println!("   The three BTreeMap lookups of `TileStore::locate`, rebuilt");
    println!("   outside the store with identical content and probe pattern.\n");

    let (state, _) = corpus(rows, 12, false);
    let row_ids = state.row_order();
    let col_ids = state.col_order();
    let probes = rows * 12;

    // Slot maps: slot order is creation order (ADR-034), which for this corpus
    // is exactly `row_order` / `col_order`.
    let row_slots: BTreeMap<RowId, u32> = row_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i as u32))
        .collect();
    let col_slots: BTreeMap<ColId, u32> = col_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i as u32))
        .collect();
    // The tile map: every (row band, col band) the corpus populates.
    let mut tiles: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    for r in 0..row_ids.len() as u32 {
        for c in 0..col_ids.len() as u32 {
            tiles.insert((r / 256, c / 64), (r as u64) << 8 | c as u64);
        }
    }

    let t = Instant::now();
    let mut sink = 0u64;
    for i in 0..probes {
        let r = row_ids[i % row_ids.len()];
        let c = col_ids[i % 12];
        if let (Some(rs), Some(cs)) = (row_slots.get(&r), col_slots.get(&c)) {
            if let Some(v) = tiles.get(&(rs / 256, cs / 64)) {
                sink = sink.wrapping_add(*v);
            }
        }
    }
    let three = t.elapsed();

    let t = Instant::now();
    let mut hits = 0usize;
    for i in 0..probes {
        let r = row_ids[i % row_ids.len()];
        let c = col_ids[i % 12];
        if state.cell(r, c).is_some() {
            hits += 1;
        }
    }
    let whole = t.elapsed();

    let ns = |d: std::time::Duration| d.as_secs_f64() * 1e9 / probes as f64;
    println!(
        "   {:<34} {:>10.1} ns/read",
        "three lookups alone (simulated)",
        ns(three)
    );
    println!(
        "   {:<34} {:>10.1} ns/read",
        "State::cell (whole)",
        ns(whole)
    );
    println!(
        "   {:<34} {:>10.1} ns/read   ({hits} hits)",
        "residue (rank + payload + call)",
        ns(whole) - ns(three)
    );
    if sink == u64::MAX {
        println!("   (unreachable {sink})");
    }
}

// ----------------------------------------------------- 3. what a read costs

/// The other half of `EngineGrid::read`: when the results miss — which is every
/// read of a plain value, and that is most reads — the answer comes from
/// `State::cell`.
///
/// Measured on its own because experiments 1 and 2 together said the results
/// map was *not* what a read costs, and "not that" is only half an answer.
fn read_experiment(rows: usize) {
    println!("-- experiment 3: what a read costs when the results miss --");
    println!("   `State::cell` at {rows} rows, the call `EngineGrid::read` makes");
    println!("   for every plain value a formula reads.\n");

    let (state, _) = corpus(rows, 12, false);
    let row_ids = state.row_order();
    let col_ids = state.col_order();
    let probes = rows * 12;

    let t = Instant::now();
    let mut hits = 0usize;
    for i in 0..probes {
        let r = row_ids[i % row_ids.len()];
        let c = col_ids[i % 12];
        if state.cell(r, c).is_some() {
            hits += 1;
        }
    }
    let elapsed = t.elapsed();
    println!(
        "   {:<28} {:>12.1?} {:>12.1} ns/read   ({hits} hits)",
        "State::cell",
        elapsed,
        elapsed.as_secs_f64() * 1e9 / probes as f64
    );

    // The same reads through the TD-71 rect path, shaped as `EngineGrid`
    // issues them during this corpus's recalc: one 1×12 run per formula.
    let t = Instant::now();
    let mut rect_hits = 0usize;
    for i in 0..rows {
        state.read_rect(&row_ids[i..i + 1], &col_ids[0..12], |_, _, _| {
            rect_hits += 1;
        });
    }
    let elapsed = t.elapsed();
    println!(
        "   {:<28} {:>12.1?} {:>12.1} ns/read   ({rect_hits} hits)",
        "State::read_rect (1x12 runs)",
        elapsed,
        elapsed.as_secs_f64() * 1e9 / probes as f64
    );
    println!("   For comparison, the slot-indexed results miss in ~5 ns and the");
    println!("   BTreeMap it replaced missed in ~149.");
}

// ------------------------------------------------------- 1. reads per formula

/// How many cells each formula reads, at a fixed number of formulas.
struct Shape {
    name: &'static str,
    reads: usize,
    /// Leave every third row without a formula. The rectangles a group reads
    /// then cannot all merge (TD-66), so the group carries thousands of them
    /// instead of one — and `BandIndex::stab` scans every one of a candidate
    /// group's rectangles per band (TD-20).
    gapped: bool,
}

fn shape_experiment(rows: usize) {
    println!("-- experiment 1: reads per formula, at a fixed formula count --");
    println!("   Every read is one `results` lookup; every written cell is one");
    println!("   lookup plus one insert. If the map dominates, time tracks reads.\n");
    println!(
        "   {:<10} {:>10} {:>10} {:>12} {:>14} {:>12}",
        "shape", "formulas", "reads/f", "total reads", "full recalc", "us/formula"
    );

    for shape in [
        Shape {
            name: "narrow",
            reads: 1,
            gapped: false,
        },
        Shape {
            name: "medium",
            reads: 4,
            gapped: false,
        },
        Shape {
            name: "wide",
            reads: 12,
            gapped: false,
        },
        Shape {
            name: "wide/gap",
            reads: 12,
            gapped: true,
        },
    ] {
        let (state, formulas) = corpus(rows, shape.reads, shape.gapped);
        let mut engine = Engine::build(&state, Profile::Compat);
        let t = Instant::now();
        let stats = engine.recalc_all(&state);
        let elapsed = t.elapsed();
        assert_eq!(
            stats.evaluated_cells, formulas,
            "{} evaluated wrong",
            shape.name
        );
        println!(
            "   {:<10} {:>10} {:>10} {:>12} {:>11.1?} {:>12.3}",
            shape.name,
            formulas,
            shape.reads,
            formulas * shape.reads,
            elapsed,
            elapsed.as_secs_f64() * 1e6 / formulas as f64,
        );
    }
}

/// A sheet of `rows` rows where every row holds `reads` values and one formula
/// summing them. Gap-free, so `extent_of` merges perfectly and TD-66's fixed
/// quadratic cannot contaminate the measurement.
fn corpus(rows: usize, reads: usize, gapped: bool) -> (State, usize) {
    let mut log = OpLog::new();
    let actor = ActorId(1);
    let (mut counter, mut lamport) = (0u64, 0u64);
    let mut push = |log: &mut OpLog, payload: Payload| -> OpId {
        counter += 1;
        lamport += 1;
        let id = OpId { actor, counter };
        log.append(Op {
            id,
            lamport,
            payload,
        });
        id
    };

    let cols = reads + 1;
    let mut col_ids = Vec::with_capacity(cols);
    let mut anchor = Anchor::Start;
    for _ in 0..cols {
        let id = push(&mut log, Payload::InsertCol { anchor });
        anchor = Anchor::After(id);
        col_ids.push(id);
    }
    let mut row_ids = Vec::with_capacity(rows);
    let mut anchor = Anchor::Start;
    for _ in 0..rows {
        let id = push(&mut log, Payload::InsertRow { anchor });
        anchor = Anchor::After(id);
        row_ids.push(id);
    }

    let mut formulas = 0usize;
    for (r, row) in row_ids.iter().enumerate() {
        if gapped && r % 3 == 2 {
            continue;
        }
        formulas += 1;
        for (c, col) in col_ids.iter().take(reads).enumerate() {
            push(
                &mut log,
                Payload::SetCell {
                    row: RowId(*row),
                    col: ColId(*col),
                    value: Value::Number((r * cols + c) as f64),
                },
            );
        }
        let last = column_label(reads - 1);
        let source = format!("=SUM(A{n}:{last}{n})", n = r + 1);
        push(
            &mut log,
            Payload::SetFormula {
                row: RowId(*row),
                col: ColId(col_ids[reads]),
                source,
                bindings: vec![RangeBinding {
                    row_start: *row,
                    row_end: *row,
                    col_start: col_ids[0],
                    col_end: col_ids[reads - 1],
                    anchors: 0,
                }],
            },
        );
    }

    (State::replay(&log), formulas)
}

fn column_label(index: usize) -> String {
    let mut out = Vec::new();
    let mut n = index + 1;
    while n > 0 {
        out.push(b'A' + ((n - 1) % 26) as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

// ------------------------------------------------- 2. the structure on its own

/// The same population, the same access pattern, outside the engine.
///
/// Three candidates: what the engine uses today, the same tree with a **packed
/// position** key, and a flat vector — the ceiling, which is only reachable
/// where positions are dense.
fn structure_experiment(rows: usize) {
    println!("-- experiment 2: the result map on its own --");
    println!("   {rows} entries, then 12 lookups per entry — the access pattern");
    println!("   `EngineGrid::read` produces for a 12-cell SUM.\n");

    let keys: Vec<(RowId, ColId)> = (0..rows)
        .map(|i| {
            (
                RowId(OpId {
                    actor: ActorId(1),
                    counter: i as u64 + 1,
                }),
                ColId(OpId {
                    actor: ActorId(1),
                    counter: 1,
                }),
            )
        })
        .collect();
    let packed: Vec<u64> = (0..rows as u64).map(|i| i << 20).collect();
    let probes = rows * 12;

    // --- what the engine uses today: a tree keyed by two 24-byte identities.
    let t = Instant::now();
    let mut identity: BTreeMap<(RowId, ColId), Value> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        identity.insert(*k, Value::Number(i as f64));
    }
    let identity_insert = t.elapsed();
    let t = Instant::now();
    let mut sink = 0.0f64;
    for i in 0..probes {
        if let Some(Value::Number(n)) = identity.get(&keys[i % rows]) {
            sink += *n;
        }
    }
    let identity_get = t.elapsed();

    // --- the same tree, keyed by a packed `(row, col)` position.
    let t = Instant::now();
    let mut position: BTreeMap<u64, Value> = BTreeMap::new();
    for (i, k) in packed.iter().enumerate() {
        position.insert(*k, Value::Number(i as f64));
    }
    let position_insert = t.elapsed();
    let t = Instant::now();
    for i in 0..probes {
        if let Some(Value::Number(n)) = position.get(&packed[i % rows]) {
            sink += *n;
        }
    }
    let position_get = t.elapsed();

    // --- the ceiling: a flat vector, indexed.
    let t = Instant::now();
    let mut flat: Vec<Option<Value>> = vec![None; rows];
    for (i, slot) in flat.iter_mut().enumerate() {
        *slot = Some(Value::Number(i as f64));
    }
    let flat_insert = t.elapsed();
    let t = Instant::now();
    for i in 0..probes {
        if let Some(Value::Number(n)) = &flat[i % rows] {
            sink += *n;
        }
    }
    let flat_get = t.elapsed();

    let ns = |d: std::time::Duration, n: usize| d.as_secs_f64() * 1e9 / n as f64;
    println!(
        "   {:<28} {:>12} {:>12} {:>12} {:>12}",
        "structure", "insert", "ns/insert", "lookup", "ns/lookup"
    );
    for (name, ins, get) in [
        ("BTreeMap<(RowId,ColId),V>", identity_insert, identity_get),
        ("BTreeMap<u64,V>  (packed)", position_insert, position_get),
        ("Vec<Option<V>>   (ceiling)", flat_insert, flat_get),
    ] {
        println!(
            "   {:<28} {:>11.1?} {:>12.1} {:>11.1?} {:>12.1}",
            name,
            ins,
            ns(ins, rows),
            get,
            ns(get, probes)
        );
    }
    // Keeps the optimiser from deleting the probes.
    if sink < 0.0 {
        println!("   (unreachable {sink})");
    }
}
