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
//! # Where the enumeration happens, and why it is not on the frame (TD-80)
//! Finding a system face means asking the host what it has, and `fontdb` answers
//! that by reading and parsing every face file it can find — **203–321 ms of the
//! 238–412 ms first miss on M1, 379 faces** (W-FALLBACK, profiled before this
//! was built). That is 15–25× docs/31's 16 ms keystroke→paint budget, landing on
//! exactly one keystroke: the first non-Latin character a session ever shows.
//!
//! So the database is built on a background thread ([`TextEngine::warm`]) and
//! handed over a channel, overlapping the seconds between launch and that
//! keystroke. Three properties make this safe rather than merely fast:
//!
//! * **The lazy path survives.** A miss that arrives before anyone warmed builds
//!   the database inline, exactly as it did before — so a `TextEngine` used
//!   without an `App` around it (every test, every benchmark) still resolves.
//! * **A miss that arrives mid-scan blocks on the hand-over**, which is what it
//!   did before too. The change can be neutral or better, never worse.
//! * **`warm` is not called by [`TextEngine::new`]**, but by `App::open` — 118
//!   tests and every benchmark construct an engine, and warming there would have
//!   all of them spawn a 300 ms file scan they never use.
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
use std::sync::mpsc;

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
///
/// **Coverage breadth is the wrong question once the character is drawn**, which
/// is TD-83 and why [`TextLang`] exists: this list stops at `Yu Gothic` for
/// Chinese text because Yu Gothic *covers* it, and a Chinese reader is shown
/// Japanese glyph forms. This list is not reordered to fix that (D-129) — a
/// language prefixes its own families onto it, and this order remains exactly
/// what an unsignalled run gets.
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

/// The language the display layer believes it is drawing (D-129, TD-83).
///
/// # Why a language and not a script
/// Han unification: `中` is Unicode script `Han` in Japanese, in both Chinese
/// traditions and in Korean, so the codepoint cannot say which glyph forms a
/// reader expects. 直, 骨 and the 门-class characters are drawn visibly
/// differently by the Japanese and Simplified-Chinese traditions, and to a
/// native reader the wrong one looks like a foreign font rather than like their
/// language. Nothing short of a language answers this — which is also why Noto
/// Sans CJK, the most pan-Unicode face there is, ships as four language
/// variants.
///
/// # Why a closed enum and not a BCP-47 string
/// The only thing this layer *does* with the value is order a list of font
/// families ([`preferred_for`]). A string would invite tag canonicalisation,
/// locale matching and a fallback chain — a second locale library — for a switch
/// with five arms. Adding an arm is an ordinary code change: this is display
/// state, so DP-A5's "op semantics are permanent" does not reach it.
///
/// # Where it may and may not live (DP-D5)
/// *"Locale never enters storage or evaluation."* A `TextLang` is set on a
/// [`TextEngine`], read only by the font search, and is never encoded, hashed,
/// carried in an op or written to a snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextLang {
    /// No signal. The answer for every run the window lays out today, and the
    /// case [`preferred_for`] must leave **exactly** as it was (D-129).
    #[default]
    Und,
    Ja,
    /// Simplified Chinese — the tradition TD-83 was found against.
    ZhHans,
    /// Traditional Chinese. Included because omitting it would be a known gap
    /// rather than an unknown one; **untested on M1**, which has no Traditional
    /// face, so its list is asserted and not measured.
    ZhHant,
    Ko,
}

impl TextLang {
    /// The families asked for **before** [`PREFERRED`], in this language's own
    /// order of preference.
    ///
    /// A prefix and not a replacement (D-129): everything in `PREFERRED` is
    /// still asked for afterwards, so a host whose only CJK face is one this
    /// list never names resolves exactly as it does today. The only thing a
    /// language changes is the order in which families are tried.
    ///
    /// Each list is that language's own tradition: the platform's native UI
    /// face for the script, the Noto variant for it, and the older bundled
    /// families that predate them — because a host that has none of the first
    /// two very often has the third.
    fn prefix(self) -> &'static [&'static str] {
        match self {
            TextLang::Und => &[],
            TextLang::Ja => &[
                "Noto Sans CJK JP",
                "Noto Sans JP",
                "Yu Gothic",
                "Meiryo",
                "MS Gothic",
                "Hiragino Sans",
            ],
            TextLang::ZhHans => &[
                "Noto Sans CJK SC",
                "Noto Sans SC",
                "Microsoft YaHei",
                "SimSun",
                "SimHei",
                "PingFang SC",
            ],
            TextLang::ZhHant => &[
                "Noto Sans CJK TC",
                "Noto Sans TC",
                "Microsoft JhengHei",
                "PMingLiU",
                "MingLiU",
                "PingFang TC",
            ],
            TextLang::Ko => &[
                "Noto Sans CJK KR",
                "Noto Sans KR",
                "Malgun Gothic",
                "Batang",
                "Gulim",
                "Apple SD Gothic Neo",
            ],
        }
    }

    /// The BCP-47-ish tag this language reports itself as, for driver output and
    /// for the font record. Display only — nothing parses it back.
    pub fn tag(self) -> &'static str {
        match self {
            TextLang::Und => "und",
            TextLang::Ja => "ja",
            TextLang::ZhHans => "zh-Hans",
            TextLang::ZhHant => "zh-Hant",
            TextLang::Ko => "ko",
        }
    }
}

/// The whole family search order for `lang`: its own families, then
/// [`PREFERRED`], with any family named twice kept at its **first** position.
///
/// `preferred_for(TextLang::Und)` is `PREFERRED` element for element, and that
/// identity is the control every assertion about this change is checked against
/// (D-129 decision 2). Built rather than stored because it is consulted once per
/// codepoint the bundled face misses — at most a few dozen strings, against a
/// file read.
pub fn preferred_for(lang: TextLang) -> Vec<&'static str> {
    let prefix = lang.prefix();
    let mut order: Vec<&'static str> = Vec::with_capacity(prefix.len() + PREFERRED.len());
    for want in prefix.iter().chain(PREFERRED.iter()) {
        if !order.iter().any(|have| have.eq_ignore_ascii_case(want)) {
            order.push(want);
        }
    }
    order
}

/// Where `name` sits in `order`, or `order.len()` for a family it does not name.
///
/// Case-insensitively, because a font family's name is compared that way
/// everywhere else in this file and a host that spells it `MS Gothic` where the
/// list says `Ms Gothic` is not a different host.
fn rank_in(order: &[&str], name: &str) -> usize {
    order
        .iter()
        .position(|want| want.eq_ignore_ascii_case(name))
        .unwrap_or(order.len())
}

/// How many families must be re-asked before an **already loaded** face called
/// `name` is allowed to answer for a new codepoint (D-129 decision 3).
///
/// [`TextEngine::resolve`] reuses a loaded face before it consults the database,
/// which is what keeps the second kana in `にほん` from costing a file read. Under
/// a language that shortcut acquires a second way to produce TD-83: in a
/// document holding both scripts, whichever language was typed first loads its
/// face and the other inherits it. So the reuse is allowed only after the
/// families ahead of it **in this language's own prefix** have been asked.
///
/// Bounded by the prefix rather than by the whole order, deliberately:
///
/// * Under [`TextLang::Und`] the prefix is empty, so this is `0` and the
///   shortcut behaves exactly as it did before this existed — the control.
/// * Once the language's best *present* family is loaded, what remains ahead of
///   it are families the host does not have, which cost a name filter over
///   metadata `fontdb` already holds and parse no faces at all.
/// * Bounding by the whole order would re-walk up to nineteen families for every
///   distinct new character, parsing each present one to test coverage — TD-82's
///   16.8–21.3 ms per character, forever, to serve a host nobody's preference
///   list anticipated.
fn recheck_before_reuse(lang: TextLang, order: &[&str], name: &str) -> usize {
    rank_in(order, name).min(lang.prefix().len())
}

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

/// Where the system font database is in its life (TD-80).
///
/// Three states rather than an `Option` because *"not built yet"* and *"being
/// built by a thread that is not finished"* are different obligations: the first
/// must build inline, the second must wait for the hand-over. Collapsing them
/// into `None` is how a warmed engine ends up enumerating twice.
enum SystemFonts {
    /// Nothing has been started. A miss builds the database inline — the
    /// pre-TD-80 behaviour, kept because it is what a `TextEngine` without an
    /// `App` around it needs.
    Cold,
    /// A warm-up thread is building it; the receiver is the hand-over. A miss
    /// that arrives first blocks here, which is what it did before TD-80.
    Warming(mpsc::Receiver<fontdb::Database>),
    /// Built and owned. Every further miss is a lookup.
    Ready(fontdb::Database),
}

/// Reads and parses every face the host will admit to.
///
/// The expensive call in this file, isolated so that the inline path and the
/// warm-up thread are demonstrably running the same thing (W-FALLBACK).
fn load_system() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
}

/// One loaded face: what shapes with it, what rasterises from it, and what it
/// is called.
///
/// The name is the record ([`Run::faces`]) **and, since D-129, an input**: it is
/// what [`recheck_before_reuse`] ranks to decide whether an already-loaded face
/// may answer for a new codepoint under the current language.
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
    /// can. Memoised because resolving costs file reads.
    ///
    /// Keyed by the codepoint alone and **not** by `(lang, char)`, because
    /// [`TextEngine::lang`] is one document-wide value (D-129 decision 4): there
    /// is never more than one language's answers in here at a time, because
    /// [`TextEngine::set_lang`] empties it. A `(lang, char)` key is what per-run
    /// language would need, and is written down in TD-85 rather than built
    /// speculatively now.
    coverage: HashMap<char, Option<usize>>,
    /// The language the font search believes it is serving (D-129).
    ///
    /// [`TextLang::Und`] until something says otherwise, which is what the
    /// window does today — so the shipped default is the pre-D-129 behaviour
    /// exactly, and the machinery is inert until a signal arrives.
    lang: TextLang,
    /// The system font database: absent, arriving on a thread, or here.
    ///
    /// Never built by [`TextEngine::new`]. `load_system_fonts` reads several
    /// hundred files, and a Latin-only workbook — which is every one the
    /// benchmarks open — must not pay for a fallback it never uses; docs/31's
    /// 1.0 s cold-launch budget is measured on exactly that path. [`Self::warm`]
    /// moves the cost off the frame without moving it onto launch.
    system: SystemFonts,
    /// How many times the database was built **inline, on the calling thread** —
    /// the path TD-80 exists to avoid.
    ///
    /// A counter rather than a flag because the interesting assertion is
    /// *"a warmed engine never built one"*, and zero is the only value that
    /// says so. Not `cfg(test)`: a field that only exists under test is a field
    /// whose bookkeeping the shipped build does not run.
    lazy_builds: u32,
    /// How many warm-up threads this engine has started.
    ///
    /// Exists because "warm is idempotent" is otherwise unobservable: a second
    /// `warm` that spawned a second scan and then handed over the *second*
    /// database would look identical from the outside and cost twice as much.
    /// This is the number that catches it.
    warm_spawns: u32,
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
            lang: TextLang::Und,
            system: SystemFonts::Cold,
            lazy_builds: 0,
            warm_spawns: 0,
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

    /// Starts building the system font database on a background thread (TD-80).
    ///
    /// Called by `App::open` and by nothing else — see the module note on why
    /// not [`TextEngine::new`]. **Idempotent**: an engine already warming or
    /// already warm is left alone, so calling it twice costs one scan, not two.
    ///
    /// Failure is silent *and correct*: if the thread cannot be spawned the
    /// engine stays [`SystemFonts::Cold`], which is the pre-TD-80 behaviour, and
    /// a later miss still resolves inline. A shell that cannot start a thread
    /// must still draw kana (DP-A10 — no panic across the boundary).
    pub fn warm(&mut self) {
        if !matches!(self.system, SystemFonts::Cold) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(String::from("ehkatra-font-warm"))
            .spawn(move || {
                // The receiver may already be gone if the process is shutting
                // down; the scan's result is then simply dropped.
                let _ = tx.send(load_system());
            });
        if spawned.is_ok() {
            self.warm_spawns += 1;
            self.system = SystemFonts::Warming(rx);
        }
    }

    /// Takes delivery of a warm-up that has finished, **without blocking**.
    ///
    /// `true` once the database is in hand. `false` while a warm-up is still
    /// running, and also `false` for an engine nobody warmed — "not warm" and
    /// "never asked" are the same answer to this question.
    ///
    /// Exists for the benchmark (`--fonts`), which has to know when the overlap
    /// this whole change buys has actually elapsed before it times the miss that
    /// follows it. Nothing in the frame path calls it: a real miss uses the
    /// blocking hand-over, because blocking is the right answer when the glyph
    /// is needed now.
    pub fn poll_warm(&mut self) -> bool {
        match &self.system {
            SystemFonts::Ready(_) => true,
            SystemFonts::Cold => false,
            SystemFonts::Warming(rx) => match rx.try_recv() {
                Ok(db) => {
                    self.system = SystemFonts::Ready(db);
                    true
                }
                Err(mpsc::TryRecvError::Empty) => false,
                // The thread died without sending. Fall back to the lazy path
                // rather than wait forever for a hand-over that is not coming.
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.system = SystemFonts::Cold;
                    false
                }
            },
        }
    }

    /// How many times the database was built inline on the calling thread.
    ///
    /// The number TD-80 is about: a warmed engine must resolve with this at
    /// **0**, and an unwarmed one reaches 1 and stays there.
    pub fn lazy_builds(&self) -> u32 {
        self.lazy_builds
    }

    /// The language the font search is serving (D-129).
    pub fn lang(&self) -> TextLang {
        self.lang
    }

    /// Tells the font search what language it is drawing (D-129, TD-83).
    ///
    /// **Forgets every resolution made under the previous language**, because
    /// that is exactly what changed: `中` answered `Yu Gothic` under
    /// [`TextLang::Und`] and must be free to answer `Microsoft YaHei` under
    /// [`TextLang::ZhHans`]. Loaded faces are *not* discarded — a slot is only a
    /// loaded file, and re-reading one the search may well choose again would
    /// pay TD-80's price for nothing.
    ///
    /// A no-op when the language is unchanged, so a caller that sets it every
    /// frame does not throw the cache away every frame.
    pub fn set_lang(&mut self, lang: TextLang) {
        if self.lang == lang {
            return;
        }
        self.lang = lang;
        self.coverage.clear();
    }

    /// How many codepoints the coverage cache currently answers for.
    ///
    /// Exists so that *"a language change forgets what the last one resolved"*
    /// is observable at all — the same reason [`TextEngine::lazy_builds`] and
    /// [`TextEngine::warm_spawns`] are counters rather than private state. Not
    /// `cfg(test)`: a field the shipped build does not maintain is a field the
    /// tests prove nothing about.
    pub fn cached_resolutions(&self) -> usize {
        self.coverage.len()
    }

    /// How many warm-up threads this engine has started.
    ///
    /// At most 1 for an engine's whole life, and that is the assertion
    /// [`TextEngine::warm`]'s idempotence claim reduces to.
    pub fn warm_spawns(&self) -> u32 {
        self.warm_spawns
    }

    /// The database, waiting for the warm-up or building it inline.
    ///
    /// This is the whole of TD-80's hand-over. Three arrivals, one exit:
    /// warming → block on the channel; cold → build here and count it; ready →
    /// hand it back.
    fn system_db(&mut self) -> Option<&fontdb::Database> {
        if let SystemFonts::Warming(rx) = &self.system {
            // Blocks only when the user beat the scan — precisely what a miss
            // did before TD-80, so this path is never worse than it was.
            self.system = match rx.recv() {
                Ok(db) => SystemFonts::Ready(db),
                // The warm-up thread is gone; rebuild inline below.
                Err(_) => SystemFonts::Cold,
            };
        }
        if matches!(self.system, SystemFonts::Cold) {
            self.lazy_builds += 1;
            self.system = SystemFonts::Ready(load_system());
        }
        match &self.system {
            SystemFonts::Ready(db) => Some(db),
            _ => None,
        }
    }

    /// Whether this engine has consulted the host's fonts at all.
    ///
    /// A Latin-only session must answer `false` — that is the property the
    /// bundled face's `cmap` buys, and the one docs/31's cold-launch budget is
    /// measured on.
    pub fn enumerated(&self) -> bool {
        !matches!(self.system, SystemFonts::Cold)
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
    /// A face already loaded is reused before anything is read: the second kana
    /// in `にほん` costs a `glyph_index` call, not a file. Under a language that
    /// reuse is **conditional** (D-129 decision 3) — the families ahead of the
    /// loaded one in the language's own prefix are asked first, so a session
    /// that loaded `Yu Gothic` for kana does not go on to answer Chinese with
    /// it. Under [`TextLang::Und`] the condition is empty and this is the same
    /// function it was before.
    fn resolve(&mut self, ch: char) -> Option<usize> {
        let order = preferred_for(self.lang);
        // The best-ranked loaded face that can draw `ch` at all, and how much of
        // the language's prefix still outranks it. `min` over `(rank, slot)`
        // rather than the first hit: two loaded faces may both cover `ch`, and
        // the language's opinion of them is the whole point.
        let loaded = self
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.face.glyph_index(ch).is_some())
            .map(|(slot, f)| (recheck_before_reuse(self.lang, &order, &f.name), slot))
            .min();
        if let Some((0, slot)) = loaded {
            return Some(slot);
        }
        // The borrow of `system` is closed before `faces` grows: the bytes are
        // copied out here and everything after this block is owned.
        let picked = {
            // A host whose database cannot be built keeps whatever is already
            // loaded rather than losing a glyph it can currently draw: the
            // language is a *preference*, and failing to improve on a face is
            // not a reason to stop having one (DP-A10).
            let Some(db) = self.system_db() else {
                return loaded.map(|(_, slot)| slot);
            };
            let found = match loaded {
                // Something already loaded can draw it, so only a family the
                // language ranks *ahead* of that one is worth a file read, and
                // the exhaustive last-resort pass is not run at all — there is
                // nothing left for it to rescue.
                Some((limit, slot)) => match pick_in(db, ch, &order[..limit]) {
                    Some(id) => id,
                    None => return Some(slot),
                },
                // Nothing loaded covers it: the full search, exactly as before.
                None => pick(db, ch, &order)?,
            };
            let id = found;
            let info = db.faces().find(|f| f.id == id)?;
            // `pick_in` only ever returns a face that *covers* `ch`, and any
            // loaded face covering `ch` is already in `loaded` — so a face
            // returned here outranks all of them and cannot be one of them. No
            // slot is ever loaded twice.
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

/// Regular weight and upright style only: the grid has one style today
/// (`CELL_PX` is a single size for the same reason), and picking a bold face
/// for a codepoint because it happened to sort first would be visibly wrong.
fn plain(f: &&fontdb::FaceInfo) -> bool {
    f.style == fontdb::Style::Normal && f.weight == fontdb::Weight::NORMAL
}

/// The first face in `families`' order that can draw `ch`, or `None`.
///
/// Split out of [`pick`] because D-129 needs the *bounded* half on its own: an
/// already-loaded face may be superseded only by a family the language ranks
/// ahead of it, and running the exhaustive pass in that case would be reaching
/// past the answer we already have.
///
/// Within one family, faces are taken in **PostScript-name order** rather than
/// enumeration order, so a host with several cuts of a family reaches the same
/// one on every run.
fn pick_in(db: &fontdb::Database, ch: char, families: &[&str]) -> Option<fontdb::ID> {
    let covers = |id: fontdb::ID| -> bool {
        db.with_face_data(id, |bytes, index| {
            rustybuzz::Face::from_slice(bytes, index)
                .and_then(|f| f.glyph_index(ch))
                .is_some()
        })
        .unwrap_or(false)
    };
    for want in families {
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
    None
}

/// Picks the face that should draw `ch`, preferring `order`'s order.
///
/// Two passes, and the split is the determinism argument (D-125):
///
/// 1. **The bundled preference list, in its order** — [`preferred_for`] of the
///    current language, which for [`TextLang::Und`] is [`PREFERRED`] itself.
///    Every host that has the same families installed reaches the same answer
///    regardless of how its font directory happens to be enumerated.
/// 2. Only if none of them covers `ch`, **every face on the host, in name
///    order** — sorted rather than in enumeration order, so even the fallback's
///    fallback is reproducible on a given machine. A host that lands here is
///    one nobody's preference list anticipated, and it gets a glyph instead of
///    a box.
fn pick(db: &fontdb::Database, ch: char, order: &[&str]) -> Option<fontdb::ID> {
    if let Some(hit) = pick_in(db, ch, order) {
        return Some(hit);
    }
    let covers = |id: fontdb::ID| -> bool {
        db.with_face_data(id, |bytes, index| {
            rustybuzz::Face::from_slice(bytes, index)
                .and_then(|f| f.glyph_index(ch))
                .is_some()
        })
        .unwrap_or(false)
    };
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
            !engine.enumerated(),
            "and no system font was enumerated: a Latin workbook must not pay \
             for a fallback it never uses"
        );
        assert_eq!(
            engine.lazy_builds(),
            0,
            "nor was one built inline on the frame's thread"
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

    /// **TD-80.** A warmed engine resolves without ever building a database on
    /// the calling thread.
    ///
    /// This asserts the *structure*, not the speed, and deliberately so. The
    /// tempting test — "warm, sleep a little, check the resolve was fast" — is
    /// a race dressed as an assertion: it passes on an idle laptop and fails on
    /// a loaded CI box, and DP-C5 calls that a defect in the test. What TD-80
    /// actually claims is that the enumeration happens *somewhere else*, and
    /// `lazy_builds` is exactly that claim as a number.
    ///
    /// Host-independent: `poll_warm` waits for the hand-over whatever the host's
    /// font count is, and the assertion afterwards holds whether or not this
    /// machine has a face for `に` — a host with none still resolves *through
    /// the warmed database* and still never builds one inline.
    ///
    /// It fails if `warm` is removed, which is the point: an unwarmed engine
    /// reaches `lazy_builds == 1` on the same line.
    #[test]
    fn a_warmed_engine_resolves_without_building_a_database_on_the_frame() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        assert!(!engine.enumerated(), "an engine starts cold");
        engine.warm();
        assert_eq!(
            engine.warm_spawns(),
            1,
            "the warm-up thread must be running"
        );

        // Wait for the hand-over rather than for a duration: a sleep long
        // enough here is a sleep that fails on a loaded box (DP-C5).
        while !engine.poll_warm() {
            assert!(
                engine.enumerated(),
                "the warm-up thread died without handing anything over"
            );
            std::thread::yield_now();
        }

        let _ = engine.layout("にほん", CELL_PX, 1.0);
        assert_eq!(
            engine.lazy_builds(),
            0,
            "a warmed engine must consume the thread's database, not build its \
             own — the whole of TD-80 is that this number stays 0"
        );
        assert_eq!(engine.warm_spawns(), 1, "and one scan, not two");
    }

    /// The lazy path survives TD-80: an engine nobody warmed still resolves.
    ///
    /// The failure this forbids is the plausible one — moving the build to
    /// `warm` and leaving `resolve` waiting for a hand-over that, for the 118
    /// tests and every benchmark that never call `warm`, never comes. That
    /// engine must enumerate inline exactly as it did before, and be able to
    /// say that it did.
    #[test]
    fn an_engine_nobody_warmed_still_enumerates_inline_exactly_once() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        let _ = engine.layout("にほん", CELL_PX, 1.0);
        assert!(engine.enumerated(), "the miss must have consulted the host");
        assert_eq!(
            engine.lazy_builds(),
            1,
            "and built the database on this thread, because nobody warmed it"
        );
        // A second miss reuses it: the cost is once per process, not per glyph.
        let _ = engine.layout("한글", CELL_PX, 1.0);
        assert_eq!(
            engine.lazy_builds(),
            1,
            "a database, once built, is never rebuilt"
        );
    }

    /// `warm` is idempotent, and warming after the lazy build is a no-op.
    ///
    /// Two ways this could have gone wrong and both cost a 300 ms file scan:
    /// a second `warm` spawning a second thread, and a `warm` arriving after a
    /// miss already built the database throwing that database away.
    #[test]
    fn warming_twice_and_warming_late_both_cost_nothing() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        engine.warm();
        engine.warm();
        assert_eq!(
            engine.warm_spawns(),
            1,
            "warming an engine that is already warming must not start a second \
             scan of several hundred files"
        );

        while !engine.poll_warm() {
            assert!(engine.enumerated(), "the warm-up thread died");
            std::thread::yield_now();
        }

        // Late: the database is already in hand, so this must not throw it away
        // and go and read every font file again.
        engine.warm();
        assert_eq!(
            engine.warm_spawns(),
            1,
            "warming an engine that is already warm must do nothing at all"
        );
        let _ = engine.layout("にほん", CELL_PX, 1.0);
        assert_eq!(engine.lazy_builds(), 0);
    }

    /// Every language this shell knows about, so a new arm cannot be added
    /// without the properties below being asserted of it.
    const EVERY: &[TextLang] = &[
        TextLang::Und,
        TextLang::Ja,
        TextLang::ZhHans,
        TextLang::ZhHant,
        TextLang::Ko,
    ];

    /// An unsignalled run asks for **exactly** the families it always did
    /// (D-129 decision 2).
    ///
    /// The control the whole change is checked against. Every run the window
    /// lays out today is `Und`, so if this drifts, D-129 stopped being a
    /// language feature and became a font-list rewrite.
    #[test]
    fn a_run_with_no_language_asks_for_exactly_the_families_it_always_did() {
        assert_eq!(
            preferred_for(TextLang::Und),
            PREFERRED.to_vec(),
            "Und must be PREFERRED element for element"
        );
    }

    /// A language reorders the search and never shortens it (D-129 decision 2).
    ///
    /// Two halves, and the second is the one that would catch a careless edit:
    /// no family is *lost* (a host whose only CJK face is one the prefix never
    /// names must still resolve), and the families a language did not promote
    /// keep `PREFERRED`'s relative order among themselves.
    #[test]
    fn a_language_prefix_reorders_the_search_without_removing_anything_from_it() {
        for lang in EVERY {
            let order = preferred_for(*lang);
            for want in PREFERRED {
                assert!(
                    order.iter().any(|have| have.eq_ignore_ascii_case(want)),
                    "{}: {want} fell out of the search entirely",
                    lang.tag()
                );
            }
            let promoted = lang.prefix();
            let kept: Vec<&str> = order
                .iter()
                .copied()
                .filter(|f| !promoted.iter().any(|p| p.eq_ignore_ascii_case(f)))
                .collect();
            let expected: Vec<&str> = PREFERRED
                .iter()
                .copied()
                .filter(|f| !promoted.iter().any(|p| p.eq_ignore_ascii_case(f)))
                .collect();
            assert_eq!(
                kept,
                expected,
                "{}: the families it did not promote changed order among \
                 themselves",
                lang.tag()
            );
        }
    }

    /// No family is asked for twice, however many lists name it.
    ///
    /// A duplicate is not a correctness bug — the second ask can only repeat the
    /// first — but it is a *parsed face* per repeat on a host that has it, which
    /// is TD-82's 16.8–21.3 ms spent to learn nothing.
    #[test]
    fn no_family_is_asked_for_twice_in_any_languages_search() {
        for lang in EVERY {
            let order = preferred_for(*lang);
            for (n, family) in order.iter().enumerate() {
                assert!(
                    !order[..n]
                        .iter()
                        .any(|seen| seen.eq_ignore_ascii_case(family)),
                    "{}: {family} is asked for twice",
                    lang.tag()
                );
            }
        }
    }

    /// The defect and the thing that must not become the defect, in one test
    /// (TD-83, D-129).
    ///
    /// A Simplified-Chinese reader must reach a Chinese face before a Japanese
    /// one — **and** a Japanese reader must still reach the Japanese one first,
    /// which is precisely what reordering `PREFERRED` would have destroyed. The
    /// third assertion is the honest one: under `Und` the old order survives
    /// unchanged, defect and all, because a run with no signal has given us
    /// nothing to prefer with.
    #[test]
    fn chinese_outranks_japanese_for_a_chinese_reader_and_the_reverse_still_holds() {
        let rank = |lang: TextLang, family: &str| rank_in(&preferred_for(lang), family);

        assert!(
            rank(TextLang::ZhHans, "Microsoft YaHei") < rank(TextLang::ZhHans, "Yu Gothic"),
            "a Chinese reader must not be served Japanese glyph forms"
        );
        assert!(
            rank(TextLang::Ja, "Yu Gothic") < rank(TextLang::Ja, "Microsoft YaHei"),
            "and the fix must not simply choose a different victim"
        );
        assert!(
            rank(TextLang::Und, "Yu Gothic") < rank(TextLang::Und, "Microsoft YaHei"),
            "an unsignalled run keeps the coverage-breadth order it always had"
        );
        assert!(
            rank(TextLang::Ko, "Malgun Gothic") < rank(TextLang::Ko, "Yu Gothic"),
            "Korean was right by accident before; it must be right on purpose now"
        );
    }

    /// An already-loaded face is reconsidered against its language's own prefix
    /// and never against the generic list (D-129 decision 3).
    ///
    /// This is the arithmetic that decides whether a session which loaded
    /// `Yu Gothic` for kana goes on to answer Chinese with it, and it is also
    /// the arithmetic that decides whether every distinct character costs a
    /// re-walk. Three cases, and each is a different consequence:
    #[test]
    fn an_already_loaded_face_is_reconsidered_only_against_its_own_language() {
        let und = preferred_for(TextLang::Und);
        assert_eq!(
            recheck_before_reuse(TextLang::Und, &und, "Yu Gothic"),
            0,
            "with no language there is nothing to prefer, so the shortcut must \
             behave exactly as it did before D-129 — the control"
        );

        let sc = preferred_for(TextLang::ZhHans);
        assert_eq!(
            recheck_before_reuse(TextLang::ZhHans, &sc, "Yu Gothic"),
            TextLang::ZhHans.prefix().len(),
            "a Japanese face loaded in a Chinese document must be re-checked \
             against the whole Chinese prefix — this is TD-83's second route"
        );
        assert_eq!(
            recheck_before_reuse(TextLang::ZhHans, &sc, "Microsoft YaHei"),
            rank_in(&sc, "Microsoft YaHei"),
            "and once the language's own face is loaded only the families \
             ahead of it are re-asked, which on any host are the ones it does \
             not have"
        );
        assert!(
            recheck_before_reuse(TextLang::ZhHans, &sc, "Microsoft YaHei")
                < TextLang::ZhHans.prefix().len(),
            "so the steady state is strictly cheaper than the first miss"
        );
    }

    /// Changing the language forgets what the previous one resolved (D-129).
    ///
    /// Host-independent: on a machine with no CJK face at all `中` resolves to
    /// `None`, and *that* answer is cached too — which is the point. A cache
    /// that survived the language change would keep answering `Yu Gothic` for a
    /// document that has just said it is Chinese, and no amount of correct
    /// ordering downstream would ever be consulted.
    #[test]
    fn changing_the_language_forgets_what_the_previous_one_resolved() {
        let mut engine = TextEngine::new().expect("the bundled font must load");
        assert_eq!(engine.lang(), TextLang::Und, "the default is no signal");
        assert_eq!(engine.cached_resolutions(), 0);

        let _ = engine.face_for('中');
        assert_eq!(
            engine.cached_resolutions(),
            1,
            "a resolution — hit or miss — is memoised, or every character pays \
             a file read"
        );

        engine.set_lang(TextLang::ZhHans);
        assert_eq!(engine.lang(), TextLang::ZhHans);
        assert_eq!(
            engine.cached_resolutions(),
            0,
            "the previous language's answers must not outlive it"
        );

        // Idempotent: a caller that sets the language every frame must not throw
        // the cache away every frame.
        let _ = engine.face_for('中');
        let before = engine.cached_resolutions();
        engine.set_lang(TextLang::ZhHans);
        assert_eq!(
            engine.cached_resolutions(),
            before,
            "setting the language it already has must cost nothing"
        );
    }

    /// Latin is untouched by any of this (D-129 decision 2).
    ///
    /// The regression that would matter most and be noticed least: the language
    /// machinery sits in `resolve`, and `resolve` is only reached for a
    /// codepoint the bundled face misses. A Latin run must still be slot 0
    /// alone, and must still enumerate nothing, under every language.
    #[test]
    fn a_latin_run_is_bundled_and_enumerates_nothing_under_every_language() {
        for lang in EVERY {
            let mut engine = TextEngine::new().expect("the bundled font must load");
            engine.set_lang(*lang);
            let run = engine.layout("Revenue 1,234.56", CELL_PX, 1.0);
            assert_eq!(
                run.faces,
                vec![0],
                "{}: Latin left the bundled face",
                lang.tag()
            );
            assert_eq!(run.unresolved, 0, "{}", lang.tag());
            assert!(
                !engine.enumerated(),
                "{}: a Latin-only session read the host's fonts",
                lang.tag()
            );
        }
    }
}
