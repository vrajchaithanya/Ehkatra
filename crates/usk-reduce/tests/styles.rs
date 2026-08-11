//! Style commands and their selective undo (ADR-041 decision 5, docs/11).
//!
//! The undo law for a *rectangle* is the cell law read one dimension up:
//! restore only while this actor's own rule still wins, and where a later
//! foreign rule overlaps, block rather than destroy. What makes the restore
//! **exact** rather than approximate is that the previous resolution over the
//! rectangle is itself a set of rectangles, so it can be replayed.

use usk_oplog::{StyleFacet, FONT_BOLD};
use usk_reduce::{Command, RectSpec, Session, SpanSpec};
use usk_state::State;
use usk_types::{ActorId, ColId, RowId, Value};

const NUMBER_FORMAT: u8 = 0x01;
const FILL: u8 = 0x03;

fn fill(argb: u32) -> StyleFacet {
    StyleFacet::Fill(argb)
}

fn bold() -> StyleFacet {
    StyleFacet::Font(usk_oplog::FontFacet {
        flags: FONT_BOLD,
        half_points: 22,
        argb: 0xFF00_0000,
        name: String::from("Calibri"),
    })
}

fn session_4x4(actor: u128) -> Session {
    let mut s = Session::new(ActorId(actor));
    for _ in 0..4 {
        s.apply(Command::InsertCol { before: 0 })
            .expect("insert col");
        s.apply(Command::InsertRow { before: 0 })
            .expect("insert row");
    }
    s
}

fn at(state: &State, row: usize, col: usize) -> (RowId, ColId) {
    (state.row_order()[row], state.col_order()[col])
}

/// The fills of the whole grid, by ordinal — the visible projection of one
/// facet, which is what an undo law has to be measured on.
fn fills(state: &State) -> Vec<Vec<Option<u32>>> {
    let resolver = state.style_resolver();
    state
        .row_order()
        .iter()
        .map(|r| {
            state
                .col_order()
                .iter()
                .map(|c| match resolver.facet(state.styles(), *r, *c, FILL) {
                    Some(StyleFacet::Fill(argb)) => Some(*argb),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn set_fill(session: &mut Session, target: RectSpec, argb: u32) {
    session
        .apply(Command::SetStyle {
            target,
            facet: fill(argb),
        })
        .expect("set style");
}

// ------------------------------------------------------------ the commands

/// A whole-column format is **one op**. This is the claim the addressing unit
/// was chosen for, and the cheapest place to check it is the reducer's own
/// output.
#[test]
fn formatting_a_whole_column_emits_exactly_one_op() {
    let mut s = session_4x4(1);
    let report = s
        .apply(Command::SetStyle {
            target: RectSpec::column(1),
            facet: fill(0xFFFF_FF00),
        })
        .expect("set style");
    assert_eq!(report.ops_emitted, 1, "one gesture, one op");

    let state = s.state();
    assert_eq!(fills(state)[0][1], Some(0xFFFF_FF00));
    assert_eq!(fills(state)[3][1], Some(0xFFFF_FF00));
    assert_eq!(fills(state)[0][0], None);
}

/// A style command binds ordinals to identities once, at the author (DP-A7),
/// so a row inserted above it afterwards does not slide the formatting.
#[test]
fn a_style_command_binds_identities_and_survives_a_later_row_insert() {
    let mut s = session_4x4(1);
    set_fill(
        &mut s,
        RectSpec {
            rows: SpanSpec::Range(1, 2),
            cols: SpanSpec::Range(0, 0),
        },
        0xFFFF_FF00,
    );
    let styled = at(s.state(), 1, 0);

    s.apply(Command::InsertRow { before: 0 }).expect("insert");
    let state = s.state();
    assert_eq!(state.row_order().len(), 5);
    assert_eq!(
        state.style_at(styled.0, styled.1).fill,
        Some(fill(0xFFFF_FF00)),
        "the styled identity kept its fill across the insert"
    );
    // And in ordinals it has moved down by one, which is the whole point.
    assert_eq!(fills(state)[0][0], None);
    assert_eq!(fills(state)[2][0], Some(0xFFFF_FF00));
}

/// Two facets, one cell, one actor: the second command must not clear the
/// first. Facet independence is not only a concurrency property.
#[test]
fn setting_a_second_facet_leaves_the_first_alone() {
    let mut s = session_4x4(1);
    set_fill(&mut s, RectSpec::cell(0, 0), 0xFFFF_FF00);
    s.apply(Command::SetStyle {
        target: RectSpec::cell(0, 0),
        facet: bold(),
    })
    .expect("set font");

    let (r, c) = at(s.state(), 0, 0);
    let style = s.state().style_at(r, c);
    assert_eq!(style.fill, Some(fill(0xFFFF_FF00)));
    assert_eq!(style.font, Some(bold()));
}

/// Out-of-range ordinals are refused rather than clamped, exactly as they are
/// for a cell write.
#[test]
fn a_style_command_off_the_grid_is_refused() {
    let mut s = session_4x4(1);
    assert!(s
        .apply(Command::SetStyle {
            target: RectSpec::cell(99, 0),
            facet: fill(1),
        })
        .is_err());
    assert!(s
        .apply(Command::ClearStyle {
            target: RectSpec::cell(0, 99),
            facet_slot: FILL,
        })
        .is_err());
    // `All` names no ordinal, so it is bindable on an axis of any length.
    assert!(s
        .apply(Command::SetStyle {
            target: RectSpec {
                rows: SpanSpec::All,
                cols: SpanSpec::All,
            },
            facet: fill(1),
        })
        .is_ok());
}

// ----------------------------------------------------------------- the undo

/// undo∘do = id on the visible projection of the facet — the same law the cell
/// commands obey, read for a rectangle.
#[test]
fn undo_after_a_style_write_is_identity_on_the_projection() {
    let mut s = session_4x4(1);
    let before = fills(s.state()).clone();

    set_fill(&mut s, RectSpec::column(2), 0xFFFF_FF00);
    assert_ne!(fills(s.state()).clone(), before, "the write did something");

    let report = s.apply(Command::Undo).expect("undo");
    assert_eq!(report.blocked, 0);
    assert_eq!(fills(s.state()).clone(), before, "undo restored the sheet");

    s.apply(Command::Redo).expect("redo");
    assert_eq!(
        fills(s.state())[0][2],
        Some(0xFFFF_FF00),
        "redo put it back"
    );
}

/// **The exactness claim.** Undoing a write that sat on top of *heterogeneous*
/// prior formatting restores the heterogeneity, not some average of it. This is
/// what the rectangle model buys: the previous resolution is itself a set of
/// rectangles, so it can be replayed rather than approximated.
#[test]
fn undoing_a_style_write_restores_a_heterogeneous_prior_exactly() {
    let mut s = session_4x4(1);
    // Two different fills underneath, on two different sub-ranges, plus one
    // cell left unformatted.
    set_fill(
        &mut s,
        RectSpec {
            rows: SpanSpec::Range(0, 1),
            cols: SpanSpec::Range(0, 0),
        },
        0xFFFF_0000,
    );
    set_fill(
        &mut s,
        RectSpec {
            rows: SpanSpec::Range(2, 2),
            cols: SpanSpec::Range(0, 0),
        },
        0xFF00_FF00,
    );
    let before = fills(s.state()).clone();
    assert_eq!(
        before.iter().map(|r| r[0]).collect::<Vec<_>>(),
        vec![
            Some(0xFFFF_0000),
            Some(0xFFFF_0000),
            Some(0xFF00_FF00),
            None
        ],
        "three distinct prior states in one column"
    );

    // Now paint the whole column over the top, and undo it.
    set_fill(&mut s, RectSpec::column(0), 0xFF0000FF);
    assert!(fills(s.state()).iter().all(|r| r[0] == Some(0xFF0000FF)));

    let report = s.apply(Command::Undo).expect("undo");
    assert_eq!(report.blocked, 0);
    assert_eq!(
        fills(s.state()).clone(),
        before,
        "every sub-range came back as it was, including the unformatted cell"
    );
}

/// **Per-actor undo.** Another actor's *later* rule overlapping the rectangle
/// blocks the undo entirely — narrowing rather than destroying (docs/11), and
/// reported rather than silent.
#[test]
fn undo_of_a_style_change_is_blocked_by_a_later_foreign_write() {
    let mut mine = session_4x4(1);
    set_fill(&mut mine, RectSpec::column(0), 0xFFFF_FF00);

    // A second actor sees my log and paints one cell of the same column.
    let mut theirs = Session::from_log(ActorId(2), mine.log.clone());
    let target = {
        let state = theirs.state();
        let (row, col) = (state.row_order()[1], state.col_order()[0]);
        (row, col)
    };
    theirs
        .apply(Command::SetStyle {
            target: RectSpec::cell(1, 0),
            facet: fill(0xFF00_FF00),
        })
        .expect("their style");
    let their_ops: Vec<_> = theirs
        .log
        .ops()
        .iter()
        .filter(|o| o.id.actor == ActorId(2))
        .cloned()
        .collect();
    mine.integrate_batch(their_ops);

    let report = mine.apply(Command::Undo).expect("undo");
    assert_eq!(
        report.blocked, 1,
        "a later foreign rule in my rectangle must block the undo"
    );
    assert_eq!(report.ops_emitted, 0, "and emit nothing");
    assert_eq!(
        mine.state().style_at(target.0, target.1).fill,
        Some(fill(0xFF00_FF00)),
        "their change survived my undo"
    );
    assert_eq!(
        fills(mine.state())[0][0],
        Some(0xFFFF_FF00),
        "and so did mine, because the undo yielded rather than half-applied"
    );
}

/// The *earlier* foreign write does not block: undo restores it, because
/// restoring what somebody else wrote before me is exactly what "put it back
/// the way it was" means.
#[test]
fn undo_restores_an_earlier_foreign_rule_rather_than_clearing_it() {
    let mut theirs = session_4x4(2);
    set_fill(&mut theirs, RectSpec::column(0), 0xFF00_FF00);

    let mut mine = Session::from_log(ActorId(1), theirs.log.clone());
    set_fill(&mut mine, RectSpec::column(0), 0xFFFF_FF00);
    assert_eq!(fills(mine.state())[0][0], Some(0xFFFF_FF00));

    let report = mine.apply(Command::Undo).expect("undo");
    assert_eq!(
        report.blocked, 0,
        "their rule is older, so it does not block"
    );
    assert_eq!(
        fills(mine.state())[0][0],
        Some(0xFF00_FF00),
        "undo restored THEIR formatting, not the workbook default"
    );
}

/// Facet independence again, this time in undo: undoing a fill must not touch a
/// font somebody set on the same cell. A blob-per-cell model could not do this
/// at all.
#[test]
fn undoing_one_facet_leaves_the_other_facets_alone() {
    let mut s = session_4x4(1);
    s.apply(Command::SetStyle {
        target: RectSpec::cell(0, 0),
        facet: bold(),
    })
    .expect("font");
    set_fill(&mut s, RectSpec::cell(0, 0), 0xFFFF_FF00);

    s.apply(Command::Undo).expect("undo the fill");
    let (r, c) = at(s.state(), 0, 0);
    let style = s.state().style_at(r, c);
    assert_eq!(style.fill, None, "the fill was undone");
    assert_eq!(style.font, Some(bold()), "the font was not");
}

/// A clear is undoable like any other write: undoing it brings the formatting
/// back.
#[test]
fn undoing_a_clear_restores_the_formatting_it_removed() {
    let mut s = session_4x4(1);
    s.apply(Command::SetStyle {
        target: RectSpec::cell(0, 0),
        facet: StyleFacet::NumberFormat(String::from("0.00%")),
    })
    .expect("format");
    s.apply(Command::ClearStyle {
        target: RectSpec::cell(0, 0),
        facet_slot: NUMBER_FORMAT,
    })
    .expect("clear");

    let (r, c) = at(s.state(), 0, 0);
    assert_eq!(s.state().style_at(r, c).number_format_code(), None);

    s.apply(Command::Undo).expect("undo the clear");
    assert_eq!(
        s.state().style_at(r, c).number_format_code(),
        Some("0.00%"),
        "undoing a clear restores what it removed"
    );
}

/// Styling a cell does not disturb its value, and vice versa — the two are
/// separate registers and their undo stacks must not interfere.
#[test]
fn a_style_and_a_value_at_one_cell_are_independent() {
    let mut s = session_4x4(1);
    s.apply(Command::SetValue {
        row: 0,
        col: 0,
        value: Value::Number(42.0),
    })
    .expect("value");
    set_fill(&mut s, RectSpec::cell(0, 0), 0xFFFF_FF00);

    let (r, c) = at(s.state(), 0, 0);
    assert_eq!(s.state().cell(r, c), Some(Value::Number(42.0)));

    s.apply(Command::Undo).expect("undo the style");
    assert_eq!(
        s.state().cell(r, c),
        Some(Value::Number(42.0)),
        "undoing a style must not touch the value"
    );
    assert_eq!(s.state().style_at(r, c).fill, None);

    s.apply(Command::Undo).expect("undo the value");
    assert_eq!(s.state().cell(r, c), Some(Value::Blank));
}
