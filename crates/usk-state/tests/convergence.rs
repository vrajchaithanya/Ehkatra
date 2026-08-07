//! Convergence tests — the heart of the CRDT bet (docs/35 layer 2).
//! Includes the canonical case: concurrent row-insert vs cell writes,
//! and a randomized-interleaving convergence sweep.

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_state::State;
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

fn set_cell(actor: u128, counter: u64, lamport: u64, row: OpId, col: OpId, v: Value) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload: Payload::SetCell {
            row: RowId(row),
            col: ColId(col),
            value: v,
        },
    }
}

/// Same op set, two arrival orders -> identical state hash.
#[test]
fn order_independence_basic() {
    let r1 = insert_row(1, 1, 1, Anchor::Start);
    let c1 = insert_col(1, 2, 2, Anchor::Start);
    let w1 = set_cell(1, 3, 3, r1.id, c1.id, Value::Number(42.0));
    let r2 = insert_row(2, 1, 3, Anchor::After(r1.id));
    let w2 = set_cell(2, 2, 4, r2.id, c1.id, Value::Text("hello".into()));

    let mut a = OpLog::new();
    for op in [&r1, &c1, &w1, &r2, &w2] {
        a.append((*op).clone());
    }
    let mut b = OpLog::new();
    for op in [&w2, &r2, &w1, &c1, &r1] {
        b.append((*op).clone());
    }

    assert_eq!(a.canonical_hash(), b.canonical_hash());
    assert_eq!(
        State::replay(&a).state_hash(),
        State::replay(&b).state_hash()
    );
}

/// THE canonical spreadsheet-CRDT case (docs/15): Alice inserts a row while
/// Bob concurrently writes cells in existing rows. Both replicas converge,
/// Bob's data lands in the rows he addressed (identity, not position).
#[test]
fn concurrent_structural_edit_converges() {
    // Shared history: two rows, one col.
    let r1 = insert_row(1, 1, 1, Anchor::Start);
    let r2 = insert_row(1, 2, 2, Anchor::After(r1.id));
    let c1 = insert_col(1, 3, 3, Anchor::Start);

    // Alice (actor 1), offline: inserts a row between r1 and r2.
    let ra = insert_row(1, 4, 10, Anchor::After(r1.id));
    // Bob (actor 2), offline concurrently: writes into r1 and r2 by identity.
    let w1 = set_cell(2, 1, 10, r1.id, c1.id, Value::Number(1.0));
    let w2 = set_cell(2, 2, 11, r2.id, c1.id, Value::Number(2.0));

    // Replica A receives Alice-then-Bob; replica B receives Bob-then-Alice.
    let mut la = OpLog::new();
    for op in [&r1, &r2, &c1, &ra, &w1, &w2] {
        la.append((*op).clone());
    }
    let mut lb = OpLog::new();
    for op in [&r1, &r2, &c1, &w1, &w2, &ra] {
        lb.append((*op).clone());
    }

    let sa = State::replay(&la);
    let sb = State::replay(&lb);
    assert_eq!(sa.state_hash(), sb.state_hash(), "replicas diverged");

    // Row order is r1, ra, r2 on both.
    let order = sa.row_order();
    assert_eq!(order.len(), 3);
    assert_eq!(order[0].0, r1.id);
    assert_eq!(order[1].0, ra.id);
    assert_eq!(order[2].0, r2.id);

    // Bob's writes are on the rows he meant, not displaced by the insert.
    assert_eq!(
        sa.cell(RowId(r1.id), ColId(c1.id)),
        Some(&Value::Number(1.0))
    );
    assert_eq!(
        sa.cell(RowId(r2.id), ColId(c1.id)),
        Some(&Value::Number(2.0))
    );
}

/// Concurrent same-cell writes: deterministic winner, loser RETAINED (ADR-006).
#[test]
fn concurrent_cell_write_retains_loser() {
    let r1 = insert_row(1, 1, 1, Anchor::Start);
    let c1 = insert_col(1, 2, 2, Anchor::Start);
    let wa = set_cell(1, 3, 5, r1.id, c1.id, Value::Number(10.0));
    let wb = set_cell(2, 1, 5, r1.id, c1.id, Value::Number(20.0)); // same lamport, higher actor

    let mut l = OpLog::new();
    for op in [&r1, &c1, &wa, &wb] {
        l.append((*op).clone());
    }
    let s = State::replay(&l);

    // (5, actor2) > (5, actor1): actor 2 wins deterministically.
    assert_eq!(
        s.cell(RowId(r1.id), ColId(c1.id)),
        Some(&Value::Number(20.0))
    );
    let losers = s.conflicts(RowId(r1.id), ColId(c1.id));
    assert_eq!(losers.len(), 1);
    assert_eq!(losers[0].2, Value::Number(10.0));
}

/// Randomized interleaving sweep: N ops shuffled deterministically many ways,
/// every permutation converges to one hash. (Simple LCG shuffle: no rand dep.)
#[test]
fn randomized_interleavings_converge() {
    // Build a small history: 4 rows, 2 cols, 8 writes, 1 delete.
    let mut ops: Vec<Op> = Vec::new();
    let r: Vec<Op> = (0..4)
        .map(|i| {
            insert_row(
                1,
                i + 1,
                i + 1,
                if i == 0 {
                    Anchor::Start
                } else {
                    Anchor::After(opid(1, i))
                },
            )
        })
        .collect();
    ops.extend(r.iter().cloned());
    let c: Vec<Op> = (0..2)
        .map(|i| {
            insert_col(
                1,
                10 + i,
                10 + i,
                if i == 0 {
                    Anchor::Start
                } else {
                    Anchor::After(opid(1, 10))
                },
            )
        })
        .collect();
    ops.extend(c.iter().cloned());
    let mut k: u64 = 0;
    for row_op in r.iter().take(4) {
        for col_op in c.iter().take(2) {
            ops.push(set_cell(
                2 + (k as u128 % 2),
                k + 1,
                20 + k,
                row_op.id,
                col_op.id,
                Value::Number(k as f64),
            ));
            k += 1;
        }
    }
    ops.push(Op {
        id: opid(3, 1),
        lamport: 40,
        payload: Payload::DeleteRow {
            row: RowId(r[2].id),
        },
    });

    let reference = {
        let mut l = OpLog::new();
        for op in &ops {
            l.append(op.clone());
        }
        State::replay(&l).state_hash()
    };

    // Deterministic LCG-based Fisher-Yates; 200 permutations.
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    for _ in 0..200 {
        let mut shuffled = ops.clone();
        for i in (1..shuffled.len()).rev() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (seed >> 33) as usize % (i + 1);
            shuffled.swap(i, j);
        }
        let mut l = OpLog::new();
        for op in &shuffled {
            l.append(op.clone());
        }
        assert_eq!(
            State::replay(&l).state_hash(),
            reference,
            "interleaving diverged"
        );
    }
}

/// merge_from is idempotent and commutative at the log level.
#[test]
fn log_merge_idempotent_commutative() {
    let r1 = insert_row(1, 1, 1, Anchor::Start);
    let c1 = insert_col(1, 2, 2, Anchor::Start);
    let w1 = set_cell(2, 1, 3, r1.id, c1.id, Value::Bool(true));

    let mut a = OpLog::new();
    a.append(r1.clone());
    a.append(c1.clone());
    let mut b = OpLog::new();
    b.append(w1.clone());

    let mut ab = a.clone();
    ab.merge_from(&b);
    ab.merge_from(&b); // idempotent
    let mut ba = b.clone();
    ba.merge_from(&a);

    assert_eq!(ab.canonical_hash(), ba.canonical_hash());
    assert_eq!(
        State::replay(&ab).state_hash(),
        State::replay(&ba).state_hash()
    );
}
