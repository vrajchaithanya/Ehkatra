//! calc-bench — the Row 7 evidence harness for assumption A-003 (docs/42).
//!
//! A-003 claims a 100k-dependent recalculation finishes in under 200 ms on
//! eight cores via level-parallel groups (docs/31 budget table).
//!
//! **What this harness can and cannot show.** It measures a *single-threaded*
//! recalculation, because the kernel is `no_std` and rayon lives behind the PAL
//! `Compute` trait, which does not exist yet (DP-A3, docs/10). So this run
//! validates the graph and the evaluator, and it reports the level *width* —
//! the parallelism actually available — but it cannot validate the "on 8 cores"
//! half of the claim. MEASUREMENTS.md says so rather than quietly presenting a
//! single-threaded number against a multi-core target.
//!
//! Timing uses `std::time::Instant`. That is legitimate here and nowhere in the
//! kernel: this is a host-side tool, and DP-A2's ban on ambient time binds the
//! kernel crates, which take their clock injected.

use std::time::Instant;
use usk_calc::{CellRef, Engine, Sheet};
use usk_types::coerce::Profile;
use usk_types::Value;

fn cell(row: u32, col: u32) -> CellRef {
    CellRef { row, col }
}

/// Builds the A-003 shape: `rows` source values, then `chain` columns of
/// formulas each reading the column to its left.
///
/// This is a *wide* dependency graph — the shape a real model has, where a
/// column of inputs feeds successive columns of derived values. Total formula
/// cells = rows × chain.
fn build(rows: u32, chain: u32) -> Sheet {
    let mut sheet = Sheet::new(rows, chain + 1);
    for r in 0..rows {
        sheet.set_literal(cell(r, 0), Value::Number((r % 97) as f64));
    }
    for c in 1..=chain {
        for r in 0..rows {
            // Column letters: this stays within A..Z for the widths used here.
            let prev = (b'A' + (c - 1) as u8) as char;
            sheet.set_formula(cell(r, c), &format!("={prev}{}+1", r + 1));
        }
    }
    sheet
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    xs[xs.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let chain: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let formula_cells = rows as u64 * chain as u64;

    println!("== A-003: recalculation of {formula_cells} dependent formula cells ==");
    println!("shape          : {rows} rows x {chain} chained columns");

    let t0 = Instant::now();
    let sheet = build(rows, chain);
    let build_sheet_ms = t0.elapsed().as_secs_f64() * 1e3;

    let t1 = Instant::now();
    let mut engine = Engine::build(sheet, Profile::Compat);
    let graph_ms = t1.elapsed().as_secs_f64() * 1e3;

    println!("sheet build    : {build_sheet_ms:.1} ms");
    println!("graph build    : {graph_ms:.1} ms");
    println!(
        "graph nodes    : {} groups for {formula_cells} formula cells  ({:.0} cells/node)",
        engine.group_count(),
        formula_cells as f64 / engine.group_count().max(1) as f64
    );

    // Full recalculation, repeated so the reported number is a median rather
    // than a lucky run.
    let mut full = Vec::new();
    let mut stats = Default::default();
    for _ in 0..5 {
        let t = Instant::now();
        stats = engine.recalc_all();
        full.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let full_ms = median(full);

    println!("\n-- full recalc (single-threaded) --");
    println!("time           : {full_ms:.1} ms   (median of 5)");
    println!("budget (A-003) : 200 ms on 8 cores — see the caveat below");
    println!("cells evaluated: {}", stats.evaluated_cells);
    println!("groups         : {}", stats.evaluated_groups);
    println!(
        "levels         : {}  (max parallel width = groups/levels ~ {:.1})",
        stats.levels,
        stats.evaluated_groups as f64 / stats.levels.max(1) as f64
    );
    println!(
        "throughput     : {:.2} M cells/s",
        stats.evaluated_cells as f64 / (full_ms / 1e3) / 1e6
    );

    // Incremental: one edit at the head of the chain. docs/31 budgets <8 ms.
    let mut incr = Vec::new();
    let mut istats = Default::default();
    for i in 0..5 {
        engine
            .sheet
            .set_literal(cell(0, 0), Value::Number(1000.0 + i as f64));
        let t = Instant::now();
        istats = engine.recalc_after(&[cell(0, 0)]);
        incr.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let incr_ms = median(incr);

    println!("\n-- incremental recalc, one edit --");
    println!("time           : {incr_ms:.3} ms   (median of 5)");
    println!("budget (docs/31, single edit): 8 ms");
    println!("dirty groups   : {}", istats.dirty_groups);
    println!("evaluated      : {} groups", istats.evaluated_groups);
    println!("cut off        : {} groups", istats.cut_off_groups);
    println!("cells          : {}", istats.evaluated_cells);
    println!(
        "\nspeed-up vs full recalc: {:.0}x",
        full_ms / incr_ms.max(1e-6)
    );
}
