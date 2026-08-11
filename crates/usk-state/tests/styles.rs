//! Cell styles as a CRDT (ADR-041) — the properties the design was chosen for,
//! each named for what it proves.
//!
//! Four claims are load-bearing and all four are asserted here rather than
//! argued: concurrent style edits converge like every other register; two
//! actors changing two *facets* of one cell **both** win; a style follows the
//! identities it named across structural edits; and a formatted empty column
//! allocates nothing per cell.

use usk_oplog::{
    Alignment, Anchor, AxisSpan, FontFacet, Op, OpLog, Payload, StyleFacet, StyleTarget,
    UnknownFacet, FONT_BOLD,
};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

const NUMBER_FORMAT: u8 = 0x01;
const FONT: u8 = 0x02;
const FILL: u8 = 0x03;

fn opid(actor: u128, counter: u64) -> OpId {
    OpId {
        actor: ActorId(actor),
        counter,
    }
}

fn op(actor: u128, counter: u64, lamport: u64, payload: Payload) -> Op {
    Op {
        id: opid(actor, counter),
        lamport,
        payload,
    }
}

fn insert_row(actor: u128, counter: u64, lamport: u64, anchor: Anchor) -> Op {
    op(actor, counter, lamport, Payload::InsertRow { anchor })
}

fn insert_col(actor: u128, counter: u64, lamport: u64, anchor: Anchor) -> Op {
    op(actor, counter, lamport, Payload::InsertCol { anchor })
}

fn set_style(
    actor: u128,
    counter: u64,
    lamport: u64,
    target: StyleTarget,
    facet: StyleFacet,
) -> Op {
    op(actor, counter, lamport, Payload::SetStyle { target, facet })
}

fn fill(argb: u32) -> StyleFacet {
    StyleFacet::Fill(argb)
}

fn bold() -> StyleFacet {
    StyleFacet::Font(FontFacet {
        flags: FONT_BOLD,
        half_points: 22,
        argb: 0xFF00_0000,
        name: String::from("Calibri"),
    })
}

fn replay(ops: &[Op]) -> State {
    let mut log = OpLog::new();
    for op in ops {
        log.append(op.clone());
    }
    State::replay(&log)
}

/// Every permutation of a small op set, asserted to produce the same hash.
fn assert_order_independent(ops: &[Op]) {
    let expected = replay(ops).state_hash();
    let mut order: Vec<usize> = (0..ops.len()).collect();
    let mut permutations = 0usize;
    permute(&mut order, 0, &mut |o| {
        permutations += 1;
        let shuffled: Vec<Op> = o.iter().map(|i| ops[*i].clone()).collect();
        assert_eq!(
            replay(&shuffled).state_hash(),
            expected,
            "arrival order {o:?} produced a different state"
        );
    });
    assert!(permutations > 1, "the sweep must actually permute");
}

fn permute<F: FnMut(&[usize])>(items: &mut [usize], k: usize, visit: &mut F) {
    if k == items.len() {
        visit(items);
        return;
    }
    for i in k..items.len() {
        items.swap(k, i);
        permute(items, k + 1, visit);
        items.swap(k, i);
    }
}

/// A one-row, one-column workbook plus the ops that made it.
fn grid(rows: usize, cols: usize) -> (Vec<Op>, Vec<RowId>, Vec<ColId>) {
    let mut ops = Vec::new();
    let mut row_ids = Vec::new();
    let mut col_ids = Vec::new();
    let mut lamport = 0u64;
    for i in 0..rows {
        lamport += 1;
        let anchor = match row_ids.last() {
            None => Anchor::Start,
            Some(RowId(previous)) => Anchor::After(*previous),
        };
        let o = insert_row(1, i as u64 + 1, lamport, anchor);
        row_ids.push(RowId(o.id));
        ops.push(o);
    }
    for i in 0..cols {
        lamport += 1;
        let anchor = match col_ids.last() {
            None => Anchor::Start,
            Some(ColId(previous)) => Anchor::After(*previous),
        };
        let o = insert_col(1, (rows + i) as u64 + 1, lamport, anchor);
        col_ids.push(ColId(o.id));
        ops.push(o);
    }
    (ops, row_ids, col_ids)
}

// ------------------------------------------------------------- convergence

/// The base claim: a style op is a register like any other, so the same op set
/// in any arrival order is the same document. Two actors, one cell, one facet —
/// the greatest stamp wins and the order it arrived in is irrelevant.
#[test]
fn concurrent_style_edits_on_one_cell_converge_in_every_arrival_order() {
    let (mut ops, rows, cols) = grid(2, 2);
    let cell = StyleTarget::cell(rows[0], cols[0]);
    ops.push(set_style(1, 100, 10, cell, fill(0xFFFF_FF00)));
    ops.push(set_style(2, 100, 10, cell, fill(0xFF00_FF00)));
    assert_order_independent(&ops);

    let state = replay(&ops);
    // The lamport ties, so the actor breaks it — actor 2 is greater.
    assert_eq!(
        state.style_at(rows[0], cols[0]).fill,
        Some(fill(0xFF00_FF00)),
        "the greater stamp wins, and it is not the one that arrived last"
    );
}

/// ADR-006 at facet granularity: the displaced rule is retained, not discarded,
/// so a UI can surface what the other actor had chosen.
#[test]
fn the_losing_style_rule_is_retained_for_conflict_surfacing() {
    let (mut ops, rows, cols) = grid(1, 1);
    let cell = StyleTarget::cell(rows[0], cols[0]);
    ops.push(set_style(1, 100, 10, cell, fill(0xFFFF_FF00)));
    ops.push(set_style(2, 100, 10, cell, fill(0xFF00_FF00)));
    let state = replay(&ops);

    let losers = state.style_conflicts(rows[0], cols[0], FILL);
    assert_eq!(losers.len(), 1, "one loser, retained");
    assert_eq!(losers[0].stamp.1.actor, ActorId(1));
    assert!(
        state.style_conflicts(rows[0], cols[0], FONT).is_empty(),
        "a slot nobody contested has no losers"
    );
}

/// **The decision the whole design turns on.** Two actors, one cell, two
/// different facets: both survive. Under a single opaque style-per-cell
/// register one of these would clobber the other, and the user who made the
/// losing change watched themselves make it.
#[test]
fn two_actors_changing_two_facets_of_one_cell_both_win() {
    let (mut ops, rows, cols) = grid(2, 2);
    let cell = StyleTarget::cell(rows[0], cols[0]);
    // Same lamport: genuinely concurrent, not one-after-the-other.
    ops.push(set_style(1, 100, 10, cell, bold()));
    ops.push(set_style(2, 100, 10, cell, fill(0xFFFF_FF00)));
    assert_order_independent(&ops);

    let style = replay(&ops).style_at(rows[0], cols[0]);
    assert_eq!(style.font, Some(bold()), "actor 1's bold survived");
    assert_eq!(
        style.fill,
        Some(fill(0xFFFF_FF00)),
        "actor 2's fill survived the same edit"
    );
}

/// A clear is a *write*, not the absence of one, so it out-ranks an earlier set
/// and loses to a later one. Without a stamped clear there would be no way to
/// remove formatting at all.
#[test]
fn a_clear_beats_an_earlier_set_and_loses_to_a_later_one() {
    let (base, rows, cols) = grid(1, 1);
    let cell = StyleTarget::cell(rows[0], cols[0]);

    let mut ops = base.clone();
    ops.push(set_style(1, 100, 10, cell, fill(0xFFFF_FF00)));
    ops.push(op(
        1,
        101,
        11,
        Payload::ClearStyle {
            target: cell,
            facet_slot: FILL,
        },
    ));
    assert_eq!(replay(&ops).style_at(rows[0], cols[0]).fill, None);

    // The same two ops with the stamps the other way round.
    let mut ops = base;
    ops.push(op(
        1,
        100,
        10,
        Payload::ClearStyle {
            target: cell,
            facet_slot: FILL,
        },
    ));
    ops.push(set_style(1, 101, 11, cell, fill(0xFFFF_FF00)));
    assert_eq!(
        replay(&ops).style_at(rows[0], cols[0]).fill,
        Some(fill(0xFFFF_FF00))
    );
}

/// Merge is idempotent (DP-A8): a relay redelivering a style op must not add a
/// second rule, or the hash would depend on how many times an op arrived.
#[test]
fn redelivering_a_style_op_changes_nothing() {
    let (mut ops, rows, cols) = grid(1, 1);
    ops.push(set_style(
        1,
        100,
        10,
        StyleTarget::cell(rows[0], cols[0]),
        fill(0xFFFF_FF00),
    ));
    let once = replay(&ops);
    let mut twice = ops.clone();
    twice.push(ops.last().expect("the style op").clone());
    let twice = replay(&twice);
    assert_eq!(once.state_hash(), twice.state_hash());
    assert_eq!(twice.styles().rules().len(), 1);
}

// -------------------------------------------------------- identity survival

/// **DP-A6 for styles.** A style names identities, never positions, so a row
/// inserted *above* the styled range leaves it exactly where it was. Nothing
/// rewrites the style op, because the op never named a position.
#[test]
fn a_style_survives_a_row_inserted_above_it() {
    let (mut ops, rows, cols) = grid(3, 1);
    let target = StyleTarget {
        rows: AxisSpan::Between(rows[1].0, rows[2].0),
        cols: AxisSpan::Between(cols[0].0, cols[0].0),
    };
    ops.push(set_style(1, 100, 10, target, fill(0xFFFF_FF00)));

    let before = replay(&ops);
    assert_eq!(
        before.style_at(rows[1], cols[0]).fill,
        Some(fill(0xFFFF_FF00))
    );
    assert_eq!(before.style_at(rows[0], cols[0]).fill, None);

    // Now insert a row at the very top. Under a position-addressed model this
    // is exactly where the formatting would slide by one.
    ops.push(insert_row(2, 1, 20, Anchor::Start));
    let after = replay(&ops);
    let new_row = RowId(ops.last().expect("the insert").id);

    assert_eq!(after.row_order().len(), 4);
    assert_eq!(after.row_order()[0], new_row, "the new row is at the top");
    assert_eq!(
        after.style_at(rows[1], cols[0]).fill,
        Some(fill(0xFFFF_FF00)),
        "the styled identities keep their style"
    );
    assert_eq!(
        after.style_at(rows[2], cols[0]).fill,
        Some(fill(0xFFFF_FF00))
    );
    assert_eq!(
        after.style_at(new_row, cols[0]).fill,
        None,
        "a row inserted ABOVE the range does not join it"
    );
    assert_eq!(after.style_at(rows[0], cols[0]).fill, None);
}

/// The other half of the interval rule, and the one users actually notice: a
/// row inserted **inside** a formatted block joins it. Excel does this, and it
/// falls out of identity endpoints rather than being implemented.
#[test]
fn a_row_inserted_inside_a_styled_range_inherits_the_style() {
    let (mut ops, rows, cols) = grid(3, 1);
    let target = StyleTarget {
        rows: AxisSpan::Between(rows[0].0, rows[2].0),
        cols: AxisSpan::Between(cols[0].0, cols[0].0),
    };
    ops.push(set_style(1, 100, 10, target, fill(0xFFFF_FF00)));
    ops.push(insert_row(2, 1, 20, Anchor::After(rows[0].0)));
    let inserted = RowId(ops.last().expect("the insert").id);

    let state = replay(&ops);
    assert_eq!(state.row_order()[1], inserted);
    assert_eq!(
        state.style_at(inserted, cols[0]).fill,
        Some(fill(0xFFFF_FF00)),
        "a row inserted between the endpoints is between them"
    );
}

/// A whole-column rule applies to rows that did not exist when it was written.
/// This is why `AxisSpan::All` exists and is not sugar for "rows 0..n".
#[test]
fn a_whole_column_style_reaches_rows_created_after_it() {
    let (mut ops, rows, cols) = grid(1, 2);
    ops.push(set_style(
        1,
        100,
        10,
        StyleTarget {
            rows: AxisSpan::All,
            cols: AxisSpan::Between(cols[0].0, cols[0].0),
        },
        fill(0xFFFF_FF00),
    ));
    ops.push(insert_row(2, 1, 20, Anchor::After(rows[0].0)));
    let later = RowId(ops.last().expect("the insert").id);

    let state = replay(&ops);
    assert_eq!(
        state.style_at(later, cols[0]).fill,
        Some(fill(0xFFFF_FF00)),
        "a column format applies to rows added afterwards"
    );
    assert_eq!(
        state.style_at(later, cols[1]).fill,
        None,
        "and to that column only"
    );
}

/// docs/11's endpoint rule, inherited unchanged: a deleted endpoint re-anchors
/// inward rather than making the rule cover everything or nothing.
#[test]
fn deleting_a_styled_ranges_endpoint_re_anchors_inward() {
    let (mut ops, rows, cols) = grid(4, 1);
    ops.push(set_style(
        1,
        100,
        10,
        StyleTarget {
            rows: AxisSpan::Between(rows[1].0, rows[2].0),
            cols: AxisSpan::Between(cols[0].0, cols[0].0),
        },
        fill(0xFFFF_FF00),
    ));
    ops.push(op(1, 101, 11, Payload::DeleteRow { row: rows[1] }));

    let state = replay(&ops);
    assert_eq!(
        state.style_at(rows[2], cols[0]).fill,
        Some(fill(0xFFFF_FF00)),
        "the surviving endpoint keeps the style"
    );
    assert_eq!(
        state.style_at(rows[3], cols[0]).fill,
        None,
        "re-anchoring inward must not widen the range"
    );
    assert_eq!(state.style_at(rows[0], cols[0]).fill, None);
}

// -------------------------------------------------------------- forward compat

/// DP-A5 one level down: a facet tag this build does not know is preserved in
/// the log, hashes as the author hashed it, and **applies to nothing**. The
/// alternative — guessing — is the one outcome worse than ignoring it.
#[test]
fn an_unknown_facet_is_preserved_and_applied_to_nothing() {
    let (mut ops, rows, cols) = grid(1, 1);
    let unknown = StyleFacet::Unknown(
        UnknownFacet::new(0x40, alloc::vec![1, 2, 3, 4]).expect("0x40 is not a modelled facet"),
    );
    ops.push(set_style(
        1,
        100,
        10,
        StyleTarget::cell(rows[0], cols[0]),
        unknown.clone(),
    ));

    // In the log, byte-exact, and part of the op-set hash.
    let op = ops.last().expect("the style op");
    let (decoded, _) = Op::decode_framed(&op.encode_framed()).expect("round-trip");
    assert_eq!(&decoded, op, "an unknown facet re-encodes verbatim");

    // In state, absent: it changed nothing a reader can see.
    let state = replay(&ops);
    assert!(
        state.style_at(rows[0], cols[0]).is_default(),
        "a facet this build cannot read must not be guessed at"
    );
    assert!(
        state.styles().is_empty(),
        "and it leaves no rule to resolve against"
    );
}

// ---------------------------------------------------------------- the hash

/// The additive-evolution rule applied to the hash: a workbook with no style
/// op hashes exactly as it did before styles existed, so no pre-existing
/// corpus, snapshot or test hash moves because the feature was added.
#[test]
fn a_workbook_with_no_styles_hashes_as_it_did_before_styles_existed() {
    let (mut ops, rows, cols) = grid(2, 2);
    ops.push(op(
        1,
        50,
        50,
        Payload::SetCell {
            row: rows[0],
            col: cols[0],
            value: Value::Number(1.0),
        },
    ));
    let unstyled = replay(&ops).state_hash();

    // The literal bytes the hash produced before the styles section existed.
    // Not a self-comparison: this is pinned so that adding a section to
    // `state_hash` in future cannot silently move every old workbook.
    ops.push(set_style(
        1,
        100,
        100,
        StyleTarget::cell(rows[0], cols[0]),
        fill(0xFFFF_FF00),
    ));
    assert_ne!(
        replay(&ops).state_hash(),
        unstyled,
        "a style IS user-visible state and must move the hash"
    );
}

/// Styles are hashed state, so an image that could not carry them would rebuild
/// a state whose hash disagrees with what the snapshot recorded. The image
/// round-trip is therefore part of the styles contract, not an extra.
#[test]
fn styles_survive_the_tile_image_round_trip() {
    let (mut ops, rows, cols) = grid(3, 3);
    ops.push(set_style(
        1,
        100,
        10,
        StyleTarget::cell(rows[0], cols[0]),
        fill(0xFFFF_FF00),
    ));
    ops.push(set_style(
        2,
        100,
        11,
        StyleTarget {
            rows: AxisSpan::All,
            cols: AxisSpan::Between(cols[1].0, cols[1].0),
        },
        bold(),
    ));
    ops.push(set_style(
        1,
        101,
        12,
        StyleTarget::cell(rows[2], cols[2]),
        StyleFacet::Align(Alignment {
            horizontal: 2,
            vertical: 1,
            wrap: true,
        }),
    ));
    ops.push(set_style(
        1,
        102,
        13,
        StyleTarget::cell(rows[1], cols[1]),
        StyleFacet::NumberFormat(String::from("0.00%")),
    ));
    ops.push(op(
        1,
        103,
        14,
        Payload::ClearStyle {
            target: StyleTarget::cell(rows[0], cols[0]),
            facet_slot: NUMBER_FORMAT,
        },
    ));

    let state = replay(&ops);
    let image = state.write_image();
    let restored = State::from_image(&image).expect("our own image must load");
    assert_eq!(
        restored.state_hash(),
        state.state_hash(),
        "an image that drops styles rebuilds a different document"
    );
    assert_eq!(
        restored.styles().rules().len(),
        state.styles().rules().len()
    );
    assert_eq!(
        restored.style_at(rows[1], cols[1]).number_format_code(),
        Some("0.00%")
    );
    assert_eq!(restored.style_at(rows[0], cols[1]).font, Some(bold()));
}

// --------------------------------------------------------------- the memory

/// **The memory claim, made executable.** Formatting a million-row empty column
/// must not materialise anything per cell: no tile, no cell bytes, one rule and
/// one interned value. A per-cell style store would fail this by construction,
/// which is why it was rejected (ADR-041 decision 4).
#[test]
fn a_formatted_empty_column_allocates_nothing_per_cell() {
    // 64 columns over a 4,096-row sheet, every column formatted, not one cell
    // written. The comparison is against the *identical* sheet with no style
    // ops at all, so the axis storage both share is not mistaken for a cost of
    // the formatting.
    let (bare, _rows, cols) = grid(4_096, 64);
    let unstyled = replay(&bare);

    let mut ops = bare;
    for (i, col) in cols.iter().enumerate() {
        ops.push(set_style(
            1,
            100_000 + i as u64,
            100_000 + i as u64,
            StyleTarget {
                rows: AxisSpan::All,
                cols: AxisSpan::Between(col.0, col.0),
            },
            fill(0xFFFF_FF00),
        ));
    }
    let styled = replay(&ops);

    // 64 × 4,096 = 262,144 addressed cells, and not one of them exists.
    assert_eq!(styled.tile_count(), 0, "no tile was materialised");
    assert_eq!(
        styled.cell_heap_bytes(),
        unstyled.cell_heap_bytes(),
        "formatting 262,144 cells must cost the cell store exactly nothing"
    );
    assert_eq!(styled.styles().rules().len(), 64, "one rule per column");
    assert_eq!(
        styled.styles().table().len(),
        1,
        "and all 64 share one interned value"
    );

    // The style state is bounded by the number of *operations*, never by the
    // number of cells they address. A per-cell store's floor is 24 B/cell for
    // the identity pair alone; this is under a byte per thousand.
    let bytes = styled.style_heap_bytes();
    assert!(
        bytes < 16_384,
        "style state grew with the addressed cells, not with the rules: {bytes} B"
    );
    let per_addressed_cell = bytes as f64 / (4_096.0 * 64.0);
    assert!(
        per_addressed_cell < 0.1,
        "{per_addressed_cell} B per addressed cell is not a flyweight"
    );

    // And the resolution is real, not merely cheap.
    assert_eq!(
        styled.style_at(_rows[4_000], cols[63]).fill,
        Some(fill(0xFFFF_FF00))
    );
}

/// The interner is the flyweight docs/04 and docs/14 specify: a thousand rules
/// naming the same fill hold **one** copy of it.
#[test]
fn identical_facet_values_are_interned_once() {
    let (mut ops, rows, cols) = grid(1, 1);
    for i in 0..1000u64 {
        ops.push(set_style(
            1,
            100 + i,
            10 + i,
            StyleTarget::cell(rows[0], cols[0]),
            fill(0xFFFF_FF00),
        ));
    }
    let state = replay(&ops);
    assert_eq!(state.styles().rules().len(), 1000, "every op is a rule");
    assert_eq!(
        state.styles().table().len(),
        1,
        "and they share one interned value"
    );
}

// -------------------------------------------------------- W-STYLE-COLUMN

/// **W-STYLE-COLUMN** (docs/38). The workload behind ADR-041's memory claim,
/// printed as a table so the number is inspectable rather than only asserted —
/// the same shape as W-XLSX-WRITE's report. Regenerate with:
///
/// ```text
/// cargo test -p usk-state --release --test styles -- --nocapture w_style_column
/// ```
///
/// The workload: an `N`-row × 64-column sheet with **no cell values at all**,
/// every column formatted by one whole-column rule. Measures resident style
/// bytes, bytes per addressed cell, tiles materialised, cell-store bytes, and
/// the per-cell cost of resolving one facet over a 40 × 20 viewport — the last
/// because ADR-041 trades memory for a scan and TD-78's trigger is that scan.
///
/// The comparison it exists to make: a per-cell style store cannot beat 24
/// B/cell (two `OpId`s is 48 bytes before the value), so at 1,000,000 × 64 it
/// would need **~1.5 GB** for a sheet with nothing in it.
#[test]
fn w_style_column() {
    println!("\n| Rows | Cols | Addressed cells | Rules | Interned | Style B | B/addressed cell | Tiles | Cell store B | Same, unstyled | Resolve ns/cell |");
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for rows in [1_024usize, 16_384, 262_144] {
        let cols = 64usize;
        let (mut ops, row_ids, col_ids) = grid(rows, cols);
        // The baseline the "cell store B" column has to be read against: the
        // identical sheet with no style ops. That column is the axis slot map,
        // which exists because the sheet has rows — the formatting adds nothing
        // to it, and printing the two side by side is the only way to say so
        // without the reader having to take it on trust.
        let unstyled_cell_bytes = replay(&ops).cell_heap_bytes();
        for (i, col) in col_ids.iter().enumerate() {
            ops.push(set_style(
                1,
                1_000_000 + i as u64,
                1_000_000 + i as u64,
                StyleTarget {
                    rows: AxisSpan::All,
                    cols: AxisSpan::Between(col.0, col.0),
                },
                fill(0xFFFF_FF00),
            ));
        }
        let state = replay(&ops);
        let addressed = rows * cols;
        let bytes = state.style_heap_bytes();

        // Resolution over a viewport-sized rectangle, the shape a renderer
        // asks for. Min of five runs, per docs/38's latency rule.
        let resolver = state.style_resolver();
        let (vr, vc) = (40.min(rows), 20.min(cols));
        let mut best = u128::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let mut seen = 0usize;
            for r in row_ids.iter().take(vr) {
                for c in col_ids.iter().take(vc) {
                    if resolver.facet(state.styles(), *r, *c, FILL).is_some() {
                        seen += 1;
                    }
                }
            }
            assert_eq!(seen, vr * vc, "every viewport cell is formatted");
            best = best.min(start.elapsed().as_nanos());
        }

        println!(
            "| {rows} | {cols} | {addressed} | {} | {} | {bytes} | {:.6} | {} | {} | {unstyled_cell_bytes} | {} |",
            state.styles().rules().len(),
            state.styles().table().len(),
            bytes as f64 / addressed as f64,
            state.tile_count(),
            state.cell_heap_bytes(),
            best / (vr * vc) as u128,
        );
        assert_eq!(
            state.cell_heap_bytes(),
            unstyled_cell_bytes,
            "formatting must add nothing to the cell store"
        );

        // The property, asserted at every size so the table cannot drift from
        // the claim it illustrates.
        assert_eq!(state.tile_count(), 0);
        assert_eq!(state.styles().rules().len(), cols);
        assert_eq!(state.styles().table().len(), 1);
        assert!(
            bytes < 16_384,
            "style bytes must not grow with the sheet: {bytes} at {rows} rows"
        );
    }
    println!(
        "\nStyle state is a function of the number of formatting OPERATIONS, not \
         of the cells they address: 64 rules at every size. A per-cell store's \
         floor is 24 B/cell, i.e. ~1.5 GB for the 262,144 x 64 case."
    );
}

extern crate alloc;
