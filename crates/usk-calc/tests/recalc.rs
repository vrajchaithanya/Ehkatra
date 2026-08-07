//! Row 7 proofs: formula grouping, range-granular edges, incremental recalc
//! with early cutoff, level ordering and cycle detection (docs/13).

use usk_calc::graph::r1c1;
use usk_calc::{CellRef, Engine, Sheet};
use usk_formula::parse::parse;
use usk_types::coerce::Profile;
use usk_types::{ErrorKind, Value};

fn cell(row: u32, col: u32) -> CellRef {
    CellRef { row, col }
}

fn num(v: f64) -> Value {
    Value::Number(v)
}

/// Reads a cell's current value as `f64`, whatever numeric type it holds.
fn approx(engine: &Engine, c: CellRef) -> f64 {
    match engine.sheet.value(c) {
        Some(Value::Number(n)) => n,
        Some(Value::Decimal(d)) => d.to_f64(),
        other => panic!("expected a number at {c:?}, got {other:?}"),
    }
}

fn kind_at(engine: &Engine, c: CellRef) -> Option<ErrorKind> {
    engine
        .sheet
        .value(c)
        .and_then(|v| v.as_error())
        .map(|e| e.kind)
}

/// A column of literals with a formula column beside it — the canonical fill.
fn filled_sheet(rows: u32) -> Sheet {
    let mut sheet = Sheet::new(rows, 3);
    for r in 0..rows {
        sheet.set_literal(cell(r, 0), num(r as f64));
        sheet.set_formula(cell(r, 1), "=A1*2");
    }
    // Fix each formula so it points at its own row.
    for r in 0..rows {
        let src = alloc_src(r);
        sheet.set_formula(cell(r, 1), &src);
    }
    sheet
}

fn alloc_src(row: u32) -> String {
    format!("=A{}*2", row + 1)
}

// ------------------------------------------------------- formula groups

/// **The claim that makes the graph scale** (docs/13): a filled column is *one*
/// node, not one per cell, because every member shares an R1C1 pattern.
#[test]
fn a_filled_column_collapses_to_one_group() {
    let engine = Engine::build(filled_sheet(1_000), Profile::Compat);
    assert_eq!(
        engine.group_count(),
        1,
        "1,000 filled cells should be a single node, got {} groups",
        engine.group_count()
    );
    let group = &engine.groups()[0];
    assert_eq!(group.cells.len(), 1_000);
    // Its read set is one rectangle covering the whole source column, not
    // 1,000 single-cell rectangles.
    assert_eq!(group.reads.len(), 1, "reads: {:?}", group.reads);
    assert_eq!(group.reads[0].cell_count(), 1_000);
}

/// R1C1 is the grouping key: relative references normalise, absolute ones do
/// not, so `$A$1` never merges with a relative reference to the same cell.
#[test]
fn r1c1_normalises_relative_and_pins_absolute() {
    let a = r1c1(&parse("=A1*2").ast, cell(0, 1));
    let b = r1c1(&parse("=A2*2").ast, cell(1, 1));
    assert_eq!(a, b, "a fill-down must produce one pattern");

    let absolute = r1c1(&parse("=$A$1*2").ast, cell(0, 1));
    assert_ne!(
        a, absolute,
        "an absolute reference is a different pattern from a relative one"
    );

    // Different shapes stay different.
    assert_ne!(a, r1c1(&parse("=A1*3").ast, cell(0, 1)));
    assert_ne!(a, r1c1(&parse("=B1*2").ast, cell(0, 1)));
}

/// Distinct patterns get distinct nodes, so grouping does not over-merge.
#[test]
fn different_patterns_form_different_groups() {
    let mut sheet = Sheet::new(4, 3);
    for r in 0..4 {
        sheet.set_literal(cell(r, 0), num(r as f64 + 1.0));
    }
    sheet.set_formula(cell(0, 1), "=A1*2");
    sheet.set_formula(cell(1, 1), "=A2*2");
    sheet.set_formula(cell(2, 1), "=A3+100");
    sheet.set_formula(cell(3, 1), "=A4+100");
    let engine = Engine::build(sheet, Profile::Compat);
    assert_eq!(engine.group_count(), 2);
}

// ----------------------------------------------------------- evaluation

#[test]
fn recalc_all_computes_every_formula() {
    let mut engine = Engine::build(filled_sheet(10), Profile::Compat);
    let stats = engine.recalc_all();
    assert_eq!(stats.evaluated_cells, 10);
    for r in 0..10 {
        assert_eq!(approx(&engine, cell(r, 1)), r as f64 * 2.0);
    }
}

/// Evaluation follows dependency order across chained groups, so a value never
/// reads a stale upstream within one pass.
#[test]
fn chained_groups_evaluate_in_dependency_order() {
    let mut sheet = Sheet::new(1, 4);
    sheet.set_literal(cell(0, 0), num(5.0));
    sheet.set_formula(cell(0, 1), "=A1*2"); // 10
    sheet.set_formula(cell(0, 2), "=B1+1"); // 11
    sheet.set_formula(cell(0, 3), "=C1*10"); // 110
    let mut engine = Engine::build(sheet, Profile::Compat);
    let stats = engine.recalc_all();

    assert_eq!(approx(&engine, cell(0, 3)), 110.0);
    assert_eq!(
        stats.levels, 3,
        "three chained groups must occupy three levels"
    );
}

/// Independent groups land in the *same* level. That width is precisely what a
/// parallel evaluator would exploit, which is why the level count is recorded.
#[test]
fn independent_groups_share_a_level() {
    let mut sheet = Sheet::new(1, 5);
    sheet.set_literal(cell(0, 0), num(2.0));
    sheet.set_formula(cell(0, 1), "=A1*2");
    sheet.set_formula(cell(0, 2), "=A1*3");
    sheet.set_formula(cell(0, 3), "=A1*4");
    let mut engine = Engine::build(sheet, Profile::Compat);
    let stats = engine.recalc_all();
    assert_eq!(stats.evaluated_groups, 3);
    assert_eq!(
        stats.levels, 1,
        "three mutually independent groups are one parallel level"
    );
}

// --------------------------------------------------------- incremental

/// The incremental path touches only what the edit can reach — the whole point
/// of a range-granular graph.
#[test]
fn an_edit_recalculates_only_its_dependents() {
    let mut sheet = Sheet::new(1, 4);
    sheet.set_literal(cell(0, 0), num(1.0));
    sheet.set_formula(cell(0, 1), "=A1*2");
    // An independent group that must NOT be touched by an edit to A1.
    sheet.set_literal(cell(0, 2), num(7.0));
    sheet.set_formula(cell(0, 3), "=C1*2");

    let mut engine = Engine::build(sheet, Profile::Compat);
    engine.recalc_all();
    assert_eq!(approx(&engine, cell(0, 1)), 2.0);
    assert_eq!(approx(&engine, cell(0, 3)), 14.0);

    engine.sheet.set_literal(cell(0, 0), num(10.0));
    let stats = engine.recalc_after(&[cell(0, 0)]);

    assert_eq!(approx(&engine, cell(0, 1)), 20.0);
    assert_eq!(
        approx(&engine, cell(0, 3)),
        14.0,
        "untouched value survives"
    );
    assert_eq!(
        stats.dirty_groups, 1,
        "only the group reading A1 should be dirty"
    );
    assert_eq!(stats.evaluated_groups, 1);
}

/// Early cutoff (docs/13): a group whose recomputed values are unchanged does
/// not force its downstream to be re-evaluated.
#[test]
fn unchanged_results_cut_off_propagation() {
    let mut sheet = Sheet::new(1, 4);
    sheet.set_literal(cell(0, 0), num(3.0));
    // SIGN flattens a range of inputs to the same output, so an edit that
    // changes A1 need not change B1.
    sheet.set_formula(cell(0, 1), "=SIGN(A1)");
    sheet.set_formula(cell(0, 2), "=B1*100");

    let mut engine = Engine::build(sheet, Profile::Compat);
    engine.recalc_all();
    assert_eq!(approx(&engine, cell(0, 2)), 100.0);

    // 3 → 9 leaves SIGN(A1) at 1, so C1 must be cut off.
    engine.sheet.set_literal(cell(0, 0), num(9.0));
    let stats = engine.recalc_after(&[cell(0, 0)]);
    assert_eq!(approx(&engine, cell(0, 2)), 100.0);
    assert_eq!(
        stats.dirty_groups, 2,
        "both groups are reachable and so both are marked"
    );
    assert_eq!(
        stats.cut_off_groups, 1,
        "the downstream group should be cut off, not evaluated"
    );

    // A change that *does* propagate still gets through.
    engine.sheet.set_literal(cell(0, 0), num(-2.0));
    engine.recalc_after(&[cell(0, 0)]);
    assert_eq!(approx(&engine, cell(0, 2)), -100.0);
}

/// Editing a cell inside a range that a group reads dirties that group, even
/// though no cell-to-cell edge was ever materialised.
#[test]
fn range_reads_are_stabbed_not_enumerated() {
    let mut sheet = Sheet::new(100, 2);
    for r in 0..100 {
        sheet.set_literal(cell(r, 0), num(1.0));
    }
    sheet.set_formula(cell(0, 1), "=SUM(A1:A100)");
    let mut engine = Engine::build(sheet, Profile::Compat);
    engine.recalc_all();
    assert_eq!(approx(&engine, cell(0, 1)), 100.0);

    // Touch a cell in the middle of the range.
    engine.sheet.set_literal(cell(50, 0), num(11.0));
    let stats = engine.recalc_after(&[cell(50, 0)]);
    assert_eq!(approx(&engine, cell(0, 1)), 110.0);
    assert_eq!(stats.evaluated_groups, 1);

    // A cell outside the range must not dirty anything.
    engine.sheet.set_literal(cell(0, 1), num(0.0));
    let untouched = engine.recalc_after(&[cell(99, 1)]);
    assert_eq!(untouched.dirty_groups, 0);
}

// --------------------------------------------------------------- cycles

/// A cycle becomes `#CIRC!` rather than a hang or a stack overflow (docs/13).
#[test]
fn cycles_are_detected_and_reported_as_circ() {
    let mut sheet = Sheet::new(1, 3);
    sheet.set_formula(cell(0, 0), "=B1+1");
    sheet.set_formula(cell(0, 1), "=A1+1");
    let mut engine = Engine::build(sheet, Profile::Compat);
    let stats = engine.recalc_all();

    assert_eq!(stats.circular_groups, 2);
    assert_eq!(kind_at(&engine, cell(0, 0)), Some(ErrorKind::Circ));
    assert_eq!(kind_at(&engine, cell(0, 1)), Some(ErrorKind::Circ));
}

/// A self-reference is a cycle of one and must not be mistaken for an
/// acyclic node.
#[test]
fn self_reference_is_circular() {
    let mut sheet = Sheet::new(1, 2);
    sheet.set_formula(cell(0, 0), "=A1+1");
    let mut engine = Engine::build(sheet, Profile::Compat);
    let stats = engine.recalc_all();
    assert_eq!(stats.circular_groups, 1);
    assert_eq!(kind_at(&engine, cell(0, 0)), Some(ErrorKind::Circ));
}

/// A cycle must not poison the rest of the sheet: acyclic groups still compute.
#[test]
fn a_cycle_does_not_stop_unrelated_groups() {
    let mut sheet = Sheet::new(1, 5);
    sheet.set_formula(cell(0, 0), "=B1+1");
    sheet.set_formula(cell(0, 1), "=A1+1");
    sheet.set_literal(cell(0, 2), num(4.0));
    sheet.set_formula(cell(0, 3), "=C1*5");
    let mut engine = Engine::build(sheet, Profile::Compat);
    let stats = engine.recalc_all();

    assert_eq!(stats.circular_groups, 2);
    assert_eq!(approx(&engine, cell(0, 3)), 20.0);
}

// ------------------------------------------------------------ volatiles

/// Volatiles are read from the engine's materialised bindings, never from a
/// clock (docs/13 T2, DP-A2). Two recalculations with the same binding produce
/// the same value, which is what lets replicas converge.
#[test]
fn volatiles_are_materialised_not_computed() {
    let mut sheet = Sheet::new(1, 2);
    sheet.set_formula(cell(0, 0), "=TODAY()+1");
    let mut engine = Engine::build(sheet, Profile::Compat);
    engine.today = 45_000;
    engine.recalc_all();
    assert_eq!(approx(&engine, cell(0, 0)), 45_001.0);

    engine.recalc_all();
    assert_eq!(approx(&engine, cell(0, 0)), 45_001.0);

    // Re-materialising the binding is an explicit, attributed event.
    engine.today = 45_100;
    engine.recalc_all();
    assert_eq!(approx(&engine, cell(0, 0)), 45_101.0);
}

// ------------------------------------------------------------ ordering

/// Recalculation is deterministic: the same sheet always produces the same
/// values and the same level structure (DP-A2).
#[test]
fn recalculation_is_deterministic() {
    let build = || {
        let mut sheet = Sheet::new(20, 4);
        for r in 0..20 {
            sheet.set_literal(cell(r, 0), num(r as f64));
            sheet.set_formula(cell(r, 1), &format!("=A{}*2", r + 1));
            sheet.set_formula(cell(r, 2), &format!("=B{}+1", r + 1));
        }
        sheet.set_formula(cell(0, 3), "=SUM(C1:C20)");
        let mut e = Engine::build(sheet, Profile::Compat);
        let stats = e.recalc_all();
        (stats, approx(&e, cell(0, 3)), e.group_count())
    };
    let first = build();
    let second = build();
    assert_eq!(first.0, second.0);
    assert_eq!(first.1, second.1);
    assert_eq!(first.2, second.2);
    // Two fill columns plus one aggregate = three nodes over 41 formulas.
    assert_eq!(first.2, 3);
}
