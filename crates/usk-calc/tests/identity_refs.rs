//! Row 8 proofs: identity references (docs/11, docs/04 invariant 3, DP-A6).
//!
//! The headline is `concurrent_row_insert_against_sum_converges` — BOOTSTRAP
//! names it "the canonical test". Everything else pins one shift rule that is
//! supposed to fall out *structurally* rather than being implemented, and the
//! way to prove that is to show the reference is never rewritten and still
//! behaves the way a spreadsheet user expects.

use usk_calc::refs::{AnchorMode, Binder, StateGrid};
use usk_formula::eval::Grid;
use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, ErrorKind, OpId, RowId, Value};

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

fn delete_row(actor: u128, counter: u64, lamport: u64, row: OpId) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::DeleteRow { row: RowId(row) },
    }
}

fn set_cell(actor: u128, counter: u64, lamport: u64, row: OpId, col: OpId, v: f64) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::SetCell {
            row: RowId(row),
            col: ColId(col),
            value: Value::Number(v),
        },
    }
}

/// A sheet of `rows` rows and one column, with 1..=rows written down it.
struct Fixture {
    log: OpLog,
    rows: Vec<OpId>,
    col: OpId,
    lamport: u64,
    counter: u64,
}

impl Fixture {
    fn new(rows: usize) -> Fixture {
        let mut log = OpLog::new();
        let mut lamport = 1u64;
        let mut counter = 1u64;

        let col_op = insert_col(1, counter, lamport, Anchor::Start);
        let col = col_op.id;
        log.append(col_op);
        lamport += 1;
        counter += 1;

        let mut row_ids = Vec::new();
        for _ in 0..rows {
            let anchor = row_ids
                .last()
                .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
            let op = insert_row(1, counter, lamport, anchor);
            row_ids.push(op.id);
            log.append(op);
            lamport += 1;
            counter += 1;
        }
        for (i, r) in row_ids.iter().enumerate() {
            log.append(set_cell(1, counter, lamport, *r, col, (i + 1) as f64));
            lamport += 1;
            counter += 1;
        }
        Fixture {
            log,
            rows: row_ids,
            col,
            lamport,
            counter,
        }
    }

    fn next(&mut self) -> (u64, u64) {
        self.lamport += 1;
        self.counter += 1;
        (self.counter, self.lamport)
    }

    fn state(&self) -> State {
        State::replay(&self.log)
    }
}

/// Sums the values a reference currently covers.
fn sum(values: &[Value]) -> f64 {
    values
        .iter()
        .map(|v| match v {
            Value::Number(n) => *n,
            Value::Decimal(d) => d.to_f64(),
            _ => 0.0,
        })
        .sum()
}

// ------------------------------------------------------- binding

/// A1 is a view: the same identity renders at a different ordinal after an
/// insert above it, while the binding is untouched.
#[test]
fn a1_is_a_view_and_the_binding_is_not() {
    let mut f = Fixture::new(5);
    let before = f.state();
    let binder = Binder::from_state(&before);

    // Bind what the user typed as "A3".
    let reference = binder
        .bind_cell(2, 0)
        .expect("A3 exists in a five-row sheet");
    assert_eq!(binder.rows.position_of(&reference.row_start), Some(2));

    // Insert a row at the very top.
    let (c, l) = f.next();
    f.log.append(insert_row(2, c, l, Anchor::Start));
    let after = f.state();
    let binder2 = Binder::from_state(&after);

    // The identity now renders one row lower — and the reference still names it.
    assert_eq!(binder2.rows.position_of(&reference.row_start), Some(3));
    assert_eq!(
        reference.read(&after, &binder2).expect("still resolvable"),
        alloc::vec![Value::Number(3.0)],
        "the value travelled with its identity, not its address"
    );
}

// --------------------------------------------------- insert semantics

/// Inserting **above** a range leaves its span alone: the endpoints did not
/// move relative to each other.
#[test]
fn inserting_above_a_range_leaves_its_span() {
    let mut f = Fixture::new(10);
    let state = f.state();
    let binder = Binder::from_state(&state);
    // "A1:A10"
    let range = binder
        .bind(0, 9, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");
    assert_eq!(sum(&range.read(&state, &binder).expect("resolves")), 55.0);

    let (c, l) = f.next();
    f.log.append(insert_row(2, c, l, Anchor::Start));
    let after = f.state();
    let binder2 = Binder::from_state(&after);

    let resolved = range.resolve(&binder2);
    assert_eq!(resolved.rows.len(), 10, "still exactly ten rows");
    assert_eq!(sum(&range.read(&after, &binder2).expect("resolves")), 55.0);
}

/// Inserting **inside** a range extends it — the new row is between the
/// endpoints, so it is in the range. This is what a spreadsheet user expects,
/// and nothing rewrote the formula to achieve it.
#[test]
fn inserting_inside_a_range_extends_it() {
    let mut f = Fixture::new(10);
    let state = f.state();
    let binder = Binder::from_state(&state);
    let range = binder
        .bind(0, 9, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");

    // New row after the 5th, with a value.
    let (c, l) = f.next();
    let new_row = insert_row(2, c, l, Anchor::After(f.rows[4]));
    let new_id = new_row.id;
    f.log.append(new_row);
    let (c, l) = f.next();
    f.log.append(set_cell(2, c, l, new_id, f.col, 100.0));

    let after = f.state();
    let binder2 = Binder::from_state(&after);
    let resolved = range.resolve(&binder2);
    assert_eq!(resolved.rows.len(), 11, "the inserted row joined the range");
    assert_eq!(
        sum(&range.read(&after, &binder2).expect("resolves")),
        155.0,
        "and its value is included"
    );
}

/// Inserting **below** the range does not join it.
#[test]
fn inserting_below_a_range_stays_outside() {
    let mut f = Fixture::new(10);
    let state = f.state();
    let binder = Binder::from_state(&state);
    // "A1:A5" — only the first half.
    let range = binder
        .bind(0, 4, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");

    let (c, l) = f.next();
    let new_row = insert_row(2, c, l, Anchor::After(f.rows[7]));
    let new_id = new_row.id;
    f.log.append(new_row);
    let (c, l) = f.next();
    f.log.append(set_cell(2, c, l, new_id, f.col, 100.0));

    let after = f.state();
    let binder2 = Binder::from_state(&after);
    assert_eq!(range.resolve(&binder2).rows.len(), 5);
    assert_eq!(sum(&range.read(&after, &binder2).expect("resolves")), 15.0);
}

// --------------------------------------------------- delete semantics

/// Deleting inside a range shrinks it.
#[test]
fn deleting_inside_a_range_shrinks_it() {
    let mut f = Fixture::new(10);
    let state = f.state();
    let binder = Binder::from_state(&state);
    let range = binder
        .bind(0, 9, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");

    let (c, l) = f.next();
    f.log.append(delete_row(2, c, l, f.rows[4])); // the row holding 5
    let after = f.state();
    let binder2 = Binder::from_state(&after);

    assert_eq!(range.resolve(&binder2).rows.len(), 9);
    assert_eq!(sum(&range.read(&after, &binder2).expect("resolves")), 50.0);
}

/// Deleting an **endpoint** re-anchors inward rather than breaking the
/// reference (docs/11).
#[test]
fn deleting_an_endpoint_reanchors_inward() {
    let mut f = Fixture::new(10);
    let state = f.state();
    let binder = Binder::from_state(&state);
    let range = binder
        .bind(0, 9, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");

    // Delete both endpoints: rows holding 1 and 10.
    let (c, l) = f.next();
    f.log.append(delete_row(2, c, l, f.rows[0]));
    let (c, l) = f.next();
    f.log.append(delete_row(2, c, l, f.rows[9]));

    let after = f.state();
    let binder2 = Binder::from_state(&after);
    let resolved = range.resolve(&binder2);

    assert_eq!(resolved.rows.len(), 8, "narrowed to the survivors");
    assert_eq!(
        resolved.rows[0],
        RowId(f.rows[1]),
        "start moved inward, not outward"
    );
    assert_eq!(resolved.rows[7], RowId(f.rows[8]), "end moved inward");
    assert_eq!(
        sum(&range.read(&after, &binder2).expect("resolves")),
        44.0,
        "2..=9"
    );
}

/// A range whose every identity is deleted is `#REF!`, not an empty sum. A
/// range that lost its target is a broken formula.
#[test]
fn an_emptied_range_is_a_ref_error() {
    let mut f = Fixture::new(4);
    let state = f.state();
    let binder = Binder::from_state(&state);
    let range = binder
        .bind(1, 2, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");

    for i in [1, 2] {
        let (c, l) = f.next();
        f.log.append(delete_row(2, c, l, f.rows[i]));
    }
    let after = f.state();
    let binder2 = Binder::from_state(&after);

    assert!(range.resolve(&binder2).rows.is_empty());
    let err = range
        .read(&after, &binder2)
        .expect_err("an emptied interval must be an error");
    assert_eq!(err.kind, ErrorKind::Ref);
}

// ------------------------------------------------ THE canonical test

/// **The canonical test** (BOOTSTRAP row 8): Alice inserts a row inside the
/// span of `SUM(A1:A10)` while Bob writes cells, concurrently. The replicas
/// must converge — same state hash — and the reference must resolve to the same
/// answer on both, without anything having rewritten the formula.
#[test]
fn concurrent_row_insert_against_sum_converges() {
    let f = Fixture::new(10);
    let base = f.state();
    let binder = Binder::from_state(&base);
    // Bob authored =SUM(A1:A10) before either concurrent edit happened.
    let range = binder
        .bind(0, 9, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");
    assert_eq!(sum(&range.read(&base, &binder).expect("resolves")), 55.0);

    // Alice: insert a row in the middle of the range, and give it a value.
    let alice_row = insert_row(10, 1, 100, Anchor::After(f.rows[4]));
    let alice_id = alice_row.id;
    let alice_value = set_cell(10, 2, 101, alice_id, f.col, 1000.0);

    // Bob, concurrently: overwrite two cells inside the same range.
    let bob_a = set_cell(20, 1, 100, f.rows[1], f.col, 20.0);
    let bob_b = set_cell(20, 2, 101, f.rows[7], f.col, 80.0);

    // Two replicas receive the four ops in opposite orders.
    let mut replica_a = f.log.clone();
    for op in [&alice_row, &alice_value, &bob_a, &bob_b] {
        replica_a.append(op.clone());
    }
    let mut replica_b = f.log.clone();
    for op in [&bob_b, &bob_a, &alice_value, &alice_row] {
        replica_b.append(op.clone());
    }

    let state_a = State::replay(&replica_a);
    let state_b = State::replay(&replica_b);

    // Convergence of the document.
    assert_eq!(
        state_a.state_hash(),
        state_b.state_hash(),
        "replicas must converge regardless of arrival order"
    );

    // Convergence of the *reference*: same rows, same answer, on both.
    let binder_a = Binder::from_state(&state_a);
    let binder_b = Binder::from_state(&state_b);
    let resolved_a = range.resolve(&binder_a);
    let resolved_b = range.resolve(&binder_b);
    assert_eq!(resolved_a, resolved_b);
    assert_eq!(resolved_a.rows.len(), 11, "Alice's row joined the range");

    let total_a = sum(&range.read(&state_a, &binder_a).expect("resolves"));
    let total_b = sum(&range.read(&state_b, &binder_b).expect("resolves"));
    assert_eq!(total_a, total_b);
    // 55 - 2 - 8 (Bob's overwrites) + 20 + 80 + 1000 (Alice's row) = 1145.
    assert_eq!(total_a, 1145.0);
}

/// The same guarantee through the formula engine: `=SUM(A1:A10)` evaluated
/// over a `State` gives the same answer on both replicas.
#[test]
fn sum_over_state_grid_agrees_across_replicas() {
    let f = Fixture::new(10);
    let alice_row = insert_row(10, 1, 100, Anchor::After(f.rows[4]));
    let alice_id = alice_row.id;
    let alice_value = set_cell(10, 2, 101, alice_id, f.col, 1000.0);

    let mut a = f.log.clone();
    a.append(alice_row.clone());
    a.append(alice_value.clone());
    let mut b = f.log.clone();
    b.append(alice_value);
    b.append(alice_row);

    let state_a = State::replay(&a);
    let state_b = State::replay(&b);
    assert_eq!(state_a.state_hash(), state_b.state_hash());

    let grid_a = StateGrid::new(&state_a);
    let grid_b = StateGrid::new(&state_b);
    assert_eq!(grid_a.extent(), grid_b.extent());

    // A1:A11 in the post-insert view covers everything.
    let va = usk_formula::evaluate("=SUM(A1:A11)", &grid_a, usk_types::coerce::Profile::Compat);
    let vb = usk_formula::evaluate("=SUM(A1:A11)", &grid_b, usk_types::coerce::Profile::Compat);
    assert_eq!(va, vb);
}

// --------------------------------------------------------- properties

/// Deterministic randomized sweep: under any sequence of inserts and deletes,
/// a resolved reference is always a **contiguous run of live rows**, always in
/// axis order, and never contains a deleted identity.
///
/// Hand-rolled LCG rather than a property-testing crate, matching the existing
/// convention in `usk-state` (`randomized_interleavings_converge`): the corpus
/// must be a pure function of its seed so a failure is reproducible by number
/// alone (DP-A2).
#[test]
fn resolution_is_always_a_contiguous_live_run() {
    let mut seed = 0xD1CE_5EEDu64;
    let mut rng = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };

    for trial in 0..200u64 {
        let mut f = Fixture::new(8);
        let state = f.state();
        let binder = Binder::from_state(&state);
        let lo = (rng() % 8) as usize;
        let hi = (rng() % 8) as usize;
        let range = binder
            .bind(
                lo.min(hi),
                lo.max(hi),
                0,
                0,
                AnchorMode::Relative,
                AnchorMode::Relative,
            )
            .expect("in range");

        let mut live: Vec<OpId> = f.rows.clone();
        for _ in 0..(rng() % 8) {
            if live.is_empty() {
                break;
            }
            let victim = (rng() as usize) % live.len();
            if rng() % 2 == 0 {
                let (c, l) = f.next();
                f.log.append(delete_row(3, c, l, live[victim]));
                live.remove(victim);
            } else {
                let (c, l) = f.next();
                let op = insert_row(3, c, l, Anchor::After(live[victim]));
                live.insert(victim + 1, op.id);
                f.log.append(op);
            }
        }

        let after = f.state();
        let binder2 = Binder::from_state(&after);
        let resolved = range.resolve(&binder2);
        let order = binder2.rows.order();

        // Every resolved row is live and appears in the axis order.
        for row in &resolved.rows {
            assert!(
                order.contains(row),
                "trial {trial}: resolved a row that is not live"
            );
        }
        // The run is contiguous in the live order.
        if let Some(first) = resolved.rows.first() {
            let start = order
                .iter()
                .position(|r| r == first)
                .expect("first is live");
            for (i, row) in resolved.rows.iter().enumerate() {
                assert_eq!(
                    order.get(start + i),
                    Some(row),
                    "trial {trial}: resolution is not a contiguous run"
                );
            }
        }
    }
}

/// Deterministic sweep of the canonical scenario: whatever order the ops
/// arrive in, the reference resolves identically.
#[test]
fn reference_resolution_is_arrival_order_independent() {
    let mut seed = 0xBEEF_1234u64;
    let mut rng = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };

    let f = Fixture::new(6);
    let base = f.state();
    let binder = Binder::from_state(&base);
    let range = binder
        .bind(0, 5, 0, 0, AnchorMode::Relative, AnchorMode::Relative)
        .expect("range exists");

    let extra = alloc::vec![
        insert_row(10, 1, 100, Anchor::After(f.rows[2])),
        set_cell(20, 1, 101, f.rows[0], f.col, 99.0),
        delete_row(30, 1, 102, f.rows[4]),
        set_cell(40, 1, 103, f.rows[5], f.col, 7.0),
    ];

    let mut reference: Option<(usk_calc::Resolved, f64)> = None;
    for _ in 0..60 {
        let mut shuffled = extra.clone();
        // Fisher–Yates with the seeded LCG.
        for i in (1..shuffled.len()).rev() {
            let j = (rng() as usize) % (i + 1);
            shuffled.swap(i, j);
        }
        let mut log = f.log.clone();
        for op in &shuffled {
            log.append(op.clone());
        }
        let state = State::replay(&log);
        let b = Binder::from_state(&state);
        let resolved = range.resolve(&b);
        let total = sum(&range.read(&state, &b).expect("resolves"));

        match &reference {
            None => reference = Some((resolved, total)),
            Some((r, t)) => {
                assert_eq!(&resolved, r, "resolution depended on arrival order");
                assert_eq!(total, *t, "the answer depended on arrival order");
            }
        }
    }
}

extern crate alloc;
