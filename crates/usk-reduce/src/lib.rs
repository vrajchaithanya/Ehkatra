//! usk-reduce — Commands, the versioned reducer, and selective undo
//! (BOOTSTRAP row 9; ADR-001, DP-A7, DP-A12, docs/11).
//!
//! # The contract
//! `reduce_v1(Command, &state) → Vec<Op>` is **pure and versioned**: commands
//! compile to ops exactly once, at the author, and remote replicas only ever
//! see ops (DP-A7). The reducer's semantics are immutable once shipped — new
//! behaviour is `reduce_v2`, never an edit to v1.
//!
//! # Selective undo (docs/11, DP-A12)
//! Undo is an *inverse synthesized against current state*, not a stack pop:
//!
//! * a value/formula write undoes only if the actor's **own write still wins**
//!   the cell — if someone else overwrote it, undo is a no-op and their intent
//!   is preserved;
//! * undoing an insert is **blocked** when another actor has written into the
//!   inserted row/column — narrowing rather than destroying their work;
//! * undoing a delete emits `UndeleteRow`/`UndeleteCol` — the row returns with
//!   its cells, because deletion tombstoned the identity and never erased the
//!   data (DP-A1).
//!
//! Redo is undo-of-undo, using the same synthesis machinery on the inverse
//! group, so the two directions cannot drift apart.

#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use usk_calc::refs::Binder;
use usk_calc::Engine;
use usk_formula::parse::{parse, Ast, A1};
use usk_oplog::{Anchor, AxisSpan, Op, OpLog, Payload, RangeBinding, StyleFacet, StyleTarget};
use usk_state::State;
use usk_types::coerce::Profile;
use usk_types::{ActorId, ColId, OpId, RowId, Value};

/// The command vocabulary — what UI, API and MCP all speak (DP-D1).
/// Coordinates are view ordinals, because that is what an author sees;
/// the reducer turns them into identities immediately.
#[derive(Clone, PartialEq, Debug)]
pub enum Command {
    SetValue {
        row: u32,
        col: u32,
        value: Value,
    },
    SetFormula {
        row: u32,
        col: u32,
        source: String,
    },
    ClearCell {
        row: u32,
        col: u32,
    },
    /// Insert so the new row renders at ordinal `before` (0 = at the top;
    /// `row_count` = append).
    InsertRow {
        before: u32,
    },
    DeleteRow {
        at: u32,
    },
    InsertCol {
        before: u32,
    },
    DeleteCol {
        at: u32,
    },
    /// Sets one formatting facet over a rectangle (ADR-041). The rectangle is
    /// bound to identities here, once, exactly as a formula's references are.
    SetStyle {
        target: RectSpec,
        facet: StyleFacet,
    },
    /// Returns one facet over a rectangle to the workbook default.
    ClearStyle {
        target: RectSpec,
        facet_slot: u8,
    },
    Undo,
    Redo,
}

/// A rectangle as an author names it: view ordinals, or "the whole axis".
///
/// `All` is what a column header click means, and it is not sugar for
/// `Range(0, row_count - 1)`: the reducer binds it to [`AxisSpan::All`], so the
/// rule keeps applying to rows that do not exist yet. Binding it to today's
/// last row would silently make tomorrow's rows unformatted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanSpec {
    All,
    /// Inclusive, in view ordinals.
    Range(u32, u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RectSpec {
    pub rows: SpanSpec,
    pub cols: SpanSpec,
}

impl RectSpec {
    pub fn cell(row: u32, col: u32) -> RectSpec {
        RectSpec {
            rows: SpanSpec::Range(row, row),
            cols: SpanSpec::Range(col, col),
        }
    }

    /// A whole column, by view ordinal — the gesture that has to cost one op.
    pub fn column(col: u32) -> RectSpec {
        RectSpec {
            rows: SpanSpec::All,
            cols: SpanSpec::Range(col, col),
        }
    }

    pub fn row(row: u32) -> RectSpec {
        RectSpec {
            rows: SpanSpec::Range(row, row),
            cols: SpanSpec::All,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandError {
    /// A coordinate names a row/column the current view does not have.
    OutOfRange,
    /// A formula reference could not be bound to identities.
    UnboundReference,
}

/// What one `apply` actually did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ApplyReport {
    pub ops_emitted: usize,
    /// Undo entries skipped to preserve another actor's work (docs/11's
    /// "blocked-and-narrowed", surfaced rather than silent).
    pub blocked: usize,
}

/// A cell's content before a write — what undo restores.
#[derive(Clone, PartialEq, Debug)]
enum Prior {
    /// Never written. Restored as a clear: the visible projection (blank) is
    /// identical, and ops cannot un-write history (DP-A1).
    Empty,
    Value(Value),
    Formula(String, Vec<RangeBinding>),
}

/// One undoable effect. Records *what would need inverting*, not the inverse
/// itself — the inverse is synthesized at undo time against current state.
#[derive(Clone, Debug)]
enum UndoEntry {
    CellWrite {
        row: RowId,
        col: ColId,
        prior: Prior,
        my_op: OpId,
    },
    RowInserted {
        row: RowId,
    },
    RowDeleted {
        row: RowId,
    },
    ColInserted {
        col: ColId,
    },
    ColDeleted {
        col: ColId,
    },
    /// A style write over a rectangle (ADR-041 decision 5).
    ///
    /// `prior` is what the facet resolved to over this exact rectangle *before*
    /// the write: every earlier rule that overlapped it, **clipped to the
    /// overlap**, ascending by the original stamp. Replaying that list on top
    /// of a clear reproduces the previous resolution cell for cell, and touches
    /// nothing outside the rectangle — which is why a rectangle-addressed
    /// undo can be exact rather than approximate.
    StyleWrite {
        target: StyleTarget,
        slot: u8,
        prior: Vec<(StyleTarget, Option<StyleFacet>)>,
    },
}

/// One command's worth of effects — the labeled undo group (docs/11).
#[derive(Default)]
struct Group {
    entries: Vec<UndoEntry>,
}

/// Output of the pure reducer: the ops to append and the undo record.
pub struct Reduction {
    pub ops: Vec<Op>,
    group: Group,
}

/// Mints op identities for one actor. `(actor, counter)` is globally unique;
/// lamport advances past everything the session has seen.
struct Mint {
    actor: ActorId,
    counter: u64,
    lamport: u64,
}

impl Mint {
    fn op(&mut self, payload: Payload) -> Op {
        self.counter += 1;
        self.lamport += 1;
        Op {
            id: OpId {
                actor: self.actor,
                counter: self.counter,
            },
            lamport: self.lamport,
            payload,
        }
    }
}

/// The pure, versioned reducer (ADR-001). Undo/Redo are session concerns and
/// are rejected here: they need the log, and the reducer must not.
pub fn reduce_v1(
    cmd: &Command,
    state: &State,
    mint: &mut Mint_,
) -> Result<Reduction, CommandError> {
    let binder = Binder::from_state(state);
    let mut ops = Vec::new();
    let mut group = Group::default();

    match cmd {
        Command::SetValue { row, col, value } => {
            let (r, c) = bind_cell(&binder, *row, *col)?;
            let prior = current_content(state, r, c);
            let op = mint.0.op(Payload::SetCell {
                row: r,
                col: c,
                value: value.clone(),
            });
            group.entries.push(UndoEntry::CellWrite {
                row: r,
                col: c,
                prior,
                my_op: op.id,
            });
            ops.push(op);
        }
        Command::ClearCell { row, col } => {
            let (r, c) = bind_cell(&binder, *row, *col)?;
            let prior = current_content(state, r, c);
            let op = mint.0.op(Payload::ClearCell { row: r, col: c });
            group.entries.push(UndoEntry::CellWrite {
                row: r,
                col: c,
                prior,
                my_op: op.id,
            });
            ops.push(op);
        }
        Command::SetFormula { row, col, source } => {
            let (r, c) = bind_cell(&binder, *row, *col)?;
            let bindings = bind_references(&binder, source)?;
            let prior = current_content(state, r, c);
            let op = mint.0.op(Payload::SetFormula {
                row: r,
                col: c,
                source: source.clone(),
                bindings,
            });
            group.entries.push(UndoEntry::CellWrite {
                row: r,
                col: c,
                prior,
                my_op: op.id,
            });
            ops.push(op);
        }
        Command::InsertRow { before } => {
            let n = binder.rows.len();
            if *before as usize > n {
                return Err(CommandError::OutOfRange);
            }
            let anchor = if *before == 0 {
                Anchor::Start
            } else {
                Anchor::After(
                    binder
                        .rows
                        .at(*before as usize - 1)
                        .ok_or(CommandError::OutOfRange)?
                        .0,
                )
            };
            let op = mint.0.op(Payload::InsertRow { anchor });
            group
                .entries
                .push(UndoEntry::RowInserted { row: RowId(op.id) });
            ops.push(op);
        }
        Command::DeleteRow { at } => {
            let row = binder
                .rows
                .at(*at as usize)
                .ok_or(CommandError::OutOfRange)?;
            let op = mint.0.op(Payload::DeleteRow { row });
            group.entries.push(UndoEntry::RowDeleted { row });
            ops.push(op);
        }
        Command::InsertCol { before } => {
            let n = binder.cols.len();
            if *before as usize > n {
                return Err(CommandError::OutOfRange);
            }
            let anchor = if *before == 0 {
                Anchor::Start
            } else {
                Anchor::After(
                    binder
                        .cols
                        .at(*before as usize - 1)
                        .ok_or(CommandError::OutOfRange)?
                        .0,
                )
            };
            let op = mint.0.op(Payload::InsertCol { anchor });
            group
                .entries
                .push(UndoEntry::ColInserted { col: ColId(op.id) });
            ops.push(op);
        }
        Command::DeleteCol { at } => {
            let col = binder
                .cols
                .at(*at as usize)
                .ok_or(CommandError::OutOfRange)?;
            let op = mint.0.op(Payload::DeleteCol { col });
            group.entries.push(UndoEntry::ColDeleted { col });
            ops.push(op);
        }
        Command::SetStyle { target, facet } => {
            let t = bind_target(&binder, target)?;
            let slot = facet.slot();
            let prior = prior_rules(state, &t, slot);
            let op = mint.0.op(Payload::SetStyle {
                target: t,
                facet: facet.clone(),
            });
            group.entries.push(UndoEntry::StyleWrite {
                target: t,
                slot,
                prior,
            });
            ops.push(op);
        }
        Command::ClearStyle { target, facet_slot } => {
            let t = bind_target(&binder, target)?;
            let prior = prior_rules(state, &t, *facet_slot);
            let op = mint.0.op(Payload::ClearStyle {
                target: t,
                facet_slot: *facet_slot,
            });
            group.entries.push(UndoEntry::StyleWrite {
                target: t,
                slot: *facet_slot,
                prior,
            });
            ops.push(op);
        }
        Command::Undo | Command::Redo => return Err(CommandError::OutOfRange),
    }

    Ok(Reduction { ops, group })
}

/// Public wrapper for the minting state so `reduce_v1`'s signature stays
/// honest about what it consumes: identities to mint, nothing else.
pub struct Mint_(Mint);

impl Mint_ {
    pub fn new(actor: ActorId, counter: u64, lamport: u64) -> Mint_ {
        Mint_(Mint {
            actor,
            counter,
            lamport,
        })
    }
    pub fn counter(&self) -> u64 {
        self.0.counter
    }
    pub fn lamport(&self) -> u64 {
        self.0.lamport
    }
}

fn bind_cell(binder: &Binder, row: u32, col: u32) -> Result<(RowId, ColId), CommandError> {
    let r = binder
        .rows
        .at(row as usize)
        .ok_or(CommandError::OutOfRange)?;
    let c = binder
        .cols
        .at(col as usize)
        .ok_or(CommandError::OutOfRange)?;
    Ok((r, c))
}

/// Binds a rectangle of view ordinals to identities (ADR-041).
///
/// `SpanSpec::All` becomes `AxisSpan::All` rather than the current first/last
/// pair, which is the difference between "this column" and "these 20 rows that
/// happen to be in this column right now".
fn bind_target(binder: &Binder, spec: &RectSpec) -> Result<StyleTarget, CommandError> {
    let rows = match spec.rows {
        SpanSpec::All => AxisSpan::All,
        SpanSpec::Range(a, b) => AxisSpan::Between(
            binder
                .rows
                .at(a as usize)
                .ok_or(CommandError::OutOfRange)?
                .0,
            binder
                .rows
                .at(b as usize)
                .ok_or(CommandError::OutOfRange)?
                .0,
        ),
    };
    let cols = match spec.cols {
        SpanSpec::All => AxisSpan::All,
        SpanSpec::Range(a, b) => AxisSpan::Between(
            binder
                .cols
                .at(a as usize)
                .ok_or(CommandError::OutOfRange)?
                .0,
            binder
                .cols
                .at(b as usize)
                .ok_or(CommandError::OutOfRange)?
                .0,
        ),
    };
    Ok(StyleTarget { rows, cols })
}

/// What one facet resolved to over `target` before this write — the list undo
/// replays. Ascending by the original stamp, each rule clipped to its overlap
/// with `target`.
fn prior_rules(
    state: &State,
    target: &StyleTarget,
    slot: u8,
) -> Vec<(StyleTarget, Option<StyleFacet>)> {
    if state.styles().is_empty() {
        return Vec::new();
    }
    state
        .style_resolver()
        .overlapping(state.styles(), target, slot)
        .into_iter()
        .map(|(t, value, _)| (t, value))
        .collect()
}

/// The actor whose rule currently wins `slot` anywhere in `target` — the style
/// analogue of `cell_winner`, and the whole of the undo block rule.
///
/// Taken from the registry rather than by scanning the log because they are the
/// same thing: every style op becomes exactly one rule, so the greatest-stamped
/// overlapping rule *is* the greatest-stamped overlapping op.
fn style_winner(state: &State, target: &StyleTarget, slot: u8) -> Option<ActorId> {
    if state.styles().is_empty() {
        return None;
    }
    state
        .style_resolver()
        .overlapping(state.styles(), target, slot)
        .last()
        .map(|(_, _, stamp)| stamp.1.actor)
}

/// Binds every reference in a formula's AST, in traversal order — the order
/// `SetFormula` records and every replica later relies on.
fn bind_references(binder: &Binder, source: &str) -> Result<Vec<RangeBinding>, CommandError> {
    let ast = parse(source).ast;
    let mut refs: Vec<(A1, A1)> = Vec::new();
    collect_refs(&ast, &mut refs);
    let mut out = Vec::with_capacity(refs.len());
    for (a, b) in refs {
        let range = binder
            .bind(
                a.row as usize,
                b.row as usize,
                a.col as usize,
                b.col as usize,
                anchor_mode(a.row_absolute),
                anchor_mode(a.col_absolute),
            )
            .ok_or(CommandError::UnboundReference)?;
        out.push(RangeBinding {
            row_start: range.row_start.0,
            row_end: range.row_end.0,
            col_start: range.col_start.0,
            col_end: range.col_end.0,
            anchors: (a.row_absolute as u8) | ((a.col_absolute as u8) << 1),
        });
    }
    Ok(out)
}

fn anchor_mode(absolute: bool) -> usk_calc::refs::AnchorMode {
    if absolute {
        usk_calc::refs::AnchorMode::Absolute
    } else {
        usk_calc::refs::AnchorMode::Relative
    }
}

fn collect_refs(ast: &Ast, out: &mut Vec<(A1, A1)>) {
    match ast {
        Ast::Reference(r) => out.push((*r, *r)),
        Ast::Range(a, b) => out.push((*a, *b)),
        Ast::Call { args, .. } => {
            for a in args {
                collect_refs(a, out);
            }
        }
        Ast::Unary(_, inner) | Ast::Percent(inner) | Ast::Paren(inner) => collect_refs(inner, out),
        Ast::Binary(_, l, r) => {
            collect_refs(l, out);
            collect_refs(r, out);
        }
        Ast::Literal(_) | Ast::Name(_) | Ast::Invalid(_) => {}
    }
}

fn current_content(state: &State, row: RowId, col: ColId) -> Prior {
    if let Some(f) = state.formula(row, col) {
        return Prior::Formula(f.source.clone(), f.bindings.clone());
    }
    match state.cell(row, col) {
        None => Prior::Empty,
        Some(v) => Prior::Value(v),
    }
}

fn restore_payload(row: RowId, col: ColId, prior: Prior) -> Payload {
    match prior {
        // Visible-blank equivalence: ops cannot un-write history, so "was
        // empty" restores as a clear. The projection is identical.
        Prior::Empty | Prior::Value(Value::Blank) => Payload::ClearCell { row, col },
        Prior::Value(v) => Payload::SetCell { row, col, value: v },
        Prior::Formula(source, bindings) => Payload::SetFormula {
            row,
            col,
            source,
            bindings,
        },
    }
}

/// The winning op at a cell, by the canonical total order — exact, from the
/// log, because summary tiles deliberately carry no per-cell stamps.
fn cell_winner(log: &OpLog, row: RowId, col: ColId) -> Option<OpId> {
    let mut best: Option<(u64, OpId)> = None;
    for op in log.ops() {
        let hit = match &op.payload {
            Payload::SetCell { row: r, col: c, .. } => *r == row && *c == col,
            Payload::ClearCell { row: r, col: c } => *r == row && *c == col,
            Payload::SetFormula { row: r, col: c, .. } => *r == row && *c == col,
            _ => false,
        };
        if hit {
            let key = (op.lamport, op.id);
            if best.is_none_or(|b| key > b) {
                best = Some(key);
            }
        }
    }
    best.map(|(_, id)| id)
}

/// True when any *other* actor has written a cell in this row.
fn others_wrote_in_row(log: &OpLog, row: RowId, me: ActorId) -> bool {
    log.ops().iter().any(|op| {
        op.id.actor != me
            && match &op.payload {
                Payload::SetCell { row: r, .. }
                | Payload::ClearCell { row: r, .. }
                | Payload::SetFormula { row: r, .. } => *r == row,
                _ => false,
            }
    })
}

fn others_wrote_in_col(log: &OpLog, col: ColId, me: ActorId) -> bool {
    log.ops().iter().any(|op| {
        op.id.actor != me
            && match &op.payload {
                Payload::SetCell { col: c, .. }
                | Payload::ClearCell { col: c, .. }
                | Payload::SetFormula { col: c, .. } => *c == col,
                _ => false,
            }
    })
}

/// Synthesizes the inverse of a group against current state. Returns the ops,
/// the inverse group (which is what redo will invert back), and how many
/// entries were blocked to preserve others' work.
fn invert_group(
    group: &Group,
    log: &OpLog,
    state: &State,
    mint: &mut Mint,
) -> (Vec<Op>, Group, usize) {
    let mut ops = Vec::new();
    let mut inverse = Group::default();
    let mut blocked = 0usize;

    // Reverse order: undo unwinds the group back-to-front.
    for entry in group.entries.iter().rev() {
        match entry {
            UndoEntry::CellWrite {
                row,
                col,
                prior,
                my_op,
            } => {
                // "Restore only if own write still wins" (docs/11) — own
                // meaning *my actor's*, not this group's specific op: a later
                // undo of a newer group restores through a fresh op id that is
                // still mine, and LIFO undo order guarantees any newer edit of
                // mine has already been unwound by the time this group is
                // reached. Only a foreign winner blocks (DP-A12).
                let _ = my_op;
                let winner = cell_winner(log, *row, *col);
                if winner.map(|id| id.actor) != Some(mint.actor) {
                    blocked += 1;
                    continue;
                }
                let now = current_content(state, *row, *col);
                let op = mint.op(restore_payload(*row, *col, prior.clone()));
                inverse.entries.push(UndoEntry::CellWrite {
                    row: *row,
                    col: *col,
                    prior: now,
                    my_op: op.id,
                });
                ops.push(op);
            }
            UndoEntry::RowInserted { row } => {
                if others_wrote_in_row(log, *row, mint.actor) {
                    // Deleting the row would destroy their cells: block.
                    blocked += 1;
                    continue;
                }
                let op = mint.op(Payload::DeleteRow { row: *row });
                inverse.entries.push(UndoEntry::RowDeleted { row: *row });
                ops.push(op);
            }
            UndoEntry::RowDeleted { row } => {
                let op = mint.op(Payload::UndeleteRow { row: *row });
                inverse.entries.push(UndoEntry::RowInserted { row: *row });
                ops.push(op);
            }
            UndoEntry::ColInserted { col } => {
                if others_wrote_in_col(log, *col, mint.actor) {
                    blocked += 1;
                    continue;
                }
                let op = mint.op(Payload::DeleteCol { col: *col });
                inverse.entries.push(UndoEntry::ColDeleted { col: *col });
                ops.push(op);
            }
            UndoEntry::ColDeleted { col } => {
                let op = mint.op(Payload::UndeleteCol { col: *col });
                inverse.entries.push(UndoEntry::ColInserted { col: *col });
                ops.push(op);
            }
            UndoEntry::StyleWrite {
                target,
                slot,
                prior,
            } => {
                // Same rule as a cell write, read for a rectangle: undo only
                // while this actor's own rule still wins. A *later* rule from
                // somebody else anywhere in the rectangle blocks the whole
                // entry — narrowing rather than destroying (docs/11).
                if style_winner(state, target, *slot).is_some_and(|a| a != mint.actor) {
                    blocked += 1;
                    continue;
                }
                // Read before anything is emitted: this is the resolution
                // *including* my rule, which is what redo has to restore.
                let now = prior_rules(state, target, *slot);
                ops.push(mint.op(Payload::ClearStyle {
                    target: *target,
                    facet_slot: *slot,
                }));
                // Ascending by the original stamp, minted with ascending fresh
                // stamps: the layering that produced the previous resolution is
                // reproduced rather than approximated.
                for (t, value) in prior {
                    ops.push(mint.op(match value {
                        None => Payload::ClearStyle {
                            target: *t,
                            facet_slot: *slot,
                        },
                        Some(facet) => Payload::SetStyle {
                            target: *t,
                            facet: facet.clone(),
                        },
                    }));
                }
                inverse.entries.push(UndoEntry::StyleWrite {
                    target: *target,
                    slot: *slot,
                    prior: now,
                });
            }
        }
    }
    (ops, inverse, blocked)
}

/// One actor's editing session: applies commands, owns that actor's undo and
/// redo stacks (scopes are per-user, docs/11), and keeps the log and derived
/// state together.
pub struct Session {
    pub log: OpLog,
    state: State,
    /// The calc graph, kept in step by feeding it every op batch. It routes
    /// itself between regroup and incremental recalc (docs/13, TD-18).
    engine: Engine,
    actor: ActorId,
    counter: u64,
    lamport: u64,
    undo_stack: Vec<Group>,
    redo_stack: Vec<Group>,
    /// Ops appended to the log but not yet folded into `state` (TD-24).
    ///
    /// State is a fold over the log, and DP-A9 says caches are watermarked
    /// folds — so the fold is taken **when the state is read**, not when the
    /// log grows. Sync made the difference measurable: a 50-replica relay
    /// delivers ~30,000 batches to each replica, and folding per batch made
    /// W-SYNC-RELAY cost 120 minutes of wall clock for 60 seconds of simulated
    /// session. Reads are what actually need the answer, and there are two
    /// orders of magnitude fewer of them.
    pending: Vec<Op>,
}

impl Session {
    pub fn new(actor: ActorId) -> Session {
        let state = State::default();
        let engine = Engine::build(&state, Profile::Compat);
        Session {
            log: OpLog::new(),
            state,
            engine,
            actor,
            counter: 0,
            lamport: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Opens a session over a log that already exists — which is what *opening
    /// a workbook* is.
    ///
    /// The alternative, feeding the log through [`integrate_batch`], is
    /// quadratic: that method checks each arriving op against every op already
    /// held, which is right for a relay redelivering a handful and wrong for a
    /// two-million-op document. Here the log *is* the starting point, so it is
    /// folded once and the mint is advanced past everything in it.
    ///
    /// Advancing the mint is the part that must not be skipped. `(actor,
    /// counter)` is the globally unique op identity; reopening a document and
    /// minting from zero would re-issue counters this actor has already spent,
    /// and the log's own merge rule would silently discard the second op
    /// bearing each id (DP-A4).
    ///
    /// [`integrate_batch`]: Session::integrate_batch
    pub fn from_log(actor: ActorId, log: OpLog) -> Session {
        let mut lamport = 0;
        let mut counter = 0;
        for op in log.ops() {
            lamport = lamport.max(op.lamport);
            if op.id.actor == actor {
                counter = counter.max(op.id.counter);
            }
        }
        let state = State::replay(&log);
        let mut engine = Engine::build(&state, Profile::Compat);
        // `Engine::build` builds the *graph* and evaluates nothing — every
        // other caller reaches evaluation through `observe`, because every
        // other caller got here by making an edit. Opening a document is the
        // one path with no edit behind it, so without this a freshly opened
        // workbook shows a blank cell wherever it holds a formula: the tile
        // store has no value there, and the value is what the engine has not
        // computed yet.
        //
        // Found by looking at the rendered frame rather than by a test. The
        // whole editing suite passed, because the first formula a *user* types
        // is a structural change that forces a full recalc and fills the rest
        // of the sheet in behind it.
        engine.recalc_all(&state);
        Session {
            log,
            state,
            engine,
            actor,
            counter,
            lamport,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Takes the outstanding fold, if any. Idempotent and cheap when nothing is
    /// pending, which is the common case for a reader in a loop.
    ///
    /// Every accessor below calls this, so a caller cannot observe stale state:
    /// the `&mut` on those accessors is not an inconvenience, it is the type
    /// system saying "reading may cost you a fold".
    pub fn settle(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = core::mem::take(&mut self.pending);
        // The fold, incrementally where the ops allow it (TD-24's residual).
        //
        // A locally authored edit is minted above every lamport in the log, so
        // for that edit arrival order *is* canonical order and `apply_tip`
        // takes it in time proportional to the batch. Anything else — a remote
        // batch that interleaves with local history, a first contention at a
        // cell an unpromoted tile keeps no stamp for — is refused and re-folded
        // here, producing exactly the state a full replay would.
        //
        // Measured: at 60,000 cells this took keystroke->paint from **25.3 ms
        // to 2.5 ms**, against docs/31's 16 ms budget, because a full replay
        // costs the whole document where an edit costs the edit.
        if self.state.apply_tip(&batch).is_err() {
            self.state = State::replay(&self.log);
        }
        // Not asserted here that the two agree: proving it costs the full
        // replay this exists to avoid, and a debug build that re-folded on
        // every keystroke would be unusable for exactly the workload the fast
        // path was built for. The property is a test instead —
        // `crates/usk-state/tests/tip.rs`, which asserts hash equality against
        // a full replay after every edit of a sixty-edit run.
        self.engine.observe(&self.state, &batch);
    }

    pub fn state(&mut self) -> &State {
        self.settle();
        &self.state
    }

    pub fn engine(&mut self) -> &Engine {
        self.settle();
        &self.engine
    }

    /// State and engine together, settled once.
    ///
    /// A renderer needs both — the state for what a cell *holds* and the engine
    /// for what it *shows* — and taking them one at a time cannot be written:
    /// `state()` borrows the session for as long as its result lives, so the
    /// `&mut self` that `engine()` wants is already spoken for. Returning the
    /// pair from one settle is the whole fix, and it also guarantees both
    /// halves of a frame were read at the same generation (docs/27 §3).
    pub fn view(&mut self) -> (&State, &Engine) {
        self.settle();
        (&self.state, &self.engine)
    }

    /// What a reader sees at a cell: the computed formula result if the cell
    /// holds a formula, otherwise the stored value.
    pub fn value(&mut self, row: RowId, col: ColId) -> Option<Value> {
        self.settle();
        self.engine.value(&self.state, row, col)
    }

    pub fn actor(&self) -> ActorId {
        self.actor
    }

    /// Integrates a remote op (another actor's edit arriving via sync).
    pub fn integrate(&mut self, op: Op) {
        self.integrate_batch(alloc::vec![op]);
    }

    /// Integrates a batch of remote ops as one unit.
    ///
    /// Sync delivers ops in batches, and folding once per batch rather than
    /// once per op is the difference between O(n) and O(n²) over a session's
    /// history. Ops already held are skipped, so a relay redelivery costs
    /// nothing — merge is idempotent (DP-A8).
    pub fn integrate_batch(&mut self, ops: Vec<Op>) {
        let mut fresh = Vec::with_capacity(ops.len());
        for op in ops {
            if self.log.ops().iter().any(|o| o.id == op.id) {
                continue;
            }
            if op.lamport > self.lamport {
                self.lamport = op.lamport;
            }
            // Ops this actor authored may arrive here rather than through
            // `apply` — a restart replaying its own durable log, or the relay
            // echoing our work back. The mint must never re-issue a counter it
            // has already spent: `(actor, counter)` is the globally unique op
            // identity, and two different ops sharing one id would make the
            // log's own merge rule silently discard the second (DP-A4).
            if op.id.actor == self.actor && op.id.counter > self.counter {
                self.counter = op.id.counter;
            }
            fresh.push(op);
        }
        if fresh.is_empty() {
            return;
        }
        for op in &fresh {
            self.log.append(op.clone());
        }
        self.refresh(&fresh);
    }

    pub fn apply(&mut self, cmd: Command) -> Result<ApplyReport, CommandError> {
        // The reducer binds view ordinals to identities against *current*
        // state, so a deferred fold must be taken before a command is compiled.
        self.settle();
        match cmd {
            Command::Undo => Ok(self.undo()),
            Command::Redo => Ok(self.redo()),
            other => {
                let mut mint = Mint_::new(self.actor, self.counter, self.lamport);
                let reduction = reduce_v1(&other, &self.state, &mut mint)?;
                self.counter = mint.counter();
                self.lamport = mint.lamport();
                let emitted = reduction.ops.len();
                let batch = reduction.ops.clone();
                for op in reduction.ops {
                    self.log.append(op);
                }
                self.undo_stack.push(reduction.group);
                // A fresh edit invalidates the redo branch, as everywhere.
                self.redo_stack.clear();
                self.refresh(&batch);
                Ok(ApplyReport {
                    ops_emitted: emitted,
                    blocked: 0,
                })
            }
        }
    }

    fn undo(&mut self) -> ApplyReport {
        let Some(group) = self.undo_stack.pop() else {
            return ApplyReport::default();
        };
        self.run_inverse(&group, true)
    }

    fn redo(&mut self) -> ApplyReport {
        let Some(group) = self.redo_stack.pop() else {
            return ApplyReport::default();
        };
        self.run_inverse(&group, false)
    }

    fn run_inverse(&mut self, group: &Group, into_redo: bool) -> ApplyReport {
        // Undo is an inverse synthesized against current state (docs/11), so
        // "current" has to mean folded.
        self.settle();
        let mut mint = Mint {
            actor: self.actor,
            counter: self.counter,
            lamport: self.lamport,
        };
        let (ops, inverse, blocked) = invert_group(group, &self.log, &self.state, &mut mint);
        self.counter = mint.counter;
        self.lamport = mint.lamport;
        let emitted = ops.len();
        let batch = ops.clone();
        for op in ops {
            self.log.append(op);
        }
        if !inverse.entries.is_empty() {
            if into_redo {
                self.redo_stack.push(inverse);
            } else {
                self.undo_stack.push(inverse);
            }
        }
        self.refresh(&batch);
        ApplyReport {
            ops_emitted: emitted,
            blocked,
        }
    }

    /// Records that the log has grown; the fold itself happens in [`settle`].
    ///
    /// The batch is remembered rather than applied because the calc graph needs
    /// it: `observe` routes structural/formula ops to a regroup and value ops
    /// to incremental recalc (TD-18), so deferred batches accumulate and are
    /// handed over together. Their union routes exactly as the individual
    /// batches would have — a structural op anywhere in the union forces the
    /// regroup that the same op alone would have forced.
    ///
    /// [`settle`]: Session::settle
    fn refresh(&mut self, applied: &[Op]) {
        self.pending.extend_from_slice(applied);
    }
}
