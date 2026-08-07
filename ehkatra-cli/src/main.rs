//! ehkatra — CLI demo of the kernel: builds a workbook from ops, prints the
//! grid, then simulates two replicas receiving the same ops in different
//! orders and proves convergence by state hash.

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn main() {
    let a1 = ActorId(1);
    let a2 = ActorId(2);
    let mut ctr = 0u64;
    let mut next = |actor: ActorId, lamport: u64, payload: Payload| {
        ctr += 1;
        Op {
            id: OpId {
                actor,
                counter: ctr,
            },
            lamport,
            payload,
        }
    };

    // Alice builds a 2x2 grid.
    let r1 = next(
        a1,
        1,
        Payload::InsertRow {
            anchor: Anchor::Start,
        },
    );
    let (r1id, _) = (r1.id, ());
    let r2 = next(
        a1,
        2,
        Payload::InsertRow {
            anchor: Anchor::After(r1id),
        },
    );
    let c1 = next(
        a1,
        3,
        Payload::InsertCol {
            anchor: Anchor::Start,
        },
    );
    let c2 = next(
        a1,
        4,
        Payload::InsertCol {
            anchor: Anchor::After(c1.id),
        },
    );
    let w1 = next(
        a1,
        5,
        Payload::SetCell {
            row: RowId(r1id),
            col: ColId(c1.id),
            value: Value::Text("Revenue".into()),
        },
    );
    let w2 = next(
        a1,
        6,
        Payload::SetCell {
            row: RowId(r1id),
            col: ColId(c2.id),
            value: Value::Number(1250.5),
        },
    );
    // Bob concurrently inserts a row after r1 and writes into r2.
    let rb = next(
        a2,
        6,
        Payload::InsertRow {
            anchor: Anchor::After(r1id),
        },
    );
    let w3 = next(
        a2,
        7,
        Payload::SetCell {
            row: RowId(r2.id),
            col: ColId(c1.id),
            value: Value::Text("Costs".into()),
        },
    );
    let w4 = next(
        a2,
        8,
        Payload::SetCell {
            row: RowId(r2.id),
            col: ColId(c2.id),
            value: Value::Number(730.25),
        },
    );

    let all = [r1, r2, c1, c2, w1, w2, rb, w3, w4];

    // Replica A: natural order. Replica B: reversed arrival.
    let mut la = OpLog::new();
    for op in &all {
        la.append(op.clone());
    }
    let mut lb = OpLog::new();
    for op in all.iter().rev() {
        lb.append(op.clone());
    }

    let sa = State::replay(&la);
    let sb = State::replay(&lb);

    println!("Ehkatra kernel demo — two replicas, different op arrival orders\n");
    print_grid(&sa);
    println!("\nreplica A state hash: {}", sa.state_hash());
    println!("replica B state hash: {}", sb.state_hash());
    if sa.state_hash() == sb.state_hash() {
        println!("\nCONVERGED ✓  (identical op sets ⇒ identical state, any order)");
    } else {
        println!("\nDIVERGED ✗ — this is a kernel bug");
        std::process::exit(1);
    }
}

fn print_grid(s: &State) {
    let rows = s.row_order();
    let cols = s.col_order();
    for r in &rows {
        let mut line = String::new();
        for c in &cols {
            let cell = match s.cell(*r, *c) {
                Some(Value::Text(t)) => t,
                Some(Value::Number(n)) => format!("{n}"),
                Some(Value::Bool(b)) => format!("{b}"),
                Some(Value::Error(e)) => format!("{e:?}"),
                Some(Value::Blank) | None => String::from("·"),
            };
            line.push_str(&format!("{cell:>12} "));
        }
        println!("{line}");
    }
}
