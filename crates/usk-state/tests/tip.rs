//! `State::apply_tip` — the incremental fold, and the guards that make it safe
//! (TD-24's residual, docs/35 layer 2).
//!
//! # The one property that matters
//! A state grown incrementally must be **bit-identical** to one folded from the
//! whole log. Everything else here exists to make that true in cases where it
//! would not otherwise be, and every guard below is a case where the incremental
//! path must *refuse* rather than guess.
//!
//! The interesting failure is already recorded in a test: `plan_promotions`
//! interns a tile slot when a row is **inserted**, not when it is first written
//! to, so an incremental `InsertRow` that skipped the intern shifted every later
//! slot by one — and slot order is the order the state hash folds in. The relay
//! convergence suite is what caught it.

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_state::{State, TipError};
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn opid(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

fn insert_row(actor: u128, counter: u64, lamport: u64, anchor: Anchor) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::InsertRow { anchor },
    }
}

fn insert_col(actor: u128, counter: u64, lamport: u64, anchor: Anchor) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::InsertCol { anchor },
    }
}

fn set_cell(actor: u128, counter: u64, lamport: u64, row: OpId, col: OpId, n: f64) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::SetCell {
            row: RowId(row),
            col: ColId(col),
            value: Value::Number(n),
        },
    }
}

fn set_formula(actor: u128, counter: u64, lamport: u64, row: OpId, col: OpId, src: &str) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::SetFormula {
            row: RowId(row),
            col: ColId(col),
            source: String::from(src),
            bindings: Vec::new(),
        },
    }
}

fn log_of(ops: &[Op]) -> OpLog {
    let mut log = OpLog::new();
    for op in ops {
        log.append(op.clone());
    }
    log
}

/// Folds `base`, then applies `tail` incrementally, and asserts the result is
/// what a full replay of `base ++ tail` produces.
fn assert_tip_matches_replay(base: &[Op], tail: &[Op]) {
    let mut incremental = State::replay(&log_of(base));
    incremental
        .apply_tip(tail)
        .expect("this tail is meant to be tip-eligible");

    let mut whole: Vec<Op> = base.to_vec();
    whole.extend_from_slice(tail);
    let folded = State::replay(&log_of(&whole));

    assert_eq!(
        incremental.state_hash(),
        folded.state_hash(),
        "an incrementally grown state must hash exactly as a folded one"
    );
    assert_eq!(incremental.row_order(), folded.row_order());
    assert_eq!(incremental.col_order(), folded.col_order());
}

/// A sheet skeleton: `cols` columns then `rows` rows, all from actor 1.
fn skeleton(rows: usize, cols: usize) -> (Vec<Op>, Vec<OpId>, Vec<OpId>, u64) {
    let mut ops = Vec::new();
    let mut n = 0u64;
    let mut col_ids = Vec::new();
    let mut anchor = Anchor::Start;
    for _ in 0..cols {
        n += 1;
        let op = insert_col(1, n, n, anchor);
        anchor = Anchor::After(op.id);
        col_ids.push(op.id);
        ops.push(op);
    }
    let mut row_ids = Vec::new();
    let mut anchor = Anchor::Start;
    for _ in 0..rows {
        n += 1;
        let op = insert_row(1, n, n, anchor);
        anchor = Anchor::After(op.id);
        row_ids.push(op.id);
        ops.push(op);
    }
    (ops, row_ids, col_ids, n)
}

#[test]
fn a_value_written_at_the_tip_matches_a_full_replay() {
    let (base, rows, cols, n) = skeleton(8, 4);
    let tail = vec![
        set_cell(1, n + 1, n + 1, rows[2], cols[1], 42.0),
        set_cell(1, n + 2, n + 2, rows[3], cols[0], 7.0),
    ];
    assert_tip_matches_replay(&base, &tail);
}

#[test]
fn rows_inserted_at_the_tip_claim_their_slots_in_insertion_order() {
    // The defect the relay's convergence suite found, reduced.
    //
    // Two rows are inserted and then written to in the **opposite** order. A
    // tile slot is claimed by `plan_promotions` when a row is *inserted*
    // (ADR-034: "slot order follows creation order"), so a replay gives the
    // first-inserted row the lower slot. An incremental apply that interned
    // only on the first *write* gives it to the first-written row instead —
    // the two rows swap slots, `TileStore::for_each` yields their cells in the
    // other order, and the state hash, which folds in that order, diverges.
    //
    // Note what does *not* reproduce it: an unwritten row on its own only
    // shifts every later slot uniformly, which leaves the fold order intact.
    // It takes a swap.
    let (base, rows, cols, n) = skeleton(4, 3);
    let first = insert_row(1, n + 1, n + 1, Anchor::After(rows[3]));
    let second = insert_row(1, n + 2, n + 2, Anchor::After(first.id));
    let tail = vec![
        first.clone(),
        second.clone(),
        set_cell(1, n + 3, n + 3, second.id, cols[0], 5.0),
        set_cell(1, n + 4, n + 4, first.id, cols[0], 6.0),
    ];
    assert_tip_matches_replay(&base, &tail);
}

#[test]
fn columns_inserted_at_the_tip_claim_their_slots_in_insertion_order() {
    // The same defect on the other axis, which has its own slot map.
    let (base, rows, cols, n) = skeleton(3, 3);
    let first = insert_col(1, n + 1, n + 1, Anchor::After(cols[2]));
    let second = insert_col(1, n + 2, n + 2, Anchor::After(first.id));
    let tail = vec![
        first.clone(),
        second.clone(),
        set_cell(1, n + 3, n + 3, rows[0], second.id, 5.0),
        set_cell(1, n + 4, n + 4, rows[0], first.id, 6.0),
    ];
    assert_tip_matches_replay(&base, &tail);
}

#[test]
fn a_formula_written_at_the_tip_matches_a_full_replay() {
    // The formula registry is seeded by the replay pre-pass, which by
    // definition never saw a tip op. `apply_tip` seeds as it goes.
    let (base, rows, cols, n) = skeleton(6, 3);
    let tail = vec![
        set_formula(1, n + 1, n + 1, rows[0], cols[2], "=A1+B1"),
        set_cell(1, n + 2, n + 2, rows[0], cols[0], 3.0),
    ];
    assert_tip_matches_replay(&base, &tail);
    let state = {
        let mut s = State::replay(&log_of(&base));
        s.apply_tip(&tail).unwrap();
        s
    };
    assert!(state.formula(RowId(rows[0]), ColId(cols[2])).is_some());
}

#[test]
fn a_value_write_after_a_formula_at_the_tip_shadows_it_as_a_replay_would() {
    let (base, rows, cols, n) = skeleton(6, 3);
    let tail = vec![
        set_formula(1, n + 1, n + 1, rows[0], cols[2], "=A1+B1"),
        set_cell(1, n + 2, n + 2, rows[0], cols[2], 99.0),
    ];
    assert_tip_matches_replay(&base, &tail);
    let mut state = State::replay(&log_of(&base));
    state.apply_tip(&tail).unwrap();
    assert!(
        state.formula(RowId(rows[0]), ColId(cols[2])).is_none(),
        "the later value write must shadow the formula"
    );
    assert_eq!(
        state.cell(RowId(rows[0]), ColId(cols[2])),
        Some(Value::Number(99.0))
    );
}

#[test]
fn a_long_run_of_edits_never_drifts_from_a_full_replay() {
    // Applied one batch at a time, checking after every one — so a drift that
    // only appears on the fortieth edit cannot hide behind the first.
    let (base, rows, cols, mut n) = skeleton(40, 6);
    let mut incremental = State::replay(&log_of(&base));
    let mut whole = base.clone();
    // A seeded LCG, as D-052 uses elsewhere: reproducible, and not a pattern a
    // fix could accidentally be shaped around.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for i in 0..60 {
        n += 1;
        let r = rows[next() % rows.len()];
        let c = cols[next() % cols.len()];
        let op = if i % 7 == 3 {
            set_formula(1, n, n, r, c, "=1+1")
        } else if i % 11 == 5 {
            Op {
                id: opid(1, n),
                lamport: n,
                payload: Payload::ClearCell {
                    row: RowId(r),
                    col: ColId(c),
                },
            }
        } else {
            set_cell(1, n, n, r, c, i as f64)
        };
        incremental.apply_tip(core::slice::from_ref(&op)).unwrap();
        whole.push(op);
        assert_eq!(
            incremental.state_hash(),
            State::replay(&log_of(&whole)).state_hash(),
            "drifted at edit {i}"
        );
    }
}

// ------------------------------------------------------------------- refusals

#[test]
fn an_op_that_is_not_at_the_tip_is_refused_and_nothing_is_applied() {
    let (base, rows, cols, n) = skeleton(4, 2);
    let mut state = State::replay(&log_of(&base));
    let before = state.state_hash();
    // Lamport 2 is far below the skeleton's tip.
    let stale = set_cell(2, 1, 2, rows[0], cols[0], 1.0);
    assert!(matches!(
        state.apply_tip(&[stale]),
        Err(TipError::NotAtTip { .. })
    ));
    assert_eq!(state.state_hash(), before, "a refusal must change nothing");
    let _ = n;
}

#[test]
fn a_batch_is_refused_whole_when_any_op_in_it_is_out_of_order() {
    // The half-applied batch is the dangerous outcome: it would leave the state
    // in a shape no replay produces and no error names.
    let (base, rows, cols, n) = skeleton(4, 2);
    let mut state = State::replay(&log_of(&base));
    let before = state.state_hash();
    let batch = vec![
        set_cell(1, n + 1, n + 1, rows[0], cols[0], 1.0),
        set_cell(1, n + 2, 2, rows[1], cols[0], 2.0), // lamport 2: out of order
    ];
    assert!(state.apply_tip(&batch).is_err());
    assert_eq!(
        state.state_hash(),
        before,
        "the first op of a refused batch must not have landed"
    );
}

#[test]
fn a_second_actors_write_to_an_occupied_uncontested_cell_is_refused() {
    // The cell holds actor 1's value in a summary tile, which keeps no per-cell
    // stamp. Applying actor 2's write there would displace a value nothing can
    // identify, so the loser ADR-006 promises could not be retained.
    let (mut base, rows, cols, mut n) = skeleton(4, 2);
    n += 1;
    base.push(set_cell(1, n, n, rows[0], cols[0], 1.0));
    let mut state = State::replay(&log_of(&base));
    let intruder = set_cell(2, 1, n + 1, rows[0], cols[0], 2.0);
    assert!(matches!(
        state.apply_tip(&[intruder]),
        Err(TipError::MayContend { .. })
    ));
}

#[test]
fn a_second_actors_write_to_an_empty_cell_is_taken() {
    // Nothing to displace, so nothing to lose — and refusing here would make
    // every collaborative keystroke re-fold the log for no reason.
    let (mut base, rows, cols, mut n) = skeleton(4, 2);
    n += 1;
    base.push(set_cell(1, n, n, rows[0], cols[0], 1.0));
    let tail = vec![set_cell(2, 1, n + 1, rows[1], cols[1], 2.0)];
    assert_tip_matches_replay(&base, &tail);
}

#[test]
fn a_write_to_an_already_promoted_cell_is_taken_from_any_actor() {
    // Two actors have written this cell, so the pre-pass promoted it and the
    // stamped path compares `(lamport, id)` — order-independent, so a third
    // write from either actor is safe.
    let (mut base, rows, cols, mut n) = skeleton(4, 2);
    n += 1;
    base.push(set_cell(1, n, n, rows[0], cols[0], 1.0));
    n += 1;
    base.push(set_cell(2, 1, n, rows[0], cols[0], 2.0));
    let state = State::replay(&log_of(&base));
    assert!(
        state.is_cell_promoted(RowId(rows[0]), ColId(cols[0])),
        "two writers must have promoted this cell"
    );
    let tail = vec![set_cell(2, 2, n + 1, rows[0], cols[0], 3.0)];
    assert_tip_matches_replay(&base, &tail);
}

#[test]
fn an_image_adopted_state_refuses_the_tip_path() {
    // An image is not a fold over a whole log, so no promotion plan covers it.
    // What it has instead is `apply_tail`, which carries the winner stamps.
    let (mut base, rows, cols, mut n) = skeleton(4, 2);
    n += 1;
    base.push(set_cell(1, n, n, rows[0], cols[0], 1.0));
    let folded = State::replay(&log_of(&base));
    let image = folded.write_image();
    let (mut adopted, _stamps) = State::from_image_with_stamps(&image).expect("a valid image");
    assert!(matches!(
        adopted.apply_tip(&[set_cell(1, n + 1, n + 1, rows[1], cols[0], 5.0)]),
        Err(TipError::NotFolded)
    ));
}
