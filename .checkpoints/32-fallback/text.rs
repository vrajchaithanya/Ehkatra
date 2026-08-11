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
///
/// `uv` is in **device** texels because that is what the atlas holds; `size`
/// and `bearing` are in **logical** pixels because that is the space the scene
/// is laid out in. On a 1× display the two are numerically equal; on a 2×
/// display the atlas entry is twice as large and the quad that samples it is
/// not, which is the whole of what "the text is sharp on a HiDPI monitor"
/// means (docs/33 §Displays).
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    /// Atlas rect in device texels: `(x, y, w, h)`.
    pub uv: [f32; 4],
    /// Quad size in logical pixels.
    pub size: [f32; 2],
    /// Offset from the pen position to the bitmap's top-left, in logical
    /// pixels.
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
    /// `(source byte offset, pen x)` for every shaped glyph, in visual order,
    /// **including glyphs with no outline**.
    ///
    /// The editor's caret needs this and `glyphs` cannot supply it: a space has
    /// no outline, so it never becomes a `Placed`, and a caret placed from the
    /// drawn glyphs alone would refuse to sit after one.
    pub clusters: Vec<(u32, f32)>,
}

impl Run {
    /// The pen x of the caret before byte offset `at`.
    ///
    /// Clusters are the unit rather than characters because shaping is: one
    /// glyph can cover several bytes (a ligature) and several glyphs one byte
    /// (a decomposed mark), and the caret belongs at a cluster boundary in
    /// either case.
    pub fn caret_x(&self, at: usize) -> f32 {
        let at = at as u32;
        for (offset, x) in &self.clusters {
            if *offset >= at {
                return *x;
            }
        }
        self.width
    }
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
    ///
    /// `px` is a **logical** size and `device_scale` is the display's scale
    /// factor. Layout — every advance, offset and width this returns — is
    /// computed at the logical size and is therefore identical on every
    /// display; only the rasterisation is done at `px * device_scale`. That
    /// split is exactly docs/31's rule: *"layout determinism is
    /// metric-determinism; pixels may differ per rasterizer"*.
    pub fn layout(&mut self, text: &str, px: f32, device_scale: f32) -> Run {
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
            run.clusters.push((info.cluster, pen));
            if let Some(glyph) = self.rasterise(id, px, device_scale) {
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
    ///
    /// Keyed by the **device** size, so a window dragged between a 1× and a 2×
    /// monitor grows the atlas rather than needing it rebuilt: both entries
    /// coexist and each display samples its own (docs/33 §Displays,
    /// mixed-DPI window drag).
    fn rasterise(&mut self, id: u16, px: f32, device_scale: f32) -> Option<Glyph> {
        let device_px = px * device_scale;
        let key = (id, device_px.to_bits());
        if let Some(hit) = self.cache.get(&key) {
            return *hit;
        }
        let glyph = ab_glyph::GlyphId(id).with_scale(device_px);
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
                size: [w as f32 / device_scale, h as f32 / device_scale],
                bearing: [bounds.min.x / device_scale, bounds.min.y / device_scale],
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What the bundled font can and cannot draw, stated as a measurement
    /// rather than as an assumption (**TD-79**).
    ///
    /// The IME path (docs/33 §IME) delivers a CJK composition into the editor
    /// correctly — `app.rs` proves that without a display — and then this
    /// happens: `DejaVu Sans` carries no CJK block, so every one of those
    /// characters shapes to glyph 0, `.notdef`, and the grid draws boxes. The
    /// plumbing is right and the *pixels* are wrong, which is a different
    /// defect than the one the IME work fixed and needs a different fix (font
    /// fallback, TD-79).
    ///
    /// This test exists so that gap is a fact in the suite instead of a
    /// sentence in a register nobody re-reads. **It is expected to fail the day
    /// font fallback lands** — that is the signal to delete it and close TD-79,
    /// not a regression.
    #[test]
    fn the_bundled_font_covers_latin_and_not_cjk_which_is_what_td79_is() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        let latin = engine.layout("Revenue", CELL_PX, 1.0);
        assert_eq!(
            latin.glyphs.len(),
            7,
            "every Latin letter must reach the atlas"
        );

        // `にほん` — what a Japanese IME commits for `nihon`.
        let kana = engine.layout("にほん", CELL_PX, 1.0);
        assert_eq!(kana.clusters.len(), 3, "three characters are still shaped");
        assert!(
            kana.width > 0.0,
            "and they still advance, so the caret lands in the right place"
        );
        // `.notdef` in DejaVu is a hollow box with an outline, so the glyphs do
        // reach the atlas — they are simply not the characters the user typed.
        // The proof that they are boxes and not kana: all three sample the
        // *same* atlas rect, which three distinct characters never would.
        assert_eq!(
            kana.glyphs.len(),
            3,
            "three boxes, which is the visible half of TD-79"
        );
        assert!(
            kana.glyphs
                .windows(2)
                .all(|w| w[0].glyph.uv == w[1].glyph.uv),
            "three different characters rendering identically is `.notdef`, three times"
        );
        // The control: three different Latin letters do not.
        let abc = engine.layout("abc", CELL_PX, 1.0);
        assert!(
            abc.glyphs
                .windows(2)
                .any(|w| w[0].glyph.uv != w[1].glyph.uv),
            "the same check must be able to fail, or it proves nothing"
        );
    }
}
