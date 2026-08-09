//! The **view model**: identity-anchored scroll and virtual scrolling
//! (ADR-021, ADR-022, docs/31 §15.2, docs/25).
//!
//! # Why this is a kernel crate and not part of the shell
//! None of this is graphics. It is the mapping between *pixels* and
//! *identities*, and identity is a kernel concept (DP-A6): a row is an `OpId`,
//! not an ordinal. Keeping it here means it is `no_std`, has no dependencies,
//! and — the part that matters — is **testable without a GPU**. The renderer
//! consumes what this produces and decides nothing.
//!
//! # The property this exists to guarantee (ADR-022)
//! > docs/31 §15.2: *Scroll position is `(anchor RowId/ColId, pixel offset)` —
//! > identity-based, so concurrent structural edits never teleport the
//! > viewport (ordinal-based scroll positions, as in every DOM-virtualized
//! > competitor, jump when rows are inserted above).*
//!
//! That is the whole design. A viewport that stored "scrolled 4,000 px down"
//! would show different content the instant a collaborator inserted a row
//! above — silently, mid-keystroke. A viewport that stores "the top row is
//! *this row*, offset 7 px" shows the same content forever, whatever happens
//! above it. `a_row_inserted_above_the_viewport_does_not_move_it` is that
//! sentence as a test.
//!
//! # Virtual scrolling
//! The grid is infinite (ADR-004); the renderer must never walk it. [`Axis`]
//! keeps cumulative pixel extents so pixel→identity is a binary search and
//! identity→pixel is a lookup, and [`Viewport::visible`] returns only the rows
//! and columns that actually intersect the viewport — a few dozen out of a
//! million.

#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_types::{ColId, OpId, RowId};

/// Row heights and column widths, in logical pixels.
///
/// Overrides are stored per identity rather than per ordinal for the same
/// reason the scroll anchor is: a resized row keeps its height when rows are
/// inserted above it.
#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub default_row_height: f32,
    pub default_col_width: f32,
    row_heights: BTreeMap<RowId, f32>,
    col_widths: BTreeMap<ColId, f32>,
}

impl Default for Metrics {
    /// Excel's defaults at 96 DPI, which is what "muscle-memory compatibility"
    /// means for something a user never consciously sees but immediately feels
    /// if it is wrong (docs/25).
    fn default() -> Self {
        Metrics {
            default_row_height: 20.0,
            default_col_width: 64.0,
            row_heights: BTreeMap::new(),
            col_widths: BTreeMap::new(),
        }
    }
}

impl Metrics {
    pub fn set_row_height(&mut self, row: RowId, height: f32) {
        self.row_heights.insert(row, height);
    }

    pub fn set_col_width(&mut self, col: ColId, width: f32) {
        self.col_widths.insert(col, width);
    }

    pub fn row_height(&self, row: RowId) -> f32 {
        *self
            .row_heights
            .get(&row)
            .unwrap_or(&self.default_row_height)
    }

    pub fn col_width(&self, col: ColId) -> f32 {
        *self.col_widths.get(&col).unwrap_or(&self.default_col_width)
    }
}

/// One axis of the grid, with cumulative pixel extents.
///
/// `starts[i]` is the pixel offset of `ids[i]`'s leading edge, so pixel→index
/// is a binary search and index→pixel is an index. docs/31 specifies an
/// **order-statistic tree** so that a structural edit costs O(log n) to
/// maintain; this rebuilds the prefix sums in O(n) instead. Queries already
/// meet the documented O(log n); it is *maintenance* that does not, which is
/// filed as TD-58 rather than left implied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Axis {
    ids: Vec<OpId>,
    /// `starts.len() == ids.len() + 1`; the last entry is the total extent, so
    /// every span is `starts[i + 1] - starts[i]` with no special case at the
    /// end.
    starts: Vec<f32>,
}

impl Axis {
    /// Builds an axis from the live order and a size function.
    pub fn build<F: Fn(OpId) -> f32>(order: &[OpId], size_of: F) -> Axis {
        let mut starts = Vec::with_capacity(order.len() + 1);
        let mut at = 0.0f32;
        for id in order {
            starts.push(at);
            at += size_of(*id);
        }
        starts.push(at);
        Axis {
            ids: order.to_vec(),
            starts,
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Total pixel extent of the axis.
    pub fn extent(&self) -> f32 {
        *self.starts.last().unwrap_or(&0.0)
    }

    pub fn id_at(&self, index: usize) -> Option<OpId> {
        self.ids.get(index).copied()
    }

    /// The ordinal of an identity, or `None` if it is not live.
    ///
    /// Linear because the axis order is not sorted by `OpId` — identities are
    /// created in edit order, not in position order (DP-A6). The renderer
    /// calls this once per frame for the anchor, not per cell.
    pub fn index_of(&self, id: OpId) -> Option<usize> {
        self.ids.iter().position(|x| *x == id)
    }

    /// The leading-edge pixel offset of an identity.
    pub fn pixel_of(&self, id: OpId) -> Option<f32> {
        self.index_of(id).map(|i| self.starts[i])
    }

    pub fn size_at(&self, index: usize) -> f32 {
        if index + 1 < self.starts.len() {
            self.starts[index + 1] - self.starts[index]
        } else {
            0.0
        }
    }

    /// The index whose span contains `pixel`, clamped into the axis.
    ///
    /// Binary search over the prefix sums — O(log n) over a million rows,
    /// which is what makes a scrollbar drag to the far end cost nothing.
    pub fn index_at_pixel(&self, pixel: f32) -> Option<usize> {
        if self.ids.is_empty() {
            return None;
        }
        if pixel <= 0.0 {
            return Some(0);
        }
        if pixel >= self.extent() {
            return Some(self.ids.len() - 1);
        }
        // Largest i with starts[i] <= pixel.
        let (mut lo, mut hi) = (0usize, self.ids.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.starts[mid] <= pixel {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        Some(lo)
    }
}

/// Where the viewport is, expressed as an **identity plus an offset**.
///
/// `anchor` is the first row/column at least partly visible; `offset` is how
/// far *into* it the top/left edge sits, so a half-scrolled row is
/// representable. `None` means the axis is empty or the anchor was deleted;
/// [`Viewport::reanchor`] resolves that against the current order rather than
/// leaving a dangling identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub id: Option<OpId>,
    pub offset: f32,
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor {
            id: None,
            offset: 0.0,
        }
    }
}

/// A rectangular view onto the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub rows: Anchor,
    pub cols: Anchor,
}

impl Viewport {
    pub fn new(width: f32, height: f32) -> Viewport {
        Viewport {
            width,
            height,
            rows: Anchor::default(),
            cols: Anchor::default(),
        }
    }

    /// The rows and columns that intersect the viewport — **only** those.
    ///
    /// This is the virtual scroll. A million-row sheet produces the few dozen
    /// entries that are actually on screen, so the renderer's cost is a
    /// function of the window and not of the document (docs/31: *the render
    /// loop never walks the document*).
    pub fn visible(&self, rows: &Axis, cols: &Axis) -> Visible {
        Visible {
            rows: span(rows, &self.rows, self.height),
            cols: span(cols, &self.cols, self.width),
        }
    }

    /// Scrolls by a pixel delta, re-expressing the position as a new anchor.
    ///
    /// Converting to pixels and back on every scroll is deliberate: it keeps
    /// exactly one representation of "where we are" — the anchor — so there is
    /// no second, drifting pixel counter to reconcile with it.
    pub fn scroll_by(&mut self, rows: &Axis, cols: &Axis, dx: f32, dy: f32) {
        self.rows = scrolled(rows, &self.rows, dy, self.height);
        self.cols = scrolled(cols, &self.cols, dx, self.width);
    }

    /// Re-resolves anchors against the current order.
    ///
    /// Called after a structural edit. An anchor whose identity is gone
    /// re-anchors to the nearest live identity **at the same pixel position**,
    /// which is what "the viewport does not teleport" means when the row it was
    /// anchored to is the one that got deleted.
    pub fn reanchor(
        &mut self,
        rows: &Axis,
        cols: &Axis,
        previous_rows: &Axis,
        previous_cols: &Axis,
    ) {
        self.rows = reanchored(rows, previous_rows, &self.rows);
        self.cols = reanchored(cols, previous_cols, &self.cols);
    }
}

/// One visible row or column: its identity, where it sits relative to the
/// viewport's top-left, and how big it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub id: OpId,
    /// Pixel offset from the viewport edge. Negative for the first entry when
    /// it is partly scrolled off, which is what makes smooth scrolling smooth.
    pub at: f32,
    pub size: f32,
    /// Position on the axis, 0-based.
    ///
    /// A *display* concern and nothing more: A1 notation is a view over
    /// identities (DP-A6), so the header needs an ordinal to print and the
    /// renderer would otherwise have to search the axis per row to find one.
    /// It is carried here rather than derived because the walk already knows
    /// it.
    pub index: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visible {
    pub rows: Vec<Slot>,
    pub cols: Vec<Slot>,
}

impl Visible {
    /// Row identities only, in view order.
    pub fn row_ids(&self) -> Vec<RowId> {
        self.rows.iter().map(|s| RowId(s.id)).collect()
    }

    pub fn col_ids(&self) -> Vec<ColId> {
        self.cols.iter().map(|s| ColId(s.id)).collect()
    }
}

fn span(axis: &Axis, anchor: &Anchor, extent: f32) -> Vec<Slot> {
    let mut out = Vec::new();
    let Some(start) = anchor_index(axis, anchor) else {
        return out;
    };
    let mut at = -anchor.offset;
    let mut i = start;
    while i < axis.len() && at < extent {
        let size = axis.size_at(i);
        out.push(Slot {
            id: axis.ids[i],
            at,
            size,
            index: i,
        });
        at += size;
        i += 1;
    }
    out
}

/// The index the anchor names, falling back to the top when it names nothing.
fn anchor_index(axis: &Axis, anchor: &Anchor) -> Option<usize> {
    if axis.is_empty() {
        return None;
    }
    match anchor.id {
        Some(id) => axis.index_of(id).or(Some(0)),
        None => Some(0),
    }
}

fn scrolled(axis: &Axis, anchor: &Anchor, delta: f32, extent: f32) -> Anchor {
    if axis.is_empty() {
        return Anchor::default();
    }
    let base = anchor_index(axis, anchor).map_or(0.0, |i| axis.starts[i]);
    let target = base + anchor.offset + delta;
    // Clamp so the last row cannot be scrolled past the bottom edge, and the
    // first cannot be scrolled above the top. A viewport taller than the sheet
    // pins to zero rather than going negative.
    let max = (axis.extent() - extent).max(0.0);
    let target = target.clamp(0.0, max);
    let index = axis.index_at_pixel(target).unwrap_or(0);
    Anchor {
        id: Some(axis.ids[index]),
        offset: target - axis.starts[index],
    }
}

fn reanchored(axis: &Axis, previous: &Axis, anchor: &Anchor) -> Anchor {
    if axis.is_empty() {
        return Anchor::default();
    }
    match anchor.id {
        // The anchor is still live: nothing to do, and *that is the point*.
        // Whatever happened above it, this identity is still the top row and
        // still `offset` pixels in, so the content under the cursor does not
        // move (ADR-022).
        Some(id) if axis.index_of(id).is_some() => *anchor,
        // The anchored row itself was deleted. Fall back to the pixel position
        // it *had*, so the viewport lands where the user was looking rather
        // than at the top of the sheet.
        Some(id) => {
            let pixel = previous.pixel_of(id).unwrap_or(0.0) + anchor.offset;
            let index = axis.index_at_pixel(pixel).unwrap_or(0);
            Anchor {
                id: Some(axis.ids[index]),
                offset: (pixel - axis.starts[index]).max(0.0),
            }
        }
        None => Anchor {
            id: Some(axis.ids[0]),
            offset: 0.0,
        },
    }
}

/// A column's A1 label: bijective base-26, so column 26 is `AA` and not `AZ`
/// or `BA` (DP-A6 — A1 is a *view*, and this is the whole of that view for a
/// header).
pub fn column_label(index: usize) -> alloc::string::String {
    let mut out = alloc::vec::Vec::new();
    let mut n = index + 1;
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    alloc::string::String::from_utf8(out).unwrap_or_default()
}

/// A row's A1 label: 1-based, which is the only thing spreadsheet users have
/// ever agreed on.
pub fn row_label(index: usize) -> alloc::string::String {
    let mut buf = alloc::string::String::new();
    let mut n = index + 1;
    let mut digits = alloc::vec::Vec::new();
    if n == 0 {
        digits.push(b'0');
    }
    while n > 0 {
        digits.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    digits.reverse();
    for d in digits {
        buf.push(d as char);
    }
    buf
}
