//! A scripted editing session, rendered to a PNG (docs/35 §evidence).
//!
//! # Why this exists
//! A window is the one part of the shell that cannot be asserted on in a unit
//! test, and "the tests pass" is not evidence that a user can edit a cell. This
//! drives the **same** [`App`] the window drives, through the **same** keymap,
//! and writes the frame that results — so the editing surface can be looked at,
//! and its per-step effects checked as text, on a machine nobody is sitting at.
//!
//! Every keystroke below goes through [`crate::input::translate`]. Nothing here
//! calls an editing method directly, because a script that bypassed the keymap
//! would prove the keymap works when it does not.

use usk_types::Value;

use crate::app::App;
use crate::gpu;
use crate::input::{self, Key, Mods};
use crate::png;
use crate::text;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// Types a string as a user would: each character through the keymap, so the
/// first one opens the editor and the rest insert.
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        let Some(intent) = input::translate(Key::Character(c), Mods::NONE, app.mode()) else {
            continue;
        };
        app.handle(intent);
    }
}

fn press(app: &mut App, key: Key, mods: Mods) {
    if let Some(intent) = input::translate(key, mods, app.mode()) {
        app.handle(intent);
    }
}

/// Clicks the cell at view ordinals `(row, col)`, which is what a user's mouse
/// does — the app hit-tests it back to identities.
fn click(app: &mut App, row: usize, col: usize) -> bool {
    let visible = app.visible();
    let (Some(r), Some(c)) = (
        visible.rows.iter().find(|s| s.index == row),
        visible.cols.iter().find(|s| s.index == col),
    ) else {
        return false;
    };
    let (ox, oy) = app.theme().grid_origin();
    let (x, y) = (ox + c.at + 4.0, oy + r.at + 4.0);
    app.pointer_down(x, y, false);
    true
}

/// Drags the fill handle from the current selection to `(row, col)` — press,
/// move, release, exactly as a mouse produces them.
fn drag_fill(app: &mut App, row: usize, col: usize) {
    let Some(handle) = app.fill_handle() else {
        return;
    };
    app.pointer_down(handle[0] + 3.0, handle[1] + 3.0, false);
    let (ox, oy) = app.theme().grid_origin();
    let visible = app.visible();
    let (Some(r), Some(c)) = (
        visible.rows.iter().find(|s| s.index == row),
        visible.cols.iter().find(|s| s.index == col),
    ) else {
        return;
    };
    app.pointer_drag(ox + c.at + 4.0, oy + r.at + 4.0);
    app.pointer_up();
}

fn source_at(app: &mut App, row: usize, col: usize) -> String {
    let (rows, cols) = app.axes();
    let (Some(r), Some(c)) = (rows.id_at(row), cols.id_at(col)) else {
        return String::from("<no such cell>");
    };
    app.source_at(usk_types::RowId(r), usk_types::ColId(c))
}

fn shown(app: &mut App, row: usize, col: usize) -> String {
    let (rows, cols) = app.axes();
    let (Some(r), Some(c)) = (rows.id_at(row), cols.id_at(col)) else {
        return String::from("<no such cell>");
    };
    match app.value(usk_types::RowId(r), usk_types::ColId(c)) {
        Some(Value::Blank) | None => String::from("<blank>"),
        Some(v) => text::render_value(&v).unwrap_or_default(),
    }
}

/// Runs the session and writes the final frame.
pub fn run(path: &str, rows: usize) -> Result<String, String> {
    let session = usk_reduce::Session::from_log(usk_types::ActorId(1), crate::workbook(rows, 40));
    let mut app = App::open(session, WIDTH as f32, HEIGHT as f32, 1.0)
        .ok_or("the bundled font failed to load, or the workbook has no cells")?;
    let mut log: Vec<String> = Vec::new();

    // The corpus fills columns A..L (values), N (a formula) and P/Q (errors).
    // The script works in R, which the corpus leaves empty, so what it writes
    // is unambiguously its own — and R is on screen at 1280 px wide, which
    // column U was not.
    const SCRATCH: usize = 17; // column R

    // 1 — a literal, typed over an empty cell and committed with Enter.
    if !click(&mut app, 0, SCRATCH) {
        return Err("the scratch column is not on screen".into());
    }
    type_text(&mut app, "125");
    press(&mut app, Key::Enter, Mods::NONE);
    log.push(format!(
        "typed 125 into R1        -> R1 = {}",
        shown(&mut app, 0, SCRATCH)
    ));

    // 2 — Enter advanced the cursor, so the next value needs no navigation.
    type_text(&mut app, "37.5");
    press(&mut app, Key::Enter, Mods::NONE);
    log.push(format!(
        "typed 37.5 into R2       -> R2 = {}",
        shown(&mut app, 1, SCRATCH)
    ));

    // 3 — a formula over the two, which must *compute* and not sit blank
    // (TD-61). Written in A1 terms, which is what a user types.
    type_text(&mut app, "=R1+R2");
    press(&mut app, Key::Enter, Mods::NONE);
    log.push(format!(
        "typed =R1+R2 into R3     -> R3 = {}",
        shown(&mut app, 2, SCRATCH)
    ));

    // 4 — editing a precedent must recalculate the dependent. This is the whole
    // point of wiring the engine in.
    click(&mut app, 0, SCRATCH);
    type_text(&mut app, "200");
    press(&mut app, Key::Enter, Mods::NONE);
    log.push(format!(
        "changed R1 to 200        -> R3 = {}  (recalculated)",
        shown(&mut app, 2, SCRATCH)
    ));

    // 5 — an error, rendered by name rather than as a blank.
    click(&mut app, 3, SCRATCH);
    type_text(&mut app, "=R1/0");
    press(&mut app, Key::Enter, Mods::NONE);
    log.push(format!(
        "typed =R1/0 into R4      -> R4 = {}",
        shown(&mut app, 3, SCRATCH)
    ));

    // 6 — text, and a formula over text, so the frame shows left alignment and
    // an error with a different name.
    click(&mut app, 4, SCRATCH);
    type_text(&mut app, "revenue");
    press(&mut app, Key::Enter, Mods::NONE);
    log.push(format!(
        "typed revenue into R5    -> R5 = {}",
        shown(&mut app, 4, SCRATCH)
    ));

    // 7 — F2 opens the cell's *source*, not its value: a formula cell must edit
    // as `=R1+R2` and not as `237.5`.
    click(&mut app, 2, SCRATCH);
    press(&mut app, Key::F2, Mods::NONE);
    let source = app
        .editor()
        .map(|e| e.text.clone())
        .unwrap_or_else(|| String::from("<no editor>"));
    log.push(format!(
        "F2 on R3                 -> editor holds {source:?}"
    ));

    // 8 — Escape abandons it, changing nothing.
    press(&mut app, Key::Escape, Mods::NONE);
    log.push(format!(
        "Escape                   -> R3 = {} (unchanged), editor {}",
        shown(&mut app, 2, SCRATCH),
        if app.editor().is_some() {
            "open"
        } else {
            "closed"
        }
    ));

    // 9 — undo walks back through the writes the reducer recorded.
    press(&mut app, Key::Character('z'), Mods::ctrl());
    log.push(format!(
        "Ctrl+Z                   -> R5 = {}",
        shown(&mut app, 4, SCRATCH)
    ));
    press(&mut app, Key::Character('y'), Mods::ctrl());
    log.push(format!(
        "Ctrl+Y                   -> R5 = {}",
        shown(&mut app, 4, SCRATCH)
    ));

    // 10 — a Shift-extended selection, so the frame shows the range wash with
    // the active cell left clear inside it.
    click(&mut app, 0, 12);
    press(&mut app, Key::Down, Mods::shift());
    press(&mut app, Key::Down, Mods::shift());
    press(&mut app, Key::Right, Mods::shift());
    log.push(format!(
        "Shift+arrows             -> selection {}",
        app.reference()
    ));

    // 11 — a formula filled down by dragging the handle: the gesture the whole
    // fill feature exists for.
    click(&mut app, 0, 14); // O1
    type_text(&mut app, "=A1+B1");
    press(&mut app, Key::Enter, Mods::NONE);
    click(&mut app, 0, 14);
    drag_fill(&mut app, 4, 14);
    log.push(format!(
        "filled O1 down to O5     -> O2 = {}, O5 = {}, and O5 now reads {}",
        shown(&mut app, 1, 14),
        shown(&mut app, 4, 14),
        source_at(&mut app, 4, 14)
    ));

    // 12 — copy a block, then read the **real OS clipboard** back. This is the
    // one part of the feature a unit test cannot reach: the tests use a
    // detached clipboard so the suite runs headless (and so one test cannot see
    // another's copy).
    click(&mut app, 0, SCRATCH);
    press(&mut app, Key::Down, Mods::shift());
    press(&mut app, Key::Down, Mods::shift());
    press(&mut app, Key::Character('c'), Mods::ctrl());
    let os_text = match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(text) => format!("{text:?}"),
        Err(err) => format!("<unavailable: {err}>"),
    };
    log.push(format!(
        "Ctrl+C on R1:R3          -> OS clipboard holds {os_text}"
    ));

    // 13 — and paste it back somewhere else.
    click(&mut app, 7, SCRATCH);
    press(&mut app, Key::Character('v'), Mods::ctrl());
    log.push(format!(
        "Ctrl+V at R8             -> R8 = {}, R9 = {}, R10 = {} from {}          (a formula, translated 7 rows - the in-process block survived the          round trip through the real OS clipboard)",
        shown(&mut app, 7, SCRATCH),
        shown(&mut app, 8, SCRATCH),
        shown(&mut app, 9, SCRATCH),
        source_at(&mut app, 9, SCRATCH)
    ));

    // 15 — a second frame for the gesture the first one cannot show. The fill
    // handle is suppressed while the editor is open (there is nothing to fill
    // from a cell that has not been committed), so a frame that shows the caret
    // cannot also show the handle. This one is captured **mid-drag**: selection
    // wash, handle, and the preview outline of what a release would fill —
    // which is the one part of the gesture no test asserts by eye.
    press(&mut app, Key::Escape, Mods::NONE);
    click(&mut app, 0, SCRATCH);
    press(&mut app, Key::Down, Mods::shift());
    press(&mut app, Key::Down, Mods::shift());
    if let Some(handle) = app.fill_handle() {
        app.pointer_down(handle[0] + 3.0, handle[1] + 3.0, false);
        let (ox, oy) = app.theme().grid_origin();
        let at = app
            .visible()
            .rows
            .iter()
            .find(|s| s.index == 6)
            .map(|s| s.at);
        if let Some(at) = at {
            app.pointer_drag(ox + (SCRATCH as f32) * 64.0 + 4.0, oy + at + 4.0);
        }
    }
    let mid_drag = app.frame();
    let fill_path = path.replace(".png", "-fill.png");
    log.push(format!(
        "mid-drag frame           -> {fill_path} (selection {}, preview to R7)",
        app.reference()
    ));
    // Abandon it, so the frame below is the committed state and not a document
    // this script quietly changed on its way out.
    app.pointer_drag(0.0, 0.0);
    app.drag_cancel();

    // 16 — a composition in flight (docs/33 §IME), which is the one state no
    // keystroke can produce: the platform sends it, and until now nothing in
    // this repo could show what it looks like. The composing text is **kana**,
    // which is the point: session 31 had to compose Latin here because the
    // bundled font drew `にほん` as three identical `.notdef` boxes (TD-79), and
    // a frame full of boxes proves nothing about the underline. With fallback
    // (D-125) the frame shows the characters, and the log names the face they
    // came from — which is the record that makes this frame reproducible-or-
    // explainable on another machine rather than merely believable on this one.
    click(&mut app, 8, 14);
    app.ime_preedit("にほん", Some((9, 9)));
    let composing = app.frame();
    let ime_path = path.replace(".png", "-ime.png");
    log.push(format!(
        "composing kana in O9     -> {ime_path} (editor holds {:?}, cell still {}, caret area {:?}, faces {:?})",
        app.editor().map(|e| e.text.clone()).unwrap_or_default(),
        shown(&mut app, 8, 14),
        app.ime_area().map(|a| (a[0], a[1])),
        app.resolved_faces(),
    ));
    // Escape drops the composition, a second one drops the edit — so the frame
    // below is the committed document and not one this script quietly changed.
    press(&mut app, Key::Escape, Mods::NONE);
    press(&mut app, Key::Escape, Mods::NONE);

    click(&mut app, 5, 18);
    type_text(&mut app, "=SUM(R1:R3)");

    let quads = app.frame();
    if app.atlas_overflowed() {
        return Err("the glyph atlas filled up".into());
    }
    let mut renderer = gpu::Renderer::headless()
        .ok_or("no GPU adapter is available on this machine (not a shell defect)")?;
    if let Some((size, bytes)) = app.take_atlas_upload() {
        renderer.upload_atlas(size, bytes);
    }
    let rgba = renderer.render_to_rgba(WIDTH, HEIGHT, &quads);
    std::fs::write(path, png::encode_rgba(WIDTH, HEIGHT, &rgba))
        .map_err(|e| format!("writing {path}: {e}"))?;
    let rgba = renderer.render_to_rgba(WIDTH, HEIGHT, &mid_drag);
    std::fs::write(&fill_path, png::encode_rgba(WIDTH, HEIGHT, &rgba))
        .map_err(|e| format!("writing {fill_path}: {e}"))?;
    let rgba = renderer.render_to_rgba(WIDTH, HEIGHT, &composing);
    std::fs::write(&ime_path, png::encode_rgba(WIDTH, HEIGHT, &rgba))
        .map_err(|e| format!("writing {ime_path}: {e}"))?;

    Ok(format!(
        "scripted editing session -> {path}  ({} quads, 1 draw call)\n  {}",
        quads.len(),
        log.join("\n  ")
    ))
}
