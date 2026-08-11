//! The Ehkatra desktop shell (ADR-037, docs/25, docs/31, docs/33).
//!
//! One application, several ways in:
//!
//! * no arguments — **opens a window** and presents the grid. This is the
//!   product (ADR-037).
//! * `--render <path> [rows]` renders one frame offscreen and writes a PNG,
//!   so the renderer is verifiable on a machine with no display and `demo/`
//!   has deterministic evidence.
//! * `--script <path> [rows]` drives a scripted keyboard/mouse session through
//!   the **same** [`app::App`] the window drives and writes the resulting
//!   frame. Editing is inspectable as an image without a person at a keyboard.
//! * `--bench [rows]` — W-SCROLL, an offscreen scroll frame.
//! * `--present [frames]` — W-PRESENT, a **presented** scroll frame, which the
//!   offscreen path cannot measure because it pays for a readback instead.
//! * `--keystroke [rows]` — W-KEYSTROKE, keystroke → paint.
//! * `--open [rows]` — W-OPEN-SHELL, the cost of opening, split by phase.
//!
//! **Nothing here is mocked.** The workbook is built by applying real ops to a
//! real `usk_state::State` through a real `usk_reduce::Session`, the visible
//! rows and columns come from `usk_view::Viewport`, values come from
//! `usk_calc::Engine`, and every edit is a `Command` the reducer compiles to
//! ops. A shell that lied about its data would be worse than no shell.

mod app;
mod clipboard;
mod fill;
mod gpu;
mod input;
mod png;
mod scene;
mod script;
mod text;
mod window;

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_reduce::Session;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

use app::App;

/// The window's default document. Small enough that launching is instant
/// (docs/31: *desktop cold launch → blank workbook < 1.0 s*) and large enough
/// that the scroll is a real virtual scroll rather than one screen of cells.
const WINDOW_ROWS: usize = 5_000;
const COLS: usize = 40;

fn main() {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("--render") => {
            let path = args.next().unwrap_or_else(|| "grid.png".into());
            let rows = parse_rows(args.next(), 200_000);
            render_to_file(&path, rows)
        }
        Some("--script") => {
            let path = args.next().unwrap_or_else(|| "script.png".into());
            let rows = parse_rows(args.next(), 200);
            script::run(&path, rows)
        }
        Some("--bench") => bench(parse_rows(args.next(), 1_000_000)),
        // W-KEYSTROKE (docs/31): keystroke -> paint. The budget the editing
        // surface has to meet, and the one an offscreen scroll bench says
        // nothing about.
        Some("--keystroke") => keystroke(parse_rows(args.next(), 850)),
        // W-OPEN-SHELL (TD-66): what opening a workbook costs, split by phase,
        // because "opening is slow" is a complaint and "the graph build is 80%
        // of it and superlinear" is a work item.
        Some("--open") => {
            let rows = parse_rows(args.next(), 20_000);
            // `dense` fills every row instead of skipping every third. The
            // difference is the experiment TD-66 needed: the gap is what
            // defeats `extent_of`'s rect merge.
            let dense = args.next().as_deref() == Some("dense");
            open_cost(rows, dense)
        }
        // W-PRESENT (TD-60): the *presented* frame cost, which the offscreen
        // path cannot measure because it pays for a readback instead of a
        // present. Opens a real window against the real compositor.
        Some("--present") => {
            let frames = parse_rows(args.next(), 240);
            open_window(WINDOW_ROWS, Some(frames))
        }
        // W-FALLBACK (TD-79): what the first codepoint the bundled font cannot
        // draw costs, and which face the host resolves it to. Both halves are
        // the point — the cost, because enumerating system fonts reads several
        // hundred files and must never sit on the launch path; the *name*,
        // because a layout that came from a system face is only reproducible
        // between two machines that agree on which face it was (D-125).
        Some("--fonts") => fonts(),
        Some(other) => Err(format!(
            "unknown argument {other:?}; try --render <path>, --script <path>, --bench, \
             --present, --fonts, or no arguments to open a window"
        )),
        None => open_window(WINDOW_ROWS, None),
    };
    match result {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("ehkatra-shell: {err}");
            std::process::exit(1);
        }
    }
}

fn parse_rows(arg: Option<String>, default: usize) -> usize {
    arg.and_then(|v| v.parse().ok()).unwrap_or(default)
}

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

fn open_window(rows: usize, frames: Option<usize>) -> Result<String, String> {
    let opened = std::time::Instant::now();
    let session = Session::from_log(ActorId(1), workbook(rows, COLS));
    let app = App::open(session, WIDTH as f32, HEIGHT as f32, 1.0)
        .ok_or("the bundled font failed to load, or the workbook has no cells")?;
    // Cold launch, measured rather than assumed (docs/31: < 1.0 s). Printed on
    // exit with the frame report, because printing before the window opens
    // would time the wrong thing.
    let launch = opened.elapsed();
    let shell = window::Shell::new(app);
    let shell = match frames {
        Some(n) => shell.measuring(n),
        None => shell,
    };
    let report = shell.run()?;
    Ok(format!(
        "{report}\n           document        {rows} rows x {COLS} cols\n           \
         open to first frame  {launch:?}  (budget 1.0 s, docs/31)"
    ))
}

/// W-FALLBACK (TD-79, D-125): the cost and the outcome of font fallback.
///
/// Three numbers, in the order a frame pays them:
///
/// * **Latin** — a shaped Latin run, to show the ordinary path is untouched and
///   that it enumerates nothing.
/// * **first miss** — the one expensive event: building the system database and
///   picking a face. It happens once per process and only if a codepoint the
///   bundled font cannot draw is ever shown.
/// * **after** — the same script again, which must be back at the Latin cost,
///   because a resolved face is cached and a second kana is a `glyph_index`
///   call.
///
/// **TD-80 added a fourth and fifth**, and they are what says whether the fix
/// worked: how long the background warm-up takes to finish, and what the first
/// miss costs once it has. The second is the first miss *minus the enumeration*
/// — the pick alone — and if it is not, the hand-over is not happening.
fn fonts() -> Result<String, String> {
    use std::time::Instant;
    let mut engine = text::TextEngine::new().ok_or("the bundled font failed to load")?;

    let t = Instant::now();
    let latin = engine.layout("Revenue 1,234.56", text::CELL_PX, 1.0);
    let latin_cost = t.elapsed();

    let t = Instant::now();
    let first = engine.layout("にほん", text::CELL_PX, 1.0);
    let first_cost = t.elapsed();

    let t = Instant::now();
    let again = engine.layout("にほんご", text::CELL_PX, 1.0);
    let again_cost = t.elapsed();

    // The split, because "the first miss is slow" is a complaint and "the
    // enumeration is N% of it" is a work item — this project is five-for-five
    // on the named cause not being the measured one. A second, independent
    // database so the number is the enumeration alone and not a cache effect.
    let t = Instant::now();
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let enumerate = t.elapsed();
    let faces = db.faces().count();

    // TD-80: the same first miss on an engine `App::open` warmed. A second
    // engine rather than the one above, because the one above has already
    // resolved and would answer from its cache — which measures nothing.
    //
    // `poll_warm` rather than a sleep: the wait is itself a number worth having
    // (it is how much of the user's first seconds the scan actually needs), and
    // a fixed sleep would either overstate it or measure a half-finished scan.
    let mut warm = text::TextEngine::new().ok_or("the bundled font failed to load")?;
    let t = Instant::now();
    warm.warm();
    while warm.enumerated() && !warm.poll_warm() {
        std::thread::yield_now();
    }
    let warm_wait = t.elapsed();

    let t = Instant::now();
    let warm_first = warm.layout("にほん", text::CELL_PX, 1.0);
    let warm_first_cost = t.elapsed();

    Ok(format!(
        "W-FALLBACK (TD-79, D-125, TD-80) - font fallback on this host\n           \
         latin run         {latin_cost:?}   faces {:?}, {} unresolved\n           \
         first miss COLD   {first_cost:?}   faces {:?}, {} unresolved  \
         (system enumeration + pick, on the frame)\n           \
         same script again {again_cost:?}   faces {:?}, {} unresolved\n           \
           of which enum   {enumerate:?}   ({faces} faces on this host)\n           \
         warm-up wait      {warm_wait:?}   (background thread, App::open starts it)\n           \
         first miss WARM   {warm_first_cost:?}   faces {:?}, {} unresolved  \
         (the pick alone - TD-80's whole claim is that this is the cold number \
         minus the enumeration)\n           \
         inline builds     cold {} / warm {}   (0 on the warm engine or the \
         hand-over is not happening; {} warm-up thread started)\n           \
         loaded faces      {:?}\n           \
         preference order is a constant in text.rs; only which of them the host \
         has installed varies",
        latin.faces,
        latin.unresolved,
        first.faces,
        first.unresolved,
        again.faces,
        again.unresolved,
        warm_first.faces,
        warm_first.unresolved,
        engine.lazy_builds(),
        warm.lazy_builds(),
        warm.warm_spawns(),
        engine.face_names(),
    ))
}

/// W-OPEN-SHELL (TD-66): the cost of opening a workbook, by phase.
///
/// Uses the kernel's public constructors directly rather than
/// `Session::from_log`, which is the same three calls with no way to time them
/// apart. docs/31 budgets *"cold open 1M-cell workbook (skeleton+viewport)
/// < 1.5 s"* — and the point of the split is that the skeleton is the cheap
/// part and the graph is not.
fn open_cost(rows: usize, dense: bool) -> Result<String, String> {
    use std::time::Instant;
    use usk_calc::Engine;
    use usk_state::State;
    use usk_types::coerce::Profile;

    let log = workbook_with(rows, COLS, dense);
    let ops = log.ops().len();

    let t = Instant::now();
    let state = State::replay(&log);
    let replay = t.elapsed();

    let t = Instant::now();
    let mut engine = Engine::build(&state, Profile::Compat);
    let graph = t.elapsed();
    let groups = engine.group_count();

    let t = Instant::now();
    let stats = engine.recalc_all(&state);
    let recalc = t.elapsed();

    // The skeleton and viewport alone — what docs/31's 1.5 s actually budgets.
    let t = Instant::now();
    let metrics = usk_view::Metrics::default();
    let row_ids: Vec<OpId> = state.row_order().into_iter().map(|r| r.0).collect();
    let col_ids: Vec<OpId> = state.col_order().into_iter().map(|c| c.0).collect();
    let row_axis = usk_view::Axis::build(&row_ids, |o| metrics.row_height(RowId(o)));
    let col_axis = usk_view::Axis::build(&col_ids, |o| metrics.col_width(ColId(o)));
    let axes = t.elapsed();
    let skeleton = replay + axes;
    let total = skeleton + graph + recalc;
    let (visible_rows, visible_cols) = (row_axis.len(), col_axis.len());

    let corpus = if dense {
        "dense (a formula in every row)"
    } else {
        "gapped (two rows in three)"
    };
    Ok(format!(
        "W-OPEN-SHELL (TD-66) - {rows} rows x {COLS} cols, {ops} ops, \
         {} formulas in {groups} group(s)
           corpus          {corpus}
           axis            {visible_rows} rows x {visible_cols} cols live
           replay          {replay:?}    (log -> State; linear)
           axis build      {axes:?}    (prefix sums; TD-58 rebuilds these O(n))
           = skeleton      {skeleton:?}    <- what docs/31's 1.5 s cold-open budget names
           graph build     {graph:?}    (parse + regroup every formula; TD-19)
           full recalc     {recalc:?}    ({} cells over {} levels)
           = total         {total:?}
           note            three fixes, each found by measuring rather than by
                           reading: TD-66 (the graph build was quadratic on a
                           gapped sheet), TD-23 (results keyed by identity) and
                           TD-20 (the band index scanned every rectangle a
                           candidate group owned, not the ones near the query).
                           This corpus at 1M rows went 218 s -> ~12 s in total,
                           and its full recalc 16.1 -> ~3.7 s. What remains is
                           that both phases still run *in front of the window*;
                           docs/25 would put them behind a generation shimmer",
        stats.evaluated_cells, stats.evaluated_cells, stats.levels,
    ))
}

/// W-KEYSTROKE (docs/31): *keystroke → paint, 10k sheet, < 16 ms*, and
/// *keystroke → paint including a 10k-cell recalc, < 50 ms*.
///
/// Measured as the whole round trip a user feels: the intent goes through the
/// keymap, the reducer compiles it to ops, the session folds and recalculates,
/// and a complete frame is built. Rendering is offscreen so no vsync wait is
/// counted — the budget is about the *work*, and a frame that misses it is a
/// dropped frame whatever the display is doing.
fn keystroke(rows: usize) -> Result<String, String> {
    use crate::input::{Key, Mods};
    use std::time::Instant;

    // ~12 cells per row in the corpus, so the row count sets the cell count.
    let cells = rows * 12;
    let session = Session::from_log(ActorId(1), workbook(rows, COLS));
    let mut app = App::open(session, WIDTH as f32, HEIGHT as f32, 1.0)
        .ok_or("the bundled font failed to load, or the workbook has no cells")?;

    let mut typing = Vec::new();
    let mut committing = Vec::new();
    // Split, so a breach names a layer rather than the whole round trip.
    let mut model = Vec::new();
    let mut paint = Vec::new();
    const N: usize = 40;
    for i in 0..N {
        // A character into the open editor: the pure typing path, which must
        // never be slower than a frame.
        let t = Instant::now();
        if let Some(intent) = input::translate(Key::Character('7'), Mods::NONE, app.mode()) {
            app.handle(intent);
        }
        let _ = app.frame();
        typing.push(t.elapsed().as_secs_f64() * 1000.0);

        // Enter: commit through the reducer, fold the log, recalculate, repaint.
        // This is the expensive keystroke and the one the budget is about.
        let t = Instant::now();
        if let Some(intent) = input::translate(Key::Enter, Mods::NONE, app.mode()) {
            app.handle(intent);
        }
        let edit = t.elapsed().as_secs_f64() * 1000.0;
        let t2 = Instant::now();
        let _ = app.frame();
        paint.push(t2.elapsed().as_secs_f64() * 1000.0);
        model.push(edit);
        committing.push(t.elapsed().as_secs_f64() * 1000.0);
        let _ = i;
    }

    // The IME path (docs/33 §IME). A composition update is a keystroke and
    // carries the same 16 ms budget: the user is typing, and the input method
    // repaints the cell on every key of a word before committing any of it.
    // Measured because latency is the one thing about IME quality obtainable
    // without a native JP/CN/KR typist — which docs/48 still requires, and
    // which this number does not replace.
    const KANA: [&str; 4] = ["に", "にほ", "にほん", "にほんご"];
    let mut composing = Vec::new();
    for i in 0..N {
        let t = Instant::now();
        app.ime_preedit(KANA[i % KANA.len()], None);
        let _ = app.frame();
        composing.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    app.ime_preedit("", None);

    let pct = |v: &mut Vec<f64>, p: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() as f64 - 1.0) * p) as usize]
    };
    Ok(format!(
        "W-KEYSTROKE (docs/31) - {rows} rows x {COLS} cols (~{cells} cells), {N} edits
           type a character  p50 {:.3} ms   p95 {:.3} ms   (edit buffer + repaint)
           IME composition   p50 {:.3} ms   p95 {:.3} ms   (preedit + repaint, docs/33)
           commit with Enter p50 {:.3} ms   p95 {:.3} ms   (reduce + fold + recalc + repaint)
             of which model  p50 {:.3} ms                    (reduce + fold + recalc)
             of which paint  p50 {:.3} ms                    (viewport + scene)
           budget            16 ms keystroke->paint; 50 ms incl. a 10k-cell recalc",
        pct(&mut typing, 0.50),
        pct(&mut typing, 0.95),
        pct(&mut composing, 0.50),
        pct(&mut composing, 0.95),
        pct(&mut committing, 0.50),
        pct(&mut committing, 0.95),
        pct(&mut model, 0.50),
        pct(&mut paint, 0.50),
    ))
}

fn bench(rows: usize) -> Result<String, String> {
    use std::time::Instant;

    let authored = Instant::now();
    let log = workbook(rows, COLS);
    let authoring = authored.elapsed();
    let build = Instant::now();
    let session = Session::from_log(ActorId(1), log);
    let built = build.elapsed();

    let mut app = App::open(session, WIDTH as f32, HEIGHT as f32, 1.0)
        .ok_or("the bundled font failed to load, or the workbook has no cells")?;
    let mut renderer = gpu::Renderer::headless()
        .ok_or("no GPU adapter is available on this machine (not a shell defect)")?;

    const FRAMES: usize = 120;
    let mut cpu = Vec::with_capacity(FRAMES);
    let mut total = Vec::with_capacity(FRAMES);
    let mut quads = 0usize;
    for frame in 0..FRAMES {
        let t = Instant::now();
        // A different scroll offset every frame, so nothing is reused.
        app.scroll(3.0, 17.0 + frame as f32);
        let scene = app.frame();
        let c = t.elapsed();
        quads = scene.len();
        // Only when new glyphs appeared, which after the first few frames is
        // never. Uploading unconditionally would measure a mebibyte of texture
        // write as if it were a frame cost.
        if let Some((size, bytes)) = app.take_atlas_upload() {
            renderer.upload_atlas(size, bytes);
        }
        let _ = renderer.render_to_rgba(WIDTH, HEIGHT, &scene);
        total.push(t.elapsed().as_secs_f64() * 1000.0);
        cpu.push(c.as_secs_f64() * 1000.0);
    }
    cpu.sort_by(f64::total_cmp);
    total.sort_by(f64::total_cmp);
    let pct = |v: &[f64], p: f64| v[((v.len() as f64 - 1.0) * p) as usize];

    Ok(format!(
        "W-SCROLL (docs/38) - {rows} rows x {COLS} cols, {WIDTH}x{HEIGHT}, {FRAMES} scrolled frames
           author corpus    {authoring:?}  (the test fixture, not the product)
           open workbook    {built:?}  (replay + graph build + full recalc; TD-66)
           quads per frame  {quads}  in 1 draw call
           CPU frame        p50 {:.3} ms   p99 {:.3} ms   (viewport + scene)
           CPU+GPU frame    p50 {:.3} ms   p99 {:.3} ms   (incl. readback)
           budget           8.3 ms (docs/31, scroll frame)",
        pct(&cpu, 0.50),
        pct(&cpu, 0.99),
        pct(&total, 0.50),
        pct(&total, 0.99),
    ))
}

fn render_to_file(path: &str, rows: usize) -> Result<String, String> {
    let session = Session::from_log(ActorId(1), workbook(rows, COLS));
    let mut app = App::open(session, WIDTH as f32, HEIGHT as f32, 1.0)
        .ok_or("the bundled font failed to load, or the workbook has no cells")?;
    // Scroll a long way in, so the frame proves virtual scrolling rather than
    // just showing the top-left corner every document has.
    app.scroll(320.0, 40_000.0);
    // Put the cursor somewhere visible, so the frame also shows the selection
    // and the formula bar doing their jobs.
    let visible = app.visible();
    if let (Some(r), Some(c)) = (visible.rows.get(3), visible.cols.get(2)) {
        let theme = app.theme();
        let (ox, oy) = theme.grid_origin();
        app.pointer_down(ox + c.at + 2.0, oy + r.at + 2.0, false);
    }

    let quads = app.frame();
    if app.atlas_overflowed() {
        return Err("the glyph atlas filled up".into());
    }
    let visible = app.visible();
    let reference = app.reference();

    let mut renderer = gpu::Renderer::headless()
        .ok_or("no GPU adapter is available on this machine (not a shell defect)")?;
    // After the scene, because building it is what rasterises the glyphs.
    if let Some((size, bytes)) = app.take_atlas_upload() {
        renderer.upload_atlas(size, bytes);
    }
    let rgba = renderer.render_to_rgba(WIDTH, HEIGHT, &quads);
    let encoded = png::encode_rgba(WIDTH, HEIGHT, &rgba);
    std::fs::write(path, &encoded).map_err(|e| format!("writing {path}: {e}"))?;

    let (rows_axis, cols_axis) = app.axes();
    Ok(format!(
        "rendered {WIDTH}x{HEIGHT} to {path}\n  \
         document      {} rows x {} cols\n  \
         visible       {} rows x {} cols  (virtual scroll)\n  \
         quads         {}  in 1 draw call\n  \
         active cell   {reference}\n  \
         anchored to   row {:?}, offset {:.1} px",
        rows_axis.len(),
        cols_axis.len(),
        visible.rows.len(),
        visible.cols.len(),
        quads.len(),
        app.viewport().rows.id.map(|o| o.counter),
        app.viewport().rows.offset,
    ))
}

/// A real workbook, built the only way a workbook can be built: by applying
/// ops (DP-A1).
///
/// Returned as a log rather than a `State` because that is what a document
/// *is* — `Session::from_log` folds it once, which is what opening a file will
/// do when there is a file to open.
pub fn workbook(rows: usize, cols: usize) -> OpLog {
    workbook_with(rows, cols, false)
}

/// `dense` puts a formula in every row; the default leaves the corpus's
/// two-of-three gap, which is what TD-66's experiment varies.
pub fn workbook_with(rows: usize, cols: usize, dense: bool) -> OpLog {
    let mut log = OpLog::new();
    let actor = ActorId(1);
    let mut counter = 0u64;
    let mut lamport = 0u64;
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

    let mut col_ids = Vec::with_capacity(cols);
    let mut anchor = Anchor::Start;
    for _ in 0..cols {
        let id = push(&mut log, Payload::InsertCol { anchor });
        anchor = Anchor::After(id);
        col_ids.push(id);
    }
    let mut row_ids = Vec::with_capacity(rows);
    let mut anchor = Anchor::Start;
    for _ in 0..rows {
        let id = push(&mut log, Payload::InsertRow { anchor });
        anchor = Anchor::After(id);
        row_ids.push(id);
    }

    // A sparse, structured sheet: most cells empty, a block of values, a
    // formula column, and a few errors — so the frame shows the three cell
    // kinds the scene distinguishes.
    for (r, row) in row_ids.iter().enumerate() {
        if !dense && r % 3 == 2 {
            continue;
        }
        for (c, col) in col_ids.iter().enumerate().take(12) {
            let value = Value::Number((r * 12 + c) as f64);
            push(
                &mut log,
                Payload::SetCell {
                    row: RowId(*row),
                    col: ColId(*col),
                    value,
                },
            );
        }
        if let Some(col) = col_ids.get(13) {
            // Row-relative, so every row's formula sums its own row and the
            // formula column shows 12 different totals rather than one repeated
            // number.
            //
            // **The bindings are not optional.** A `SetFormula` carries the
            // *identities* its references resolve to; the A1 text is the
            // display of that binding and not its source (DP-A6). A corpus
            // that left the list empty rendered a column of `#REF!` — which
            // was the renderer telling the truth about an unbound formula, and
            // is exactly the mistake an importer would make.
            let source = format!("=SUM(A{n}:L{n})", n = r + 1);
            let bindings = vec![usk_oplog::RangeBinding {
                row_start: *row,
                row_end: *row,
                col_start: col_ids[0],
                col_end: col_ids[11],
                anchors: 0,
            }];
            push(
                &mut log,
                Payload::SetFormula {
                    row: RowId(*row),
                    col: ColId(*col),
                    source,
                    bindings,
                },
            );
        }
        if r % 37 == 0 {
            if let Some(col) = col_ids.get(15) {
                // An authored error and a *computed* one, side by side: the
                // frame should show `#DIV/0!` whether it was written or
                // derived, and only the second exercises the engine.
                push(
                    &mut log,
                    Payload::SetCell {
                        row: RowId(*row),
                        col: ColId(*col),
                        value: Value::Error(usk_types::CellError::new(
                            usk_types::ErrorKind::Div0,
                            usk_types::Origin::Authored,
                        )),
                    },
                );
            }
            if let Some(col) = col_ids.get(16) {
                push(
                    &mut log,
                    Payload::SetFormula {
                        row: RowId(*row),
                        col: ColId(*col),
                        source: String::from("=1/0"),
                        bindings: Vec::new(),
                    },
                );
            }
        }
    }

    log
}
