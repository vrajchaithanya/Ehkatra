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
//! # Fallback, and the exact shape of the promise above (TD-79, D-125)
//! A bundled font is a *finite* font. `DejaVu Sans` covers Latin, Greek and
//! Cyrillic and no CJK, so before this the IME could deliver `にほん` correctly
//! and the grid drew three `.notdef` boxes. The fix is a fallback chain, and it
//! costs the sentence above some of its scope, so the scope is stated rather
//! than quietly lost:
//!
//! * **Every codepoint the bundled face covers is still laid out from bundled
//!   metrics.** That is all Latin text, which is every benchmark, every corpus
//!   file and every demo frame in the repo. Nothing that was deterministic
//!   became less so.
//! * **A codepoint it does not cover is laid out from a system face**, because
//!   the alternative is to draw a box. Two hosts with different fonts installed
//!   will lay that run out differently — that is real, and it is why the
//!   resolution is *recorded*: [`Run::faces`] names the slots a run used and
//!   [`TextEngine::face_name`] names the face in each, so a divergence is
//!   explainable instead of mysterious (docs/31's rule kept where it can be,
//!   and its loss named where it cannot).
//! * **The line box never moves.** Ascent and line height come from the bundled
//!   face whatever a cell contains, so a row's height does not depend on which
//!   scripts happen to be in it, and does not depend on the host at all.
//! * **The order of preference is bundled**, not the host's opinion: two
//!   machines that both have `Yu Gothic` installed pick `Yu Gothic`, because
//!   [`PREFERRED`] is a constant in this file and is consulted before anything
//!   is enumerated.
//!
//! # The atlas, and why there is still one draw call
//! Glyphs are rasterised once into an `R8` coverage texture and cached by
//! `(face, glyph, size)` — **`face` is the part TD-79 added**, and it is not
//! cosmetic: glyph 42 of the bundled face and glyph 42 of a fallback face are
//! different pictures, and a key without the face in it would serve the first
//! one to draw for both. A glyph quad and a cell fill are the *same instance
//! type*, differing only in which part of the atlas they sample — solid quads
//! point at a reserved white texel. So neither text nor fallback added a
//! pipeline, a pass, or a draw call (docs/31: *instanced fills/borders/gridlines*).

use ab_glyph::{Font as _, FontRef, ScaleFont as _};
use std::collections::HashMap;

/// The families a missing codepoint is looked for in, **in this order**.
///
/// This list is the half of layout determinism that survives fallback (D-125):
/// which face draws `に` is decided by a constant in this binary and not by the
/// host's font enumeration order, so two machines with the same fonts installed
/// agree. Only *which of these are present* varies, and [`Run::faces`] records
/// the answer per run.
///
/// Ordered by coverage breadth within each platform's usual set: pan-Unicode
/// Noto first (a machine that has it can draw nearly everything from one face,
/// which is also the most reproducible outcome), then the platform's own CJK
/// families, then broad Latin-plus faces for scripts like Greek extended or
/// Arabic that the platform UI font usually carries.
const PREFERRED: &[&str] = &[
    "Noto Sans",
    "Noto Sans CJK JP",
    "Noto Sans CJK SC",
    "Noto Sans CJK KR",
    "Noto Sans JP",
    "Arial Unicode MS",
    "Yu Gothic",
    "Meiryo",
    "MS Gothic",
    "Microsoft YaHei",
    "SimSun",
    "Malgun Gothic",
    "PingFang SC",
    "Hiragino Sans",
    "Segoe UI",
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Tahoma",
    "Arial",
];

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
    /// The face slots this run was shaped with, ascending, without repeats.
    ///
    /// `[0]` means "entirely bundled metrics", which is the deterministic case
    /// and is what every Latin run reports. Anything else names the fallback
    /// faces involved, and [`TextEngine::face_name`] turns a slot into the name
    /// that was resolved on this host — the record D-125 requires so that two
    /// machines disagreeing about a layout can say *why*.
    pub faces: Vec<u16>,
    /// How many characters no face on this host could draw.
    ///
    /// Counted rather than hidden: a `.notdef` box is still drawn, because
    /// drawing nothing would silently shorten the text, but the run says how
    /// many boxes it contains so a caller can report a font shortfall instead
    /// of the user discovering it.
    pub unresolved: u32,
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

/// One loaded face: what shapes with it, what rasterises from it, and what it
/// is called.
///
/// The name exists for the record ([`Run::faces`]) and for nothing else — no
/// decision is made from it after loading.
struct Face {
    name: String,
    face: rustybuzz::Face<'static>,
    font: FontRef<'static>,
}

pub struct TextEngine {
    /// Slot 0 is the bundled face and is always present; 1.. are fallbacks
    /// resolved from the host, in the order they were first needed.
    faces: Vec<Face>,
    /// `char` → the slot that draws it, or `None` when nothing on this host
    /// can. Memoised because resolving costs file reads and a codepoint's
    /// answer cannot change while the process runs.
    coverage: HashMap<char, Option<usize>>,
    /// The system font database, built on the **first** codepoint the bundled
    /// face cannot draw and never before.
    ///
    /// Lazy on purpose: `load_system_fonts` reads several hundred files, and a
    /// Latin-only workbook — which is every one the benchmarks open — must not
    /// pay for a fallback it never uses. docs/31's 1.0 s cold-launch budget is
    /// measured on exactly that path.
    system: Option<fontdb::Database>,
    atlas: Vec<u8>,
    /// Shelf allocator: current row's top and height, and the pen within it.
    cursor: (u32, u32, u32),
    /// `(face slot, glyph id, device px)`. The face is in the key because two
    /// faces number their glyphs independently.
    cache: HashMap<(u16, u16, u32), Option<Glyph>>,
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
        let mut engine = TextEngine {
            faces: vec![Face {
                name: String::from("DejaVu Sans"),
                face: rustybuzz::Face::from_slice(bytes, 0)?,
                font: FontRef::try_from_slice(bytes).ok()?,
            }],
            coverage: HashMap::new(),
            system: None,
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

    /// Ascent, **always from the bundled face**.
    ///
    /// A row's height must not depend on what is typed into it, so the line box
    /// is a property of the grid and not of the scripts a cell happens to
    /// contain. This is the part of docs/31's determinism rule that fallback
    /// does not touch at all (D-125).
    pub fn ascent(&self, px: f32) -> f32 {
        self.faces[0].font.as_scaled(px).ascent()
    }

    /// Line height, from the bundled face — see [`TextEngine::ascent`].
    pub fn line_height(&self, px: f32) -> f32 {
        let f = self.faces[0].font.as_scaled(px);
        f.ascent() - f.descent()
    }

    /// The name of the face in a slot, for the run record ([`Run::faces`]).
    pub fn face_name(&self, slot: u16) -> Option<&str> {
        self.faces.get(slot as usize).map(|f| f.name.as_str())
    }

    /// Every loaded face in slot order, bundled first.
    pub fn face_names(&self) -> Vec<&str> {
        (0..self.faces.len() as u16)
            .filter_map(|slot| self.face_name(slot))
            .collect()
    }

    /// Which slot draws `ch`, or `None` when no face on this host can.
    ///
    /// The bundled face is asked first and answers for nearly all text without
    /// touching the disk; only a miss consults — and, the first time, builds —
    /// the system database.
    pub fn face_for(&mut self, ch: char) -> Option<usize> {
        if self.faces[0].face.glyph_index(ch).is_some() {
            return Some(0);
        }
        if let Some(hit) = self.coverage.get(&ch) {
            return *hit;
        }
        let slot = self.resolve(ch);
        self.coverage.insert(ch, slot);
        slot
    }

    /// Finds and loads a face for `ch`, appending it as a new slot.
    ///
    /// A face already loaded is reused before anything is read: the second
    /// kana in `にほん` costs a `glyph_index` call, not a file.
    fn resolve(&mut self, ch: char) -> Option<usize> {
        if let Some(slot) = self
            .faces
            .iter()
            .position(|f| f.face.glyph_index(ch).is_some())
        {
            return Some(slot);
        }
        if self.system.is_none() {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            self.system = Some(db);
        }
        // The borrow of `system` is closed before `faces` grows: the bytes are
        // copied out here and everything after this block is owned.
        let picked = {
            let db = self.system.as_ref()?;
            let id = pick(db, ch)?;
            let info = db.faces().find(|f| f.id == id)?;
            let name = info
                .families
                .first()
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| info.post_script_name.clone());
            let index = info.index;
            let data = db.with_face_data(id, |bytes, _| bytes.to_vec())?;
            (name, data, index)
        };
        let (name, data, index) = picked;
        // Leaked on purpose, once per resolved face: `rustybuzz::Face` and
        // `FontRef` both borrow their bytes, the faces live as long as the
        // process, and a self-referential struct to say so would be unsafe code
        // bought for nothing. The bound is `PREFERRED.len()` fonts, not a leak
        // that grows with use.
        let bytes: &'static [u8] = Box::leak(data.into_boxed_slice());
        let face = rustybuzz::Face::from_slice(bytes, index)?;
        let font = FontRef::try_from_slice_and_index(bytes, index).ok()?;
        self.faces.push(Face { name, face, font });
        Some(self.faces.len() - 1)
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
    /// Text is split into maximal same-face segments and each is shaped with
    /// its own face (TD-79). A run of Latin is one segment against the bundled
    /// face, which is the case that has to stay exactly as fast and exactly as
    /// deterministic as it was.
    pub fn layout(&mut self, text: &str, px: f32, device_scale: f32) -> Run {
        let mut run = Run::default();
        let mut pen = 0.0f32;
        let mut start = 0usize;
        let mut current: Option<usize> = None;
        for (at, ch) in text.char_indices() {
            let slot = match self.face_for(ch) {
                Some(slot) => slot,
                None => {
                    run.unresolved += 1;
                    // Drawn from the bundled face, which spells it `.notdef`.
                    // A box is a worse answer than the character and a better
                    // one than a silently missing character.
                    0
                }
            };
            match current {
                Some(prev) if prev == slot => {}
                None => current = Some(slot),
                Some(prev) => {
                    self.shape(
                        &text[start..at],
                        start,
                        prev,
                        px,
                        device_scale,
                        &mut pen,
                        &mut run,
                    );
                    start = at;
                    current = Some(slot);
                }
            }
        }
        if let Some(slot) = current {
            self.shape(
                &text[start..],
                start,
                slot,
                px,
                device_scale,
                &mut pen,
                &mut run,
            );
        }
        run.width = pen;
        run
    }

    /// Shapes one same-face segment and appends it to `run`.
    ///
    /// `offset` is the segment's byte position in the whole string, because
    /// rustybuzz reports clusters relative to the buffer it was given and the
    /// caret indexes the *string* — getting this wrong would put the caret in
    /// the right place only for the first segment.
    #[allow(clippy::too_many_arguments)]
    fn shape(
        &mut self,
        text: &str,
        offset: usize,
        slot: usize,
        px: f32,
        device_scale: f32,
        pen: &mut f32,
        run: &mut Run,
    ) {
        if text.is_empty() {
            return;
        }
        if let Err(at) = run.faces.binary_search(&(slot as u16)) {
            run.faces.insert(at, slot as u16);
        }
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&self.faces[slot].face, &[], buffer);

        let upem = self.faces[slot].face.units_per_em() as f32;
        let scale = px / upem;
        for (info, pos) in shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions().iter())
        {
            let id = info.glyph_id as u16;
            run.clusters.push((info.cluster + offset as u32, *pen));
            if let Some(glyph) = self.rasterise(slot, id, px, device_scale) {
                run.glyphs.push(Placed {
                    glyph,
                    at: [
                        *pen + pos.x_offset as f32 * scale,
                        -(pos.y_offset as f32 * scale),
                    ],
                });
            }
            *pen += pos.x_advance as f32 * scale;
        }
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
    fn rasterise(&mut self, slot: usize, id: u16, px: f32, device_scale: f32) -> Option<Glyph> {
        let device_px = px * device_scale;
        let key = (slot as u16, id, device_px.to_bits());
        if let Some(hit) = self.cache.get(&key) {
            return *hit;
        }
        let glyph = ab_glyph::GlyphId(id).with_scale(device_px);
        let outlined = self.faces[slot].font.outline_glyph(glyph);
        let entry = outlined.and_then(|outlined| {
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

/// Picks the face that should draw `ch`, preferring [`PREFERRED`]'s order.
///
/// Two passes, and the split is the determinism argument (D-125):
///
/// 1. **The bundled preference list, in its order.** Every host that has the
///    same families installed reaches the same answer regardless of how its
///    font directory happens to be enumerated.
/// 2. Only if none of them covers `ch`, **every face on the host, in name
///    order** — sorted rather than in enumeration order, so even the fallback's
///    fallback is reproducible on a given machine. A host that lands here is
///    one nobody's preference list anticipated, and it gets a glyph instead of
///    a box.
///
/// Regular weight and upright style only: the grid has one style today
/// (`CELL_PX` is a single size for the same reason), and picking a bold face
/// for a codepoint because it happened to sort first would be visibly wrong.
fn pick(db: &fontdb::Database, ch: char) -> Option<fontdb::ID> {
    let covers = |id: fontdb::ID| -> bool {
        db.with_face_data(id, |bytes, index| {
            rustybuzz::Face::from_slice(bytes, index)
                .and_then(|f| f.glyph_index(ch))
                .is_some()
        })
        .unwrap_or(false)
    };
    let plain = |f: &&fontdb::FaceInfo| {
        f.style == fontdb::Style::Normal && f.weight == fontdb::Weight::NORMAL
    };
    for want in PREFERRED {
        let mut named: Vec<&fontdb::FaceInfo> = db
            .faces()
            .filter(plain)
            .filter(|f| {
                f.families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(want))
            })
            .collect();
        named.sort_by(|a, b| a.post_script_name.cmp(&b.post_script_name));
        if let Some(hit) = named.into_iter().find(|f| covers(f.id)) {
            return Some(hit.id);
        }
    }
    let mut all: Vec<&fontdb::FaceInfo> = db.faces().filter(plain).collect();
    all.sort_by(|a, b| a.post_script_name.cmp(&b.post_script_name));
    all.into_iter().find(|f| covers(f.id)).map(|f| f.id)
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

    /// Latin is still laid out by the bundled face alone — the half of docs/31's
    /// determinism rule that fallback must not cost (D-125).
    #[test]
    fn latin_is_shaped_entirely_from_the_bundled_face_and_says_so() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        let latin = engine.layout("Revenue", CELL_PX, 1.0);
        assert_eq!(
            latin.glyphs.len(),
            7,
            "every Latin letter must reach the atlas"
        );
        assert_eq!(
            latin.faces,
            vec![0],
            "slot 0 is the bundled face, and Latin must never leave it"
        );
        assert_eq!(latin.unresolved, 0);
        assert!(
            engine.system.is_none(),
            "and no system font was enumerated: a Latin workbook must not pay \
             for a fallback it never uses"
        );
    }

    /// The atlas key gained a face (**TD-79**), and this is why it had to.
    ///
    /// Host-independent by construction: instead of hoping the machine has a
    /// CJK font, it loads a *second bundled* face — `DejaVu Sans Mono`, which
    /// the same crate ships — and asks for the same glyph id from both. Mono
    /// and proportional draw the same character at different widths, so if the
    /// cache were still keyed by `(glyph, size)` the second face would be
    /// served the first face's picture and the text would be subtly, silently
    /// wrong. The old key is exactly what this test fails against.
    #[test]
    fn two_faces_do_not_share_an_atlas_entry_for_the_same_glyph_id() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        let mono: &'static [u8] = dejavu::sans_mono::regular();
        engine.faces.push(Face {
            name: String::from("DejaVu Sans Mono"),
            face: rustybuzz::Face::from_slice(mono, 0).expect("the mono face must load"),
            font: FontRef::try_from_slice(mono).expect("the mono face must load"),
        });

        // Glyph 50 is a letter in both faces and a different shape in each.
        let from_sans = engine.rasterise(0, 50, CELL_PX, 1.0).expect("sans glyph");
        let from_mono = engine.rasterise(1, 50, CELL_PX, 1.0).expect("mono glyph");
        assert_ne!(
            from_sans.uv, from_mono.uv,
            "each face must own its atlas entry, or one face draws the other's glyphs"
        );
        // And the cache still works within a face: asking twice allocates once.
        let again = engine.rasterise(0, 50, CELL_PX, 1.0).expect("sans glyph");
        assert_eq!(from_sans.uv, again.uv);
    }

    /// A run that changes face mid-string keeps its caret arithmetic (TD-79).
    ///
    /// Cluster offsets come back from rustybuzz relative to the buffer it was
    /// handed, and fallback hands it several — so a segment's offset has to be
    /// added back or the caret is right only inside the first segment. Built
    /// from two bundled faces again, so it proves the seam on any host.
    #[test]
    fn a_run_that_changes_face_keeps_its_cluster_offsets_in_string_space() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        let mono: &'static [u8] = dejavu::sans_mono::regular();
        engine.faces.push(Face {
            name: String::from("DejaVu Sans Mono"),
            face: rustybuzz::Face::from_slice(mono, 0).expect("the mono face must load"),
            font: FontRef::try_from_slice(mono).expect("the mono face must load"),
        });
        // Force the middle character onto the second face. `に` rather than a
        // Latin letter because the bundled face answers for anything it covers
        // without consulting the map at all — which is the fast path, and
        // reaching around it would be testing a different function. Three
        // bytes wide on purpose: a segment offset added back wrongly shows up
        // as `2` where the answer is `4`.
        engine.coverage.insert('に', Some(1));

        let run = engine.layout("aにc", CELL_PX, 1.0);
        assert_eq!(run.faces, vec![0, 1], "both faces are recorded, ascending");
        let offsets: Vec<u32> = run.clusters.iter().map(|(at, _)| *at).collect();
        assert_eq!(
            offsets,
            vec![0, 1, 4],
            "clusters index the string, not the segment"
        );
        let xs: Vec<f32> = run.clusters.iter().map(|(_, x)| *x).collect();
        assert!(
            xs[0] < xs[1] && xs[1] < xs[2] && xs[2] < run.width,
            "and the pen advances across the seam: {xs:?} within {}",
            run.width
        );
    }

    /// **TD-79, closed.** `にほん` draws three different characters.
    ///
    /// This replaces the test that asserted the opposite. That one shaped the
    /// same three kana and asserted all three sampled the *same* atlas rect —
    /// three `.notdef` boxes — and it was written to fail the day fallback
    /// landed. This is that day.
    ///
    /// The one honest caveat, stated in the test rather than in a register
    /// nobody re-reads: fallback cannot conjure a face the host does not have.
    /// A Windows or macOS install has CJK faces and this asserts the strong
    /// property; a stripped container may not, and there the assertion is that
    /// the engine *reports* the shortfall (`unresolved`) rather than drawing
    /// boxes and saying nothing. Both branches are a real property; neither is
    /// a skip.
    #[test]
    fn the_three_kana_of_nihon_are_three_different_glyphs() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        assert!(
            engine.faces[0].face.glyph_index('に').is_none(),
            "the premise: the bundled face still has no kana, so this can only \
             pass through fallback"
        );

        let kana = engine.layout("にほん", CELL_PX, 1.0);
        assert_eq!(kana.clusters.len(), 3, "three characters are shaped");
        assert!(kana.width > 0.0, "and they advance");

        if kana.unresolved > 0 {
            assert_eq!(
                (kana.unresolved, kana.faces.clone()),
                (3, vec![0]),
                "a host with no CJK face must say so for every character and \
                 fall back to the bundled `.notdef`, not report a partial success"
            );
            return;
        }

        assert_eq!(kana.glyphs.len(), 3, "three glyphs, and now three pictures");
        assert!(
            kana.glyphs
                .windows(2)
                .all(|w| w[0].glyph.uv != w[1].glyph.uv),
            "three distinct characters must not share an atlas rect — that was \
             the shape of TD-79 and it is what this test now forbids"
        );
        assert_ne!(kana.faces, vec![0], "and they came from a fallback face");
        let named = kana
            .faces
            .iter()
            .filter(|s| **s != 0)
            .filter_map(|s| engine.face_name(*s))
            .count();
        assert!(
            named > 0,
            "the resolved face must be nameable — that record is what makes a \
             layout divergence between two hosts explainable (D-125)"
        );
    }
}
