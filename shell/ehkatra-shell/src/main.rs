//! The Ehkatra desktop shell (ADR-037, docs/25, docs/31).
//!
//! Two entry points, one scene:
//!
//! * `--render <path>` renders one frame offscreen and writes a PNG. This is
//!   how the grid is verified and how `demo/` gets its evidence — it needs no
//!   display, so it runs anywhere.
//! * no arguments opens a window (winit) and presents the same scene.
//!
//! **Nothing here is mocked.** The workbook is built by applying real ops to a
//! real `usk_state::State`, the visible rows and columns come from
//! `usk_view::Viewport`, and the cells drawn are the cells the kernel says are
//! there. A shell that lied about its data would be worse than no shell.

mod gpu;
mod png;
mod scene;

use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_state::State;
use usk_types::{ActorId, ColId, OpId, RowId, Value};
use usk_view::{Axis, Metrics, Viewport};

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--render") => {
            let path = args.next().unwrap_or_else(|| "grid.png".into());
            let rows: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(200_000);
            match render_to_file(&path, rows) {
                Ok(report) => println!("{report}"),
                Err(err) => {
                    eprintln!("ehkatra-shell: {err}");
                    std::process::exit(1);
                }
            }
        }
        // W-SCROLL (docs/38): the budget docs/31 sets for the renderer is a
        // scroll frame under 8.3 ms, which is 120 Hz. Measured over real
        // frames on a real document, scrolling every frame so nothing is
        // cached between them.
        Some("--bench") => {
            let rows: usize = args
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1_000_000);
            match bench(rows) {
                Ok(report) => println!("{report}"),
                Err(err) => {
                    eprintln!("ehkatra-shell: {err}");
                    std::process::exit(1);
                }
            }
        }
        Some(other) => {
            eprintln!("ehkatra-shell: unknown argument {other:?}; try --render <path>");
            std::process::exit(2);
        }
        // The windowed path is deliberately still a stub: it is the editing
        // surface and IME work (docs/33), which is its own unit. Saying so
        // beats opening a window that does nothing.
        None => eprintln!("ehkatra-shell: the window is not built yet; use --render <path>"),
    }
}

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

fn bench(rows: usize) -> Result<String, String> {
    use std::time::Instant;

    let build = Instant::now();
    let (state, row_ids, col_ids) = workbook(rows, 40);
    let built = build.elapsed();

    let metrics = Metrics::default();
    let axes = Instant::now();
    let row_axis = Axis::build(&row_ids, |o| metrics.row_height(RowId(o)));
    let col_axis = Axis::build(&col_ids, |o| metrics.col_width(ColId(o)));
    let axes = axes.elapsed();

    let theme = scene::Theme::default();
    let mut view = Viewport::new(
        WIDTH as f32 - theme.header_width,
        HEIGHT as f32 - theme.header_height,
    );
    let renderer = gpu::Renderer::headless()
        .ok_or("no GPU adapter is available on this machine (not a shell defect)")?;

    const FRAMES: usize = 120;
    let mut cpu = Vec::with_capacity(FRAMES);
    let mut total = Vec::with_capacity(FRAMES);
    let mut quads = 0usize;
    for frame in 0..FRAMES {
        let t = Instant::now();
        // A different scroll offset every frame, so nothing is reused.
        view.scroll_by(&row_axis, &col_axis, 3.0, 17.0 + frame as f32);
        let visible = view.visible(&row_axis, &col_axis);
        let scene = scene::build(&state, &visible, &theme, scene::Selection::default());
        let c = t.elapsed();
        quads = scene.len();
        let _ = renderer.render_to_rgba(WIDTH, HEIGHT, &scene);
        total.push(t.elapsed().as_secs_f64() * 1000.0);
        cpu.push(c.as_secs_f64() * 1000.0);
    }
    cpu.sort_by(|a, b| a.partial_cmp(b).unwrap());
    total.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |v: &[f64], p: f64| v[((v.len() as f64 - 1.0) * p) as usize];

    Ok(format!(
        "W-SCROLL (docs/38) - {rows} rows x {} cols, {WIDTH}x{HEIGHT}, {FRAMES} scrolled frames
           corpus build     {:?}  (one-off, not a frame cost)
           axis build       {:?}  (one-off; O(n) prefix sums, TD-58)
           quads per frame  {quads}  in 1 draw call
           CPU frame        p50 {:.3} ms   p99 {:.3} ms   (viewport + scene)
           CPU+GPU frame    p50 {:.3} ms   p99 {:.3} ms   (incl. readback)
           budget           8.3 ms (docs/31, scroll frame)",
        col_ids.len(),
        built,
        axes,
        pct(&cpu, 0.50),
        pct(&cpu, 0.99),
        pct(&total, 0.50),
        pct(&total, 0.99),
    ))
}

fn render_to_file(path: &str, rows: usize) -> Result<String, String> {
    let (state, row_ids, col_ids) = workbook(rows, 40);

    let metrics = Metrics::default();
    let row_axis = Axis::build(&row_ids, |o| metrics.row_height(RowId(o)));
    let col_axis = Axis::build(&col_ids, |o| metrics.col_width(ColId(o)));

    let theme = scene::Theme::default();
    let mut view = Viewport::new(
        WIDTH as f32 - theme.header_width,
        HEIGHT as f32 - theme.header_height,
    );
    // Scroll a long way in, so the frame proves virtual scrolling rather than
    // just showing the top-left corner every document has.
    view.scroll_by(&row_axis, &col_axis, 320.0, 40_000.0);
    let visible = view.visible(&row_axis, &col_axis);

    let selection = scene::Selection {
        row: visible.rows.get(3).map(|s| RowId(s.id)),
        col: visible.cols.get(2).map(|s| ColId(s.id)),
    };
    let quads = scene::build(&state, &visible, &theme, selection);

    let renderer = gpu::Renderer::headless()
        .ok_or("no GPU adapter is available on this machine (not a shell defect)")?;
    let rgba = renderer.render_to_rgba(WIDTH, HEIGHT, &quads);
    let encoded = png::encode_rgba(WIDTH, HEIGHT, &rgba);
    std::fs::write(path, &encoded).map_err(|e| format!("writing {path}: {e}"))?;

    Ok(format!(
        "rendered {WIDTH}x{HEIGHT} to {path}\n  \
         document      {} rows x {} cols\n  \
         visible       {} rows x {} cols  (virtual scroll)\n  \
         quads         {}  in 1 draw call\n  \
         anchored to   row {:?}, offset {:.1} px",
        row_ids.len(),
        col_ids.len(),
        visible.rows.len(),
        visible.cols.len(),
        quads.len(),
        view.rows.id.map(|o| o.counter),
        view.rows.offset,
    ))
}

/// A real workbook, built the only way a workbook can be built: by applying
/// ops (DP-A1).
fn workbook(rows: usize, cols: usize) -> (State, Vec<OpId>, Vec<OpId>) {
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
        if r % 3 == 2 {
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
            push(
                &mut log,
                Payload::SetFormula {
                    row: RowId(*row),
                    col: ColId(*col),
                    source: String::from("=SUM(A1:L1)"),
                    bindings: Vec::new(),
                },
            );
        }
        if r % 37 == 0 {
            if let Some(col) = col_ids.get(15) {
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
        }
    }

    let state = State::replay(&log);
    (state, row_ids, col_ids)
}
