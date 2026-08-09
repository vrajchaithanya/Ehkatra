//! View-model proofs (ADR-021, ADR-022, docs/31 §15.2).
//!
//! The load-bearing one is `a_row_inserted_above_the_viewport_does_not_move_it`.
//! Everything else here is arithmetic; that one is the design.

use usk_types::{ActorId, ColId, OpId, RowId};
use usk_view::{Anchor, Axis, Metrics, Viewport};

fn id(n: u64) -> OpId {
    OpId {
        actor: ActorId(1),
        counter: n,
    }
}

/// `n` rows of uniform height, ids 1..=n in order.
fn uniform(n: u64, height: f32) -> Axis {
    let order: Vec<OpId> = (1..=n).map(id).collect();
    Axis::build(&order, |_| height)
}

#[test]
fn cumulative_extents_map_pixels_to_identities_and_back() {
    let mut metrics = Metrics::default();
    metrics.set_row_height(RowId(id(2)), 50.0);
    let order: Vec<OpId> = (1..=4).map(id).collect();
    let axis = Axis::build(&order, |o| metrics.row_height(RowId(o)));

    // 20, 50, 20, 20 -> starts 0, 20, 70, 90, total 110.
    assert_eq!(axis.extent(), 110.0);
    assert_eq!(axis.pixel_of(id(1)), Some(0.0));
    assert_eq!(axis.pixel_of(id(2)), Some(20.0));
    assert_eq!(axis.pixel_of(id(3)), Some(70.0));
    assert_eq!(axis.size_at(1), 50.0);

    // A pixel inside a span resolves to that span, and the boundary belongs to
    // the row that starts there.
    assert_eq!(axis.index_at_pixel(0.0), Some(0));
    assert_eq!(axis.index_at_pixel(19.9), Some(0));
    assert_eq!(axis.index_at_pixel(20.0), Some(1));
    assert_eq!(axis.index_at_pixel(69.9), Some(1));
    assert_eq!(axis.index_at_pixel(70.0), Some(2));
    // Outside clamps rather than failing: a scrollbar drag past the end is an
    // ordinary gesture, not an error.
    assert_eq!(axis.index_at_pixel(-100.0), Some(0));
    assert_eq!(axis.index_at_pixel(1e9), Some(3));

    // Round trip over every row.
    for i in 0..axis.len() {
        let at = axis.pixel_of(axis.id_at(i).unwrap()).unwrap();
        assert_eq!(axis.index_at_pixel(at), Some(i));
    }
}

/// **ADR-022.** A collaborator inserting a row above the viewport must not
/// move what the user is looking at.
///
/// This is the test the whole identity-anchored design exists for. An
/// ordinal-based viewport — "scrolled 2,000 px down" — shows different content
/// the instant a row appears above, silently and mid-keystroke. An
/// identity-anchored one shows the same row forever.
#[test]
fn a_row_inserted_above_the_viewport_does_not_move_it() {
    let before = uniform(1_000, 20.0);
    let mut view = Viewport::new(300.0, 200.0);
    view.scroll_by(&before, &Axis::default(), 0.0, 2_000.0);

    let anchored_to = view.rows.id.expect("scrolling anchors to a row");
    let top_before = view.visible(&before, &Axis::default()).row_ids();
    assert_eq!(top_before[0], RowId(anchored_to));

    // Someone inserts 5 rows at the very top. Every ordinal below shifts by 5,
    // and the anchored row is now 100 px further down the sheet.
    let mut order: Vec<OpId> = (2_001..=2_005).map(id).collect();
    order.extend((1..=1_000).map(id));
    let after = Axis::build(&order, |_| 20.0);
    assert_eq!(
        after.pixel_of(anchored_to).unwrap(),
        before.pixel_of(anchored_to).unwrap() + 100.0,
        "the fixture must actually move the row, or this test proves nothing"
    );

    view.reanchor(&after, &Axis::default(), &before, &Axis::default());

    assert_eq!(
        view.rows.id,
        Some(anchored_to),
        "the viewport must still be anchored to the same row"
    );
    assert_eq!(
        view.visible(&after, &Axis::default()).row_ids(),
        top_before,
        "and must still be showing exactly the same rows"
    );
}

/// The other half of ADR-022: when the anchor *itself* is deleted there is no
/// identity to keep, so the viewport falls back to the pixel position it had
/// rather than jumping to the top of the sheet.
#[test]
fn deleting_the_anchored_row_lands_where_the_user_was_looking() {
    let before = uniform(1_000, 20.0);
    let mut view = Viewport::new(300.0, 200.0);
    view.scroll_by(&before, &Axis::default(), 0.0, 2_000.0);
    let anchored_to = view.rows.id.unwrap();
    let was_at = before.pixel_of(anchored_to).unwrap() + view.rows.offset;

    // That row is deleted; everything else keeps its identity.
    let order: Vec<OpId> = (1..=1_000).map(id).filter(|o| *o != anchored_to).collect();
    let after = Axis::build(&order, |_| 20.0);

    view.reanchor(&after, &Axis::default(), &before, &Axis::default());
    let now = view.rows.id.expect("re-anchored to something live");
    assert!(after.index_of(now).is_some(), "and it must be live");

    let now_at = after.pixel_of(now).unwrap() + view.rows.offset;
    assert!(
        (now_at - was_at).abs() <= 20.0,
        "the viewport moved {} px when one row of 20 px was deleted",
        (now_at - was_at).abs()
    );
}

/// Virtual scrolling: the work is a function of the window, not the document.
#[test]
fn only_the_rows_on_screen_are_produced() {
    let rows = uniform(1_000_000, 20.0);
    let cols = Axis::build(&(1..=16_384).map(id).collect::<Vec<_>>(), |_| 64.0);
    let mut view = Viewport::new(1_280.0, 800.0);
    view.scroll_by(&rows, &cols, 100_000.0, 9_000_000.0);

    let visible = view.visible(&rows, &cols);
    // 800 px of 20 px rows is 40, plus at most one partly-scrolled row.
    assert!(
        visible.rows.len() <= 41,
        "produced {} rows for an 800 px viewport",
        visible.rows.len()
    );
    assert!(
        visible.cols.len() <= 21,
        "produced {} cols for a 1280 px viewport",
        visible.cols.len()
    );
    assert!(!visible.rows.is_empty() && !visible.cols.is_empty());

    // The slots tile the viewport contiguously, starting at or before its edge.
    assert!(visible.rows[0].at <= 0.0);
    for pair in visible.rows.windows(2) {
        assert_eq!(pair[1].at, pair[0].at + pair[0].size);
    }
    let last = visible.rows.last().unwrap();
    assert!(
        last.at < view.height && last.at + last.size >= view.height - 20.0,
        "the visible run must reach the bottom edge"
    );
}

#[test]
fn scrolling_clamps_at_both_ends() {
    let rows = uniform(100, 20.0);
    let cols = Axis::default();
    let mut view = Viewport::new(300.0, 200.0);

    view.scroll_by(&rows, &cols, 0.0, -5_000.0);
    assert_eq!(view.rows.id, Some(id(1)));
    assert_eq!(view.rows.offset, 0.0, "cannot scroll above the first row");

    view.scroll_by(&rows, &cols, 0.0, 1e9);
    let visible = view.visible(&rows, &cols);
    let last = visible.rows.last().unwrap();
    assert_eq!(
        last.id,
        id(100),
        "scrolling to the end must show the last row"
    );
    assert!(
        last.at + last.size <= view.height + 0.001,
        "and must not scroll it past the bottom edge"
    );
}

/// A sheet shorter than the window pins to the top instead of floating.
#[test]
fn a_sheet_shorter_than_the_viewport_does_not_scroll() {
    let rows = uniform(3, 20.0);
    let cols = Axis::default();
    let mut view = Viewport::new(300.0, 800.0);
    view.scroll_by(&rows, &cols, 0.0, 500.0);
    assert_eq!(view.rows.id, Some(id(1)));
    assert_eq!(view.rows.offset, 0.0);
    assert_eq!(view.visible(&rows, &cols).rows.len(), 3);
}

#[test]
fn an_empty_axis_produces_nothing_rather_than_panicking() {
    let empty = Axis::default();
    let mut view = Viewport::new(300.0, 200.0);
    view.scroll_by(&empty, &empty, 10.0, 10.0);
    let visible = view.visible(&empty, &empty);
    assert!(visible.rows.is_empty() && visible.cols.is_empty());
    assert_eq!(view.rows, Anchor::default());
}

/// Sizes are keyed by identity, so a resized row keeps its height when rows
/// are inserted above it — the same argument as the scroll anchor.
#[test]
fn a_resized_row_keeps_its_height_across_a_structural_edit() {
    let mut metrics = Metrics::default();
    metrics.set_row_height(RowId(id(7)), 80.0);
    metrics.set_col_width(ColId(id(3)), 200.0);

    let before = Axis::build(&(1..=10).map(id).collect::<Vec<_>>(), |o| {
        metrics.row_height(RowId(o))
    });
    let mut order: Vec<OpId> = vec![id(99)];
    order.extend((1..=10).map(id));
    let after = Axis::build(&order, |o| metrics.row_height(RowId(o)));

    assert_eq!(before.size_at(before.index_of(id(7)).unwrap()), 80.0);
    assert_eq!(after.size_at(after.index_of(id(7)).unwrap()), 80.0);
    assert_eq!(metrics.col_width(ColId(id(3))), 200.0);
    assert_eq!(metrics.col_width(ColId(id(4))), metrics.default_col_width);
}

/// A1 labels: bijective base-26, which is the one place spreadsheet column
/// naming surprises people — column 26 is `AA`, not `BA` or `AZ`.
#[test]
fn column_labels_are_bijective_base_26() {
    use usk_view::{column_label, row_label};
    for (index, expected) in [
        (0, "A"),
        (25, "Z"),
        (26, "AA"),
        (27, "AB"),
        (51, "AZ"),
        (52, "BA"),
        (701, "ZZ"),
        (702, "AAA"),
        (16_383, "XFD"), // Excel's last column
    ] {
        assert_eq!(column_label(index), expected, "column {index}");
    }
    assert_eq!(row_label(0), "1");
    assert_eq!(row_label(1_048_575), "1048576"); // Excel's last row
}

/// The ordinal a slot carries must be the one the axis would report, or the
/// header prints a label for a different row than the one beside it.
#[test]
fn a_visible_slot_knows_its_own_ordinal() {
    let rows = uniform(1_000, 20.0);
    let cols = Axis::default();
    let mut view = Viewport::new(300.0, 200.0);
    view.scroll_by(&rows, &cols, 0.0, 4_000.0);
    for slot in &view.visible(&rows, &cols).rows {
        assert_eq!(rows.index_of(slot.id), Some(slot.index));
    }
}
