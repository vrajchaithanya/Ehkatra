//! Text: shaping, a glyph atlas, and layout (TD-59, docs/31, docs/25).
//!
//! docs/31 is specific about the stack and about *why*:
//!
//! > *Shaping: rustybuzz + **bundled fonts only for layout metrics** (26.6
//! > fixed-point) — layout determinism is metric-determinism; pixels may differ
//! > per rasterizer (DirectWrite/CoreText raster is fine; their metrics are not
//! > consulted).*
//!
//! So the font is **bundled**, not the system's: two machines must agree on
//! where every glyph goes, and that follows from agreeing on advances. What
//! they draw into those boxes may differ by a subpixel and nobody minds.
//!
//! # The atlas, and why there is still one draw call
//! Glyphs are rasterised once into an `R8` coverage texture and cached by
//! `(glyph, size)`. A glyph quad and a cell fill are the *same instance type*,
//! differing only in which part of the atlas they sample — solid quads point at
//! a reserved white texel. So adding text did not add a pipeline, a pass, or a
//! draw call (docs/31: *instanced fills/borders/gridlines*).

use ab_glyph::{Font as _, FontRef, ScaleFont as _};
use std::collections::HashMap;

/// The grid's text size in logical pixels. One size for now; the atlas is
/// keyed by size already, so styles are an addition rather than a rewrite.
pub const CELL_PX: f32 = 12.0;

/// Atlas dimensions. 1024² of `R8` is 1 MiB and holds far more Latin glyphs at
/// grid sizes than a viewport can show.
const ATLAS: u32 = 1024;

/// Where a glyph lives in the atlas, and how to place it.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    /// Atlas rect in texels: `(x, y, w, h)`.
    pub uv: [f32; 4],
    /// Offset from the pen position to the bitmap's top-left, in pixels.
    pub bearing: [f32; 2],
}

/// One positioned glyph in a laid-out run.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub glyph: Glyph,
    /// Pen position relative to the run's origin (baseline-left).
    pub at: [f32; 2],
}

/// A shaped, rasterised run ready to be turned into quads.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub glyphs: Vec<Placed>,
    /// Total advance, for right-alignment and for deciding what fits.
    pub width: f32,
}

pub struct TextEngine {
    face: rustybuzz::Face<'static>,
    font: FontRef<'static>,
    atlas: Vec<u8>,
    /// Shelf allocator: current row's top and height, and the pen within it.
    cursor: (u32, u32, u32),
    cache: HashMap<(u16, u32), Option<Glyph>>,
    /// Set when a glyph did not fit; the atlas is full and the caller should
    /// know rather than silently render blanks.
    pub overflowed: bool,
    /// Set when a glyph was rasterised since the last upload.
    ///
    /// The atlas only changes when text the viewport has never shown appears,
    /// which after a moment of scrolling is never — so re-uploading a mebibyte
    /// every frame would be pure waste, and measuring it would overstate what a
    /// frame costs.
    dirty: bool,
}

impl TextEngine {
    /// Loads the bundled font.
    ///
    /// `DejaVu Sans` from the `dejavu` crate — a *versioned, licence-checked
    /// dependency* rather than a binary blob committed to the repo, so the
    /// supply-chain gate covers it like anything else (ADR-038).
    pub fn new() -> Option<TextEngine> {
        let bytes: &'static [u8] = dejavu::sans::regular();
        let face = rustybuzz::Face::from_slice(bytes, 0)?;
        let font = FontRef::try_from_slice(bytes).ok()?;
        let mut engine = TextEngine {
            face,
            font,
            atlas: vec![0u8; (ATLAS * ATLAS) as usize],
            cursor: (0, 0, 0),
            cache: HashMap::new(),
            overflowed: false,
            dirty: true,
        };
        // Texel (0,0) is opaque white and is what solid quads sample, so a fill
        // and a glyph are the same instance and share one draw call.
        engine.atlas[0] = 255;
        engine.cursor = (0, 1, 1);
        Some(engine)
    }

    /// Whether the atlas has changed since [`TextEngine::mark_uploaded`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_uploaded(&mut self) {
        self.dirty = false;
    }

    pub fn atlas_size(&self) -> u32 {
        ATLAS
    }

    pub fn atlas_bytes(&self) -> &[u8] {
        &self.atlas
    }

    /// The uv rect of the reserved white texel.
    pub fn white_uv() -> [f32; 4] {
        [0.0, 0.0, 1.0, 1.0]
    }

    pub fn ascent(&self, px: f32) -> f32 {
        self.font.as_scaled(px).ascent()
    }

    pub fn line_height(&self, px: f32) -> f32 {
        let f = self.font.as_scaled(px);
        f.ascent() - f.descent()
    }

    /// Shapes and rasterises a string.
    ///
    /// Shaping is rustybuzz's, so advances come from the font's own tables
    /// including kerning — which is what makes the layout reproducible on a
    /// machine that has never seen this text before.
    pub fn layout(&mut self, text: &str, px: f32) -> Run {
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&self.face, &[], buffer);

        let upem = self.face.units_per_em() as f32;
        let scale = px / upem;
        let infos = shaped.glyph_infos();
        let positions = shaped.glyph_positions();

        let mut run = Run::default();
        let mut pen = 0.0f32;
        for (info, pos) in infos.iter().zip(positions.iter()) {
            let id = info.glyph_id as u16;
            if let Some(glyph) = self.rasterise(id, px) {
                run.glyphs.push(Placed {
                    glyph,
                    at: [
                        pen + pos.x_offset as f32 * scale,
                        -(pos.y_offset as f32 * scale),
                    ],
                });
            }
            pen += pos.x_advance as f32 * scale;
        }
        run.width = pen;
        run
    }

    /// Rasterises one glyph into the atlas, or returns the cached entry.
    ///
    /// `None` for a glyph with no outline — a space is the common case, and it
    /// is not an error.
    fn rasterise(&mut self, id: u16, px: f32) -> Option<Glyph> {
        let key = (id, px.to_bits());
        if let Some(hit) = self.cache.get(&key) {
            return *hit;
        }
        let glyph = ab_glyph::GlyphId(id).with_scale(px);
        let entry = self.font.outline_glyph(glyph).and_then(|outlined| {
            let bounds = outlined.px_bounds();
            let (w, h) = (bounds.width().ceil() as u32, bounds.height().ceil() as u32);
            if w == 0 || h == 0 {
                return None;
            }
            let (x, y) = self.alloc(w, h)?;
            self.dirty = true;
            outlined.draw(|gx, gy, coverage| {
                let px_i = (x + gx) as usize;
                let py_i = (y + gy) as usize;
                if px_i < ATLAS as usize && py_i < ATLAS as usize {
                    let at = py_i * ATLAS as usize + px_i;
                    // Max, not overwrite: two glyphs never share a cell, but a
                    // rounding overlap at the edge should brighten rather than
                    // erase.
                    self.atlas[at] = self.atlas[at].max((coverage * 255.0) as u8);
                }
            });
            Some(Glyph {
                uv: [x as f32, y as f32, w as f32, h as f32],
                bearing: [bounds.min.x, bounds.min.y],
            })
        });
        self.cache.insert(key, entry);
        entry
    }

    /// Shelf allocation: glyphs of a similar height pack into a row, and a new
    /// row opens when one does not fit. Wasteful at the top of each shelf and
    /// entirely adequate for a few hundred Latin glyphs.
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (mut x, mut y, mut row_h) = self.cursor;
        if x + w + 1 > ATLAS {
            x = 0;
            y += row_h + 1;
            row_h = 0;
        }
        if y + h + 1 > ATLAS {
            // Full. Reported rather than wrapped: silently reusing occupied
            // texels would draw the wrong glyphs, which looks like a font bug
            // and is not one.
            self.overflowed = true;
            return None;
        }
        let at = (x, y);
        self.cursor = (x + w + 1, y, row_h.max(h));
        Some(at)
    }
}

/// How a cell's value is rendered as text.
///
/// Deliberately minimal, and deliberately *named* as minimal: Excel's real
/// display path is the number-format grammar, which is TD-36 and does not
/// exist yet. What is here is enough to read a grid — integers without a
/// decimal point, a bounded number of fractional digits otherwise, and errors
/// by their canonical name.
pub fn render_value(value: &usk_types::Value) -> Option<String> {
    use usk_types::Value;
    Some(match value {
        Value::Blank => return None,
        Value::Bool(b) => {
            if *b {
                String::from("TRUE")
            } else {
                String::from("FALSE")
            }
        }
        Value::Error(e) => String::from(e.kind.as_str()),
        Value::Text(s) => s.clone(),
        Value::Decimal(d) => format!("{d}"),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                let s = format!("{n:.4}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
    })
}

/// Numbers right, everything else left — Excel's default, and the cheapest
/// signal that a column is numeric.
pub fn is_numeric(value: &usk_types::Value) -> bool {
    matches!(
        value,
        usk_types::Value::Number(_) | usk_types::Value::Decimal(_)
    )
}
