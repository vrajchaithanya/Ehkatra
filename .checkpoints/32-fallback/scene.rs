//! Turning a viewport into quads (ADR-021, docs/25 §the grid).
//!
//! The scene reads the **real** workbook — there is no fixture layer and no
//! placeholder data anywhere in the shell. Which cells are drawn comes from
//! `usk_view::Viewport::visible`, so this walks the window and never the
//! document (docs/31).
//!
//! # What a cell shows (TD-61, closed)
//! A cell's displayed value is `usk_calc::Engine::value`, not
//! `State::cell`. That distinction is the whole of TD-61: a cell holding only a
//! formula has **no value in the tile store**, because the value is what the
//! calculation engine produces. Reading `State` alone drew the formula
//! column's fills and nothing inside them.
//!
//! # Layers
//! Painter's order, one pass, one draw call:
//! fills → gridlines → headers → selection range → cell text → header labels →
//! active-cell border → in-cell editor → formula bar.
//!
//! The editor is drawn last among the grid layers because it is an overlay: it
//! occludes the cell beneath it and may spill into the cells to its right,
//! which is what Excel does and what makes a long formula readable while it is
//! being typed.

use usk_calc::Engine;
use usk_state::State;
use usk_types::{ColId, RowId, Value};
use usk_view::{column_label, row_label, Visible};

use crate::gpu::Quad;
use crate::text::{self, TextEngine};

/// Design tokens (docs/25 §Visual system: one token file, light and dark from
/// day one).
///
/// Written as **sRGB** — the space a designer picks colours in — and converted
/// to linear on the way to the GPU. The target is an sRGB format, so the
/// hardware encodes linear→sRGB on write; handing it sRGB values directly
/// encodes them twice and everything comes out washed out. That is exactly
/// what the first frame looked like.
pub struct Theme {
    pub gridline: [f32; 4],
    pub header_fill: [f32; 4],
    pub header_rule: [f32; 4],
    pub header_active: [f32; 4],
    pub formula_cell: [f32; 4],
    pub error_cell: [f32; 4],
    /// Range tint. Excel washes the selected range and leaves the active cell
    /// clear, so the cell you are about to type into is the one cell that is
    /// *not* tinted.
    pub selection: [f32; 4],
    pub selection_border: [f32; 4],
    pub editor_fill: [f32; 4],
    pub caret: [f32; 4],
    pub text: [f32; 4],
    pub header_text: [f32; 4],
    pub warning: [f32; 4],
    pub fill_preview: [f32; 4],
    pub bar_fill: [f32; 4],
    pub header_width: f32,
    pub header_height: f32,
    /// The formula bar (docs/25 core surface 2). Above the column headers,
    /// showing the active cell's A1 reference and its **source** — the formula
    /// text where the grid shows the computed value.
    pub bar_height: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            gridline: srgb(0.85, 0.86, 0.88, 1.0),
            header_fill: srgb(0.95, 0.95, 0.96, 1.0),
            header_rule: srgb(0.78, 0.80, 0.83, 1.0),
            header_active: srgb(0.80, 0.85, 0.93, 1.0),
            // A wash, not a fill. When the grid drew no text a saturated tint
            // was the only signal a cell had content; now the content is the
            // signal, and a spreadsheet whose every filled cell is blue is
            // below docs/25's bar. What survives is the information text cannot
            // carry: *this value was computed*, and *this value is an error*.
            formula_cell: srgb(0.93, 0.97, 0.94, 1.0),
            error_cell: srgb(0.99, 0.92, 0.91, 1.0),
            selection: srgb(0.13, 0.38, 0.80, 0.12),
            selection_border: srgb(0.10, 0.32, 0.72, 1.0),
            editor_fill: srgb(1.0, 1.0, 1.0, 1.0),
            caret: srgb(0.08, 0.09, 0.11, 1.0),
            text: srgb(0.10, 0.11, 0.13, 1.0),
            header_text: srgb(0.32, 0.35, 0.39, 1.0),
            warning: srgb(0.68, 0.24, 0.10, 1.0),
            fill_preview: srgb(0.35, 0.38, 0.44, 1.0),
            bar_fill: srgb(0.98, 0.98, 0.99, 1.0),
            header_width: 44.0,
            header_height: 22.0,
            bar_height: 26.0,
        }
    }
}

impl Theme {
    /// Top-left of the cell area, in logical pixels.
    pub fn grid_origin(&self) -> (f32, f32) {
        (self.header_width, self.bar_height + self.header_height)
    }

    /// The viewport's extent inside a window of `width` x `height` logical
    /// pixels — the window minus the chrome.
    pub fn viewport_size(&self, width: f32, height: f32) -> (f32, f32) {
        let (ox, oy) = self.grid_origin();
        ((width - ox).max(0.0), (height - oy).max(0.0))
    }
}

/// sRGB to linear, per the sRGB transfer function. Alpha is already linear.
fn srgb(r: f32, g: f32, b: f32, a: f32) -> [f32; 4] {
    fn c(v: f32) -> f32 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    [c(r), c(g), c(b), a]
}

/// What is selected, by identity for the active cell and by **ordinal
/// rectangle** for the range.
///
/// The split is deliberate. The active cell is a thing the model remembers
/// across structural edits, so it is an identity (DP-A6). A range is a
/// *rectangle in the current view* — "these forty cells" only means anything
/// against an order — so it is resolved to ordinals by the caller, which is
/// the layer that owns the axes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub active: Option<(RowId, ColId)>,
    /// Inclusive `(r0, r1, c0, c1)` in axis ordinals.
    pub range: Option<(usize, usize, usize, usize)>,
    /// What a released fill drag would cover. Outlined, not filled: it is a
    /// promise about what is *about* to happen, and drawing it like a selection
    /// would make the two indistinguishable at the moment it matters most.
    pub fill_preview: Option<(usize, usize, usize, usize)>,
    /// Where the fill handle sits, when the selection's corner is on screen.
    pub fill_handle: Option<[f32; 4]>,
}

/// The fill handle's side, in logical pixels. Excel's is about this size; the
/// hit target around it is deliberately larger (see `App::over_fill_handle`).
pub const FILL_HANDLE: f32 = 7.0;

impl Selection {
    fn covers(&self, row_index: usize, col_index: usize) -> bool {
        match self.range {
            Some((r0, r1, c0, c1)) => {
                row_index >= r0 && row_index <= r1 && col_index >= c0 && col_index <= c1
            }
            None => false,
        }
    }
}

/// The in-cell editor, when one is open.
#[derive(Clone, Copy, Debug)]
pub struct EditView<'a> {
    pub row: RowId,
    pub col: ColId,
    /// The text to *show*, which during an IME composition is the committed
    /// buffer with the composition spliced in at the caret. What the cell will
    /// be given on commit is a different string and lives on `App`.
    pub text: &'a str,
    /// Caret position as a **byte offset** into `text`.
    pub caret: usize,
    /// The byte span of `text` an input method is composing, when one is
    /// (docs/33 §IME). Drawn underlined, which is the convention on both
    /// platforms for "this is a proposal and not yet your text".
    pub preedit: Option<(usize, usize)>,
}

/// The formula bar's two fields.
#[derive(Clone, Copy, Debug, Default)]
pub struct BarView<'a> {
    /// A1 reference of the active cell, or a range like `B4:D9`.
    pub reference: &'a str,
    /// The cell's *source*: its formula text where it has one, its value
    /// otherwise. This is the honesty the grid cannot provide — the grid shows
    /// `42`, and only the bar can show that `42` came from `=SUM(A1:L1)`.
    pub content: &'a str,
    /// The last refusal or narrowing, right-aligned in the bar.
    ///
    /// docs/25: *"every async state is visible and honest"*, and docs/11's
    /// blocked-and-narrowed undo is the case that matters — an undo that
    /// preserved another actor's work and therefore did less than asked must
    /// say so, not appear to have done nothing.
    pub status: &'a str,
}

/// Everything one frame needs. Grouped rather than passed as nine arguments
/// because the frame is a single coherent thing and a nine-argument call is
/// where the wrong two get swapped.
pub struct Frame<'a> {
    pub state: &'a State,
    pub engine: &'a Engine,
    pub visible: &'a Visible,
    pub selection: Selection,
    pub editor: Option<EditView<'a>>,
    pub bar: BarView<'a>,
    /// Window extent in logical pixels. Chrome is drawn to it, so it is needed
    /// even though the cells come from `visible`.
    pub size: (f32, f32),
    /// Display scale factor: glyphs rasterise at `CELL_PX * scale` and lay out
    /// at `CELL_PX`.
    pub scale: f32,
}

/// Emits one laid-out run as glyph quads, clipped to `[left, right)`.
///
/// Clipping by dropping whole glyphs rather than scissoring per cell: one draw
/// call is worth more than a glyph that spills a pixel, and a value too wide
/// for its column is dropped rather than drawn over its neighbour.
#[allow(clippy::too_many_arguments)]
fn push_run(
    quads: &mut Vec<Quad>,
    run: &text::Run,
    origin_x: f32,
    baseline: f32,
    color: [f32; 4],
    left: f32,
    right: f32,
) {
    for placed in &run.glyphs {
        let gx = origin_x + placed.at[0] + placed.glyph.bearing[0];
        let gy = baseline + placed.at[1] + placed.glyph.bearing[1];
        if gx < left || gx + placed.glyph.size[0] > right {
            continue;
        }
        quads.push(Quad {
            rect: [gx, gy, placed.glyph.size[0], placed.glyph.size[1]],
            color,
            uv: placed.glyph.uv,
        });
    }
}

/// A 1 px hairline rectangle outline, as four quads.
fn push_border(quads: &mut Vec<Quad>, rect: [f32; 4], width: f32, color: [f32; 4]) {
    let [x, y, w, h] = rect;
    for r in [
        [x, y, w, width],
        [x, y + h - width, w, width],
        [x, y, width, h],
        [x + w - width, y, width, h],
    ] {
        quads.push(Quad {
            rect: r,
            color,
            uv: TextEngine::white_uv(),
        });
    }
}

/// One frame's output.
///
/// The caret rides along with the quads because only this function knows where
/// it is: its x is the shaped width of the text before it, and the shaping
/// happens here. The window hands the rectangle to the platform so an IME
/// candidate list appears under the composing text (docs/33 §IME).
pub struct Scene {
    pub quads: Vec<Quad>,
    /// The caret's rectangle in logical pixels, when an editor is open **and**
    /// its cell is on screen.
    pub caret: Option<[f32; 4]>,
}

/// Builds the frame's quads.
pub fn build(frame: &Frame, theme: &Theme, text_engine: &mut TextEngine) -> Scene {
    let visible = frame.visible;
    let mut caret_area: Option<[f32; 4]> = None;
    let mut quads = Vec::with_capacity(visible.rows.len() * visible.cols.len() + 128);
    let (ox, oy) = theme.grid_origin();
    let (width, height) = frame.size;
    let scale = frame.scale;
    let px = text::CELL_PX;
    let ascent = text_engine.ascent(px);
    let line = text_engine.line_height(px);

    // --- cell fills, from the real document -------------------------------
    //
    // Only what text cannot say: a computed cell and an error cell. An ordinary
    // value cell gets no quad at all, which is both what a spreadsheet looks
    // like and one fewer instance per filled cell on the bus.
    for r in &visible.rows {
        for c in &visible.cols {
            let row = RowId(r.id);
            let col = ColId(c.id);
            let color = match frame.engine.value(frame.state, row, col) {
                Some(Value::Error(_)) => theme.error_cell,
                _ if frame.state.formula(row, col).is_some() => theme.formula_cell,
                _ => continue,
            };
            quads.push(Quad {
                rect: [ox + c.at, oy + r.at, c.size, r.size],
                color,
                uv: TextEngine::white_uv(),
            });
        }
    }

    // --- selection range ---------------------------------------------------
    //
    // Under the gridlines and under the text, so a selected cell reads as
    // tinted rather than as veiled. The active cell is skipped: Excel leaves it
    // clear and marks it with the border instead, which is what tells you where
    // typing will land inside a large selection.
    for r in &visible.rows {
        for c in &visible.cols {
            if !frame.selection.covers(r.index, c.index) {
                continue;
            }
            if frame.selection.active == Some((RowId(r.id), ColId(c.id))) {
                continue;
            }
            quads.push(Quad {
                rect: [ox + c.at, oy + r.at, c.size, r.size],
                color: theme.selection,
                uv: TextEngine::white_uv(),
            });
        }
    }

    // --- gridlines --------------------------------------------------------
    // Hairlines drawn as 1 px quads: the same pipeline, so they cost nothing
    // extra and cannot disagree with the cells about where a boundary is.
    for r in &visible.rows {
        quads.push(Quad {
            rect: [ox, oy + r.at + r.size - 1.0, width - ox, 1.0],
            color: theme.gridline,
            uv: TextEngine::white_uv(),
        });
    }
    for c in &visible.cols {
        quads.push(Quad {
            rect: [ox + c.at + c.size - 1.0, oy, 1.0, height - oy],
            color: theme.gridline,
            uv: TextEngine::white_uv(),
        });
    }

    // --- headers ----------------------------------------------------------
    quads.push(Quad {
        rect: [0.0, theme.bar_height, width, theme.header_height],
        color: theme.header_fill,
        uv: TextEngine::white_uv(),
    });
    quads.push(Quad {
        rect: [0.0, oy, ox, height - oy],
        color: theme.header_fill,
        uv: TextEngine::white_uv(),
    });
    // The active row and column headers are highlighted — the cheapest way to
    // find the cursor in a large selection, and Excel does it.
    if let Some((ar, ac)) = frame.selection.active {
        for r in visible.rows.iter().filter(|s| RowId(s.id) == ar) {
            quads.push(Quad {
                rect: [0.0, oy + r.at, ox, r.size],
                color: theme.header_active,
                uv: TextEngine::white_uv(),
            });
        }
        for c in visible.cols.iter().filter(|s| ColId(s.id) == ac) {
            quads.push(Quad {
                rect: [ox + c.at, theme.bar_height, c.size, theme.header_height],
                color: theme.header_active,
                uv: TextEngine::white_uv(),
            });
        }
    }
    // A tick per row/column, so the header shows where the boundaries are.
    for r in &visible.rows {
        quads.push(Quad {
            rect: [0.0, oy + r.at + r.size - 1.0, ox, 1.0],
            color: theme.header_rule,
            uv: TextEngine::white_uv(),
        });
    }
    for c in &visible.cols {
        quads.push(Quad {
            rect: [
                ox + c.at + c.size - 1.0,
                theme.bar_height,
                1.0,
                theme.header_height,
            ],
            color: theme.header_rule,
            uv: TextEngine::white_uv(),
        });
    }
    quads.push(Quad {
        rect: [0.0, oy - 1.0, width, 1.0],
        color: theme.header_rule,
        uv: TextEngine::white_uv(),
    });
    quads.push(Quad {
        rect: [ox - 1.0, theme.bar_height, 1.0, height - theme.bar_height],
        color: theme.header_rule,
        uv: TextEngine::white_uv(),
    });

    // --- cell text --------------------------------------------------------
    //
    // The value shown is the **engine's**, which is what closes TD-61: a
    // formula cell has no stored value, and `State::cell` alone drew its fill
    // and nothing inside it.
    for r in &visible.rows {
        for c in &visible.cols {
            let row = RowId(r.id);
            let col = ColId(c.id);
            // The cell being edited shows the editor, not its old value.
            if let Some(ed) = &frame.editor {
                if ed.row == row && ed.col == col {
                    continue;
                }
            }
            let Some(value) = frame.engine.value(frame.state, row, col) else {
                continue;
            };
            let Some(rendered) = text::render_value(&value) else {
                continue;
            };
            let run = text_engine.layout(&rendered, px, scale);
            if run.glyphs.is_empty() {
                continue;
            }
            // Numbers right, text left - Excel's default, and the cheapest
            // signal that a column is numeric.
            let pad = 3.0;
            let origin_x = if text::is_numeric(&value) {
                ox + c.at + c.size - pad - run.width
            } else {
                ox + c.at + pad
            };
            // Vertically centred on the cell, which is where a spreadsheet
            // puts a single line.
            let baseline = oy + r.at + (r.size - line) * 0.5 + ascent;

            push_run(
                &mut quads,
                &run,
                origin_x,
                baseline,
                theme.text,
                ox + c.at,
                ox + c.at + c.size,
            );
        }
    }

    // --- header labels ----------------------------------------------------
    //
    // A1 notation is a *view* over identities (DP-A6), so the label comes from
    // the slot's ordinal rather than from anything stored on the row.
    for r in &visible.rows {
        let label = row_label(r.index);
        let run = text_engine.layout(&label, px, scale);
        let x = ox - 4.0 - run.width;
        let baseline = oy + r.at + (r.size - line) * 0.5 + ascent;
        push_run(
            &mut quads,
            &run,
            x,
            baseline,
            theme.header_text,
            0.0,
            ox - 2.0,
        );
    }
    for c in &visible.cols {
        let label = column_label(c.index);
        let run = text_engine.layout(&label, px, scale);
        let x = ox + c.at + (c.size - run.width) * 0.5;
        let baseline = theme.bar_height + (theme.header_height - line) * 0.5 + ascent;
        push_run(
            &mut quads,
            &run,
            x,
            baseline,
            theme.header_text,
            ox + c.at,
            ox + c.at + c.size,
        );
    }

    // --- active cell border ------------------------------------------------
    if let Some((ar, ac)) = frame.selection.active {
        let row = visible.rows.iter().find(|s| RowId(s.id) == ar);
        let col = visible.cols.iter().find(|s| ColId(s.id) == ac);
        if let (Some(r), Some(c)) = (row, col) {
            push_border(
                &mut quads,
                [ox + c.at, oy + r.at, c.size, r.size],
                2.0,
                theme.selection_border,
            );
        }
    }

    // --- fill preview and handle -------------------------------------------
    //
    // Above the border so the preview reads as the outer edge of what is about
    // to happen, and below the editor because the two are never open at once.
    if let Some((r0, r1, c0, c1)) = frame.selection.fill_preview {
        let rows: Vec<_> = visible
            .rows
            .iter()
            .filter(|s| s.index >= r0 && s.index <= r1)
            .collect();
        let cols: Vec<_> = visible
            .cols
            .iter()
            .filter(|s| s.index >= c0 && s.index <= c1)
            .collect();
        if let (Some(first_row), Some(last_row), Some(first_col), Some(last_col)) =
            (rows.first(), rows.last(), cols.first(), cols.last())
        {
            push_border(
                &mut quads,
                [
                    ox + first_col.at,
                    oy + first_row.at,
                    last_col.at + last_col.size - first_col.at,
                    last_row.at + last_row.size - first_row.at,
                ],
                1.0,
                theme.fill_preview,
            );
        }
    }
    if let Some(rect) = frame.selection.fill_handle {
        // A white ring under the square, so the handle stays visible against a
        // gridline or a tinted cell.
        push_border(
            &mut quads,
            [rect[0] - 1.0, rect[1] - 1.0, rect[2] + 2.0, rect[3] + 2.0],
            1.0,
            theme.editor_fill,
        );
        quads.push(Quad {
            rect,
            color: theme.selection_border,
            uv: TextEngine::white_uv(),
        });
    }

    // --- in-cell editor ----------------------------------------------------
    //
    // An overlay at the cell, which is what docs/31 specifies ("in-cell editor
    // = native text field overlay at caret") and what a native IME will attach
    // to when it lands. It may spill right past its own column, because a
    // formula is usually wider than the cell it lives in and hiding what you
    // are typing is not an option.
    if let Some(ed) = &frame.editor {
        let row = visible.rows.iter().find(|s| RowId(s.id) == ed.row);
        let col = visible.cols.iter().find(|s| ColId(s.id) == ed.col);
        if let (Some(r), Some(c)) = (row, col) {
            let run = text_engine.layout(ed.text, px, scale);
            let pad = 3.0;
            let needed = run.width + pad * 2.0 + 2.0;
            let box_w = needed.max(c.size).min((width - (ox + c.at)).max(c.size));
            let rect = [ox + c.at, oy + r.at, box_w, r.size];
            quads.push(Quad {
                rect,
                color: theme.editor_fill,
                uv: TextEngine::white_uv(),
            });
            push_border(&mut quads, rect, 2.0, theme.selection_border);
            let baseline = oy + r.at + (r.size - line) * 0.5 + ascent;
            push_run(
                &mut quads,
                &run,
                ox + c.at + pad,
                baseline,
                theme.text,
                ox + c.at,
                ox + c.at + box_w,
            );
            // The composition's underline (docs/33 §IME). Under the *spliced*
            // text, so it tracks the same shaping the glyphs did rather than a
            // second guess at where the composition starts. Clipped to the
            // editor box for the same reason the run is: a long composition
            // must not draw over the cells to the right.
            if let Some((start, end)) = ed.preedit {
                let left = (ox + c.at + pad + run.caret_x(start)).min(ox + c.at + box_w);
                let right = (ox + c.at + pad + run.caret_x(end)).min(ox + c.at + box_w);
                if right > left {
                    quads.push(Quad {
                        rect: [left, oy + r.at + r.size - 4.0, right - left, 1.0],
                        color: theme.text,
                        uv: TextEngine::white_uv(),
                    });
                }
            }
            // The caret. Solid rather than blinking: a blink needs a timer in
            // the frame loop, and a grid that redraws only on change should not
            // acquire one for a cosmetic effect.
            let caret_x = ox + c.at + pad + run.caret_x(ed.caret);
            let caret = [caret_x, oy + r.at + 3.0, 1.5, r.size - 6.0];
            quads.push(Quad {
                rect: caret,
                color: theme.caret,
                uv: TextEngine::white_uv(),
            });
            caret_area = Some(caret);
        }
    }

    // --- formula bar -------------------------------------------------------
    quads.push(Quad {
        rect: [0.0, 0.0, width, theme.bar_height],
        color: theme.bar_fill,
        uv: TextEngine::white_uv(),
    });
    quads.push(Quad {
        rect: [0.0, theme.bar_height - 1.0, width, 1.0],
        color: theme.header_rule,
        uv: TextEngine::white_uv(),
    });
    quads.push(Quad {
        rect: [ox + 30.0, 3.0, 1.0, theme.bar_height - 7.0],
        color: theme.header_rule,
        uv: TextEngine::white_uv(),
    });
    let bar_baseline = (theme.bar_height - line) * 0.5 + ascent;
    if !frame.bar.reference.is_empty() {
        let run = text_engine.layout(frame.bar.reference, px, scale);
        push_run(
            &mut quads,
            &run,
            6.0,
            bar_baseline,
            theme.text,
            0.0,
            ox + 28.0,
        );
    }
    // The status is laid out first so the content field knows where to stop:
    // a long formula must not run underneath a warning.
    let mut content_right = width - 6.0;
    if !frame.bar.status.is_empty() {
        let run = text_engine.layout(frame.bar.status, px, scale);
        let x = (width - 6.0 - run.width).max(ox + 38.0);
        content_right = x - 10.0;
        push_run(
            &mut quads,
            &run,
            x,
            bar_baseline,
            theme.warning,
            x - 1.0,
            width,
        );
    }
    if !frame.bar.content.is_empty() {
        let run = text_engine.layout(frame.bar.content, px, scale);
        push_run(
            &mut quads,
            &run,
            ox + 38.0,
            bar_baseline,
            theme.text,
            ox + 36.0,
            content_right,
        );
    }

    Scene {
        quads,
        caret: caret_area,
    }
}

#[cfg(test)]
mod tests {
    //! The render-layer half of TD-61 and of the selection work.
    //!
    //! `app.rs` proves the *model* computes `42`; these prove the *frame*
    //! contains glyphs for it, inside the right cell. Both are needed: a
    //! renderer that computed the value and then dropped it on the floor would
    //! pass every test in `app.rs`.

    use super::*;
    use crate::app::harness::{self, press, type_text};
    use crate::input::{Key, Mods};

    /// A glyph instance samples somewhere other than the reserved white texel.
    fn is_glyph(q: &Quad) -> bool {
        q.uv != TextEngine::white_uv()
    }

    fn within(q: &Quad, rect: [f32; 4]) -> bool {
        let [x, y, w, h] = rect;
        q.rect[0] >= x - 0.5
            && q.rect[1] >= y - 0.5
            && q.rect[0] + q.rect[2] <= x + w + 0.5
            && q.rect[1] + q.rect[3] <= y + h + 0.5
    }

    /// The screen rect of a cell at view ordinals, for a freshly opened app.
    fn cell_rect(theme: &Theme, row: usize, col: usize) -> [f32; 4] {
        let (ox, oy) = theme.grid_origin();
        [ox + col as f32 * 64.0, oy + row as f32 * 20.0, 64.0, 20.0]
    }

    #[test]
    fn a_formula_cell_is_drawn_with_glyphs_for_its_computed_value() {
        // TD-61 at the render layer. Before the engine was wired in this cell
        // produced a fill and nothing else.
        let mut app = harness::app(30, 6);
        type_text(&mut app, "=6*7");
        press(&mut app, Key::Enter, Mods::NONE);
        let theme_rect = cell_rect(app.theme(), 0, 0);
        let quads = app.frame();
        let glyphs = quads
            .iter()
            .filter(|q| is_glyph(q) && within(q, theme_rect))
            .count();
        assert_eq!(
            glyphs, 2,
            "`42` is two glyphs; the cell drew {glyphs} of them"
        );
    }

    #[test]
    fn an_error_cell_is_tinted_and_names_the_error() {
        let mut app = harness::app(30, 6);
        type_text(&mut app, "=1/0");
        press(&mut app, Key::Enter, Mods::NONE);
        let rect = cell_rect(app.theme(), 0, 0);
        let error_fill = app.theme().error_cell;
        let quads = app.frame();
        assert!(
            quads
                .iter()
                .any(|q| !is_glyph(q) && q.color == error_fill && within(q, rect)),
            "an error cell must be tinted"
        );
        // `#DIV/0!` is seven characters, of which all have outlines.
        let glyphs = quads
            .iter()
            .filter(|q| is_glyph(q) && within(q, rect))
            .count();
        assert_eq!(glyphs, 7, "`#DIV/0!` is seven glyphs; drew {glyphs}");
    }

    #[test]
    fn an_ordinary_value_cell_gets_no_fill_quad() {
        // The theme change that came with text: a spreadsheet is white, and a
        // fill per filled cell is both wrong-looking and an instance per cell.
        let mut app = harness::app(30, 6);
        type_text(&mut app, "5");
        press(&mut app, Key::Enter, Mods::NONE);
        let rect = cell_rect(app.theme(), 0, 0);
        let formula_fill = app.theme().formula_cell;
        let error_fill = app.theme().error_cell;
        let quads = app.frame();
        assert!(
            !quads.iter().any(|q| within(q, rect)
                && !is_glyph(q)
                && (q.color == formula_fill || q.color == error_fill)),
            "a plain value must not be tinted as computed or as an error"
        );
        assert_eq!(
            quads
                .iter()
                .filter(|q| is_glyph(q) && within(q, rect))
                .count(),
            1,
            "`5` is one glyph"
        );
    }

    #[test]
    fn the_selected_range_is_tinted_and_the_active_cell_is_left_clear() {
        let mut app = harness::app(30, 6);
        press(&mut app, Key::Down, Mods::shift());
        press(&mut app, Key::Down, Mods::shift());
        let tint = app.theme().selection;
        let active = cell_rect(app.theme(), 2, 0);
        let other = cell_rect(app.theme(), 1, 0);
        let quads = app.frame();
        assert!(
            quads.iter().any(|q| q.color == tint && within(q, other)),
            "a cell inside the range must be tinted"
        );
        assert!(
            !quads.iter().any(|q| q.color == tint && within(q, active)),
            "the active cell must be left clear inside its own selection"
        );
        // And it carries the border instead.
        let border = app.theme().selection_border;
        assert!(
            quads
                .iter()
                .filter(|q| q.color == border && within(q, active))
                .count()
                >= 4,
            "the active cell must be outlined"
        );
    }

    #[test]
    fn the_fill_handle_is_drawn_at_the_corner_of_the_selection() {
        let mut app = harness::app(30, 6);
        press(&mut app, Key::Down, Mods::shift());
        let handle = app.fill_handle().expect("on screen");
        let border = app.theme().selection_border;
        let quads = app.frame();
        assert!(
            quads.iter().any(|q| q.color == border
                && (q.rect[0] - handle[0]).abs() < 0.01
                && (q.rect[1] - handle[1]).abs() < 0.01),
            "the handle must be drawn where the hit test says it is"
        );
    }

    #[test]
    fn a_fill_drag_draws_its_preview_and_the_committed_frame_does_not() {
        let mut app = harness::app(30, 6);
        let preview = app.theme().fill_preview;
        assert!(
            !app.frame().iter().any(|q| q.color == preview),
            "nothing is being dragged, so nothing should be previewed"
        );

        let handle = app.fill_handle().unwrap();
        app.pointer_down(handle[0] + 3.0, handle[1] + 3.0, false);
        let (ox, oy) = app.theme().grid_origin();
        app.pointer_drag(ox + 4.0, oy + 4.0 * 20.0 + 4.0);
        assert!(
            app.frame().iter().any(|q| q.color == preview),
            "a drag in progress must show what it would fill"
        );

        app.pointer_up();
        assert!(
            !app.frame().iter().any(|q| q.color == preview),
            "the preview must not survive the release that consumed it"
        );
    }

    #[test]
    fn a_composition_is_drawn_in_the_cell_and_underlined_as_unconfirmed() {
        // The render half of the IME work (docs/33 §IME). `app.rs` proves the
        // composition does not reach the cell; this proves the user can *see*
        // it while it does not, which is the entire purpose of a preedit.
        let mut app = harness::app(30, 6);
        type_text(&mut app, "ab");
        let before = app.frame().len();

        app.ime_preedit("cde", None);
        let quads = app.frame();
        let rect = cell_rect(app.theme(), 0, 0);
        assert!(
            quads.len() > before,
            "the composing text must add glyphs, not replace them"
        );
        // The underline: a solid quad one logical pixel high, sitting on the
        // cell's baseline area and wider than the caret's 1.5 px.
        let underline = quads
            .iter()
            .find(|q| !is_glyph(q) && q.rect[3] == 1.0 && q.rect[2] > 2.0 && within(q, rect));
        let underline = underline.expect("an unconfirmed composition must be underlined");
        assert!(
            underline.rect[1] > rect[1] + rect[3] * 0.5,
            "the underline belongs under the text, not through it"
        );

        // And it goes when the composition does.
        app.ime_preedit("", None);
        let quads = app.frame();
        assert!(
            !quads
                .iter()
                .any(|q| !is_glyph(q) && q.rect[3] == 1.0 && q.rect[2] > 2.0 && within(q, rect)),
            "the underline must not outlive the composition it marks"
        );
    }

    #[test]
    fn the_frame_is_one_draw_call_worth_of_quads_and_never_walks_the_document() {
        let mut app = harness::app(200_000, 40);
        let quads = app.frame();
        // A 1280x800 window shows about 39 rows and 20 columns; every quad is
        // bounded by that, plus chrome. The number that matters is that it is
        // not a function of 200,000.
        assert!(
            quads.len() < 4_000,
            "{} quads for a 200,000-row sheet",
            quads.len()
        );
    }
}
