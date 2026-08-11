//! Scripted IME conformance for the three scripts docs/48 names (D-127).
//!
//! # Why this exists, and what it is careful not to claim
//! docs/48 requires *"IME validated by native JP/CN/KR typists"*. There is no
//! native typist here and there will not be one, so D-127 splits the item: the
//! half that is a property of **our** code is closed mechanically, and the half
//! that needs a person is published as a list rather than absorbed. This module
//! is the first half.
//!
//! What it replays is the **event shape** each script's input method produces —
//! the sequence of `Ime::Preedit` / `Ime::Commit` winit delivers for one real
//! word — and it asserts, at every step, the four things a user can see:
//! the display string, the caret's byte offset inside it, the span the
//! composition underline covers, and **that the cell is still untouched**. The
//! last is the one that matters most and the one no screenshot shows: a
//! composition is the input method's proposal, and a document that acquired
//! text from it would be wrong in a way nobody notices until it ships.
//!
//! # The three shapes, and why three
//! They are here because they differ. A suite that composed kana three times
//! would prove one shape thrice.
//!
//! * **JP** — kana per keystroke, then a *conversion* that replaces the whole
//!   composition with kanji. Multi-clause, so the focused clause arrives as a
//!   **selection range** in the cursor field rather than a caret.
//! * **CN** — Microsoft Pinyin, whose composition is **ASCII for most of its
//!   life** and becomes Han only at partial conversion. So a face seam opens
//!   *inside* a composition, with the caret sitting on it.
//! * **KR** — the Microsoft Korean IME composes **one syllable at a time**, and
//!   the keystroke that begins the next syllable *commits* the previous one. So
//!   `Commit` and `Preedit` interleave inside a single word, which is the shape
//!   most likely to break an editor that assumes a commit ends the composing.
//!
//! # Provenance, stated as a limit (D-127 decision 2)
//! Nobody here captured a TSF trace. These sequences are authored from
//! documented input-method behaviour. Where they may be wrong they are wrong
//! about *content* — whether Microsoft Pinyin shows `zhong'wen` or `zhong wen` —
//! and never about shape; content is on the typist's list in MEASUREMENTS.md
//! §W-IME-SCRIPTS.

use usk_types::{ColId, RowId, Value};

use crate::app::App;
use crate::gpu::{self, Quad};
use crate::input::{self, Key, Mods};
use crate::png;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// Column R, which the corpus leaves empty — so what a script writes is
/// unambiguously its own (the same column `script.rs` works in, for the same
/// reason).
const SCRATCH: usize = 17;

/// One event a platform input method delivers to the window.
///
/// The same two cases `winit::event::Ime` carries and that `window.rs` forwards
/// verbatim; named here so a script is a data table rather than a function.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// The composition as it currently stands, with the input method's own
    /// caret — or, during conversion, its focused **range** — inside it.
    Preedit(&'static str, Option<(usize, usize)>),
    /// The input method finalised this text. Note that finalising a
    /// *composition* is not committing an *edit*: the cell is still untouched.
    Commit(&'static str),
}

/// One event and everything a user could see after it.
///
/// Every field is what the *user* sees, not what a struct holds internally.
/// A test that asserted `Editor::preedit` would pass while the cell showed
/// something else entirely.
pub struct Step {
    pub event: Event,
    /// The whole string the cell shows — buffer with the composition spliced in.
    pub shown: &'static str,
    /// The caret's byte offset into `shown`.
    pub caret: usize,
    /// The byte span the composition underline covers, or `None` when nothing
    /// is composing.
    pub underline: Option<(usize, usize)>,
}

/// One script's whole session: what the user typed, what the platform sent, and
/// what the document holds afterwards.
pub struct Script {
    pub id: &'static str,
    pub language: &'static str,
    /// What a person actually pressed. This is the line a native typist reads
    /// first, and the one they would reject if the sequence below is not what
    /// their input method does.
    pub keys: &'static str,
    /// The view row this script writes into, so the final frame shows all three.
    pub row: usize,
    pub steps: &'static [Step],
    /// What the cell holds after Enter.
    pub committed: &'static str,
    /// Which step to photograph — the one a person should look at.
    pub frame_at: usize,
    /// Why that step, and what the picture is evidence of.
    pub note: &'static str,
}

/// **JP** — `kyouhahare` → `今日は晴れ`, converted in two clauses.
///
/// The conversion is the half session 31 never had: steps 1–5 are kana arriving
/// per keystroke, and step 6 onwards is MS-IME replacing the entire composition
/// with kanji while reporting *which clause has the focus* as a byte range.
/// Steps 6, 7 and 9 differ only in that range — same underline, and for 7 and 9
/// the same text as well — which is TD-84 visible as data rather than as prose.
#[rustfmt::skip]
const JP: &[Step] = &[
    Step { event: Event::Preedit("き", Some((3, 3))), shown: "き", caret: 3, underline: Some((0, 3)) },
    Step { event: Event::Preedit("きょ", Some((6, 6))), shown: "きょ", caret: 6, underline: Some((0, 6)) },
    Step { event: Event::Preedit("きょう", Some((9, 9))), shown: "きょう", caret: 9, underline: Some((0, 9)) },
    Step { event: Event::Preedit("きょうは", Some((12, 12))), shown: "きょうは", caret: 12, underline: Some((0, 12)) },
    Step { event: Event::Preedit("きょうははれ", Some((18, 18))), shown: "きょうははれ", caret: 18, underline: Some((0, 18)) },
    // Space: convert. The whole composition is replaced and the first clause
    // takes the focus — a *range*, not a caret.
    Step { event: Event::Preedit("今日は晴れ", Some((0, 9))), shown: "今日は晴れ", caret: 0, underline: Some((0, 15)) },
    // Right: the focus moves to the second clause. Nothing else changes, which
    // is precisely the problem TD-84 records.
    Step { event: Event::Preedit("今日は晴れ", Some((9, 15))), shown: "今日は晴れ", caret: 9, underline: Some((0, 15)) },
    // Space: the next candidate for the focused clause only.
    Step { event: Event::Preedit("今日はハレ", Some((9, 15))), shown: "今日はハレ", caret: 9, underline: Some((0, 15)) },
    Step { event: Event::Preedit("今日は晴れ", Some((9, 15))), shown: "今日は晴れ", caret: 9, underline: Some((0, 15)) },
    // Enter: finalise the composition. The cell is still untouched.
    Step { event: Event::Commit("今日は晴れ"), shown: "今日は晴れ", caret: 15, underline: None },
];

/// **CN** — Microsoft Pinyin, `zhongwen` → `中文`.
///
/// The composition is Latin for six of its eight steps, which is the difference
/// from JP that matters to this shell: those steps are laid out entirely from
/// the *bundled* face, and step 7 opens a face seam in the middle of a live
/// composition with the caret sitting on it (TD-79's cluster arithmetic, now
/// exercised through the IME path rather than by a synthetic two-face string).
#[rustfmt::skip]
const CN: &[Step] = &[
    Step { event: Event::Preedit("z", Some((1, 1))), shown: "z", caret: 1, underline: Some((0, 1)) },
    Step { event: Event::Preedit("zh", Some((2, 2))), shown: "zh", caret: 2, underline: Some((0, 2)) },
    Step { event: Event::Preedit("zho", Some((3, 3))), shown: "zho", caret: 3, underline: Some((0, 3)) },
    Step { event: Event::Preedit("zhon", Some((4, 4))), shown: "zhon", caret: 4, underline: Some((0, 4)) },
    Step { event: Event::Preedit("zhong", Some((5, 5))), shown: "zhong", caret: 5, underline: Some((0, 5)) },
    // The IME has recognised two syllables and shows its own separator.
    Step { event: Event::Preedit("zhong'we", Some((8, 8))), shown: "zhong'we", caret: 8, underline: Some((0, 8)) },
    Step { event: Event::Preedit("zhong'wen", Some((9, 9))), shown: "zhong'wen", caret: 9, underline: Some((0, 9)) },
    // Partial conversion: the first syllable becomes Han and the rest stays
    // pinyin. One composition, two scripts, caret on the boundary.
    Step { event: Event::Preedit("中wen", Some((3, 3))), shown: "中wen", caret: 3, underline: Some((0, 6)) },
    Step { event: Event::Commit("中文"), shown: "中文", caret: 6, underline: None },
];

/// **KR** — the Microsoft Korean IME, `hangeul` → `한글`.
///
/// One syllable is composed at a time and the keystroke that starts the next
/// one *commits* the previous — so step 4 is a `Commit` in the middle of a
/// word, and step 5's composition splices in after text the editor now holds.
/// Step 8 is a Backspace, which decomposes rather than deletes: the composition
/// **shrinks**, which every other script here only ever grows.
#[rustfmt::skip]
const KR: &[Step] = &[
    Step { event: Event::Preedit("ㅎ", Some((3, 3))), shown: "ㅎ", caret: 3, underline: Some((0, 3)) },
    Step { event: Event::Preedit("하", Some((3, 3))), shown: "하", caret: 3, underline: Some((0, 3)) },
    Step { event: Event::Preedit("한", Some((3, 3))), shown: "한", caret: 3, underline: Some((0, 3)) },
    // `g` finishes 한 and begins 글: a commit no key press asked for.
    Step { event: Event::Commit("한"), shown: "한", caret: 3, underline: None },
    Step { event: Event::Preedit("ㄱ", Some((3, 3))), shown: "한ㄱ", caret: 6, underline: Some((3, 6)) },
    Step { event: Event::Preedit("그", Some((3, 3))), shown: "한그", caret: 6, underline: Some((3, 6)) },
    Step { event: Event::Preedit("글", Some((3, 3))), shown: "한글", caret: 6, underline: Some((3, 6)) },
    // Backspace: the syllable loses its final jamo instead of vanishing.
    Step { event: Event::Preedit("그", Some((3, 3))), shown: "한그", caret: 6, underline: Some((3, 6)) },
    Step { event: Event::Preedit("글", Some((3, 3))), shown: "한글", caret: 6, underline: Some((3, 6)) },
    Step { event: Event::Commit("글"), shown: "한글", caret: 6, underline: None },
];

/// The three scripts docs/48 names, in its order.
pub const SCRIPTS: &[Script] = &[
    Script {
        id: "jp",
        language: "Japanese",
        keys: "k y o u h a h a r e, Space, Right, Space, Space, Enter",
        row: 0,
        steps: JP,
        committed: "今日は晴れ",
        // Step 7, the second clause focused: the state TD-84 says a typist
        // cannot see. The picture is the evidence *for* the debt row — five
        // characters under one undifferentiated underline.
        frame_at: 7,
        note: "conversion in flight, focus on the second clause",
    },
    Script {
        id: "cn",
        language: "Chinese (Simplified)",
        keys: "z h o n g w e n, Space, Enter",
        row: 2,
        steps: CN,
        committed: "中文",
        // Step 8, partial conversion: Han and pinyin in one composition, with
        // the caret on the face seam between them.
        frame_at: 8,
        note: "partial conversion — one composition across a face seam",
    },
    Script {
        id: "kr",
        language: "Korean",
        keys: "h a n g e u l, Backspace, l, Enter",
        row: 4,
        steps: KR,
        committed: "한글",
        // Step 7: the second syllable complete but still composing, after the
        // mid-word commit put the first one in the buffer. The underline
        // covering only the second character is the whole shape in one picture.
        frame_at: 7,
        note: "second syllable composing after a mid-word commit",
    },
];

/// Feeds one event to the app, exactly as `window.rs` does.
fn deliver(app: &mut App, event: Event) {
    match event {
        Event::Preedit(text, cursor) => app.ime_preedit(text, cursor),
        Event::Commit(text) => app.ime_commit(text),
    };
}

/// What the user sees: the display string, caret and underline span.
fn seen(app: &App) -> (String, usize, Option<(usize, usize)>) {
    match app.editor() {
        Some(editor) => editor.display(),
        None => (String::new(), 0, None),
    }
}

/// The cell's committed value, or `None` when it is still blank.
fn cell(app: &mut App, row: usize, col: usize) -> Option<String> {
    let (rows, cols) = app.axes();
    let (r, c) = (rows.id_at(row)?, cols.id_at(col)?);
    match app.value(RowId(r), ColId(c)) {
        Some(Value::Blank) | None => None,
        Some(value) => crate::text::render_value(&value),
    }
}

/// Which face a **fresh** engine picks for `text` — `PREFERRED`'s answer alone,
/// with no session history in front of it.
///
/// Reported beside the session's answer because the two can differ and the
/// difference names the cause. `TextEngine::resolve` reuses an **already
/// loaded** face before it consults the database at all, so in a session that
/// composed Japanese first, `中` may be drawn by the face the kana loaded rather
/// than by the one the preference list would have chosen. That is a different
/// defect from a preference list in the wrong order, and guessing which of the
/// two is in play is exactly the mistake this project keeps a rule about. Costs
/// one system-font enumeration per call, which is why only the driver does it.
fn isolated_faces(text: &str) -> (Vec<String>, u32) {
    let Some(mut engine) = crate::text::TextEngine::new() else {
        return (Vec::new(), 0);
    };
    let run = engine.layout(text, crate::text::CELL_PX, 1.0);
    let names = run
        .faces
        .iter()
        .filter_map(|slot| engine.face_name(*slot).map(String::from))
        .collect();
    (names, run.unresolved)
}

/// Puts the cursor on the script's cell the way a user does — a click, which
/// the app hit-tests back to identities.
fn select(app: &mut App, row: usize, col: usize) -> Result<(), String> {
    let visible = app.visible();
    let (Some(r), Some(c)) = (
        visible.rows.iter().find(|s| s.index == row),
        visible.cols.iter().find(|s| s.index == col),
    ) else {
        return Err(format!("cell ({row}, {col}) is not on screen"));
    };
    let (ox, oy) = app.theme().grid_origin();
    app.pointer_down(ox + c.at + 4.0, oy + r.at + 4.0, false);
    Ok(())
}

/// Replays one script and checks every step. `Err` names the first divergence.
///
/// Shared by the driver below and by the test suite, so the two can never drift
/// apart — the tests prove the semantics on a host with no CJK font at all, and
/// the driver adds the frames and the face record that a headless suite cannot
/// have. `shot` is where the driver asks for a frame; the suite passes `None`
/// and builds no scene at all.
pub fn replay(
    app: &mut App,
    script: &Script,
    mut shot: Option<&mut Option<Vec<Quad>>>,
) -> Result<Vec<String>, String> {
    let mut log = Vec::with_capacity(script.steps.len() + 1);
    for (n, step) in script.steps.iter().enumerate() {
        deliver(app, step.event);
        let (shown, caret, underline) = seen(app);
        if shown != step.shown || caret != step.caret || underline != step.underline {
            return Err(format!(
                "{} step {}: expected {:?} caret {} underline {:?}, saw {:?} caret {} underline {:?}",
                script.id,
                n + 1,
                step.shown,
                step.caret,
                step.underline,
                shown,
                caret,
                underline
            ));
        }
        if app.composing() != underline.is_some() {
            return Err(format!(
                "{} step {}: composing() says {} but the underline says {:?}",
                script.id,
                n + 1,
                app.composing(),
                underline
            ));
        }
        // The invariant no picture shows and the one that matters most: nothing
        // an input method proposes — not even something it *finalised* — is in
        // the document until the user commits the edit.
        if let Some(early) = cell(app, script.row, SCRATCH) {
            return Err(format!(
                "{} step {}: the cell already holds {early:?}; a composition is \
                 not an edit",
                script.id,
                n + 1
            ));
        }
        let kind = match step.event {
            Event::Preedit(text, cursor) => format!("preedit {text:?} cursor {cursor:?}"),
            Event::Commit(text) => format!("COMMIT  {text:?}"),
        };
        log.push(format!(
            "      {:>2}  {kind:<38} shown {shown:?} caret {caret} underline {underline:?}",
            n + 1
        ));
        // Photographed here rather than after the replay, so the picture is of
        // a live composition and not of a document that has already settled.
        if n + 1 == script.frame_at {
            if let Some(slot) = shot.as_deref_mut() {
                *slot = Some(app.frame());
            }
        }
    }

    if let Some(intent) = input::translate(Key::Enter, Mods::NONE, app.mode()) {
        app.handle(intent);
    }
    match cell(app, script.row, SCRATCH).as_deref() {
        Some(text) if text == script.committed => {
            log.push(format!("      {:>2}  {:<38} cell  {text:?}", "->", "ENTER"));
            Ok(log)
        }
        other => Err(format!(
            "{}: Enter should have written {:?}, cell holds {other:?}",
            script.id, script.committed
        )),
    }
}

/// Runs all three scripts, checks them, and writes one frame each (D-127).
///
/// Fails loudly rather than printing a report with a divergence buried in it:
/// a driver whose output has to be read carefully is not evidence.
pub fn run(dir: &str, rows: usize) -> Result<String, String> {
    let session = usk_reduce::Session::from_log(usk_types::ActorId(1), crate::workbook(rows, 40));
    // `App::open` and not `open_cold`: a real session warms the font database,
    // and a CJK script that resolved through a cold engine would be measuring
    // and photographing a path no user is on (D-126).
    let mut app = App::open(session, WIDTH as f32, HEIGHT as f32, 1.0)
        .ok_or("the bundled font failed to load, or the workbook has no cells")?;

    let mut report = Vec::new();
    let mut frames = Vec::new();
    for script in SCRIPTS {
        select(&mut app, script.row, SCRATCH)?;
        let mut shot = None;
        let log = replay(&mut app, script, Some(&mut shot))?;
        // Asked *after* the replay and of the committed string, so the answer
        // is the face that actually drew this script's characters rather than
        // every face the session has ever loaded (TD-83 is only visible here).
        let (faces, unresolved) = app.faces_for(script.committed);
        let (alone, _) = isolated_faces(script.committed);
        let path = format!("{dir}/ime-{}.png", script.id);
        report.push(format!(
            "  {}  {} — {}\n      keys        {}\n{}\n      faces       \
             in session {faces:?}, alone {alone:?}, {unresolved} unresolved\
             \n      frame       {path}  (at step {})",
            script.id.to_uppercase(),
            script.language,
            script.note,
            script.keys,
            log.join("\n"),
            script.frame_at,
        ));
        match shot {
            Some(scene) => frames.push((path, scene)),
            None => return Err(format!("{}: no frame was captured", script.id)),
        }
    }

    if app.atlas_overflowed() {
        return Err("the glyph atlas filled up".into());
    }
    let mut renderer = gpu::Renderer::headless()
        .ok_or("no GPU adapter is available on this machine (not a shell defect)")?;
    // After every scene is built, because building one is what rasterises its
    // glyphs — an upload between frames would leave the earlier ones sampling
    // an atlas that did not yet hold their text.
    if let Some((size, bytes)) = app.take_atlas_upload() {
        renderer.upload_atlas(size, bytes);
    }
    for (path, scene) in &frames {
        let rgba = renderer.render_to_rgba(WIDTH, HEIGHT, scene);
        std::fs::write(path, png::encode_rgba(WIDTH, HEIGHT, &rgba))
            .map_err(|e| format!("writing {path}: {e}"))?;
    }

    Ok(format!(
        "W-IME-SCRIPTS (D-127, docs/33 §IME, docs/48) - scripted composition, \
         {} scripts, {} steps, all checked\n{}\n  \
         what this does NOT establish: docs/48 asks for validation by native \
         typists, and the event sequences above are authored from documented IME \
         behaviour, not captured from one. The residue is the checklist in \
         MEASUREMENTS.md §W-IME-SCRIPTS.",
        SCRIPTS.len(),
        SCRIPTS.iter().map(|s| s.steps.len()).sum::<usize>(),
        report.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use usk_oplog::{Anchor, Op, OpLog, Payload};
    use usk_reduce::Session;
    use usk_types::{ActorId, OpId};

    /// A bare sheet — the scripts write into empty cells and need nothing else.
    fn empty_log(rows: usize, cols: usize) -> OpLog {
        let mut log = OpLog::new();
        let actor = ActorId(1);
        let (mut counter, mut lamport) = (0u64, 0u64);
        let mut push = |log: &mut OpLog, payload: Payload| -> OpId {
            counter += 1;
            lamport += 1;
            let id = OpId { actor, counter };
            log.append(Op {
                id,
                lamport,
                payload,
            });
            id
        };
        let mut anchor = Anchor::Start;
        for _ in 0..cols {
            anchor = Anchor::After(push(&mut log, Payload::InsertCol { anchor }));
        }
        let mut anchor = Anchor::Start;
        for _ in 0..rows {
            anchor = Anchor::After(push(&mut log, Payload::InsertRow { anchor }));
        }
        log
    }

    fn app() -> App {
        let session = Session::from_log(ActorId(1), empty_log(20, 24));
        App::open_detached(session, 1280.0, 800.0, 1.0).expect("the bundled font must load")
    }

    /// Every script, replayed end to end with every step checked.
    ///
    /// **Host-independent on purpose.** Not one assertion here is about a glyph,
    /// a face or a pixel: a machine with no CJK font installed draws `한글` as
    /// two boxes and must still pass, because what is under test is what this
    /// shell does with the platform's events, not what the platform's fonts
    /// contain. The glyphs are `text.rs`'s subject and the frames are the
    /// driver's.
    #[test]
    fn every_script_composes_and_commits_exactly_as_its_input_method_dictates() {
        for script in SCRIPTS {
            let mut app = app();
            select(&mut app, script.row, SCRATCH).expect("the scratch column is on screen");
            replay(&mut app, script, None).unwrap_or_else(|why| panic!("{why}"));
        }
    }

    /// The mid-word `Commit` is Korean's alone, and it is the shape most likely
    /// to break an editor (D-127).
    ///
    /// A commit that ended the *edit* rather than the *composition* would put
    /// `한` in the cell and drop `글` on the floor — and every other script here
    /// would still pass, because no other script commits before Enter.
    #[test]
    fn a_korean_syllable_commit_mid_word_does_not_end_the_edit() {
        let script = SCRIPTS
            .iter()
            .find(|s| s.id == "kr")
            .expect("the Korean script");
        let mut app = app();
        select(&mut app, script.row, SCRATCH).expect("the scratch column is on screen");
        for step in &script.steps[..4] {
            deliver(&mut app, step.event);
        }
        // The syllable is in the *buffer* and the editor is still open.
        assert!(!app.composing(), "the first syllable finalised");
        assert_eq!(
            app.editor().map(|e| e.text.as_str()),
            Some("한"),
            "a mid-word commit belongs in the edit buffer"
        );
        assert_eq!(
            cell(&mut app, script.row, SCRATCH),
            None,
            "and not in the document: the user has not committed the edit"
        );
        // And the next composition splices in *after* it rather than replacing
        // it — the failure that would silently lose every syllable but the last.
        deliver(&mut app, script.steps[4].event);
        assert_eq!(seen(&app).0, "한ㄱ");
    }

    /// A conversion reports its focused clause as a *range*, and two steps that
    /// differ only in that range are two different states (D-127, TD-84).
    ///
    /// This test is written to pass on today's behaviour and to *record* what
    /// today's behaviour is: the caret moves, the underline does not. When TD-84
    /// is paid the second assertion changes, and it should — a test that passes
    /// before and after a fix proves nothing about the fix.
    #[test]
    fn a_multi_clause_conversion_moves_the_caret_but_cannot_show_the_focus() {
        let mut app = app();
        select(&mut app, 0, SCRATCH).expect("the scratch column is on screen");
        app.ime_preedit("今日は晴れ", Some((0, 9)));
        let first = seen(&app);
        app.ime_preedit("今日は晴れ", Some((9, 15)));
        let second = seen(&app);
        assert_eq!(first.0, second.0, "the same text is being converted");
        assert_ne!(
            first.1, second.1,
            "the caret must follow the focused clause, or arrow keys look dead"
        );
        assert_eq!(
            (first.2, second.2),
            (Some((0, 15)), Some((0, 15))),
            "TD-84: the underline spans the whole composition in both states, \
             so the focused clause is invisible. Change this when TD-84 is paid."
        );
    }

    /// A composition that a script leaves in flight must never reach the cell,
    /// however far through the script it got (D-127).
    ///
    /// Belt and braces over `replay`'s per-step check: this one abandons at
    /// every possible point rather than only running to the end, which is what a
    /// user does when they change their mind.
    #[test]
    fn abandoning_at_any_point_in_any_script_leaves_the_cell_blank() {
        for script in SCRIPTS {
            for stop in 1..=script.steps.len() {
                let mut app = app();
                select(&mut app, script.row, SCRATCH).expect("on screen");
                for step in &script.steps[..stop] {
                    deliver(&mut app, step.event);
                }
                // Escape drops the composition, and a second drops the edit.
                for _ in 0..2 {
                    if let Some(intent) = input::translate(Key::Escape, Mods::NONE, app.mode()) {
                        app.handle(intent);
                    }
                }
                assert_eq!(
                    cell(&mut app, script.row, SCRATCH),
                    None,
                    "{} abandoned after step {stop} wrote to the document",
                    script.id
                );
            }
        }
    }
}
