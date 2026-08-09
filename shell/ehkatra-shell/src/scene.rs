//! Turning a viewport into quads (ADR-021, docs/25 §the grid).
//!
//! The scene reads the **real** `State` — there is no fixture layer and no
//! placeholder data anywhere in the shell. Which cells are drawn comes from
//! `usk_view::Viewport::visible`, so this walks the window and never the
//! document (docs/31).

use usk_state::State;
use usk_types::{ColId, RowId, Value};
use usk_view::Visible;

use crate::gpu::Quad;

/// Design tokens (docs/25 §Visual system: one token file, light and dark from
/// day one).
///
/// Written as **sRGB** — the space a designer picks colours in — and converted
/// to linear on the way to the GPU. The target is `Rgba8UnormSrgb`, so the
/// hardware encodes linear→sRGB on write; handing it sRGB values directly
/// encodes them twice and everything comes out washed out. That is exactly
/// what the first frame looked like.
pub struct Theme {
    pub gridline: [f32; 4],
    pub header_fill: [f32; 4],
    pub header_rule: [f32; 4],
    pub filled_cell: [f32; 4],
    pub formula_cell: [f32; 4],
    pub error_cell: [f32; 4],
    pub selection: [f32; 4],
    pub header_width: f32,
    pub header_height: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            gridline: srgb(0.78, 0.80, 0.83, 1.0),
            header_fill: srgb(0.90, 0.91, 0.93, 1.0),
            header_rule: srgb(0.55, 0.58, 0.62, 1.0),
            filled_cell: srgb(0.72, 0.82, 0.93, 1.0),
            formula_cell: srgb(0.66, 0.86, 0.72, 1.0),
            error_cell: srgb(0.95, 0.66, 0.62, 1.0),
            selection: srgb(0.13, 0.38, 0.80, 0.35),
            header_width: 44.0,
            header_height: 22.0,
        }
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

/// Which cell the selection is on, by identity — like everything else the view
/// remembers (DP-A6).
#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub row: Option<RowId>,
    pub col: Option<ColId>,
}

/// Builds the frame's quads.
///
/// Ordering is painter's: fills, then gridlines, then headers, then selection
/// on top. One pass, one draw call.
pub fn build(state: &State, visible: &Visible, theme: &Theme, selection: Selection) -> Vec<Quad> {
    let mut quads = Vec::with_capacity(visible.rows.len() * visible.cols.len() + 64);
    let (ox, oy) = (theme.header_width, theme.header_height);

    // --- cell fills, from the real document -------------------------------
    //
    // No text yet (TD-59): docs/31 specifies a glyph atlas over rustybuzz-shaped
    // runs, which is a font stack and a decision of its own. Until then a cell's
    // *kind* is drawn — has a value, holds a formula, is an error — which is
    // real information from the kernel rather than a placeholder.
    for r in &visible.rows {
        for c in &visible.cols {
            let row = RowId(r.id);
            let col = ColId(c.id);
            // A formula cell is checked **first and independently**: a cell
            // holding only a formula has no value in the tile store, so gating
            // on `cell()` skipped the entire formula column. The first frame
            // rendered showed exactly that — no green anywhere.
            let color = if state.formula(row, col).is_some() {
                theme.formula_cell
            } else {
                match state.cell(row, col) {
                    Some(Value::Error(_)) => theme.error_cell,
                    Some(Value::Blank) | None => continue,
                    Some(_) => theme.filled_cell,
                }
            };
            quads.push(Quad {
                rect: [ox + c.at, oy + r.at, c.size, r.size],
                color,
            });
        }
    }

    // --- gridlines --------------------------------------------------------
    // Hairlines drawn as 1 px quads: the same pipeline, so they cost nothing
    // extra and cannot disagree with the cells about where a boundary is.
    for r in &visible.rows {
        quads.push(Quad {
            rect: [ox, oy + r.at + r.size - 1.0, f32::MAX.min(4096.0), 1.0],
            color: theme.gridline,
        });
    }
    for c in &visible.cols {
        quads.push(Quad {
            rect: [ox + c.at + c.size - 1.0, oy, 1.0, f32::MAX.min(4096.0)],
            color: theme.gridline,
        });
    }

    // --- headers ----------------------------------------------------------
    quads.push(Quad {
        rect: [0.0, 0.0, 4096.0, oy],
        color: theme.header_fill,
    });
    quads.push(Quad {
        rect: [0.0, 0.0, ox, 4096.0],
        color: theme.header_fill,
    });
    // A tick per row/column, so the header shows where the boundaries are even
    // before there are labels on it.
    for r in &visible.rows {
        quads.push(Quad {
            rect: [0.0, oy + r.at + r.size - 1.0, ox, 1.0],
            color: theme.header_rule,
        });
    }
    for c in &visible.cols {
        quads.push(Quad {
            rect: [ox + c.at + c.size - 1.0, 0.0, 1.0, oy],
            color: theme.header_rule,
        });
    }
    quads.push(Quad {
        rect: [0.0, oy - 1.0, 4096.0, 1.0],
        color: theme.header_rule,
    });
    quads.push(Quad {
        rect: [ox - 1.0, 0.0, 1.0, 4096.0],
        color: theme.header_rule,
    });

    // --- selection --------------------------------------------------------
    if let (Some(sr), Some(sc)) = (selection.row, selection.col) {
        let row = visible.rows.iter().find(|s| RowId(s.id) == sr);
        let col = visible.cols.iter().find(|s| ColId(s.id) == sc);
        if let (Some(r), Some(c)) = (row, col) {
            quads.push(Quad {
                rect: [ox + c.at, oy + r.at, c.size, r.size],
                color: theme.selection,
            });
        }
    }

    quads
}
