//! Row 9 proofs: commands compile to ops, and undo obeys its laws
//! (BOOTSTRAP row 9, docs/11, DP-A7, DP-A12).
//!
//! The law under test: **undo∘do = id on the actor's own scope** — and only on
//! it. Where another actor's work is in the way, undo yields rather than
//! destroys, and says so via `ApplyReport::blocked`.

use usk_reduce::{ApplyReport, Command, CommandError, Session};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

fn num(v: f64) -> Value {
    Value::Number(v)
}

/// A session with a 3×3 sheet built entirely through commands — dogfooding the
/// reducer for its own fixture.
fn session_3x3(actor: u128) -> Session {
    let mut s = Session::new(ActorId(actor));
    for _ in 0..3 {
        s.apply(Command::InsertCol { before: 0 })
            .expect("insert col");
        s.apply(Command::InsertRow { before: 0 })
            .expect("insert row");
    }
    s
}

/// The visible projection: live grid contents by ordinal, plus formula text.
/// This is what "identity" means in the undo law — the user-visible sheet,
/// not the ever-growing log.
fn projection(state: &State) -> Vec<Vec<String>> {
    let rows = state.row_order();
    let cols = state.col_order();
    rows.iter()
        .map(|r| {
            cols.iter()
                .map(|c| match state.formula(*r, *c) {
                    Some(f) => format!("={}", f.source),
                    None => match state.cell(*r, *c) {
                        None | Some(Value::Blank) => String::from("·"),
                        Some(Value::Number(n)) => format!("{n}"),
                        Some(v) => format!("{v:?}"),
                    },
                })
                .collect()
        })
        .collect()
}

// ------------------------------------------------------------ the law

/// undo∘do = id, for every command kind, measured on the visible projection.
#[test]
fn undo_after_do_is_identity_on_the_projection() {
    let cmds = [
        Command::SetValue {
            row: 1,
            col: 1,
            value: num(42.0),
        },
        Command::SetFormula {
            row: 0,
            col: 0,
            source: String::from("=B2*2"),
        },
        Command::ClearCell { row: 1, col: 1 },
        Command::InsertRow { before: 1 },
        Command::DeleteRow { at: 2 },
        Command::InsertCol { before: 3 },
        Command::DeleteCol { at: 0 },
    ];
    for cmd in cmds {
        let mut s = session_3x3(1);
        s.apply(Command::SetValue {
            row: 1,
            col: 1,
            value: num(7.0),
        })
        .expect("seed");
        let before = projection(s.state());
        s.apply(cmd.clone()).expect("do");
        s.apply(Command::Undo).expect("undo");
        assert_eq!(
            projection(s.state()),
            before,
            "undo∘do failed to restore the projection for {cmd:?}"
        );
    }
}

/// redo∘undo∘do = do: the redo branch reproduces what was undone.
#[test]
fn redo_restores_what_undo_removed() {
    let mut s = session_3x3(1);
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(5.0),
    })
    .expect("set");
    let after_do = projection(s.state());
    s.apply(Command::Undo).expect("undo");
    assert_ne!(projection(s.state()), after_do);
    s.apply(Command::Redo).expect("redo");
    assert_eq!(projection(s.state()), after_do);
}

/// A fresh edit invalidates the redo branch, as in every editor.
#[test]
fn a_new_edit_clears_redo() {
    let mut s = session_3x3(1);
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(1.0),
    })
    .expect("set");
    s.apply(Command::Undo).expect("undo");
    s.apply(Command::SetValue {
        row: 2,
        col: 2,
        value: num(9.0),
    })
    .expect("new edit");
    let report = s.apply(Command::Redo).expect("redo is a no-op");
    assert_eq!(report, ApplyReport::default(), "redo stack must be empty");
}

// ----------------------------------------------------- value round trips

/// Undo restores the *previous* value, not blank, when there was one.
#[test]
fn undo_restores_the_prior_value() {
    let mut s = session_3x3(1);
    let set = |v| Command::SetValue {
        row: 0,
        col: 1,
        value: num(v),
    };
    s.apply(set(1.0)).expect("first");
    s.apply(set(2.0)).expect("second");

    let rows = s.state().row_order();
    let cols = s.state().col_order();
    let at = |s: &mut Session| s.state().cell(rows[0], cols[1]);

    assert_eq!(at(&mut s), Some(num(2.0)));
    s.apply(Command::Undo).expect("undo second");
    assert_eq!(at(&mut s), Some(num(1.0)), "prior value, not blank");
    s.apply(Command::Undo).expect("undo first");
    assert_eq!(at(&mut s), Some(Value::Blank), "back to blank");
}

/// Undoing a formula write restores the formula that was there before.
#[test]
fn undo_restores_a_prior_formula() {
    let mut s = session_3x3(1);
    s.apply(Command::SetFormula {
        row: 0,
        col: 0,
        source: String::from("=B1+1"),
    })
    .expect("first formula");
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(3.0),
    })
    .expect("overwrite with value");

    let rows = s.state().row_order();
    let cols = s.state().col_order();
    assert!(s.state().formula(rows[0], cols[0]).is_none());

    s.apply(Command::Undo).expect("undo");
    let f = s
        .state()
        .formula(rows[0], cols[0])
        .expect("formula restored");
    assert_eq!(f.source, "=B1+1");
    assert_eq!(f.bindings.len(), 1, "its identity binding came back too");
}

// ------------------------------------------------------ others' work

/// **The selective-undo law**: my undo never clobbers a later write by someone
/// else. Their value stays; the undo reports itself blocked.
#[test]
fn undo_yields_to_a_later_write_by_another_actor() {
    let mut s = session_3x3(1);
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(1.0),
    })
    .expect("mine");

    // Actor 2 overwrites the same cell, arriving via sync.
    let rows = s.state().row_order();
    let cols = s.state().col_order();
    let theirs = usk_oplog::Op {
        id: OpId {
            actor: ActorId(2),
            counter: 1,
        },
        lamport: 1_000,
        payload: usk_oplog::Payload::SetCell {
            row: rows[0],
            col: cols[0],
            value: num(99.0),
        },
    };
    s.integrate(theirs);
    assert_eq!(s.state().cell(rows[0], cols[0]), Some(num(99.0)));

    let report = s.apply(Command::Undo).expect("undo");
    assert_eq!(report.blocked, 1, "the undo must report it yielded");
    assert_eq!(report.ops_emitted, 0);
    assert_eq!(
        s.state().cell(rows[0], cols[0]),
        Some(num(99.0)),
        "their write survives my undo"
    );
}

/// Undoing my row-insert is blocked once someone else has put work in it.
#[test]
fn undo_of_insert_is_blocked_when_others_wrote_into_the_row() {
    let mut s = session_3x3(1);
    s.apply(Command::InsertRow { before: 1 }).expect("insert");
    let inserted: RowId = s.state().row_order()[1];

    let col: ColId = s.state().col_order()[0];
    s.integrate(usk_oplog::Op {
        id: OpId {
            actor: ActorId(2),
            counter: 1,
        },
        lamport: 1_000,
        payload: usk_oplog::Payload::SetCell {
            row: inserted,
            col,
            value: num(7.0),
        },
    });

    let report = s.apply(Command::Undo).expect("undo");
    assert_eq!(report.blocked, 1);
    assert_eq!(
        s.state().row_order().len(),
        4,
        "the row stays: deleting it would destroy their cell"
    );
    assert_eq!(s.state().cell(inserted, col), Some(num(7.0)));
}

/// With no foreign work in the row, undoing the insert removes it.
#[test]
fn undo_of_insert_removes_an_untouched_row() {
    let mut s = session_3x3(1);
    s.apply(Command::InsertRow { before: 1 }).expect("insert");
    assert_eq!(s.state().row_order().len(), 4);
    let report = s.apply(Command::Undo).expect("undo");
    assert_eq!(report.blocked, 0);
    assert_eq!(s.state().row_order().len(), 3);
}

// --------------------------------------------------------- resurrection

/// Undoing a delete brings the row back **with its cells**: deletion
/// tombstoned the identity, never the data (DP-A1).
#[test]
fn undo_of_delete_resurrects_the_row_with_its_cells() {
    let mut s = session_3x3(1);
    s.apply(Command::SetValue {
        row: 1,
        col: 0,
        value: num(11.0),
    })
    .expect("value in the doomed row");
    let doomed: RowId = s.state().row_order()[1];
    let col: ColId = s.state().col_order()[0];

    s.apply(Command::DeleteRow { at: 1 }).expect("delete");
    assert_eq!(s.state().row_order().len(), 2);

    s.apply(Command::Undo).expect("undo");
    let rows = s.state().row_order();
    assert_eq!(rows.len(), 3, "the row is back");
    assert_eq!(rows[1], doomed, "at its original place in the order");
    assert_eq!(
        s.state().cell(doomed, col),
        Some(num(11.0)),
        "with its data intact"
    );
}

/// And redo deletes it again — undo-of-undo through the same machinery.
#[test]
fn redo_of_a_delete_deletes_again() {
    let mut s = session_3x3(1);
    s.apply(Command::DeleteRow { at: 0 }).expect("delete");
    s.apply(Command::Undo).expect("undo");
    assert_eq!(s.state().row_order().len(), 3);
    s.apply(Command::Redo).expect("redo");
    assert_eq!(s.state().row_order().len(), 2);
}

// ------------------------------------------------------------ scoping

/// Undo stacks are per-actor: my undo touches my group, never their edits —
/// even edits that came after mine.
#[test]
fn undo_scope_is_per_actor() {
    let mut s = session_3x3(1);
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(1.0),
    })
    .expect("mine");

    // Actor 2 edits a *different* cell afterwards.
    let rows = s.state().row_order();
    let cols = s.state().col_order();
    s.integrate(usk_oplog::Op {
        id: OpId {
            actor: ActorId(2),
            counter: 1,
        },
        lamport: 1_000,
        payload: usk_oplog::Payload::SetCell {
            row: rows[2],
            col: cols[2],
            value: num(50.0),
        },
    });

    s.apply(Command::Undo).expect("undo mine");
    assert_eq!(
        s.state().cell(rows[0], cols[0]),
        Some(Value::Blank),
        "my edit is undone"
    );
    assert_eq!(
        s.state().cell(rows[2], cols[2]),
        Some(num(50.0)),
        "their later edit is untouched — undo is scoped to my ops"
    );
}

// ------------------------------------------------------------- errors

/// Commands validate their coordinates; nothing panics (DP-A10 for the
/// command surface).
#[test]
fn out_of_range_commands_are_errors_not_panics() {
    let mut s = session_3x3(1);
    for cmd in [
        Command::SetValue {
            row: 9,
            col: 0,
            value: num(1.0),
        },
        Command::DeleteRow { at: 9 },
        Command::InsertRow { before: 9 },
        Command::DeleteCol { at: 9 },
    ] {
        assert_eq!(s.apply(cmd).unwrap_err(), CommandError::OutOfRange);
    }
    // A formula referencing a cell outside the grid cannot bind.
    assert_eq!(
        s.apply(Command::SetFormula {
            row: 0,
            col: 0,
            source: String::from("=Z99")
        })
        .unwrap_err(),
        CommandError::UnboundReference
    );
}

/// Undo/redo on empty stacks are quiet no-ops.
#[test]
fn undo_on_an_empty_stack_is_a_noop() {
    let mut s = Session::new(ActorId(1));
    assert_eq!(s.apply(Command::Undo).expect("ok"), ApplyReport::default());
    assert_eq!(s.apply(Command::Redo).expect("ok"), ApplyReport::default());
}

// ------------------------------- docs/27 §5: the undo state machine

/// **Transition coverage** for docs/27 §5: `LIVE ──undo──► UNDONE ──redo──►
/// LIVE`, plus the structural `NARROWED(notice)` state. Every listed edge is
/// exercised here; the spec requires it explicitly.
#[test]
fn undo_machine_covers_every_listed_transition() {
    let mut s = session_3x3(1);

    // LIVE: a group exists.
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(1.0),
    })
    .expect("LIVE");
    let live = projection(s.state());

    // LIVE ──undo──► UNDONE
    let undone = s.apply(Command::Undo).expect("undo");
    assert_eq!(undone.blocked, 0);
    assert_ne!(projection(s.state()), live);

    // UNDONE ──redo──► LIVE
    s.apply(Command::Redo).expect("redo");
    assert_eq!(projection(s.state()), live);

    // LIVE ──undo (structural, others' ops in the way)──► NARROWED(notice).
    // The notice is `ApplyReport::blocked` being non-zero — the spec's
    // "notice" made machine-readable rather than a log line.
    s.apply(Command::InsertRow { before: 1 }).expect("insert");
    let inserted: RowId = s.state().row_order()[1];
    let col: ColId = s.state().col_order()[0];
    s.integrate(usk_oplog::Op {
        id: OpId {
            actor: ActorId(2),
            counter: 77,
        },
        lamport: 5_000,
        payload: usk_oplog::Payload::SetCell {
            row: inserted,
            col,
            value: num(3.0),
        },
    });
    let narrowed = s.apply(Command::Undo).expect("undo");
    assert_eq!(narrowed.blocked, 1, "NARROWED must carry a notice");
    assert_eq!(narrowed.ops_emitted, 0);
}

/// **Forbidden transitions** for docs/27 §5. The spec says a transition not
/// listed is rejected, never silent; these are the two it names.
#[test]
fn undo_machine_rejects_its_forbidden_transitions() {
    // Forbidden 1: "undoing another actor's group."
    // Structurally impossible rather than checked: the undo stack lives on the
    // Session, is keyed to that Session's actor, and only `apply` pushes to it.
    // A remote op arrives through `integrate`, which never touches the stack —
    // so a foreign group cannot be reached to be undone. Proven by observing
    // that integrating remote work leaves nothing to undo.
    let mut s = Session::new(ActorId(1));
    for _ in 0..2 {
        s.apply(Command::InsertCol { before: 0 }).expect("col");
        s.apply(Command::InsertRow { before: 0 }).expect("row");
    }
    let rows = s.state().row_order();
    let cols = s.state().col_order();
    let mut mine = Session::new(ActorId(1));
    core::mem::swap(&mut mine, &mut s);
    // Drain my own groups so only the foreign op could possibly be undone.
    while mine.apply(Command::Undo).expect("drain").ops_emitted > 0 {}

    mine.integrate(usk_oplog::Op {
        id: OpId {
            actor: ActorId(2),
            counter: 1,
        },
        lamport: 9_000,
        payload: usk_oplog::Payload::SetCell {
            row: rows[0],
            col: cols[0],
            value: num(42.0),
        },
    });
    let report = mine.apply(Command::Undo).expect("undo");
    assert_eq!(
        report,
        ApplyReport::default(),
        "another actor's group is not reachable from my undo stack"
    );
    assert_eq!(
        mine.state().cell(rows[0], cols[0]),
        Some(num(42.0)),
        "and their work is untouched"
    );

    // Forbidden 2: "a group spanning two Commands."
    // Each `apply` of a mutating Command pushes exactly one group, so one undo
    // reverses exactly one Command — never two, never half.
    let mut t = session_3x3(1);
    t.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: num(1.0),
    })
    .expect("first");
    t.apply(Command::SetValue {
        row: 1,
        col: 1,
        value: num(2.0),
    })
    .expect("second");
    let r = t.state().row_order();
    let c = t.state().col_order();

    t.apply(Command::Undo).expect("undo one");
    assert_eq!(
        t.state().cell(r[1], c[1]),
        Some(Value::Blank),
        "the second Command was undone"
    );
    assert_eq!(
        t.state().cell(r[0], c[0]),
        Some(num(1.0)),
        "the first Command was NOT — one undo, one Command"
    );
}
