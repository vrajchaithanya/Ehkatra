//! The application model: selection, editing, navigation and recalculation
//! (docs/25, docs/27 §3, TD-60/TD-61).
//!
//! # Why this exists as its own layer
//! Everything in this file is decided without a window, a GPU or an event loop.
//! [`App`] owns a `usk_reduce::Session` — the kernel's editing session, which
//! already holds the op log, the folded state, the calculation engine and this
//! actor's undo stacks — and turns [`Intent`]s into `Command`s against it.
//! `window.rs` translates winit events into intents and asks for quads; it
//! makes no decisions of its own. That split is what makes clicking a cell,
//! typing a formula and watching it recalculate provable by unit tests rather
//! than by a person looking at a screen.
//!
//! # Ops are the only mutation path (DP-A1)
//! Nothing here writes to `State`. Every edit is a `usk_reduce::Command`, which
//! the reducer compiles to ops exactly once (DP-A7); undo is the reducer's
//! synthesized inverse, not a shell-side stack of "what it used to be". A shell
//! that kept its own history would be a second source of truth, and the first
//! divergence would be silent.
//!
//! # Identity, everywhere (DP-A6)
//! The active cell is a `(RowId, ColId)`, the scroll position is an anchored
//! identity, and the selection's *anchor* is an identity too. Ordinals appear
//! only where a human sees them: A1 labels, the range rectangle the renderer
//! tints, and the coordinates a `Command` takes. Insert a row above the cursor
//! and the cursor stays on its cell — that is a property of storing the
//! identity, not of remembering to fix up an index.

use usk_reduce::{Command, Session};
use usk_state::State;
use usk_types::coerce::Profile;
use usk_types::{ColId, OpId, RowId, Value};
use usk_view::{column_label, row_label, Axis, Metrics, Viewport, Visible};

use crate::clipboard::{self, Block, Clipboard};
use crate::fill::{self, Filled};
use crate::gpu::Quad;
use crate::input::{Intent, Seed, Step};
use crate::scene::{self, BarView, EditView, Selection, Theme};
use crate::text::{self, TextEngine};

/// The in-cell editor's state.
///
/// Held on the app rather than inside the session because an open editor is
/// **not an edit**: nothing has been written, no op exists, and abandoning it
/// must cost the document nothing. The cell is remembered by identity so a
/// concurrent structural edit cannot move the editor to a different cell
/// mid-keystroke.
#[derive(Clone, Debug)]
pub struct Editor {
    pub row: RowId,
    pub col: ColId,
    pub text: String,
    /// Byte offset into `text`. Bytes and not characters because that is what
    /// `String` insertion and shaping clusters both index by; every mutation
    /// below keeps it on a character boundary.
    pub caret: usize,
    /// An input method's composition, when one is in flight (docs/33 §IME).
    pub preedit: Option<Preedit>,
}

/// An in-flight IME composition (docs/33 §IME, ADR-039's named absence).
///
/// Held **apart from** [`Editor::text`] because a composition is a *proposal by
/// the input method*, not an edit: nothing in here reaches the cell, `Escape`
/// drops it without disturbing the buffer under it, and a commit writes the
/// buffer exactly as if the composition had never been shown. The composition's
/// contents are the platform's — docs/33 says composition is "never
/// reimplemented", and winit's `Ime` events *are* TSF on Windows and
/// NSTextInputClient on macOS, so this type carries what the platform decided
/// and never decides anything itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preedit {
    /// What the input method is currently showing.
    pub text: String,
    /// The input method's own caret or selection inside `text`, as byte
    /// offsets. `None` when the platform declines to place one, in which case
    /// the caret belongs after the composition — the convention every IME
    /// expects when it says nothing.
    ///
    /// When `start != end` this is **not** a caret: a converting IME reports the
    /// clause the arrow keys are on as a range, and that range is the only thing
    /// distinguishing two otherwise-identical conversion states (TD-84).
    pub cursor: Option<(usize, usize)>,
}

/// Where a composition sits in the display string, and which part of it the
/// input method has the focus on (TD-84, docs/33 §IME).
///
/// Two spans and not one because a converting IME draws two things: the whole
/// composition is underlined ("this is a proposal"), and the clause under
/// conversion is shaded ("this is the one the arrow keys and the candidate list
/// are working on"). Session 31 built the surface against a single-clause
/// example, where the two are the same span and the distinction is invisible;
/// `今日は晴れ` with the focus on `今日は` versus on `晴れ` is the case that
/// proves they are different, and drawing only the first is what TD-84 recorded.
///
/// A named struct rather than a second `Option<(usize, usize)>` in the tuple,
/// because two same-typed span options side by side are exactly where the wrong
/// two get swapped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Composition {
    /// The whole composition's byte span in the display string — underlined.
    pub span: (usize, usize),
    /// The focused clause's byte span, a sub-range of `span`. `None` when the
    /// input method reported a bare caret rather than a range, which is every
    /// pre-conversion keystroke and every non-converting IME (Korean composes
    /// this way throughout).
    pub focus: Option<(usize, usize)>,
}

impl Editor {
    /// Whether an input method is composing into this editor.
    pub fn composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// The buffer as it should be **shown**: the committed text with the
    /// composition spliced in at the caret.
    ///
    /// Returns the display string, the caret's byte offset within it, and the
    /// [`Composition`] the scene draws. The cell never sees this string —
    /// [`App::commit`] writes `text`, which is the whole reason the two are
    /// separate fields.
    pub fn display(&self) -> (String, usize, Option<Composition>) {
        let Some(pre) = &self.preedit else {
            return (self.text.clone(), self.caret, None);
        };
        let mut shown = String::with_capacity(self.text.len() + pre.text.len());
        shown.push_str(&self.text[..self.caret]);
        shown.push_str(&pre.text);
        shown.push_str(&self.text[self.caret..]);
        // Clamped rather than trusted: the offsets come from a platform IME,
        // and a caret past the end of the composition would be a panic in the
        // shaper's cluster search rather than a cosmetic error.
        let clamp = |at: usize| at.min(pre.text.len());
        let within = clamp(pre.cursor.map(|(start, _)| start).unwrap_or(pre.text.len()));
        // Normalised, again because the offsets are the platform's: a range
        // reported end-first is a focused clause, not an empty one, and
        // `right > left` in the scene would silently drop it.
        let focus = pre.cursor.and_then(|(start, end)| {
            let (lo, hi) = (clamp(start.min(end)), clamp(start.max(end)));
            (hi > lo).then_some((self.caret + lo, self.caret + hi))
        });
        (
            shown,
            self.caret + within,
            Some(Composition {
                span: (self.caret, self.caret + pre.text.len()),
                focus,
            }),
        )
    }
}

/// What one intent did, for a caller that needs to know whether to redraw.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Outcome {
    pub redraw: bool,
}

pub struct App {
    session: Session,
    metrics: Metrics,
    rows: Axis,
    cols: Axis,
    view: Viewport,
    theme: Theme,
    /// Window extent in **logical** pixels.
    size: (f32, f32),
    /// Display scale factor (docs/33 §Displays).
    scale: f32,
    active: (RowId, ColId),
    /// The other corner of the selection, kept as an identity so a structural
    /// edit cannot silently resize the selection.
    anchor: (RowId, ColId),
    editor: Option<Editor>,
    /// The last thing worth telling the user — a blocked undo, a reference that
    /// would not bind. docs/25: *"every async state is visible and honest"*, and
    /// a refused command is the same obligation.
    status: String,
    text: TextEngine,
    clipboard: Clipboard,
    /// The range a `Cut` marked, to be cleared when its block is pasted.
    ///
    /// Excel defers the removal to the paste rather than doing it on the cut,
    /// and the difference is visible: `Ctrl+X` followed by `Escape` must leave
    /// the document exactly as it was.
    pending_cut: Option<(usize, usize, usize, usize)>,
    /// What the pointer is currently doing, if anything.
    drag: Option<Drag>,
    /// The caret's rectangle in logical pixels as of the last [`App::frame`],
    /// which is what the platform needs to place an IME candidate window
    /// (docs/33 §IME). Cached here rather than recomputed on demand because the
    /// caret's x is a *shaped-text* answer and the scene is where text is
    /// shaped; a second implementation would be free to disagree with the first.
    ime_area: Option<[f32; 4]>,
}

/// A pointer gesture in progress.
///
/// Two gestures start with a press inside the grid and they are told apart by
/// *where*: on the fill handle, or anywhere else. Keeping that as state rather
/// than re-deciding on each move is what stops a drag that begins on the handle
/// turning into a selection the moment the pointer leaves it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drag {
    /// Extending the selection.
    Select,
    /// Dragging the fill handle. `origin` is the selection the drag started
    /// from; `preview` is what would be filled if released now.
    Fill {
        origin: (usize, usize, usize, usize),
        preview: Option<(usize, usize, usize, usize)>,
    },
}

impl App {
    /// Opens a workbook.
    ///
    /// `None` when the sheet has no rows or no columns: there is no cell to put
    /// a cursor on, and every operation below would have to carry an "or maybe
    /// there is nothing" case for a document the product cannot produce.
    pub fn open(session: Session, width: f32, height: f32, scale: f32) -> Option<App> {
        let mut app = App::open_cold(session, width, height, scale)?;
        // **TD-80.** Building the system font database costs 203–321 ms of the
        // 238–412 ms first non-Latin keystroke (W-FALLBACK, profiled before this
        // was written), against docs/31's 16 ms keystroke→paint. Started here —
        // the one place a *real* session begins — it overlaps the seconds
        // between launch and that keystroke instead of landing on it.
        //
        // Here and not in `TextEngine::new`: 114 tests and every benchmark
        // construct an engine, and warming there would have all of them spawn a
        // file scan they never use. `open_cold` below is the constructor that
        // does not, and it is what the suite goes through.
        app.text.warm();
        Some(app)
    }

    /// `open` without the font warm-up — the constructor for callers that are
    /// not a session (TD-80).
    ///
    /// Split out rather than parameterised because the difference is one line
    /// and a `bool` argument at every call site would be worse documentation
    /// than two named functions.
    fn open_cold(mut session: Session, width: f32, height: f32, scale: f32) -> Option<App> {
        let text = TextEngine::new()?;
        let theme = Theme::default();
        let metrics = Metrics::default();
        let (rows, cols) = axes(session.state(), &metrics);
        let first = (RowId(rows.id_at(0)?), ColId(cols.id_at(0)?));
        let (vw, vh) = theme.viewport_size(width, height);
        let mut app = App {
            session,
            metrics,
            rows,
            cols,
            view: Viewport::new(vw, vh),
            theme,
            size: (width, height),
            scale,
            active: first,
            anchor: first,
            editor: None,
            status: String::new(),
            text,
            clipboard: Clipboard::new(),
            pending_cut: None,
            drag: None,
            ime_area: None,
        };
        if let Some(why) = app.clipboard.unavailable() {
            app.status = String::from(why);
        }
        app.session.settle();
        Some(app)
    }

    /// Opens with an in-process clipboard that never touches the OS — what
    /// tests use, so the suite needs no display.
    ///
    /// Goes through [`App::open_cold`], so the suite also spawns **no** font
    /// warm-up: 114 tests each starting a background scan of several hundred
    /// font files is a real cost for a fallback almost none of them exercise
    /// (TD-80). The tests that do care about warming call `TextEngine::warm`
    /// directly, in `text.rs`.
    #[cfg(test)]
    pub fn open_detached(session: Session, width: f32, height: f32, scale: f32) -> Option<App> {
        let mut app = App::open_cold(session, width, height, scale)?;
        app.clipboard = Clipboard::detached();
        app.status.clear();
        Some(app)
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    pub fn viewport(&self) -> &Viewport {
        &self.view
    }

    pub fn axes(&self) -> (&Axis, &Axis) {
        (&self.rows, &self.cols)
    }

    /// The value a *reader* sees at a cell — the engine's, so a formula cell
    /// answers with its computed result and not with the nothing the tile store
    /// holds (TD-61).
    pub fn value(&mut self, row: RowId, col: ColId) -> Option<Value> {
        self.session.value(row, col)
    }

    /// Whether an input method is composing (docs/33 §IME).
    pub fn composing(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::composing)
    }

    /// Where the caret was on screen at the last [`App::frame`], in logical
    /// pixels, so the platform can put the IME candidate list under the text
    /// being composed rather than in the window's corner (docs/33 §IME).
    ///
    /// `None` when no editor is open, or when its cell has been scrolled out of
    /// view — in which case there is no honest place to put the candidates.
    pub fn ime_area(&self) -> Option<[f32; 4]> {
        self.ime_area
    }

    /// An input method updated its composition (winit `Ime::Preedit`).
    ///
    /// **Opening the editor here is not a convenience.** On Windows and macOS
    /// the keystrokes that begin a composition are swallowed by the input
    /// method, so no key event ever reaches [`App::handle`] — an implementation
    /// that only updated an already-open editor would drop a CJK user's first
    /// word on the floor and look, from the outside, exactly like a dead
    /// keyboard. The seed is empty for the same reason typing a character over
    /// a cell replaces it.
    ///
    /// An empty composition is how every backend says *withdrawn*, and it is
    /// not the same thing as a commit: the buffer underneath is left alone.
    pub fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) -> Outcome {
        if text.is_empty() {
            let dropped = self
                .editor
                .as_mut()
                .and_then(|ed| ed.preedit.take())
                .is_some();
            return Outcome { redraw: dropped };
        }
        if self.editor.is_none() {
            self.begin_edit(Seed::Empty);
        }
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        ed.preedit = Some(Preedit {
            text: String::from(text),
            cursor,
        });
        Outcome { redraw: true }
    }

    /// An input method finalised its composition (winit `Ime::Commit`).
    ///
    /// The finalised text is inserted at the caret **as a unit**, not character
    /// by character: an IME commits a word, and splitting it into inserts would
    /// invent an edit history the user never performed. Some backends deliver
    /// ordinary typing this way too when IME is enabled, which is why this path
    /// also opens the editor.
    pub fn ime_commit(&mut self, text: &str) -> Outcome {
        if self.editor.is_none() {
            if text.is_empty() {
                return Outcome::default();
            }
            self.begin_edit(Seed::Empty);
        }
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        let dropped = ed.preedit.take().is_some();
        if text.is_empty() {
            return Outcome { redraw: dropped };
        }
        ed.text.insert_str(ed.caret, text);
        ed.caret += text.len();
        Outcome { redraw: true }
    }

    /// Whether the editor is open — the one bit the keymap needs.
    pub fn mode(&self) -> crate::input::Mode {
        if self.editor.is_some() {
            crate::input::Mode::Editing
        } else {
            crate::input::Mode::Grid
        }
    }

    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        self.size = (width, height);
        self.scale = scale;
        let (vw, vh) = self.theme.viewport_size(width, height);
        self.view.width = vw;
        self.view.height = vh;
    }

    /// Scrolls by a pixel delta. The anchor is re-expressed by the view model,
    /// so this is the whole of scrolling — there is no second pixel counter
    /// (ADR-022).
    pub fn scroll(&mut self, dx: f32, dy: f32) -> Outcome {
        let before = (self.view.rows, self.view.cols);
        self.view.scroll_by(&self.rows, &self.cols, dx, dy);
        Outcome {
            redraw: before != (self.view.rows, self.view.cols),
        }
    }

    // ---------------------------------------------------------------- pointer

    /// The cell under a logical-pixel point, if the point is over the grid.
    pub fn cell_at(&self, x: f32, y: f32) -> Option<(RowId, ColId)> {
        let (ox, oy) = self.theme.grid_origin();
        if x < ox || y < oy {
            return None;
        }
        let visible = self.view.visible(&self.rows, &self.cols);
        let row = visible
            .rows
            .iter()
            .find(|s| y - oy >= s.at && y - oy < s.at + s.size)?;
        let col = visible
            .cols
            .iter()
            .find(|s| x - ox >= s.at && x - ox < s.at + s.size)?;
        Some((RowId(row.id), ColId(col.id)))
    }

    /// The screen rect of the fill handle, if the selection's bottom-right
    /// corner is on screen.
    ///
    /// A small square hanging off the corner of the selection — the affordance
    /// docs/25 means by *"fill/drag grammar per Excel"*, and the only part of
    /// the grid where a press means something other than "select this".
    pub fn fill_handle(&self) -> Option<[f32; 4]> {
        if self.editor.is_some() {
            return None;
        }
        let (_, r1, _, c1) = self.selection_range()?;
        let visible = self.view.visible(&self.rows, &self.cols);
        let row = visible.rows.iter().find(|s| s.index == r1)?;
        let col = visible.cols.iter().find(|s| s.index == c1)?;
        let (ox, oy) = self.theme.grid_origin();
        let size = scene::FILL_HANDLE;
        Some([
            ox + col.at + col.size - size * 0.5 - 1.0,
            oy + row.at + row.size - size * 0.5 - 1.0,
            size,
            size,
        ])
    }

    fn over_fill_handle(&self, x: f32, y: f32) -> bool {
        // A couple of pixels of slack in every direction: the handle is 7 px
        // and a hit target that small is a test of the user's mouse rather than
        // of their intent.
        let Some([hx, hy, w, h]) = self.fill_handle() else {
            return false;
        };
        const SLACK: f32 = 2.0;
        x >= hx - SLACK && x <= hx + w + SLACK && y >= hy - SLACK && y <= hy + h + SLACK
    }

    /// A press on the grid. `extend` (Shift-click) keeps the anchor, which is
    /// how a range is selected with the mouse.
    pub fn pointer_down(&mut self, x: f32, y: f32, extend: bool) -> Outcome {
        // The handle is tested first and against the *current* selection,
        // because it hangs over the cell to its lower-right and a hit there
        // means "fill", not "select that cell".
        if self.over_fill_handle(x, y) {
            if let Some(origin) = self.selection_range() {
                self.drag = Some(Drag::Fill {
                    origin,
                    preview: None,
                });
                return Outcome { redraw: true };
            }
        }
        let Some(cell) = self.cell_at(x, y) else {
            return Outcome::default();
        };
        // Clicking away from an open editor commits it, which is what every
        // spreadsheet does and what stops a click silently discarding typing.
        if self.editor.is_some() {
            self.commit();
        }
        self.drag = Some(Drag::Select);
        self.active = cell;
        if !extend {
            self.anchor = cell;
        }
        Outcome { redraw: true }
    }

    /// A drag with the button held: grows the selection, or previews a fill.
    pub fn pointer_drag(&mut self, x: f32, y: f32) -> Outcome {
        match self.drag {
            Some(Drag::Fill { origin, preview }) => {
                let next = self.fill_target(origin, x, y);
                if next == preview {
                    return Outcome::default();
                }
                self.drag = Some(Drag::Fill {
                    origin,
                    preview: next,
                });
                Outcome { redraw: true }
            }
            _ => {
                let Some(cell) = self.cell_at(x, y) else {
                    return Outcome::default();
                };
                if cell == self.active {
                    return Outcome::default();
                }
                self.active = cell;
                Outcome { redraw: true }
            }
        }
    }

    /// Abandons a gesture in progress without committing it.
    ///
    /// The pointer equivalent of `Escape`, and the thing a window must call
    /// when it loses focus mid-drag: a fill whose release never arrives must
    /// not be left waiting to fire the next time the button comes up somewhere
    /// unrelated.
    pub fn drag_cancel(&mut self) -> Outcome {
        Outcome {
            redraw: self.drag.take().is_some(),
        }
    }

    /// The button came up: a fill drag commits here, a selection drag just ends.
    pub fn pointer_up(&mut self) -> Outcome {
        match self.drag.take() {
            Some(Drag::Fill {
                origin,
                preview: Some(target),
            }) => self.apply_fill(origin, target),
            _ => Outcome::default(),
        }
    }

    /// Where the pointer says the fill should reach.
    ///
    /// One axis at a time, whichever the pointer has travelled further along —
    /// Excel does the same, and a fill that went diagonally would have no
    /// defined series. The target always contains the origin, so releasing back
    /// over the source fills nothing.
    fn fill_target(
        &self,
        origin: (usize, usize, usize, usize),
        x: f32,
        y: f32,
    ) -> Option<(usize, usize, usize, usize)> {
        let (r0, r1, c0, c1) = origin;
        let (ox, oy) = self.theme.grid_origin();
        let visible = self.view.visible(&self.rows, &self.cols);
        // Clamped rather than dropped: a pointer dragged past the last visible
        // row should fill to it, not stop responding.
        let row = visible
            .rows
            .iter()
            .find(|s| y - oy >= s.at && y - oy < s.at + s.size)
            .or(if y < oy {
                visible.rows.first()
            } else {
                visible.rows.last()
            })?;
        let col = visible
            .cols
            .iter()
            .find(|s| x - ox >= s.at && x - ox < s.at + s.size)
            .or(if x < ox {
                visible.cols.first()
            } else {
                visible.cols.last()
            })?;

        let down = row.index.saturating_sub(r1);
        let up = r0.saturating_sub(row.index);
        let right = col.index.saturating_sub(c1);
        let left = c0.saturating_sub(col.index);
        let vertical = down.max(up);
        let horizontal = right.max(left);
        if vertical == 0 && horizontal == 0 {
            return None;
        }
        Some(if vertical >= horizontal {
            (r0.min(row.index), r1.max(row.index), c0, c1)
        } else {
            (r0, r1, c0.min(col.index), c1.max(col.index))
        })
    }

    // ----------------------------------------------------------------- intent

    pub fn handle(&mut self, intent: Intent) -> Outcome {
        // While an input method is composing, the keyboard belongs to it
        // (docs/33: composition is the platform's, "never reimplemented"). Any
        // key that reaches a composing editor either leaked past the IME or was
        // declined by it, and acting on one would fight the composition: a
        // `Backspace` would delete a *committed* character the user cannot see
        // the caret in front of, and on backends that still deliver
        // `KeyboardInput` during composition an `Insert` would type every
        // keystroke twice — once here and once again in the commit.
        //
        // `Cancel` is the single exception, because a composition is the
        // innermost thing Escape can close and closing the outer edit first
        // would be Escape skipping a layer.
        if self.composing() && !matches!(intent, Intent::Cancel) {
            return Outcome::default();
        }
        match intent {
            Intent::Move { step, extend } => self.move_cursor(step, extend),
            Intent::BeginEdit(seed) => self.begin_edit(seed),
            Intent::Insert(c) => self.edit_insert(c),
            Intent::Backspace => self.edit_backspace(),
            Intent::DeleteForward => self.edit_delete_forward(),
            Intent::CaretLeft => self.edit_caret(-1),
            Intent::CaretRight => self.edit_caret(1),
            Intent::CaretHome => self.edit_caret_to(0),
            Intent::CaretEnd => {
                let end = self.editor.as_ref().map(|e| e.text.len()).unwrap_or(0);
                self.edit_caret_to(end)
            }
            Intent::Commit { then } => {
                if self.editor.is_none() {
                    return self.move_cursor(then, false);
                }
                self.commit();
                self.move_cursor(then, false);
                Outcome { redraw: true }
            }
            Intent::Cancel => self.cancel(),
            Intent::Clear => self.clear_selection(),
            Intent::Undo => self.history(Command::Undo),
            Intent::Redo => self.history(Command::Redo),
            Intent::Copy => self.copy(false),
            Intent::Cut => self.copy(true),
            Intent::Paste => self.paste(),
            Intent::InsertRow => self.structural(|at| Command::InsertRow { before: at.0 }),
            Intent::DeleteRow => self.structural(|at| Command::DeleteRow { at: at.0 }),
            Intent::InsertCol => self.structural(|at| Command::InsertCol { before: at.1 }),
            Intent::DeleteCol => self.structural(|at| Command::DeleteCol { at: at.1 }),
        }
    }

    // -------------------------------------------------------------- selection

    fn active_index(&self) -> (usize, usize) {
        (
            self.rows.index_of(self.active.0 .0).unwrap_or(0),
            self.cols.index_of(self.active.1 .0).unwrap_or(0),
        )
    }

    fn move_cursor(&mut self, step: Step, extend: bool) -> Outcome {
        if self.editor.is_some() {
            self.commit();
        }
        let (r, c) = self.active_index();
        let (nr, nc) = self.resolve_step(step, r, c);
        let (Some(row), Some(col)) = (self.rows.id_at(nr), self.cols.id_at(nc)) else {
            return Outcome::default();
        };
        self.active = (RowId(row), ColId(col));
        if !extend {
            self.anchor = self.active;
        }
        self.ensure_visible();
        Outcome { redraw: true }
    }

    fn resolve_step(&mut self, step: Step, r: usize, c: usize) -> (usize, usize) {
        let last_row = self.rows.len().saturating_sub(1);
        let last_col = self.cols.len().saturating_sub(1);
        let page = self.page_rows();
        match step {
            Step::Left => (r, c.saturating_sub(1)),
            Step::Right => (r, (c + 1).min(last_col)),
            Step::Up => (r.saturating_sub(1), c),
            Step::Down => ((r + 1).min(last_row), c),
            Step::RowStart => (r, 0),
            Step::PageUp => (r.saturating_sub(page), c),
            Step::PageDown => ((r + page).min(last_row), c),
            Step::SheetStart => (0, 0),
            Step::SheetEnd => (last_row, last_col),
            Step::EdgeLeft => (r, self.data_edge_col(r, c, -1)),
            Step::EdgeRight => (r, self.data_edge_col(r, c, 1)),
            Step::EdgeUp => (self.data_edge_row(r, c, -1), c),
            Step::EdgeDown => (self.data_edge_row(r, c, 1), c),
        }
    }

    fn page_rows(&self) -> usize {
        self.view
            .visible(&self.rows, &self.cols)
            .rows
            .len()
            .saturating_sub(1)
            .max(1)
    }

    fn occupied(&mut self, r: usize, c: usize) -> bool {
        let (Some(row), Some(col)) = (self.rows.id_at(r), self.cols.id_at(c)) else {
            return false;
        };
        let (row, col) = (RowId(row), ColId(col));
        if self.session.state().formula(row, col).is_some() {
            return true;
        }
        !matches!(self.session.value(row, col), None | Some(Value::Blank))
    }

    /// Excel's `Ctrl+Arrow`: from inside a run of data, jump to the last cell
    /// of the run; from outside one, jump to the first cell of the next run;
    /// with nothing ahead, jump to the edge of the sheet.
    ///
    /// The scan is bounded by the axis, so on a sheet with a million empty rows
    /// below the cursor `Ctrl+Down` costs a million cell reads. That is the
    /// same shape as Excel's and it is a *keystroke*, not a frame — but it is
    /// the one place in this file that is O(axis), so it is said out loud.
    fn data_edge_row(&mut self, r: usize, c: usize, dir: isize) -> usize {
        let last = self.rows.len().saturating_sub(1);
        self.scan(r, last, dir, |app, i| app.occupied(i, c))
    }

    fn data_edge_col(&mut self, r: usize, c: usize, dir: isize) -> usize {
        let last = self.cols.len().saturating_sub(1);
        self.scan(c, last, dir, |app, i| app.occupied(r, i))
    }

    fn scan<F: Fn(&mut Self, usize) -> bool>(
        &mut self,
        from: usize,
        last: usize,
        dir: isize,
        filled: F,
    ) -> usize {
        let next = |i: usize| -> Option<usize> {
            if dir < 0 {
                i.checked_sub(1)
            } else if i < last {
                Some(i + 1)
            } else {
                None
            }
        };
        let Some(mut i) = next(from) else { return from };
        let in_run = filled(self, i);
        let mut best = i;
        loop {
            let here = filled(self, i);
            if in_run && !here {
                return best;
            }
            if !in_run && here {
                return i;
            }
            best = i;
            match next(i) {
                Some(n) => i = n,
                None => return i,
            }
        }
    }

    /// Scrolls the minimum distance that brings the active cell fully into
    /// view. Called after every cursor move, because a cursor you cannot see is
    /// the fastest way to lose your place.
    fn ensure_visible(&mut self) {
        let (dx, dy) = self.scroll_needed();
        if dx != 0.0 || dy != 0.0 {
            self.view.scroll_by(&self.rows, &self.cols, dx, dy);
        }
    }

    fn scroll_needed(&self) -> (f32, f32) {
        fn axis_delta(axis: &Axis, anchor: &usk_view::Anchor, extent: f32, id: OpId) -> f32 {
            let top = anchor.id.and_then(|a| axis.pixel_of(a)).unwrap_or(0.0) + anchor.offset;
            let Some(index) = axis.index_of(id) else {
                return 0.0;
            };
            let at = axis.pixel_of(id).unwrap_or(0.0);
            let size = axis.size_at(index);
            if at < top {
                at - top
            } else if at + size > top + extent {
                at + size - (top + extent)
            } else {
                0.0
            }
        }
        (
            axis_delta(
                &self.cols,
                &self.view.cols,
                self.view.width,
                self.active.1 .0,
            ),
            axis_delta(
                &self.rows,
                &self.view.rows,
                self.view.height,
                self.active.0 .0,
            ),
        )
    }

    /// The selected rectangle in axis ordinals, resolved from the two
    /// identities that define it.
    fn selection_range(&self) -> Option<(usize, usize, usize, usize)> {
        let ar = self.rows.index_of(self.active.0 .0)?;
        let ac = self.cols.index_of(self.active.1 .0)?;
        let hr = self.rows.index_of(self.anchor.0 .0).unwrap_or(ar);
        let hc = self.cols.index_of(self.anchor.1 .0).unwrap_or(ac);
        Some((ar.min(hr), ar.max(hr), ac.min(hc), ac.max(hc)))
    }

    // ----------------------------------------------------------------- editing

    /// The cell's **source**: its formula text where it has one, its rendered
    /// value otherwise. This is what `F2` opens and what the formula bar shows,
    /// and it is deliberately not what the grid shows — the grid shows the
    /// computed result.
    pub fn source_at(&mut self, row: RowId, col: ColId) -> String {
        if let Some(f) = self.session.state().formula(row, col) {
            return f.source.clone();
        }
        match self.session.value(row, col) {
            Some(v) => text::render_value(&v).unwrap_or_default(),
            None => String::new(),
        }
    }

    fn begin_edit(&mut self, seed: Seed) -> Outcome {
        if self.editor.is_some() {
            return Outcome::default();
        }
        let (row, col) = self.active;
        let text = match seed {
            Seed::Existing => self.source_at(row, col),
            Seed::Empty => String::new(),
            Seed::Typed(c) => {
                let mut s = String::new();
                s.push(c);
                s
            }
        };
        // The anchor collapses: typing into a cell is not typing into a range.
        self.anchor = self.active;
        self.editor = Some(Editor {
            row,
            col,
            caret: text.len(),
            text,
            preedit: None,
        });
        self.status.clear();
        Outcome { redraw: true }
    }

    fn edit_insert(&mut self, c: char) -> Outcome {
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        ed.text.insert(ed.caret, c);
        ed.caret += c.len_utf8();
        Outcome { redraw: true }
    }

    fn edit_backspace(&mut self) -> Outcome {
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        if ed.caret == 0 {
            return Outcome::default();
        }
        // Back to the previous character boundary, not the previous byte: a
        // multi-byte character is one Backspace, and splitting one would panic
        // on the next `String::insert`.
        let prev = ed.text[..ed.caret]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        ed.text.replace_range(prev..ed.caret, "");
        ed.caret = prev;
        Outcome { redraw: true }
    }

    fn edit_delete_forward(&mut self) -> Outcome {
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        let Some(next) = ed.text[ed.caret..].chars().next() else {
            return Outcome::default();
        };
        let end = ed.caret + next.len_utf8();
        ed.text.replace_range(ed.caret..end, "");
        Outcome { redraw: true }
    }

    fn edit_caret(&mut self, dir: isize) -> Outcome {
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        let next = if dir < 0 {
            ed.text[..ed.caret]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
        } else {
            ed.text[ed.caret..]
                .chars()
                .next()
                .map(|c| ed.caret + c.len_utf8())
                .unwrap_or(ed.caret)
        };
        if next == ed.caret {
            return Outcome::default();
        }
        ed.caret = next;
        Outcome { redraw: true }
    }

    fn edit_caret_to(&mut self, at: usize) -> Outcome {
        let Some(ed) = self.editor.as_mut() else {
            return Outcome::default();
        };
        let at = at.min(ed.text.len());
        if at == ed.caret {
            return Outcome::default();
        }
        ed.caret = at;
        Outcome { redraw: true }
    }

    fn cancel(&mut self) -> Outcome {
        // A composition is the innermost thing Escape closes: the first Escape
        // abandons what the input method is proposing, the second abandons the
        // edit. Collapsing the two would make one keypress throw away text the
        // user had already committed into the cell editor.
        if let Some(ed) = self.editor.as_mut() {
            if ed.preedit.take().is_some() {
                return Outcome { redraw: true };
            }
        }
        if self.editor.take().is_some() {
            self.status.clear();
            return Outcome { redraw: true };
        }
        // No editor: collapse the selection onto the active cell, which is what
        // Escape means in a grid.
        if self.anchor != self.active {
            self.anchor = self.active;
            return Outcome { redraw: true };
        }
        Outcome::default()
    }

    /// Writes the editor's contents through the reducer and closes it.
    ///
    /// Three cases, and the split is Excel's: nothing is a clear, a leading `=`
    /// is a formula, and everything else is a literal read under the compat
    /// profile.
    fn commit(&mut self) {
        let Some(ed) = self.editor.take() else {
            return;
        };
        let (Some(r), Some(c)) = (self.rows.index_of(ed.row.0), self.cols.index_of(ed.col.0))
        else {
            // The cell was deleted while it was being edited. Dropping the text
            // is the only honest outcome — there is nowhere to put it — and
            // saying so beats writing it into whatever now occupies the slot.
            self.status = String::from("the edited cell no longer exists; the edit was discarded");
            return;
        };
        let (row, col) = (r as u32, c as u32);
        let command = if ed.text.is_empty() {
            Command::ClearCell { row, col }
        } else if ed.text.len() > 1 && ed.text.starts_with('=') {
            Command::SetFormula {
                row,
                col,
                source: ed.text.clone(),
            }
        } else {
            Command::SetValue {
                row,
                col,
                value: literal(&ed.text),
            }
        };
        match self.session.apply(command) {
            Ok(_) => self.status.clear(),
            // A formula whose references cannot be bound is refused by the
            // reducer rather than stored as a cell that can never evaluate.
            // The text is *not* silently dropped: it goes back into the editor
            // so it can be fixed.
            Err(err) => {
                self.status = format!("{err:?}: the formula was not written");
                self.editor = Some(ed);
            }
        }
    }

    // --------------------------------------------------------- the clipboard

    /// Reads a cell as the clipboard carries it: the formula where there is
    /// one, alongside the value it was showing.
    fn clip_cell(&mut self, r: usize, c: usize) -> clipboard::Cell {
        let (Some(row), Some(col)) = (self.rows.id_at(r), self.cols.id_at(c)) else {
            return clipboard::Cell::Blank;
        };
        let (row, col) = (RowId(row), ColId(col));
        let display = match self.session.value(row, col) {
            Some(v) => text::render_value(&v).unwrap_or_default(),
            None => String::new(),
        };
        match self.session.state().formula(row, col) {
            Some(f) => clipboard::Cell::Formula {
                source: f.source.clone(),
                display,
            },
            None => match self.session.value(row, col) {
                Some(Value::Blank) | None => clipboard::Cell::Blank,
                Some(v) => clipboard::Cell::Value(v),
            },
        }
    }

    /// Copies the selection. `cut` additionally marks it to be cleared when the
    /// block is pasted.
    fn copy(&mut self, cut: bool) -> Outcome {
        let Some((r0, r1, c0, c1)) = self.selection_range() else {
            return Outcome::default();
        };
        let (rows, cols) = (r1 - r0 + 1, c1 - c0 + 1);
        let mut cells = Vec::with_capacity(rows * cols);
        for r in r0..=r1 {
            for c in c0..=c1 {
                cells.push(self.clip_cell(r, c));
            }
        }
        self.clipboard.write(Block {
            rows,
            cols,
            cells,
            origin: (r0, c0),
            // A copy makes a second formula, which should say the same thing
            // about its own row; a cut *moves* the one formula, which should go
            // on meaning what it meant. Excel's rule, and not arbitrary.
            translate: !cut,
        });
        self.pending_cut = cut.then_some((r0, r1, c0, c1));
        self.status = match self.clipboard.unavailable() {
            Some(why) => String::from(why),
            None => String::new(),
        };
        Outcome { redraw: true }
    }

    /// Pastes with the block's top-left at the active cell.
    fn paste(&mut self) -> Outcome {
        let Some(block) = self.clipboard.read() else {
            return Outcome::default();
        };
        if block.is_empty() {
            return Outcome::default();
        }
        let (top, left) = self.active_index();

        // The block is a snapshot taken at copy time, so clearing the source
        // *before* writing is safe even when the two ranges overlap — there is
        // nothing left to read out of the document.
        if let Some((r0, r1, c0, c1)) = self.pending_cut.take() {
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let _ = self.session.apply(Command::ClearCell {
                        row: r as u32,
                        col: c as u32,
                    });
                }
            }
        }

        // One delta for the whole block: where it landed minus where it came
        // from. Every formula in it moves by the same amount, which is what
        // makes a pasted column of `=A1*2` read its own rows.
        let (dr, dc) = (
            top as i64 - block.origin.0 as i64,
            left as i64 - block.origin.1 as i64,
        );

        let mut refused = 0usize;
        for br in 0..block.rows {
            for bc in 0..block.cols {
                let (r, c) = (top + br, left + bc);
                if r >= self.rows.len() || c >= self.cols.len() {
                    // Off the edge of the sheet. Excel grows the grid; this one
                    // does not yet, so the overflow is dropped and counted
                    // rather than silently discarded.
                    refused += 1;
                    continue;
                }
                let (row, col) = (r as u32, c as u32);
                let command = match block.get(br, bc) {
                    None | Some(clipboard::Cell::Blank) => Command::ClearCell { row, col },
                    Some(clipboard::Cell::Value(v)) => Command::SetValue {
                        row,
                        col,
                        value: v.clone(),
                    },
                    Some(clipboard::Cell::Formula { source, .. }) => {
                        let source = if block.translate {
                            usk_formula::translate::translate(source, dr, dc)
                        } else {
                            source.clone()
                        };
                        Command::SetFormula { row, col, source }
                    }
                };
                if self.session.apply(command).is_err() {
                    refused += 1;
                }
            }
        }
        self.status = if refused > 0 {
            format!("{refused} cell(s) fell outside the sheet and were not pasted")
        } else {
            String::new()
        };
        Outcome { redraw: true }
    }

    // ---------------------------------------------------------------- filling

    /// Writes the fill a released drag described.
    fn apply_fill(
        &mut self,
        origin: (usize, usize, usize, usize),
        target: (usize, usize, usize, usize),
    ) -> Outcome {
        let (or0, or1, oc0, oc1) = origin;
        let (tr0, tr1, tc0, tc1) = target;
        let vertical = tr0 != or0 || tr1 != or1;

        // One line at a time: a vertical fill runs each column independently
        // down its own series, which is what makes filling a two-column table
        // do the right thing in both columns at once.
        let lines: Vec<usize> = if vertical {
            (oc0..=oc1).collect()
        } else {
            (or0..=or1).collect()
        };

        for line in lines {
            // The source run, read in drag order so that filling upward
            // extrapolates upward.
            let forward = if vertical { tr1 > or1 } else { tc1 > oc1 };
            let source_positions: Vec<usize> = if vertical {
                if forward {
                    (or0..=or1).collect()
                } else {
                    (or0..=or1).rev().collect()
                }
            } else if forward {
                (oc0..=oc1).collect()
            } else {
                (oc0..=oc1).rev().collect()
            };
            let source: Vec<clipboard::Cell> = source_positions
                .iter()
                .map(|p| {
                    if vertical {
                        self.clip_cell(*p, line)
                    } else {
                        self.clip_cell(line, *p)
                    }
                })
                .collect();

            // The cells to write, also in drag order.
            let targets: Vec<usize> = if vertical {
                if forward {
                    (or1 + 1..=tr1).collect()
                } else {
                    (tr0..or0).rev().collect()
                }
            } else if forward {
                (oc1 + 1..=tc1).collect()
            } else {
                (tc0..oc0).rev().collect()
            };
            if targets.is_empty() {
                continue;
            }

            let filled = fill::fill(&source, targets.len(), |i| {
                // How far this target has travelled from the source cell it
                // takes after — which is what decides a formula's references.
                let from = source_positions[i % source_positions.len()];
                let to = targets[i];
                let delta = to as i64 - from as i64;
                if vertical {
                    (delta, 0)
                } else {
                    (0, delta)
                }
            });

            for (i, value) in filled.into_iter().enumerate() {
                let at = targets[i];
                let (row, col) = if vertical {
                    (at as u32, line as u32)
                } else {
                    (line as u32, at as u32)
                };
                let command = match value {
                    Filled::Blank => Command::ClearCell { row, col },
                    Filled::Value(v) => Command::SetValue { row, col, value: v },
                    Filled::Formula(source) => Command::SetFormula { row, col, source },
                };
                let _ = self.session.apply(command);
            }
        }

        // The selection grows to cover what was filled, as Excel's does.
        self.rebuild_selection(target);
        Outcome { redraw: true }
    }

    fn rebuild_selection(&mut self, (r0, r1, c0, c1): (usize, usize, usize, usize)) {
        let (Some(ar), Some(ac), Some(hr), Some(hc)) = (
            self.rows.id_at(r0),
            self.cols.id_at(c0),
            self.rows.id_at(r1),
            self.cols.id_at(c1),
        ) else {
            return;
        };
        self.anchor = (RowId(ar), ColId(ac));
        self.active = (RowId(hr), ColId(hc));
    }

    fn clear_selection(&mut self) -> Outcome {
        let Some((r0, r1, c0, c1)) = self.selection_range() else {
            return Outcome::default();
        };
        let mut wrote = false;
        for r in r0..=r1 {
            for c in c0..=c1 {
                if self
                    .session
                    .apply(Command::ClearCell {
                        row: r as u32,
                        col: c as u32,
                    })
                    .is_ok()
                {
                    wrote = true;
                }
            }
        }
        Outcome { redraw: wrote }
    }

    fn history(&mut self, command: Command) -> Outcome {
        self.editor = None;
        let report = self.session.apply(command);
        if let Ok(report) = report {
            if report.blocked > 0 {
                // docs/11's blocked-and-narrowed, surfaced rather than silent.
                self.status = format!(
                    "{} change{} kept to preserve another actor's work",
                    report.blocked,
                    if report.blocked == 1 { "" } else { "s" }
                );
            } else {
                self.status.clear();
            }
        }
        // Undo can be structural — it can put a deleted row back — so the axes
        // are rebuilt rather than guessed at.
        self.rebuild_axes();
        Outcome { redraw: true }
    }

    fn structural<F: Fn((u32, u32)) -> Command>(&mut self, make: F) -> Outcome {
        self.editor = None;
        let (r, c) = self.active_index();
        match self.session.apply(make((r as u32, c as u32))) {
            Ok(_) => self.status.clear(),
            Err(err) => {
                self.status = format!("{err:?}");
                return Outcome { redraw: true };
            }
        }
        self.rebuild_axes();
        Outcome { redraw: true }
    }

    /// Rebuilds the axes after a structural edit and re-resolves everything
    /// that pointed into the old ones.
    ///
    /// `Viewport::reanchor` is the load-bearing call: the viewport keeps its
    /// identity anchor, so whatever happened above it, the row under the cursor
    /// stays under the cursor (ADR-022). The cursor itself is re-resolved the
    /// same way — by identity first, by position only when the identity is
    /// gone.
    fn rebuild_axes(&mut self) {
        let previous_rows = core::mem::take(&mut self.rows);
        let previous_cols = core::mem::take(&mut self.cols);
        let (rows, cols) = axes(self.session.state(), &self.metrics);
        self.rows = rows;
        self.cols = cols;
        self.view
            .reanchor(&self.rows, &self.cols, &previous_rows, &previous_cols);
        self.active = self.resolve_cell(self.active, &previous_rows, &previous_cols);
        self.anchor = self.resolve_cell(self.anchor, &previous_rows, &previous_cols);
    }

    fn resolve_cell(
        &self,
        cell: (RowId, ColId),
        previous_rows: &Axis,
        previous_cols: &Axis,
    ) -> (RowId, ColId) {
        let row = match self.rows.index_of(cell.0 .0) {
            Some(_) => cell.0,
            None => {
                let at = previous_rows.index_of(cell.0 .0).unwrap_or(0);
                RowId(
                    self.rows
                        .id_at(at.min(self.rows.len().saturating_sub(1)))
                        .unwrap_or(cell.0 .0),
                )
            }
        };
        let col = match self.cols.index_of(cell.1 .0) {
            Some(_) => cell.1,
            None => {
                let at = previous_cols.index_of(cell.1 .0).unwrap_or(0);
                ColId(
                    self.cols
                        .id_at(at.min(self.cols.len().saturating_sub(1)))
                        .unwrap_or(cell.1 .0),
                )
            }
        };
        (row, col)
    }

    // ------------------------------------------------------------------ frame

    /// The rows and columns on screen — the virtual scroll, unchanged by
    /// anything in this file.
    pub fn visible(&self) -> Visible {
        self.view.visible(&self.rows, &self.cols)
    }

    /// The formula bar's reference field: `B4`, or `B4:D9` for a range.
    pub fn reference(&self) -> String {
        label_of(self.selection_range())
    }

    /// Builds one frame's quads.
    ///
    /// The per-frame path is: settle the session (a no-op when nothing was
    /// edited), take the visible span, build quads. No document walk, no
    /// recalculation, no allocation proportional to the sheet — docs/31's *"the
    /// render loop never walks the document"*.
    pub fn frame(&mut self) -> Vec<Quad> {
        let visible = self.view.visible(&self.rows, &self.cols);
        // Resolved **once**, and the bar's label derived from the same answer
        // rather than by calling `reference()` again. `Axis::index_of` is a
        // linear scan over the axis — it has to be, since the order is creation
        // order and not identity order — so resolving the range twice was four
        // needless scans per frame.
        //
        // Worth measuring rather than assuming, because the honest result is
        // that it barely mattered: the frame went **4.18 → 4.09 ms** at 50,000
        // rows, about 2%. Kept because one source of truth for the selection is
        // better than two, not because it bought speed. What actually costs
        // that 4 ms is still unattributed and is filed as TD-65.
        let range = self.selection_range();
        let selection = Selection {
            active: Some(self.active),
            range,
            fill_preview: match self.drag {
                Some(Drag::Fill { preview, .. }) => preview,
                _ => None,
            },
            fill_handle: self.fill_handle(),
        };
        let reference = label_of(range);
        // The composition is spliced in **once**, here, and both the cell
        // overlay and the formula bar are drawn from the same string. Two
        // splices would be two chances for the bar and the cell to disagree
        // about what the user is currently typing.
        let display = self.editor.as_ref().map(Editor::display);
        let bar_content = match &display {
            // While editing, the bar mirrors the editor: they are two views of
            // one buffer, and letting them disagree is how a user loses track
            // of which one they are typing into.
            Some((shown, _, _)) => shown.clone(),
            None => self.source_at(self.active.0, self.active.1),
        };
        let editor =
            self.editor
                .as_ref()
                .zip(display.as_ref())
                .map(|(ed, (shown, caret, composition))| EditView {
                    row: ed.row,
                    col: ed.col,
                    text: shown,
                    caret: *caret,
                    preedit: *composition,
                });
        let (state, engine) = self.session.view();
        let frame = scene::Frame {
            state,
            engine,
            visible: &visible,
            selection,
            editor,
            bar: BarView {
                reference: &reference,
                content: &bar_content,
                status: &self.status,
            },
            size: self.size,
            scale: self.scale,
        };
        let scene = scene::build(&frame, &self.theme, &mut self.text);
        self.ime_area = scene.caret;
        scene.quads
    }

    /// The glyph atlas, when it has changed since the last upload. `None` is
    /// the common case after a moment of use, and uploading a mebibyte every
    /// frame instead would be pure waste.
    pub fn take_atlas_upload(&mut self) -> Option<(u32, &[u8])> {
        if !self.text.is_dirty() {
            return None;
        }
        self.text.mark_uploaded();
        Some((self.text.atlas_size(), self.text.atlas_bytes()))
    }

    pub fn atlas_overflowed(&self) -> bool {
        self.text.overflowed
    }

    /// Every face this session has drawn with, slot 0 (the bundled one) first.
    ///
    /// The record D-125 requires: fallback means a non-Latin run's metrics can
    /// come from a face that differs between hosts, so *which* face is a fact
    /// the shell can state rather than one a user has to guess at from the
    /// pixels. `--script` prints it; a font-shortfall report would read it too.
    pub fn resolved_faces(&self) -> Vec<&str> {
        self.text.face_names()
    }

    /// The faces *one string* resolves to on this host, and how many of its
    /// characters no face could draw.
    ///
    /// [`App::resolved_faces`] answers for the whole session, which cannot
    /// attribute a face to a script: once `Yu Gothic` is loaded for kana it is
    /// in the session's list whether or not it is what drew `中文`. This lays
    /// the string out and reports the faces *that* run used, which is the record
    /// D-125 asks for at the granularity D-127 needs it — and the granularity at
    /// which TD-83 is visible at all.
    pub fn faces_for(&mut self, text: &str) -> (Vec<String>, u32) {
        let run = self.text.layout(text, crate::text::CELL_PX, self.scale);
        let names = run
            .faces
            .iter()
            .filter_map(|slot| self.text.face_name(*slot).map(String::from))
            .collect();
        (names, run.unresolved)
    }
}

/// An ordinal rectangle as A1 text: `B4`, or `B4:D9` for a range.
fn label_of(range: Option<(usize, usize, usize, usize)>) -> String {
    let Some((r0, r1, c0, c1)) = range else {
        return String::new();
    };
    let one = format!("{}{}", column_label(c0), row_label(r0));
    if r0 == r1 && c0 == c1 {
        one
    } else {
        format!("{one}:{}{}", column_label(c1), row_label(r1))
    }
}

/// Builds both axes from the live order.
fn axes(state: &State, metrics: &Metrics) -> (Axis, Axis) {
    let row_ids: Vec<OpId> = state.row_order().into_iter().map(|r| r.0).collect();
    let col_ids: Vec<OpId> = state.col_order().into_iter().map(|c| c.0).collect();
    (
        Axis::build(&row_ids, |o| metrics.row_height(RowId(o))),
        Axis::build(&col_ids, |o| metrics.col_width(ColId(o))),
    )
}

/// How typed text becomes a value.
///
/// Excel's cell-entry rule, which is *not* the same as its display rule: a
/// leading apostrophe forces text, `TRUE`/`FALSE` become booleans, and anything
/// numeric-looking becomes a number under the compat profile — the same
/// `coerce_input` the CSV importer uses, so a value typed in and a value
/// imported cannot disagree. What is deliberately absent is any inference of a
/// *format* (dates, currency): that is TD-36's number-format grammar, and
/// guessing at it here would put a second, worse copy of it in the shell.
pub fn literal(text: &str) -> Value {
    if let Some(rest) = text.strip_prefix('\'') {
        return Value::Text(String::from(rest));
    }
    match text {
        "TRUE" | "true" | "True" => Value::Bool(true),
        "FALSE" | "false" | "False" => Value::Bool(false),
        other => Profile::Compat.coerce_input(other),
    }
}

#[cfg(test)]
pub mod harness {
    //! An [`App`] over an empty sheet, with no window and no GPU.
    //!
    //! Public to the crate because `scene.rs` proves its half of TD-61 against
    //! the same app the editing tests drive — two tests of one thing, not two
    //! things that resemble each other.

    use super::*;
    use usk_oplog::{Anchor, Op, OpLog, Payload};
    use usk_types::ActorId;

    pub fn empty_log(rows: usize, cols: usize) -> OpLog {
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

    /// An app whose clipboard never touches the OS, so the suite runs on a
    /// headless CI box and one test cannot see another's copy.
    pub fn app(rows: usize, cols: usize) -> App {
        let session = Session::from_log(ActorId(1), empty_log(rows, cols));
        App::open_detached(session, 800.0, 400.0, 1.0).expect("the bundled font must load")
    }

    /// Selects the rectangle `(r0, r1, c0, c1)` by identity, the way a drag
    /// would leave it.
    pub fn select(app: &mut App, r0: usize, r1: usize, c0: usize, c1: usize) {
        app.anchor = cell_id(app, r0, c0);
        app.active = cell_id(app, r1, c1);
    }

    /// Types `text` into the cell at `(row, col)` and commits it.
    pub fn put(app: &mut App, row: usize, col: usize, text: &str) {
        select(app, row, row, col, col);
        type_text(app, text);
        press(app, crate::input::Key::Enter, Mods::NONE);
    }

    /// Types a string through the keymap, exactly as `script.rs` and the window
    /// do — so a test can never pass against an editing path a user cannot
    /// reach.
    pub fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            if let Some(intent) =
                crate::input::translate(crate::input::Key::Character(c), Mods::NONE, app.mode())
            {
                app.handle(intent);
            }
        }
    }

    pub fn press(app: &mut App, key: crate::input::Key, mods: Mods) {
        if let Some(intent) = crate::input::translate(key, mods, app.mode()) {
            app.handle(intent);
        }
    }

    /// The value at view ordinals, as the grid would render it.
    pub fn shown(app: &mut App, row: usize, col: usize) -> Option<String> {
        let (rows, cols) = app.axes();
        let (r, c) = (rows.id_at(row)?, cols.id_at(col)?);
        let value = app.value(RowId(r), ColId(c))?;
        text::render_value(&value)
    }

    pub fn cell_id(app: &App, row: usize, col: usize) -> (RowId, ColId) {
        (
            RowId(app.rows.id_at(row).unwrap()),
            ColId(app.cols.id_at(col).unwrap()),
        )
    }

    use crate::input::Mods;
}

#[cfg(test)]
mod tests {
    use super::harness::*;
    use super::*;
    use crate::input::{Key, Mods};

    #[test]
    fn a_click_selects_the_cell_under_the_pointer() {
        let mut app = app(50, 10);
        let (ox, oy) = app.theme().grid_origin();
        // Third row, second column: 20 px rows and 64 px columns by default.
        let hit = app.pointer_down(ox + 64.0 + 5.0, oy + 40.0 + 5.0, false);
        assert!(hit.redraw);
        assert_eq!(app.active, cell_id(&app, 2, 1));
        assert_eq!(app.reference(), "B3");
    }

    #[test]
    fn a_click_outside_the_grid_selects_nothing() {
        let mut app = app(50, 10);
        let before = app.active;
        // In the formula bar.
        assert!(!app.pointer_down(300.0, 4.0, false).redraw);
        assert_eq!(app.active, before);
    }

    #[test]
    fn arrow_keys_move_the_active_cell_and_shift_extends_the_selection() {
        let mut app = app(50, 10);
        press(&mut app, Key::Down, Mods::NONE);
        press(&mut app, Key::Right, Mods::NONE);
        assert_eq!(app.reference(), "B2");
        press(&mut app, Key::Down, Mods::shift());
        press(&mut app, Key::Right, Mods::shift());
        // The anchor stayed where the unextended move left it.
        assert_eq!(app.reference(), "B2:C3");
        // And Escape collapses it back onto the active cell.
        press(&mut app, Key::Escape, Mods::NONE);
        assert_eq!(app.reference(), "C3");
    }

    #[test]
    fn the_cursor_cannot_leave_the_sheet() {
        let mut app = app(3, 3);
        for _ in 0..10 {
            press(&mut app, Key::Up, Mods::NONE);
            press(&mut app, Key::Left, Mods::NONE);
        }
        assert_eq!(app.reference(), "A1");
        for _ in 0..10 {
            press(&mut app, Key::Down, Mods::NONE);
            press(&mut app, Key::Right, Mods::NONE);
        }
        assert_eq!(app.reference(), "C3");
    }

    #[test]
    fn typing_a_character_opens_the_editor_holding_it() {
        let mut app = app(50, 10);
        press(&mut app, Key::Character('4'), Mods::NONE);
        let editor = app.editor().expect("typing must open the editor");
        assert_eq!(editor.text, "4");
        assert_eq!(editor.caret, 1);
    }

    #[test]
    fn a_literal_is_committed_and_enter_advances() {
        let mut app = app(50, 10);
        type_text(&mut app, "125");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("125"));
        assert!(app.editor().is_none());
        assert_eq!(app.reference(), "A2", "Enter must step down");
    }

    #[test]
    fn text_stays_text_and_an_apostrophe_forces_it() {
        let mut app = app(50, 10);
        type_text(&mut app, "revenue");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("revenue"));

        // The gene-symbol case: `1E2` is a number under the compat profile, and
        // a leading apostrophe is how a user says otherwise.
        type_text(&mut app, "1E2");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 1, 0).as_deref(), Some("100"));
        type_text(&mut app, "'1E2");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 2, 0).as_deref(), Some("1E2"));
    }

    #[test]
    fn a_formula_is_evaluated_and_its_result_is_what_the_cell_shows() {
        // The regression test for TD-61. Before the engine was wired in, a
        // formula cell had no value at all and rendered blank.
        let mut app = app(50, 10);
        type_text(&mut app, "=1+1");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("2"));
    }

    #[test]
    fn a_formula_reads_other_cells_and_recalculates_when_they_change() {
        let mut app = app(50, 10);
        type_text(&mut app, "10");
        press(&mut app, Key::Enter, Mods::NONE);
        type_text(&mut app, "32");
        press(&mut app, Key::Enter, Mods::NONE);
        type_text(&mut app, "=A1+A2");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 2, 0).as_deref(), Some("42"));

        // Change a precedent: the dependent must follow, through the engine's
        // incremental path and without anything here asking it to.
        press(&mut app, Key::Character('z'), Mods::ctrl()); // undo the formula
        press(&mut app, Key::Character('y'), Mods::ctrl()); // and put it back
        let (rows, cols) = app.axes();
        let (r0, c0) = (rows.id_at(0).unwrap(), cols.id_at(0).unwrap());
        assert_eq!(shown(&mut app, 2, 0).as_deref(), Some("42"));

        app.active = (RowId(r0), ColId(c0));
        app.anchor = app.active;
        type_text(&mut app, "100");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("100"));
        assert_eq!(
            shown(&mut app, 2, 0).as_deref(),
            Some("132"),
            "editing a precedent must recalculate the dependent"
        );
    }

    #[test]
    fn a_range_formula_over_a_column_computes() {
        let mut app = app(50, 10);
        for n in 1..=5 {
            type_text(&mut app, &n.to_string());
            press(&mut app, Key::Enter, Mods::NONE);
        }
        type_text(&mut app, "=SUM(A1:A5)");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 5, 0).as_deref(), Some("15"));
    }

    #[test]
    fn a_division_by_zero_renders_as_div0_and_not_as_a_blank() {
        let mut app = app(50, 10);
        type_text(&mut app, "=1/0");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("#DIV/0!"));
    }

    #[test]
    fn an_error_propagates_into_the_cells_that_read_it() {
        let mut app = app(50, 10);
        type_text(&mut app, "=1/0");
        press(&mut app, Key::Enter, Mods::NONE);
        type_text(&mut app, "=A1*2");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 1, 0).as_deref(), Some("#DIV/0!"));
    }

    #[test]
    fn f2_edits_the_source_and_not_the_displayed_value() {
        let mut app = app(50, 10);
        type_text(&mut app, "=6*7");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Up, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("42"));
        press(&mut app, Key::F2, Mods::NONE);
        assert_eq!(
            app.editor().map(|e| e.text.as_str()),
            Some("=6*7"),
            "F2 must open the formula, not its result"
        );
        assert_eq!(app.editor().map(|e| e.caret), Some(4));
    }

    #[test]
    fn escape_abandons_an_edit_and_changes_nothing() {
        let mut app = app(50, 10);
        type_text(&mut app, "7");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Up, Mods::NONE);
        type_text(&mut app, "999");
        press(&mut app, Key::Escape, Mods::NONE);
        assert!(app.editor().is_none());
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("7"));
    }

    #[test]
    fn the_caret_moves_by_characters_and_edits_land_where_it_is() {
        let mut app = app(50, 10);
        type_text(&mut app, "abd");
        press(&mut app, Key::Left, Mods::NONE);
        type_text(&mut app, "c");
        assert_eq!(app.editor().map(|e| e.text.as_str()), Some("abcd"));
        press(&mut app, Key::Home, Mods::NONE);
        press(&mut app, Key::Delete, Mods::NONE);
        assert_eq!(app.editor().map(|e| e.text.as_str()), Some("bcd"));
        press(&mut app, Key::End, Mods::NONE);
        press(&mut app, Key::Backspace, Mods::NONE);
        assert_eq!(app.editor().map(|e| e.text.as_str()), Some("bc"));
    }

    #[test]
    fn the_caret_never_splits_a_multibyte_character() {
        // A Backspace that moved one *byte* would panic on the next insert.
        let mut app = app(50, 10);
        type_text(&mut app, "café");
        press(&mut app, Key::Backspace, Mods::NONE);
        assert_eq!(app.editor().map(|e| e.text.as_str()), Some("caf"));
        type_text(&mut app, "é");
        press(&mut app, Key::Left, Mods::NONE);
        type_text(&mut app, "x");
        assert_eq!(app.editor().map(|e| e.text.as_str()), Some("cafxé"));
    }

    #[test]
    fn delete_clears_the_whole_selection() {
        let mut app = app(50, 10);
        for _ in 0..3 {
            type_text(&mut app, "5");
            press(&mut app, Key::Enter, Mods::NONE);
        }
        press(&mut app, Key::Character('z'), Mods::ctrl());
        press(&mut app, Key::Character('y'), Mods::ctrl());
        // Select A1:A3 and clear it.
        let (rows, cols) = app.axes();
        app.active = (RowId(rows.id_at(0).unwrap()), ColId(cols.id_at(0).unwrap()));
        app.anchor = app.active;
        press(&mut app, Key::Down, Mods::shift());
        press(&mut app, Key::Down, Mods::shift());
        assert_eq!(app.reference(), "A1:A3");
        press(&mut app, Key::Delete, Mods::NONE);
        for r in 0..3 {
            assert_eq!(shown(&mut app, r, 0), None, "row {r} should be blank");
        }
    }

    #[test]
    fn undo_and_redo_walk_the_reducers_history() {
        let mut app = app(50, 10);
        type_text(&mut app, "1");
        press(&mut app, Key::Enter, Mods::NONE);
        type_text(&mut app, "2");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 1, 0).as_deref(), Some("2"));
        press(&mut app, Key::Character('z'), Mods::ctrl());
        assert_eq!(shown(&mut app, 1, 0), None);
        press(&mut app, Key::Character('z'), Mods::ctrl());
        assert_eq!(shown(&mut app, 0, 0), None);
        press(&mut app, Key::Character('y'), Mods::ctrl());
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("1"));
        press(&mut app, Key::Character('y'), Mods::ctrl());
        assert_eq!(shown(&mut app, 1, 0).as_deref(), Some("2"));
    }

    #[test]
    fn an_empty_commit_clears_the_cell() {
        let mut app = app(50, 10);
        type_text(&mut app, "9");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Up, Mods::NONE);
        press(&mut app, Key::Backspace, Mods::NONE); // opens an empty editor
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0), None);
    }

    #[test]
    fn a_bare_equals_sign_is_text_and_not_a_formula() {
        let mut app = app(50, 10);
        type_text(&mut app, "=");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("="));
    }

    #[test]
    fn a_formula_whose_references_cannot_bind_is_refused_and_the_text_survives() {
        let mut app = app(5, 5);
        // Column ZZ does not exist on a five-column sheet.
        type_text(&mut app, "=ZZ99");
        press(&mut app, Key::Enter, Mods::NONE);
        assert!(
            app.editor().is_some(),
            "a refused formula must stay in the editor to be fixed"
        );
        assert!(!app.status.is_empty(), "and the refusal must be visible");
        assert_eq!(shown(&mut app, 0, 0), None);
    }

    #[test]
    fn clicking_away_from_an_open_editor_commits_it() {
        let mut app = app(50, 10);
        type_text(&mut app, "77");
        let (ox, oy) = app.theme().grid_origin();
        app.pointer_down(ox + 5.0, oy + 100.0, false);
        assert!(app.editor().is_none());
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("77"));
    }

    #[test]
    fn ctrl_arrow_jumps_to_the_edge_of_the_data_region() {
        let mut app = app(60, 10);
        // A1:A3 filled, A4:A9 empty, A10 filled.
        for _ in 0..3 {
            type_text(&mut app, "1");
            press(&mut app, Key::Enter, Mods::NONE);
        }
        let (rows, cols) = app.axes();
        app.active = (RowId(rows.id_at(9).unwrap()), ColId(cols.id_at(0).unwrap()));
        app.anchor = app.active;
        type_text(&mut app, "1");
        press(&mut app, Key::Enter, Mods::NONE);

        let (rows, cols) = app.axes();
        app.active = (RowId(rows.id_at(0).unwrap()), ColId(cols.id_at(0).unwrap()));
        app.anchor = app.active;
        // From inside the run: to the last cell of the run.
        press(&mut app, Key::Down, Mods::ctrl());
        assert_eq!(app.reference(), "A3");
        // From the edge of a run: across the gap to the next filled cell.
        press(&mut app, Key::Down, Mods::ctrl());
        assert_eq!(app.reference(), "A10");
        // With nothing ahead: to the end of the sheet.
        press(&mut app, Key::Down, Mods::ctrl());
        assert_eq!(app.reference(), "A60");
    }

    #[test]
    fn ctrl_home_and_ctrl_end_reach_the_corners() {
        let mut app = app(40, 7);
        press(&mut app, Key::End, Mods::ctrl());
        assert_eq!(app.reference(), "G40");
        press(&mut app, Key::Home, Mods::ctrl());
        assert_eq!(app.reference(), "A1");
    }

    #[test]
    fn moving_the_cursor_scrolls_it_into_view_and_no_further() {
        let mut app = app(1_000, 10);
        let before = app.viewport().rows;
        press(&mut app, Key::Down, Mods::NONE);
        assert_eq!(
            app.viewport().rows,
            before,
            "a visible cell must not scroll"
        );
        press(&mut app, Key::End, Mods::ctrl());
        let visible = app.visible();
        assert!(
            visible.rows.iter().any(|s| RowId(s.id) == app.active.0),
            "the active cell must be on screen after a jump to the far corner"
        );
    }

    #[test]
    fn page_down_moves_about_a_screen() {
        let mut app = app(1_000, 10);
        let page = app.visible().rows.len();
        press(&mut app, Key::PageDown, Mods::NONE);
        let (rows, _) = app.axes();
        let landed = rows.index_of(app.active.0 .0).unwrap();
        assert!(
            landed >= page - 2 && landed <= page,
            "landed on row {landed} for a {page}-row page"
        );
    }

    // -------------------------------------------------- identity under edits

    #[test]
    fn inserting_a_row_above_the_viewport_does_not_move_it() {
        // ADR-022's property, asserted at the *application* layer: the kernel
        // proves the viewport keeps its anchor, and this proves the shell
        // actually re-anchors rather than rebuilding from a pixel offset.
        let mut app = app(1_000, 10);
        app.scroll(0.0, 4_000.0);
        let anchored_to = app.viewport().rows.id;
        let offset = app.viewport().rows.offset;
        let first_visible = app.visible().rows[0].id;

        // Insert at the very top, far above anything on screen.
        let (rows, cols) = app.axes();
        app.active = (RowId(rows.id_at(0).unwrap()), ColId(cols.id_at(0).unwrap()));
        app.anchor = app.active;
        app.handle(Intent::InsertRow);

        assert_eq!(app.viewport().rows.id, anchored_to);
        assert_eq!(app.viewport().rows.offset, offset);
        assert_eq!(app.visible().rows[0].id, first_visible);
    }

    #[test]
    fn the_cursor_stays_on_its_cell_when_a_row_is_inserted_above_it() {
        let mut app = app(50, 10);
        press(&mut app, Key::Down, Mods::NONE);
        press(&mut app, Key::Down, Mods::NONE);
        type_text(&mut app, "marker");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Up, Mods::NONE);
        let cursor = app.active;
        assert_eq!(app.reference(), "A3");

        // Insert above: the identity is unchanged, the A1 label is not — which
        // is exactly what "A1 is a view over identities" means (DP-A6).
        let (rows, cols) = app.axes();
        app.active = (RowId(rows.id_at(0).unwrap()), ColId(cols.id_at(0).unwrap()));
        app.anchor = app.active;
        app.handle(Intent::InsertRow);
        app.active = cursor;
        app.anchor = cursor;
        assert_eq!(app.reference(), "A4");
        assert_eq!(shown(&mut app, 3, 0).as_deref(), Some("marker"));
    }

    #[test]
    fn deleting_the_row_the_cursor_is_on_leaves_the_cursor_somewhere_live() {
        let mut app = app(10, 5);
        press(&mut app, Key::Down, Mods::NONE);
        press(&mut app, Key::Down, Mods::NONE);
        app.handle(Intent::DeleteRow);
        let (rows, _) = app.axes();
        assert_eq!(rows.len(), 9);
        assert!(
            rows.index_of(app.active.0 .0).is_some(),
            "the cursor must land on a live row"
        );
    }

    #[test]
    fn a_formula_follows_its_precedent_when_a_row_is_inserted_between_them() {
        // The canonical identity-reference case, driven from the UI: the
        // formula's binding is to identities, so it must still read the same
        // cell after the rows below it shift.
        let mut app = app(20, 5);
        type_text(&mut app, "8");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Down, Mods::NONE);
        press(&mut app, Key::Down, Mods::NONE);
        type_text(&mut app, "=A1*3");
        press(&mut app, Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 3, 0).as_deref(), Some("24"));

        // Insert a row at position 2, between the value and the formula.
        let (rows, cols) = app.axes();
        app.active = (RowId(rows.id_at(1).unwrap()), ColId(cols.id_at(0).unwrap()));
        app.anchor = app.active;
        app.handle(Intent::InsertRow);
        assert_eq!(
            shown(&mut app, 4, 0).as_deref(),
            Some("24"),
            "the formula moved down a row and still reads A1"
        );
    }

    // ------------------------------------------------------------ the frame

    #[test]
    fn a_frame_costs_the_window_and_not_the_document() {
        // docs/31: "the render loop never walks the document". A million-row
        // sheet and a fifty-row sheet must produce the same order of quads.
        let mut small = app(50, 10);
        let mut large = app(200_000, 10);
        let small_quads = small.frame().len();
        let large_quads = large.frame().len();
        assert!(
            large_quads < small_quads * 2,
            "{large_quads} quads for 200k rows against {small_quads} for 50 — \
             the frame is walking the document"
        );
    }

    #[test]
    fn the_editor_is_drawn_and_the_cell_under_it_is_not() {
        let mut app = app(50, 10);
        type_text(&mut app, "5");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Up, Mods::NONE);
        let committed = app.frame().len();
        press(&mut app, Key::F2, Mods::NONE);
        let editing = app.frame().len();
        // The overlay adds a fill, four border quads and a caret.
        assert!(
            editing > committed,
            "an open editor must add quads ({editing} vs {committed})"
        );
    }

    #[test]
    fn the_formula_bar_shows_the_source_and_the_grid_shows_the_result() {
        let mut app = app(50, 10);
        type_text(&mut app, "=3+4");
        press(&mut app, Key::Enter, Mods::NONE);
        press(&mut app, Key::Up, Mods::NONE);
        let (row, col) = app.active;
        assert_eq!(app.source_at(row, col), "=3+4");
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("7"));
    }

    #[test]
    fn resizing_changes_how_many_cells_are_visible_and_nothing_else() {
        let mut app = app(1_000, 40);
        app.scroll(0.0, 2_000.0);
        let anchor = app.viewport().rows;
        let top = app.visible().rows[0].id;
        let tall = app.visible().rows.len();

        app.resize(800.0, 200.0, 1.0);
        let short = app.visible().rows.len();
        assert!(
            short < tall,
            "{short} rows visible in a quarter of the height of {tall}"
        );
        // The window got smaller; *where* it looks did not. A resize that moved
        // the anchor would make dragging a window edge scroll the sheet.
        assert_eq!(app.viewport().rows, anchor);
        assert_eq!(app.visible().rows[0].id, top);
    }

    // ------------------------------------------------- clipboard (TD-64 paid)

    #[test]
    fn copy_and_paste_move_a_block_of_values() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        put(&mut app, 0, 1, "2");
        put(&mut app, 1, 0, "3");
        put(&mut app, 1, 1, "4");

        select(&mut app, 0, 1, 0, 1);
        app.handle(Intent::Copy);
        select(&mut app, 5, 5, 3, 3);
        app.handle(Intent::Paste);

        assert_eq!(shown(&mut app, 5, 3).as_deref(), Some("1"));
        assert_eq!(shown(&mut app, 5, 4).as_deref(), Some("2"));
        assert_eq!(shown(&mut app, 6, 3).as_deref(), Some("3"));
        assert_eq!(shown(&mut app, 6, 4).as_deref(), Some("4"));
        // The source is untouched by a copy.
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("1"));
    }

    #[test]
    fn a_copied_formula_moves_its_relative_references() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "10");
        put(&mut app, 1, 0, "20");
        put(&mut app, 0, 1, "=A1*2");
        assert_eq!(shown(&mut app, 0, 1).as_deref(), Some("20"));

        select(&mut app, 0, 0, 1, 1);
        app.handle(Intent::Copy);
        select(&mut app, 1, 1, 1, 1);
        app.handle(Intent::Paste);

        // One row down, so it reads A2 and not A1 — the whole reason a copy
        // translates.
        assert_eq!(
            app.source_at(cell_id(&app, 1, 1).0, cell_id(&app, 1, 1).1),
            "=A2*2"
        );
        assert_eq!(shown(&mut app, 1, 1).as_deref(), Some("40"));
    }

    #[test]
    fn a_pinned_reference_survives_a_copy_unmoved() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "3");
        put(&mut app, 0, 5, "100");
        put(&mut app, 0, 1, "=A1*$F$1");

        select(&mut app, 0, 0, 1, 1);
        app.handle(Intent::Copy);
        select(&mut app, 4, 4, 1, 1);
        app.handle(Intent::Paste);
        let (r, c) = cell_id(&app, 4, 1);
        assert_eq!(app.source_at(r, c), "=A5*$F$1");
    }

    #[test]
    fn a_cut_clears_the_source_only_when_it_is_pasted() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "7");
        select(&mut app, 0, 0, 0, 0);
        app.handle(Intent::Cut);
        // Excel defers the removal; so does this. A cut followed by nothing
        // must leave the document exactly as it was.
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("7"));

        select(&mut app, 3, 3, 2, 2);
        app.handle(Intent::Paste);
        assert_eq!(shown(&mut app, 3, 2).as_deref(), Some("7"));
        assert_eq!(shown(&mut app, 0, 0), None, "the source clears on paste");
    }

    #[test]
    fn a_cut_formula_keeps_pointing_where_it_pointed() {
        // A cut *moves* the formula, so it should go on meaning what it meant.
        // This is the one place cut and copy genuinely differ.
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "5");
        put(&mut app, 0, 1, "=A1*2");
        select(&mut app, 0, 0, 1, 1);
        app.handle(Intent::Cut);
        select(&mut app, 4, 4, 1, 1);
        app.handle(Intent::Paste);
        let (r, c) = cell_id(&app, 4, 1);
        assert_eq!(app.source_at(r, c), "=A1*2", "a cut must not translate");
    }

    #[test]
    fn a_blank_cell_in_a_copied_block_clears_what_it_lands_on() {
        // Pasting must overwrite, not merge: a block is what it is, holes
        // included, and a paste that left old values showing through would be
        // the worst kind of quiet.
        let mut app = app(20, 8);
        put(&mut app, 5, 0, "old");
        put(&mut app, 0, 0, "new");
        // A1:A2 where A2 is blank.
        select(&mut app, 0, 1, 0, 0);
        app.handle(Intent::Copy);
        select(&mut app, 4, 4, 0, 0);
        app.handle(Intent::Paste);
        assert_eq!(shown(&mut app, 4, 0).as_deref(), Some("new"));
        assert_eq!(shown(&mut app, 5, 0), None, "the hole must clear the cell");
    }

    #[test]
    fn a_paste_that_would_run_off_the_sheet_is_clipped_and_says_so() {
        let mut app = app(6, 4);
        put(&mut app, 0, 0, "1");
        put(&mut app, 1, 0, "2");
        select(&mut app, 0, 1, 0, 0);
        app.handle(Intent::Copy);
        // One row from the bottom, so the second row has nowhere to go.
        select(&mut app, 5, 5, 0, 0);
        app.handle(Intent::Paste);
        assert_eq!(shown(&mut app, 5, 0).as_deref(), Some("1"));
        assert!(!app.status.is_empty(), "the clipped cell must be reported");
    }

    #[test]
    fn a_paste_is_one_undo_step_per_cell_and_undo_reaches_all_of_it() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        put(&mut app, 0, 1, "2");
        select(&mut app, 0, 0, 0, 1);
        app.handle(Intent::Copy);
        select(&mut app, 4, 4, 0, 0);
        app.handle(Intent::Paste);
        assert_eq!(shown(&mut app, 4, 0).as_deref(), Some("1"));
        assert_eq!(shown(&mut app, 4, 1).as_deref(), Some("2"));
        // Two cells pasted, two commands, two undos. Grouping a paste into one
        // undo step needs a reducer-level transaction, which is TD-70.
        press(&mut app, Key::Character('z'), Mods::ctrl());
        press(&mut app, Key::Character('z'), Mods::ctrl());
        assert_eq!(shown(&mut app, 4, 0), None);
        assert_eq!(shown(&mut app, 4, 1), None);
    }

    #[test]
    fn copying_nothing_and_pasting_nothing_are_both_harmless() {
        let mut app = app(10, 4);
        app.handle(Intent::Paste);
        assert_eq!(shown(&mut app, 0, 0), None);
        select(&mut app, 0, 0, 0, 0);
        app.handle(Intent::Copy);
        app.handle(Intent::Paste);
        assert_eq!(shown(&mut app, 0, 0), None);
    }

    // ------------------------------------------------- fill-drag (TD-64 paid)

    /// Drags the fill handle from the current selection to `(row, col)`.
    fn drag_fill_to(app: &mut App, row: usize, col: usize) {
        let handle = app.fill_handle().expect("the handle must be on screen");
        let down = app.pointer_down(handle[0] + 3.0, handle[1] + 3.0, false);
        assert!(down.redraw, "a press on the handle must start a fill");
        let visible = app.visible();
        let r = visible.rows.iter().find(|s| s.index == row).unwrap();
        let c = visible.cols.iter().find(|s| s.index == col).unwrap();
        let (ox, oy) = app.theme().grid_origin();
        app.pointer_drag(ox + c.at + 4.0, oy + r.at + 4.0);
        app.pointer_up();
    }

    #[test]
    fn the_fill_handle_sits_at_the_corner_of_the_selection() {
        let mut app = app(20, 8);
        select(&mut app, 1, 2, 1, 2);
        let handle = app.fill_handle().expect("on screen");
        let (ox, oy) = app.theme().grid_origin();
        // Bottom-right of C3 at default 64x20 metrics.
        assert!(
            (handle[0] - (ox + 3.0 * 64.0 - 4.5)).abs() < 1.0,
            "{handle:?}"
        );
        assert!(
            (handle[1] - (oy + 3.0 * 20.0 - 4.5)).abs() < 1.0,
            "{handle:?}"
        );
        // And it is not offered while the editor is open — there is nothing to
        // fill from a cell that has not been committed.
        press(&mut app, Key::Character('9'), Mods::NONE);
        assert!(app.fill_handle().is_none());
    }

    #[test]
    fn dragging_the_handle_extrapolates_a_numeric_series() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        put(&mut app, 1, 0, "2");
        select(&mut app, 0, 1, 0, 0);
        drag_fill_to(&mut app, 5, 0);
        for (row, want) in [(2, "3"), (3, "4"), (4, "5"), (5, "6")] {
            assert_eq!(shown(&mut app, row, 0).as_deref(), Some(want), "row {row}");
        }
    }

    #[test]
    fn dragging_the_handle_from_one_cell_repeats_it() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "7");
        select(&mut app, 0, 0, 0, 0);
        drag_fill_to(&mut app, 3, 0);
        for row in 1..=3 {
            assert_eq!(shown(&mut app, row, 0).as_deref(), Some("7"));
        }
    }

    #[test]
    fn dragging_a_formula_down_moves_its_references_with_it() {
        // The gesture the whole feature exists for.
        let mut app = app(20, 8);
        for r in 0..4 {
            put(&mut app, r, 0, &((r + 1) * 10).to_string());
            put(&mut app, r, 1, &((r + 1) * 2).to_string());
        }
        put(&mut app, 0, 2, "=A1+B1");
        assert_eq!(shown(&mut app, 0, 2).as_deref(), Some("12"));

        select(&mut app, 0, 0, 2, 2);
        drag_fill_to(&mut app, 3, 2);

        assert_eq!(shown(&mut app, 1, 2).as_deref(), Some("24"));
        assert_eq!(shown(&mut app, 2, 2).as_deref(), Some("36"));
        assert_eq!(shown(&mut app, 3, 2).as_deref(), Some("48"));
        let (r, c) = cell_id(&app, 3, 2);
        assert_eq!(app.source_at(r, c), "=A4+B4");
    }

    #[test]
    fn filling_sideways_fills_sideways() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "2");
        put(&mut app, 0, 1, "4");
        select(&mut app, 0, 0, 0, 1);
        drag_fill_to(&mut app, 0, 4);
        assert_eq!(shown(&mut app, 0, 2).as_deref(), Some("6"));
        assert_eq!(shown(&mut app, 0, 3).as_deref(), Some("8"));
        assert_eq!(shown(&mut app, 0, 4).as_deref(), Some("10"));
    }

    #[test]
    fn a_vertical_fill_runs_each_column_down_its_own_series() {
        // Filling a two-column table must not mix the columns together.
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        put(&mut app, 1, 0, "2");
        put(&mut app, 0, 1, "100");
        put(&mut app, 1, 1, "200");
        select(&mut app, 0, 1, 0, 1);
        drag_fill_to(&mut app, 3, 1);
        assert_eq!(shown(&mut app, 2, 0).as_deref(), Some("3"));
        assert_eq!(shown(&mut app, 3, 0).as_deref(), Some("4"));
        assert_eq!(shown(&mut app, 2, 1).as_deref(), Some("300"));
        assert_eq!(shown(&mut app, 3, 1).as_deref(), Some("400"));
    }

    #[test]
    fn dragging_the_handle_upward_extrapolates_backwards() {
        let mut app = app(20, 8);
        put(&mut app, 5, 0, "10");
        put(&mut app, 6, 0, "20");
        select(&mut app, 5, 6, 0, 0);
        drag_fill_to(&mut app, 3, 0);
        // Reading up from 10: 0, then -10.
        assert_eq!(shown(&mut app, 4, 0).as_deref(), Some("0"));
        assert_eq!(shown(&mut app, 3, 0).as_deref(), Some("-10"));
    }

    #[test]
    fn a_fill_drag_previews_before_it_commits_and_the_document_is_untouched() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        select(&mut app, 0, 0, 0, 0);
        let handle = app.fill_handle().unwrap();
        app.pointer_down(handle[0] + 3.0, handle[1] + 3.0, false);
        let visible = app.visible();
        let r = visible.rows.iter().find(|s| s.index == 4).unwrap();
        let (ox, oy) = app.theme().grid_origin();
        app.pointer_drag(ox + 4.0, oy + r.at + 4.0);
        // The preview exists...
        assert!(matches!(
            app.drag,
            Some(Drag::Fill {
                preview: Some((0, 4, 0, 0)),
                ..
            })
        ));
        // ...and nothing has been written yet.
        assert_eq!(shown(&mut app, 2, 0), None);
        app.pointer_up();
        assert_eq!(shown(&mut app, 2, 0).as_deref(), Some("1"));
    }

    #[test]
    fn releasing_a_fill_back_over_the_source_fills_nothing() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        select(&mut app, 0, 0, 0, 0);
        let handle = app.fill_handle().unwrap();
        app.pointer_down(handle[0] + 3.0, handle[1] + 3.0, false);
        let (ox, oy) = app.theme().grid_origin();
        app.pointer_drag(ox + 4.0, oy + 4.0);
        app.pointer_up();
        assert_eq!(shown(&mut app, 1, 0), None);
    }

    #[test]
    fn a_press_away_from_the_handle_still_selects() {
        // The handle hangs over its neighbour, so getting this wrong makes the
        // cell below the selection unclickable.
        let mut app = app(20, 8);
        select(&mut app, 0, 0, 0, 0);
        let (ox, oy) = app.theme().grid_origin();
        // Well inside B3, nowhere near A1's handle.
        app.pointer_down(ox + 64.0 + 20.0, oy + 40.0 + 10.0, false);
        assert_eq!(app.reference(), "B3");
        assert!(matches!(app.drag, Some(Drag::Select)));
    }

    #[test]
    fn the_selection_grows_to_cover_what_was_filled() {
        let mut app = app(20, 8);
        put(&mut app, 0, 0, "1");
        select(&mut app, 0, 0, 0, 0);
        drag_fill_to(&mut app, 3, 0);
        assert_eq!(app.reference(), "A1:A4");
    }

    #[test]
    fn a_2x_display_lays_text_out_identically_and_rasterises_it_larger() {
        // docs/31: layout determinism is metric determinism. The *positions* a
        // 2x display produces must be the ones a 1x display produces; only the
        // atlas entry differs.
        let mut engine = TextEngine::new().unwrap();
        let one = engine.layout("1234", text::CELL_PX, 1.0);
        let two = engine.layout("1234", text::CELL_PX, 2.0);
        assert_eq!(
            one.width, two.width,
            "advance must not depend on the display"
        );
        assert_eq!(one.glyphs.len(), two.glyphs.len());
        for (a, b) in one.glyphs.iter().zip(two.glyphs.iter()) {
            assert_eq!(a.at, b.at, "pen positions must not depend on the display");
        }
        let a = one.glyphs[0].glyph;
        let b = two.glyphs[0].glyph;
        assert!(
            b.uv[2] > a.uv[2],
            "the 2x atlas entry must be larger ({} vs {})",
            b.uv[2],
            a.uv[2]
        );
        assert!(
            (b.size[0] - a.size[0]).abs() <= 1.0,
            "but the quad it fills must be the same size in logical pixels"
        );
    }

    // ------------------------------------------------------------------- IME
    //
    // docs/33: *"native composition via the in-cell editor overlay (TSF on
    // Windows, NSTextInputClient on macOS) — never reimplemented"*. winit runs
    // the platform half; what is asserted below is the half this shell owns —
    // when a composition opens an edit, what it may and may not write, and who
    // owns the keyboard while it is in flight. None of it needs a window, which
    // is the point: the alternative is a Japanese keyboard and a person.
    //
    // The strings are real: `にほん` composed from `nihon`, which is the
    // shape every Windows and macOS IME produces — a preedit per keystroke,
    // then one commit.

    /// The composing text most of these use, and its intermediate forms.
    const KANA: [&str; 3] = ["に", "にほ", "にほん"];

    #[test]
    fn a_composition_opens_the_editor_because_no_key_event_ever_arrives() {
        let mut app = app(20, 10);
        // The keystrokes that begin a composition are consumed by the input
        // method: `handle` is never called, so if the preedit did not open the
        // editor the user's first word would go nowhere.
        assert_eq!(app.mode(), crate::input::Mode::Grid);
        app.ime_preedit(KANA[0], Some((3, 3)));
        assert_eq!(app.mode(), crate::input::Mode::Editing);
        assert!(app.composing());
    }

    #[test]
    fn a_composition_is_shown_but_is_not_in_the_cell_until_it_is_committed() {
        let mut app = app(20, 10);
        for step in KANA {
            app.ime_preedit(step, None);
        }
        let editor = app.editor().expect("composing implies an open editor");
        // The buffer the cell would receive is still empty — a composition is
        // the input method's proposal, not the user's text.
        assert_eq!(editor.text, "");
        assert_eq!(editor.display().0, "にほん");
        assert_eq!(shown(&mut app, 0, 0), None);

        app.ime_commit(KANA[2]);
        assert!(!app.composing());
        assert_eq!(app.editor().unwrap().text, "にほん");
        // And still not in the cell: committing a *composition* is not
        // committing an *edit*.
        assert_eq!(shown(&mut app, 0, 0), None);

        press(&mut app, crate::input::Key::Enter, Mods::NONE);
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("にほん"));
    }

    #[test]
    fn a_commit_lands_at_the_caret_and_not_at_the_end() {
        let mut app = app(20, 10);
        type_text(&mut app, "ab");
        press(&mut app, crate::input::Key::Left, Mods::NONE);
        assert_eq!(app.editor().unwrap().caret, 1);
        app.ime_preedit(KANA[0], None);
        app.ime_commit(KANA[2]);
        let editor = app.editor().unwrap();
        assert_eq!(editor.text, "aにほんb");
        // Three characters of three bytes each, after the `a`.
        assert_eq!(editor.caret, 1 + 9);
    }

    #[test]
    fn escape_drops_the_composition_and_a_second_escape_drops_the_edit() {
        let mut app = app(20, 10);
        type_text(&mut app, "abc");
        app.ime_preedit(KANA[1], None);

        press(&mut app, crate::input::Key::Escape, Mods::NONE);
        // The composition went; the text the user had already typed did not.
        // Collapsing these two into one Escape would throw away committed work.
        assert!(!app.composing());
        assert_eq!(app.editor().expect("the edit survives").text, "abc");

        press(&mut app, crate::input::Key::Escape, Mods::NONE);
        assert!(app.editor().is_none());
    }

    #[test]
    fn a_withdrawn_composition_leaves_the_buffer_underneath_alone() {
        let mut app = app(20, 10);
        type_text(&mut app, "abc");
        app.ime_preedit(KANA[1], None);
        // Every backend says "withdrawn" with an empty preedit, and it is not
        // the same event as a commit.
        let outcome = app.ime_preedit("", None);
        assert!(outcome.redraw);
        assert!(!app.composing());
        assert_eq!(app.editor().unwrap().text, "abc");
        // A second one changes nothing and must not ask for a repaint.
        assert!(!app.ime_preedit("", None).redraw);
    }

    #[test]
    fn a_key_that_leaks_past_a_composing_input_method_is_ignored() {
        let mut app = app(20, 10);
        type_text(&mut app, "abc");
        app.ime_preedit(KANA[1], None);
        // The failure this prevents is real and platform-specific: a backend
        // that keeps delivering `KeyboardInput` during composition would type
        // every keystroke twice — once as an insert here and once inside the
        // commit — and a Backspace would eat a committed character the user
        // cannot see the caret in front of.
        for key in [
            crate::input::Key::Character('x'),
            crate::input::Key::Backspace,
            crate::input::Key::Delete,
            crate::input::Key::Left,
            crate::input::Key::Home,
        ] {
            press(&mut app, key, Mods::NONE);
        }
        let editor = app.editor().unwrap();
        assert_eq!(editor.text, "abc");
        assert_eq!(editor.caret, 3);
        assert!(app.composing());
    }

    #[test]
    fn enter_during_a_composition_does_not_write_a_half_composed_cell() {
        let mut app = app(20, 10);
        type_text(&mut app, "abc");
        app.ime_preedit(KANA[1], None);
        press(&mut app, crate::input::Key::Enter, Mods::NONE);
        // Finalising is the input method's decision, not ours: an Enter that
        // reached us was declined by it, and writing `abcにほ` would put text
        // in the cell that the user never confirmed.
        assert_eq!(shown(&mut app, 0, 0), None);
        assert!(app.composing());
    }

    #[test]
    fn the_caret_sits_where_the_input_method_put_it_inside_the_composition() {
        let mut app = app(20, 10);
        type_text(&mut app, "ab");
        // The IME's cursor is a byte offset *within the composition*.
        app.ime_preedit("にほん", Some((3, 3)));
        let (shown, caret, composition) = app.editor().unwrap().display();
        assert_eq!(shown, "abにほん");
        assert_eq!(caret, 2 + 3);
        // A caret and not a range, so there is no focused clause to shade — the
        // distinction TD-84 turns on, asserted on the side of it that is *not*
        // a conversion.
        assert_eq!(
            composition,
            Some(Composition {
                span: (2, 2 + 9),
                focus: None
            })
        );

        // No cursor from the platform means "after the composition".
        app.ime_preedit("にほん", None);
        assert_eq!(app.editor().unwrap().display().1, 2 + 9);
    }

    #[test]
    fn a_composition_cursor_past_the_end_is_clamped_and_not_trusted() {
        let mut app = app(20, 10);
        // The offsets come from a platform IME. A caret past the end of the
        // composition must be a clamped caret, not a panic in the shaper.
        app.ime_preedit("にほ", Some((99, 99)));
        let (shown, caret, _) = app.editor().unwrap().display();
        assert_eq!(caret, shown.len());
    }

    /// The focused clause is the platform's number too, so it gets the same
    /// distrust the caret does (TD-84).
    ///
    /// Three ways a range can be wrong and one way it can be absent, all of
    /// which must produce a drawable span or none — never an inverted rect, and
    /// never an offset the shaper's cluster search would walk off the end of.
    #[test]
    fn a_focused_clause_is_clamped_and_normalised_before_it_is_ever_drawn() {
        let mut app = app(20, 10);
        let focus = |app: &App| {
            app.editor()
                .and_then(|e| e.display().2)
                .and_then(|c| c.focus)
        };

        // Past the end: clamped to the composition, not trusted.
        app.ime_preedit("にほ", Some((3, 99)));
        assert_eq!(focus(&app), Some((3, 6)));
        // Reported end-first. This is a focused clause, not an empty one, and a
        // scene that only checked `right > left` would silently drop it.
        app.ime_preedit("にほ", Some((6, 3)));
        assert_eq!(focus(&app), Some((3, 6)));
        // Both offsets past the end collapse to a caret at the end, which is no
        // focus at all rather than a zero-width highlight.
        app.ime_preedit("にほ", Some((99, 99)));
        assert_eq!(focus(&app), None);
        // And the platform declining to say anything is not a focus either.
        app.ime_preedit("にほ", None);
        assert_eq!(focus(&app), None);
    }

    #[test]
    fn clicking_away_mid_composition_writes_the_buffer_and_never_the_composition() {
        let mut app = app(20, 10);
        type_text(&mut app, "abc");
        app.ime_preedit(KANA[2], None);
        let (ox, oy) = app.theme().grid_origin();
        app.pointer_down(ox + 4.0, oy + 4.0 + 20.0 * 3.0, false);
        // The cell got what the user had confirmed. The composition was never
        // theirs to keep — the platform withdraws it on focus change, and
        // writing it here would put unconfirmed text in a document.
        assert_eq!(shown(&mut app, 0, 0).as_deref(), Some("abc"));
        assert!(!app.composing());
    }

    #[test]
    fn the_ime_cursor_area_follows_the_caret_so_candidates_appear_under_it() {
        let mut app = app(20, 10);
        // Nothing to place a candidate list against when nothing is being
        // edited, and saying `None` beats offering the window's corner.
        app.frame();
        assert_eq!(app.ime_area(), None);

        type_text(&mut app, "abc");
        app.frame();
        let first = app.ime_area().expect("an open editor has a caret");
        app.ime_preedit("にほん", Some((9, 9)));
        app.frame();
        let composing = app.ime_area().expect("still open");
        assert!(
            composing[0] > first[0],
            "the candidate window must follow the composing text: {} then {}",
            first[0],
            composing[0]
        );
        assert!(composing[3] > 0.0, "a zero-height caret places nothing");
    }

    /// **TD-80's wiring**, which is the part of it a `text.rs` test cannot see:
    /// a real open starts the font warm-up and a test open does not.
    ///
    /// Both halves matter and the second is the one that would rot silently.
    /// `warm` in the shared constructor would be invisible — every test would
    /// still pass, each just quietly spawning a scan of several hundred files
    /// for a fallback it never uses, and the suite would get slower for a
    /// reason nothing named. So the suite's own constructor asserts it stays
    /// cold.
    #[test]
    fn a_real_open_warms_the_font_database_and_the_suite_s_open_does_not() {
        let quiet = App::open_detached(
            Session::from_log(usk_types::ActorId(1), empty_log(8, 4)),
            800.0,
            400.0,
            1.0,
        )
        .expect("the bundled font must load");
        assert!(
            !quiet.text.enumerated(),
            "the constructor the suite uses must not touch the host's fonts"
        );

        let real = App::open(
            Session::from_log(usk_types::ActorId(1), empty_log(8, 4)),
            800.0,
            400.0,
            1.0,
        )
        .expect("the bundled font must load");
        assert!(
            real.text.enumerated(),
            "a real session must have the enumeration already under way before \
             the first keystroke arrives (TD-80)"
        );
        assert_eq!(
            real.text.lazy_builds(),
            0,
            "and it must be under way *elsewhere* — nothing was built on this \
             thread"
        );
    }
}
