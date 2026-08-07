//! calc-bench — **W-CHAIN-100K** (docs/38), the evidence harness for A-003.
//!
//! W-CHAIN-100K: 10,000 rows × 10 chained formula columns; col A holds input
//! values, cols B..K each read the column to their left — 100,000 formula cells
//! in 10 chained groups. Measures full recalc and single-edit incremental.
//!
//! **What this harness can and cannot show.** It measures a *single-threaded*
//! recalculation, because rayon lives behind the PAL `Compute` trait, which is
//! unbuilt (DP-A3, docs/10, TD-17). So it validates the graph and the
//! evaluator, and reports the level *width* — the parallelism actually
//! available — but not the "on 8 cores" half of A-003.
//!
//! Timing uses `std::time::Instant`. That is legitimate here and nowhere in the
//! kernel: this is a host-side tool, and DP-A2's ban on ambient time binds the
//! kernel crates, which take their clock injected.

use std::time::Instant;
use usk_calc::Engine;
use usk_oplog::{Anchor, Op, OpLog, Payload, RangeBinding};
use usk_state::State;
use usk_types::coerce::Profile;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

/// Builds the W-CHAIN-100K op log directly — the reducer sits above this crate,
/// so the workload is generated the way a bulk import would.
struct Book {
    log: OpLog,
    rows: Vec<RowId>,
    cols: Vec<ColId>,
    counter: u64,
    lamport: u64,
}

impl Book {
    fn new() -> Book {
        Book {
            log: OpLog::new(),
            rows: Vec::new(),
            cols: Vec::new(),
            counter: 0,
            lamport: 0,
        }
    }

    fn next(&mut self) -> OpId {
        self.counter += 1;
        self.lamport += 1;
        OpId {
            actor: ActorId(1),
            counter: self.counter,
        }
    }

    fn push(&mut self, id: OpId, payload: Payload) -> Op {
        let op = Op {
            id,
            lamport: self.lamport,
            payload,
        };
        self.log.append(op.clone());
        op
    }

    fn add_row(&mut self) {
        let anchor = self
            .rows
            .last()
            .map_or(Anchor::Start, |r: &RowId| Anchor::After(r.0));
        let id = self.next();
        self.push(id, Payload::InsertRow { anchor });
        self.rows.push(RowId(id));
    }

    fn add_col(&mut self) {
        let anchor = self
            .cols
            .last()
            .map_or(Anchor::Start, |c: &ColId| Anchor::After(c.0));
        let id = self.next();
        self.push(id, Payload::InsertCol { anchor });
        self.cols.push(ColId(id));
    }

    fn set(&mut self, row: usize, col: usize, v: f64) -> Op {
        let id = self.next();
        let (r, c) = (self.rows[row], self.cols[col]);
        self.push(
            id,
            Payload::SetCell {
                row: r,
                col: c,
                value: Value::Number(v),
            },
        )
    }

    fn formula(&mut self, row: usize, col: usize, source: &str, read_row: usize, read_col: usize) {
        let binding = RangeBinding {
            row_start: self.rows[read_row].0,
            row_end: self.rows[read_row].0,
            col_start: self.cols[read_col].0,
            col_end: self.cols[read_col].0,
            anchors: 0,
        };
        let id = self.next();
        let (r, c) = (self.rows[row], self.cols[col]);
        self.push(
            id,
            Payload::SetFormula {
                row: r,
                col: c,
                source: String::from(source),
                bindings: vec![binding],
            },
        );
    }
}

fn build(rows: usize, chain: usize) -> Book {
    let mut b = Book::new();
    for _ in 0..=chain {
        b.add_col();
    }
    for _ in 0..rows {
        b.add_row();
    }
    for r in 0..rows {
        b.set(r, 0, (r % 97) as f64);
    }
    for c in 1..=chain {
        for r in 0..rows {
            let prev = (b'A' + (c - 1) as u8) as char;
            let src = format!("={prev}{}+1", r + 1);
            b.formula(r, c, &src, r, c - 1);
        }
    }
    b
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    xs[xs.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let chain: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let formula_cells = rows * chain;

    println!("== W-CHAIN-100K (docs/38) — A-003 ==");
    println!(
        "shape          : {rows} rows x {chain} chained columns = {formula_cells} formula cells"
    );

    let t0 = Instant::now();
    let mut book = build(rows, chain);
    let gen_ms = t0.elapsed().as_secs_f64() * 1e3;

    let t1 = Instant::now();
    let mut state = State::replay(&book.log);
    let replay_ms = t1.elapsed().as_secs_f64() * 1e3;

    let t2 = Instant::now();
    let mut engine = Engine::build(&state, Profile::Compat);
    let graph_ms = t2.elapsed().as_secs_f64() * 1e3;

    println!("op generation  : {gen_ms:.1} ms");
    println!("state replay   : {replay_ms:.1} ms");
    println!("graph build    : {graph_ms:.1} ms");
    println!(
        "graph nodes    : {} groups for {formula_cells} formula cells  ({:.0} cells/node)",
        engine.group_count(),
        formula_cells as f64 / engine.group_count().max(1) as f64
    );

    let mut full = Vec::new();
    let mut stats = Default::default();
    for _ in 0..5 {
        let t = Instant::now();
        stats = engine.recalc_all(&state);
        full.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let full_ms = median(full);

    println!("\n-- full recalc (single-threaded) --");
    println!("time           : {full_ms:.1} ms   (median of 5)");
    println!("budget (A-003) : 200 ms on 8 cores — see the TD-17 caveat");
    println!("cells evaluated: {}", stats.evaluated_cells);
    println!("groups         : {}", stats.evaluated_groups);
    println!(
        "levels         : {}  (max parallel width ~ {:.1})",
        stats.levels,
        stats.evaluated_groups as f64 / stats.levels.max(1) as f64
    );
    println!(
        "throughput     : {:.2} M cells/s",
        stats.evaluated_cells as f64 / (full_ms / 1e3) / 1e6
    );

    // Incremental: edit one input cell at the head of the chain.
    let mut incr = Vec::new();
    let mut istats = Default::default();
    for i in 0..5 {
        let op = book.set(0, 0, 1000.0 + i as f64);
        state = State::replay(&book.log);
        let t = Instant::now();
        istats = engine.observe(&state, &[op]);
        incr.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let incr_ms = median(incr);

    println!("\n-- incremental recalc, one edit --");
    println!("time           : {incr_ms:.3} ms   (median of 5)");
    println!("budget (docs/31, single edit): 8 ms");
    println!("regrouped      : {}", istats.regrouped);
    println!("dirty groups   : {}", istats.dirty_groups);
    println!("evaluated      : {} groups", istats.evaluated_groups);
    println!("cut off        : {} groups", istats.cut_off_groups);
    println!("cells          : {}", istats.evaluated_cells);
    println!(
        "\nspeed-up vs full recalc: {:.0}x",
        full_ms / incr_ms.max(1e-6)
    );
}
