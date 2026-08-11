//! Row 7 + Row 9 proofs: formula grouping, range-granular edges, incremental
//! recalc with early cutoff, level ordering, cycle detection, and the
//! regrouping trigger (docs/13, docs/27 §3).
//!
//! Everything here addresses cells by **identity**. There is no ordinal
//! addressing left in `usk-calc` — the closure of TD-21.

use usk_calc::graph::r1c1;
use usk_calc::Engine;
use usk_formula::parse::parse;
use usk_oplog::{Anchor, Op, OpLog, Payload, RangeBinding};
use usk_state::State;
use usk_types::coerce::Profile;
use usk_types::{ActorId, ColId, ErrorKind, OpId, RowId, Value};

/// Builds op logs the way the reducer would, without depending on it (that
/// would be an upward dependency).
struct Book {
    log: OpLog,
    rows: Vec<RowId>,
    cols: Vec<ColId>,
    counter: u64,
    lamport: u64,
}

impl Book {
    fn new(rows: usize, cols: usize) -> Book {
        let mut b = Book {
            log: OpLog::new(),
            rows: Vec::new(),
            cols: Vec::new(),
            counter: 0,
            lamport: 0,
        };
        for _ in 0..cols {
            b.add_col();
        }
        for _ in 0..rows {
            b.add_row();
        }
        b
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

    fn add_row(&mut self) -> RowId {
        let anchor = self
            .rows
            .last()
            .map_or(Anchor::Start, |r: &RowId| Anchor::After(r.0));
        let id = self.next();
        self.push(id, Payload::InsertRow { anchor });
        self.rows.push(RowId(id));
        RowId(id)
    }

    fn add_col(&mut self) -> ColId {
        let anchor = self
            .cols
            .last()
            .map_or(Anchor::Start, |c: &ColId| Anchor::After(c.0));
        let id = self.next();
        self.push(id, Payload::InsertCol { anchor });
        self.cols.push(ColId(id));
        ColId(id)
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

    /// A formula whose references are bound to identities, exactly as the
    /// reducer would emit them. `refs` are `(row, col)` ordinal pairs for
    /// single-cell references, in AST traversal order.
    fn formula(&mut self, row: usize, col: usize, source: &str, refs: &[(usize, usize)]) -> Op {
        self.formula_ranges(
            row,
            col,
            source,
            &refs
                .iter()
                .map(|(r, c)| (*r, *r, *c, *c))
                .collect::<Vec<_>>(),
        )
    }

    /// As above, for range references: `(row_start, row_end, col_start, col_end)`.
    fn formula_ranges(
        &mut self,
        row: usize,
        col: usize,
        source: &str,
        refs: &[(usize, usize, usize, usize)],
    ) -> Op {
        let bindings: Vec<RangeBinding> = refs
            .iter()
            .map(|(r0, r1, c0, c1)| RangeBinding {
                row_start: self.rows[*r0].0,
                row_end: self.rows[*r1].0,
                col_start: self.cols[*c0].0,
                col_end: self.cols[*c1].0,
                anchors: 0,
            })
            .collect();
        let id = self.next();
        let (r, c) = (self.rows[row], self.cols[col]);
        self.push(
            id,
            Payload::SetFormula {
                row: r,
                col: c,
                source: String::from(source),
                bindings,
            },
        )
    }

    fn state(&self) -> State {
        State::replay(&self.log)
    }
}

fn approx(engine: &Engine, state: &State, row: RowId, col: ColId) -> f64 {
    match engine.value(state, row, col) {
        Some(Value::Number(n)) => n,
        Some(Value::Decimal(d)) => d.to_f64(),
        other => panic!("expected a number, got {other:?}"),
    }
}

fn kind_at(engine: &Engine, state: &State, row: RowId, col: ColId) -> Option<ErrorKind> {
    engine
        .value(state, row, col)
        .and_then(|v| v.as_error())
        .map(|e| e.kind)
}

/// `rows` rows of `=A{n}*2` in column B — the canonical fill.
fn filled(rows: usize) -> Book {
    let mut b = Book::new(rows, 3);
    for r in 0..rows {
        b.set(r, 0, r as f64);
    }
    for r in 0..rows {
        b.formula(r, 1, &format!("=A{}*2", r + 1), &[(r, 0)]);
    }
    b
}

// ------------------------------------------------------- formula groups

/// **The claim that makes the graph scale** (docs/13): a filled column is *one*
/// node, not one per cell.
#[test]
fn a_filled_column_collapses_to_one_group() {
    let b = filled(1_000);
    let state = b.state();
    let engine = Engine::build(&state, Profile::Compat);
    assert_eq!(engine.group_count(), 1);
    let group = &engine.groups()[0];
    assert_eq!(group.cells.len(), 1_000);
    assert_eq!(group.reads.len(), 1, "reads: {:?}", group.reads);
    assert_eq!(group.reads[0].cell_count(), 1_000);
}

/// R1C1 is the grouping key: relative references normalise, absolute ones pin.
#[test]
fn r1c1_normalises_relative_and_pins_absolute() {
    let a = r1c1(&parse("=A1*2").ast, 0, 1);
    let b = r1c1(&parse("=A2*2").ast, 1, 1);
    assert_eq!(a, b, "a fill-down must produce one pattern");
    assert_ne!(a, r1c1(&parse("=$A$1*2").ast, 0, 1));
    assert_ne!(a, r1c1(&parse("=A1*3").ast, 0, 1));
}

#[test]
fn different_patterns_form_different_groups() {
    let mut b = Book::new(4, 3);
    for r in 0..4 {
        b.set(r, 0, r as f64 + 1.0);
    }
    b.formula(0, 1, "=A1*2", &[(0, 0)]);
    b.formula(1, 1, "=A2*2", &[(1, 0)]);
    b.formula(2, 1, "=A3+100", &[(2, 0)]);
    b.formula(3, 1, "=A4+100", &[(3, 0)]);
    let state = b.state();
    assert_eq!(Engine::build(&state, Profile::Compat).group_count(), 2);
}

// ----------------------------------------------------------- evaluation

#[test]
fn recalc_all_computes_every_formula() {
    let b = filled(10);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let stats = engine.recalc_all(&state);
    assert_eq!(stats.evaluated_cells, 10);
    for r in 0..10 {
        assert_eq!(
            approx(&engine, &state, b.rows[r], b.cols[1]),
            r as f64 * 2.0
        );
    }
}

#[test]
fn chained_groups_evaluate_in_dependency_order() {
    let mut b = Book::new(1, 4);
    b.set(0, 0, 5.0);
    b.formula(0, 1, "=A1*2", &[(0, 0)]);
    b.formula(0, 2, "=B1+1", &[(0, 1)]);
    b.formula(0, 3, "=C1*10", &[(0, 2)]);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let stats = engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[3]), 110.0);
    assert_eq!(stats.levels, 3);
}

#[test]
fn independent_groups_share_a_level() {
    let mut b = Book::new(1, 5);
    b.set(0, 0, 2.0);
    b.formula(0, 1, "=A1*2", &[(0, 0)]);
    b.formula(0, 2, "=A1*3", &[(0, 0)]);
    b.formula(0, 3, "=A1*4", &[(0, 0)]);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let stats = engine.recalc_all(&state);
    assert_eq!(stats.evaluated_groups, 3);
    assert_eq!(stats.levels, 1, "three independents are one parallel level");
}

/// The generation mark (docs/27 §3): every completed pass advances it, so a
/// reader can tell a half-evaluated view from a settled one.
#[test]
fn every_completed_pass_advances_the_generation() {
    let b = filled(4);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let g0 = engine.generation();
    engine.recalc_all(&state);
    let g1 = engine.generation();
    assert!(g1 > g0);
    engine.recalc_after(&state, &[(b.rows[0], b.cols[0])]);
    assert!(engine.generation() > g1);
}

// --------------------------------------------------------- incremental

#[test]
fn an_edit_recalculates_only_its_dependents() {
    let mut b = Book::new(1, 4);
    b.set(0, 0, 1.0);
    b.formula(0, 1, "=A1*2", &[(0, 0)]);
    b.set(0, 2, 7.0);
    b.formula(0, 3, "=C1*2", &[(0, 2)]);
    let mut state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 2.0);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[3]), 14.0);

    let edit = b.set(0, 0, 10.0);
    state = b.state();
    let stats = engine.observe(&state, &[edit]);

    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 20.0);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[3]), 14.0);
    assert_eq!(stats.dirty_groups, 1);
    assert_eq!(stats.evaluated_groups, 1);
    assert!(!stats.regrouped, "a value edit must not force a regroup");
}

/// Early cutoff (docs/13): unchanged results do not force downstream work.
#[test]
fn unchanged_results_cut_off_propagation() {
    let mut b = Book::new(1, 4);
    b.set(0, 0, 3.0);
    b.formula(0, 1, "=SIGN(A1)", &[(0, 0)]);
    b.formula(0, 2, "=B1*100", &[(0, 1)]);
    let mut state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[2]), 100.0);

    // 3 → 9 leaves SIGN(A1) at 1, so C1 must be cut off.
    let edit = b.set(0, 0, 9.0);
    state = b.state();
    let stats = engine.observe(&state, &[edit]);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[2]), 100.0);
    assert_eq!(stats.dirty_groups, 2);
    assert_eq!(stats.cut_off_groups, 1);

    let edit = b.set(0, 0, -2.0);
    state = b.state();
    engine.observe(&state, &[edit]);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[2]), -100.0);
}

#[test]
fn range_reads_are_stabbed_not_enumerated() {
    let mut b = Book::new(100, 2);
    for r in 0..100 {
        b.set(r, 0, 1.0);
    }
    b.formula_ranges(0, 1, "=SUM(A1:A100)", &[(0, 99, 0, 0)]);
    let mut state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 100.0);

    let edit = b.set(50, 0, 11.0);
    state = b.state();
    let stats = engine.observe(&state, &[edit]);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 110.0);
    assert_eq!(stats.evaluated_groups, 1);
}

// -------------------------------------------------- the regroup trigger

/// **TD-18**: a formula op regroups; a value op does not. The caller never has
/// to know which kind of edit it just made.
#[test]
fn formula_and_structural_ops_trigger_a_regroup() {
    let mut b = Book::new(3, 3);
    b.set(0, 0, 4.0);
    b.formula(0, 1, "=A1*2", &[(0, 0)]);
    let mut state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(engine.group_count(), 1);

    // A new formula: the group set changes, so the graph must be rebuilt.
    let op = b.formula(1, 1, "=A2+100", &[(1, 0)]);
    state = b.state();
    let stats = engine.observe(&state, &[op]);
    assert!(stats.regrouped, "a formula op must regroup");
    assert_eq!(engine.group_count(), 2);

    // A value edit: no regroup.
    let op = b.set(0, 0, 5.0);
    state = b.state();
    let stats = engine.observe(&state, &[op]);
    assert!(!stats.regrouped);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 10.0);
}

/// A structural edit moves every derived position, and the formula still
/// evaluates against the identities it was bound to (DP-A6). Nothing rewrites
/// the formula text.
#[test]
fn a_row_insert_regroups_and_references_still_hold() {
    let mut b = Book::new(3, 2);
    b.set(0, 0, 10.0);
    b.set(1, 0, 20.0);
    b.set(2, 0, 30.0);
    b.formula_ranges(0, 1, "=SUM(A1:A3)", &[(0, 2, 0, 0)]);
    let mut state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 60.0);

    // Insert a row between rows 1 and 2 — inside the referenced interval.
    let id = b.next();
    let anchor = Anchor::After(b.rows[0].0);
    let op = b.push(id, Payload::InsertRow { anchor });
    let new_row = RowId(id);
    b.rows.insert(1, new_row);
    let value_op = {
        let vid = b.next();
        b.push(
            vid,
            Payload::SetCell {
                row: new_row,
                col: b.cols[0],
                value: Value::Number(5.0),
            },
        )
    };
    state = b.state();
    let stats = engine.observe(&state, &[op, value_op]);

    assert!(stats.regrouped, "a structural op must regroup");
    assert_eq!(
        approx(&engine, &state, b.rows[0], b.cols[1]),
        65.0,
        "the inserted row joined the range, with no formula rewriting"
    );
}

/// Deleting every row a formula references yields `#REF!` — docs/11's
/// empty-interval rule, surfacing through evaluation.
#[test]
fn an_emptied_reference_evaluates_to_ref() {
    let mut b = Book::new(3, 2);
    b.set(1, 0, 7.0);
    b.formula(0, 1, "=A2", &[(1, 0)]);
    let mut state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 7.0);

    let id = b.next();
    let op = b.push(id, Payload::DeleteRow { row: b.rows[1] });
    state = b.state();
    engine.observe(&state, &[op]);
    assert_eq!(
        kind_at(&engine, &state, b.rows[0], b.cols[1]),
        Some(ErrorKind::Ref)
    );
}

// --------------------------------------------------------------- cycles

#[test]
fn cycles_are_detected_and_reported_as_circ() {
    let mut b = Book::new(1, 3);
    b.formula(0, 0, "=B1+1", &[(0, 1)]);
    b.formula(0, 1, "=A1+1", &[(0, 0)]);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let stats = engine.recalc_all(&state);
    assert_eq!(stats.circular_groups, 2);
    assert_eq!(
        kind_at(&engine, &state, b.rows[0], b.cols[0]),
        Some(ErrorKind::Circ)
    );
    assert_eq!(
        kind_at(&engine, &state, b.rows[0], b.cols[1]),
        Some(ErrorKind::Circ)
    );
}

#[test]
fn self_reference_is_circular() {
    let mut b = Book::new(1, 2);
    b.formula(0, 0, "=A1+1", &[(0, 0)]);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let stats = engine.recalc_all(&state);
    assert_eq!(stats.circular_groups, 1);
    assert_eq!(
        kind_at(&engine, &state, b.rows[0], b.cols[0]),
        Some(ErrorKind::Circ)
    );
}

#[test]
fn a_cycle_does_not_stop_unrelated_groups() {
    let mut b = Book::new(1, 5);
    b.formula(0, 0, "=B1+1", &[(0, 1)]);
    b.formula(0, 1, "=A1+1", &[(0, 0)]);
    b.set(0, 2, 4.0);
    b.formula(0, 3, "=C1*5", &[(0, 2)]);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    let stats = engine.recalc_all(&state);
    assert_eq!(stats.circular_groups, 2);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[3]), 20.0);
}

// ------------------------------------------------------------ volatiles

#[test]
fn volatiles_are_materialised_not_computed() {
    let mut b = Book::new(1, 2);
    b.formula(0, 0, "=TODAY()+1", &[]);
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.today = 45_000;
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[0]), 45_001.0);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[0]), 45_001.0);

    engine.today = 45_100;
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[0]), 45_101.0);
}

// ------------------------------------------------------------ ordering

#[test]
fn recalculation_is_deterministic() {
    let build = || {
        let mut b = Book::new(20, 4);
        for r in 0..20 {
            b.set(r, 0, r as f64);
        }
        for r in 0..20 {
            b.formula(r, 1, &format!("=A{}*2", r + 1), &[(r, 0)]);
        }
        for r in 0..20 {
            b.formula(r, 2, &format!("=B{}+1", r + 1), &[(r, 1)]);
        }
        b.formula_ranges(0, 3, "=SUM(C1:C20)", &[(0, 19, 2, 2)]);
        let state = b.state();
        let mut e = Engine::build(&state, Profile::Compat);
        let stats = e.recalc_all(&state);
        let total = approx(&e, &state, b.rows[0], b.cols[3]);
        (stats, total, e.group_count())
    };
    let first = build();
    let second = build();
    assert_eq!(first.0, second.0);
    assert_eq!(first.1, second.1);
    assert_eq!(first.2, second.2);
    assert_eq!(first.2, 3, "two fills plus one aggregate over 41 formulas");
}

/// TD-66: a group's read rectangles collapse in O(n log n), and collapse to the
/// *same* cover the old quadratic scan produced.
///
/// # Why this test exists at all
/// `extent_of` used to accumulate read rectangles one at a time and linearly
/// scan everything accumulated so far looking for a merge. A column of formulas
/// in **every** row merged on the first comparison and stayed fast; a column
/// with **gaps** merged with nothing, grew one entry per formula, and became
/// O(n²) — measured at 2.5 s for 68,469 formulas against 395 ms for 102,703
/// dense ones, which is 50% more formulas and six times faster.
///
/// Gaps are what real spreadsheets are made of: blank rows, section breaks,
/// subtotal bands.
///
/// **What this test is and is not.** It pins the *shape of the cover*, which is
/// the part a rewrite could quietly get wrong — and it passes against both the
/// old algorithm and the new one, which is the point: the fix was meant to
/// change the cost and nothing else, and this is the evidence that it did.
/// There is no cheap non-flaky way to assert an asymptotic in a unit test, so
/// the complexity itself is guarded by a **measurement** rather than a test:
/// `ehkatra-shell --open <rows>` reports the graph build by phase, and
/// MEASUREMENTS.md records 2,509 ms -> 347 ms at 100,000 rows.
#[test]
fn read_rectangles_collapse_across_touching_rows_and_not_across_gaps() {
    // Formulas in rows 0, 1, 3, 4, 6, 7 — the two-in-three shape — each summing
    // columns A..C of its own row.
    let mut b = Book::new(9, 4);
    for row in [0usize, 1, 3, 4, 6, 7] {
        b.formula_ranges(row, 3, "=SUM(A1:C1)", &[(row, row, 0, 2)]);
    }
    let state = b.state();
    let engine = Engine::build(&state, Profile::Compat);

    let group = engine
        .groups()
        .iter()
        .find(|g| g.cells.len() == 6)
        .expect("all six share one R1C1 pattern");

    let mut rows: Vec<(u32, u32)> = group.reads.iter().map(|r| (r.r0, r.r1)).collect();
    rows.sort_unstable();
    assert_eq!(
        rows,
        [(0, 1), (3, 4), (6, 7)],
        "touching rows merge into one rectangle; a gap starts a new one"
    );
    for r in &group.reads {
        assert_eq!((r.c0, r.c1), (0, 2), "the column span is untouched");
    }
}

/// The dense case, which is the one that was always fast and must stay exact:
/// an unbroken run collapses to a single rectangle.
#[test]
fn an_unbroken_run_of_reads_collapses_to_one_rectangle() {
    let mut b = Book::new(8, 4);
    for row in 0..8usize {
        b.formula_ranges(row, 3, "=SUM(A1:C1)", &[(row, row, 0, 2)]);
    }
    let state = b.state();
    let engine = Engine::build(&state, Profile::Compat);
    let group = engine
        .groups()
        .iter()
        .find(|g| g.cells.len() == 8)
        .expect("one pattern");
    assert_eq!(group.reads.len(), 1);
    assert_eq!(
        (group.reads[0].r0, group.reads[0].r1),
        (0, 7),
        "eight touching rows are one rectangle"
    );
}

/// The cover is an optimisation; the *answers* are the contract. A gapped sheet
/// must still recalculate correctly and still propagate an edit through the
/// merged rectangles — a cover that collapsed too far would silently stop
/// dirtying something.
#[test]
fn a_gapped_sheet_still_recalculates_and_still_propagates_an_edit() {
    let mut b = Book::new(9, 4);
    for row in [0usize, 1, 3, 4, 6, 7] {
        for col in 0..3 {
            b.set(row, col, (row * 10 + col) as f64);
        }
        b.formula_ranges(row, 3, "=SUM(A1:C1)", &[(row, row, 0, 2)]);
    }
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);

    // `approx` because `SUM` answers in `Decimal` — exact currency math is the
    // point of that (ADR-035), and the test should not care which numeric type
    // carries the answer.
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[3]), 3.0);
    assert_eq!(approx(&engine, &state, b.rows[7], b.cols[3]), 213.0);

    // Edit a precedent in the *last* gap-separated band and check the dependent
    // follows: if the cover had merged too far or too little, this is where it
    // shows.
    let mut b2 = b;
    let op = b2.set(7, 0, 1000.0);
    let state = b2.state();
    engine.observe(&state, core::slice::from_ref(&op));
    assert_eq!(
        approx(&engine, &state, b2.rows[7], b2.cols[3]),
        1000.0 + 71.0 + 72.0,
        "the edit must reach the formula through the merged rectangle"
    );
}

/// TD-20: the band index across **more than one band**.
///
/// # A gap this fix exposed
/// `BAND` is 256 rows, and every other test in this file uses a sheet small
/// enough to fit in band 0 — so the multi-band path had no coverage at all.
/// These sheets are 600 rows, which is three bands.
///
/// Stated plainly: **these pass against the old index too.** Moving a band from
/// holding group ids to holding rectangles changes what the index *costs*, not
/// what it answers, and no behavioural test can discriminate a pure performance
/// change. What they are for is the coverage gap — the band arithmetic itself
/// was untested, and a rewrite of the structure that carries it should not have
/// been made against tests that never left band 0.
#[test]
fn an_edit_reaches_a_formula_three_bands_away() {
    let mut b = Book::new(600, 2);
    // One formula per band, all sharing an R1C1 pattern (each sums the ten
    // rows above it), so they are one group with three read rectangles in
    // three different bands.
    for row in [20usize, 300, 590] {
        b.formula_ranges(row, 1, "=SUM(A11:A20)", &[(row - 10, row, 0, 0)]);
    }
    for row in 0..600usize {
        b.set(row, 0, 1.0);
    }
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[590], b.cols[1]), 11.0);

    // An edit in the *last* band must reach the formula there and no other.
    let op = b.set(585, 0, 100.0);
    let state = b.state();
    let stats = engine.observe(&state, core::slice::from_ref(&op));
    assert_eq!(
        stats.evaluated_cells, 1,
        "only the formula whose rectangle covers row 585 should re-evaluate"
    );
    assert_eq!(approx(&engine, &state, b.rows[590], b.cols[1]), 110.0);
    assert_eq!(
        approx(&engine, &state, b.rows[20], b.cols[1]),
        11.0,
        "the formula in band 0 must not have moved"
    );
}

/// A single read rectangle that spans several bands must be found from any of
/// them — the case the band arithmetic gets wrong if the range is registered
/// only against its first band.
#[test]
fn a_read_rectangle_spanning_bands_is_found_from_every_band_it_crosses() {
    let mut b = Book::new(600, 2);
    // One formula reading rows 0..=599 — four bands' worth in one rectangle.
    b.formula_ranges(0, 1, "=SUM(A1:A600)", &[(0, 599, 0, 0)]);
    for row in 0..600usize {
        b.set(row, 0, 1.0);
    }
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 600.0);

    // Edit one row in each band; every one of them must reach the formula.
    for row in [5usize, 260, 520, 599] {
        let op = b.set(row, 0, 2.0);
        let state = b.state();
        let stats = engine.observe(&state, core::slice::from_ref(&op));
        assert_eq!(
            stats.evaluated_cells, 1,
            "an edit at row {row} did not reach the formula"
        );
        let _ = state;
    }
    let state = b.state();
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 604.0);
}

/// An edit far from every formula must reach none of them. The negative case,
/// which is what an index is *for* — a stab that matched everything would still
/// give right answers and would make the index pointless.
#[test]
fn an_edit_in_an_empty_band_dirties_nothing() {
    let mut b = Book::new(600, 3);
    b.formula_ranges(20, 1, "=SUM(A11:A20)", &[(10, 20, 0, 0)]);
    for row in 0..30usize {
        b.set(row, 0, 1.0);
    }
    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);

    // Row 500, column C: two bands away and a column nothing reads.
    let op = b.set(500, 2, 42.0);
    let state = b.state();
    let stats = engine.observe(&state, core::slice::from_ref(&op));
    assert_eq!(
        stats.evaluated_cells, 0,
        "an edit nothing reads must evaluate nothing"
    );
    assert_eq!(stats.dirty_groups, 0);
}

/// TD-71: the amortised rect read a range evaluation performs must be
/// indistinguishable from per-cell reads. The range here spans every kind of
/// cell a read can meet — another formula's computed result, stored values,
/// and a blank — and the overlay rule (results win over stored values) is
/// exercised by a formula cell that also carries an older stored value.
#[test]
fn a_range_read_sees_results_values_and_blanks_alike() {
    let mut b = Book::new(4, 2);
    b.set(0, 0, 5.0); // A1 stored
                      // A2 left blank
    b.set(2, 0, 90.0); // A3 stored, then overwritten by a formula:
    b.formula(2, 0, "=A1+1", &[(0, 0)]); // A3 computes 6, shadowing the 90
    b.set(3, 0, 2.0); // A4 stored
    b.formula_ranges(0, 1, "=SUM(A1:A4)", &[(0, 3, 0, 0)]); // B1

    let state = b.state();
    let mut engine = Engine::build(&state, Profile::Compat);
    engine.recalc_all(&state);

    // 5 (stored) + 0 (blank) + 6 (computed, not the stored 90) + 2 (stored).
    assert_eq!(approx(&engine, &state, b.rows[0], b.cols[1]), 13.0);
    assert_eq!(approx(&engine, &state, b.rows[2], b.cols[0]), 6.0);
}
