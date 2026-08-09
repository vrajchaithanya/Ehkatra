//! usk-state — CRDT state: axis order + cell registers, fold over the op log.
//!
//! State is DERIVED: `State::replay(log)` is the only constructor from data,
//! and all mutators are private to the applier (docs/03 invariant I3).
//! Convergence contract: same op set (any order) ⇒ same state ⇒ same hash.

#![no_std]
extern crate alloc;

pub mod formula;
pub mod image;
pub mod stamps;
pub mod tile;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
pub use formula::FormulaCell;
use formula::FormulaRegistry;
pub use stamps::{Stamp, WinnerStamps};
use tile::{PromotionStats, TileStore};
use usk_oplog::{Anchor, Op, OpLog, Payload};
use usk_types::{ColId, Lamport, OpId, RowId, Value};

/// One axis (rows or columns) as a neighbor-anchored ordered sequence with
/// tombstones. Deterministic concurrent-insert resolution: children of the
/// same anchor order by (lamport, actor, counter) DESCENDING so later
/// same-place inserts land nearer their anchor (RGA-style, interleaving-safe
/// for the v0.1 single-level case; full Fugue tree arrives with usk-state v2).
#[derive(Default, Clone)]
struct AxisSeq {
    /// Insertion tree: anchor op-id -> ordered child op ids.
    ///
    /// Visible to `image` because a tile image serialises the *tree*, not the
    /// flattened order: a restored workbook keeps being edited, and a later
    /// insert anchors to an existing id.
    pub(crate) children: BTreeMap<Option<OpId>, Vec<(u64, OpId)>>,
    pub(crate) tombstones: BTreeMap<OpId, ()>,
}

impl AxisSeq {
    fn insert(&mut self, anchor: &Anchor, id: OpId, lamport: u64) {
        let key = match anchor {
            Anchor::Start => None,
            Anchor::After(a) => Some(*a),
        };
        let kids = self.children.entry(key).or_default();
        // Deterministic position independent of arrival order.
        let pos = kids
            .binary_search_by(|(l, k)| (lamport, id).cmp(&(*l, *k)))
            .unwrap_or_else(|e| e);
        kids.insert(pos, (lamport, id));
    }

    fn delete(&mut self, id: OpId) {
        self.tombstones.insert(id, ());
    }

    /// Removes a tombstone (selective undo of a delete, docs/11). Correctness
    /// under concurrency comes from replay order: `State::replay` applies ops
    /// in the canonical total order, so the later of a delete/undelete pair
    /// deterministically wins on every replica.
    fn undelete(&mut self, id: OpId) {
        self.tombstones.remove(&id);
    }

    /// Depth-first walk producing the live visual order.
    fn live_order(&self) -> Vec<OpId> {
        let mut out = Vec::new();
        self.walk(None, &mut out);
        out
    }

    /// Pre-order walk of the insertion tree, **iteratively**.
    ///
    /// Iterative rather than recursive because the recursion depth is the depth
    /// of the insertion tree, and that is user data. Appending N rows one below
    /// the previous anchors each to the last, so the tree is a *chain* N deep —
    /// a 100k-row workbook built the ordinary way would have recursed 100k
    /// frames. The image fuzz test found it first on a corrupted tree, which is
    /// the cheaper way to be told (D-111).
    fn walk(&self, key: Option<OpId>, out: &mut Vec<OpId>) {
        for id in self.preorder(key) {
            if self.is_live(&id) {
                out.push(id);
            }
        }
    }

    /// The shared traversal. Children are pushed in reverse so they pop in
    /// insertion order, which is what makes this identical to the recursion it
    /// replaced rather than merely similar.
    fn preorder(&self, key: Option<OpId>) -> Vec<OpId> {
        let mut out = Vec::new();
        let mut stack: Vec<OpId> = Vec::new();
        if let Some(kids) = self.children.get(&key) {
            stack.extend(kids.iter().rev().map(|(_, id)| *id));
        }
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(kids) = self.children.get(&Some(id)) {
                stack.extend(kids.iter().rev().map(|(_, k)| *k));
            }
        }
        out
    }

    fn is_live(&self, id: &OpId) -> bool {
        !self.tombstones.contains_key(id)
    }

    /// The full axis order **including tombstones**, each tagged with whether
    /// it is still live.
    ///
    /// Needed by identity references (docs/11): when a range endpoint is
    /// deleted the reference re-anchors *inward*, and "inward" is only
    /// meaningful against the order the tombstone still occupies. The live
    /// order alone cannot answer it.
    fn full_order(&self) -> Vec<(OpId, bool)> {
        let mut out = Vec::new();
        self.walk_full(None, &mut out);
        out
    }

    fn walk_full(&self, key: Option<OpId>, out: &mut Vec<(OpId, bool)>) {
        out.extend(self.preorder(key).into_iter().map(|id| {
            let live = self.is_live(&id);
            (id, live)
        }));
    }
}

/// The workbook state (single sheet in v0.1).
///
/// Cells live in the tile store (docs/14, ADR-005), not in a flat per-cell map:
/// CRDT metadata is a per-tile causal summary until concurrency forces
/// promotion, which is what makes a 10M-cell workbook fit its memory budget.
#[derive(Default, Clone)]
pub struct State {
    pub(crate) rows: AxisSeq,
    pub(crate) cols: AxisSeq,
    pub(crate) cells: TileStore,
    /// Formulas, keyed by cell identity. A **stamped** LWW register per cell
    /// (TD-22): formula-vs-value is decided by `(lamport, op id)`, not by the
    /// order ops happen to be applied in, so an incremental merge cannot
    /// resolve it differently from a full replay. See `formula.rs`.
    pub(crate) formulas: FormulaRegistry,
}

/// Why a tail could not be applied to an adopted image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TailError {
    /// The op is canonically at or before the image's greatest key, so the
    /// image may already contain it or a newer write to the same cell.
    /// Refused rather than misplaced (ADR-036).
    NotAfterImage { op: OpId, image_greatest: OpId },
}

/// The cell a payload writes, if it writes one.
fn cell_of(payload: &Payload) -> Option<(RowId, ColId)> {
    match payload {
        Payload::SetCell { row, col, .. }
        | Payload::ClearCell { row, col }
        | Payload::SetFormula { row, col, .. } => Some((*row, *col)),
        _ => None,
    }
}

impl State {
    /// Fold a log into state. The ONLY public constructor from data.
    ///
    /// Two passes over the canonical order. The first assigns identity slots
    /// and decides which tiles must start promoted; the second applies. The
    /// split exists because promotion has to be decided *before* the first
    /// write lands: a summary tile records no per-cell stamps, so it could not
    /// reconstruct them if it were asked to promote later (see `tile::Meta`).
    pub fn replay(log: &OpLog) -> Self {
        let mut ops: Vec<&Op> = log.ops().iter().collect();
        // Total order: applying in this order is equivalent to any causal
        // order because every apply_* below is commutative for concurrent ops.
        ops.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
        let mut s = State::from_plan(tile::plan_promotions(ops.iter().copied()));
        for op in ops {
            s.apply(op);
        }
        s
    }

    /// Builds the empty state the pre-pass planned: tiles that know which cells
    /// are contested, and a formula registry that knows which cells a formula
    /// will ever name.
    fn from_plan(mut plan: tile::Plan) -> Self {
        let formulas = FormulaRegistry::seeded(core::mem::take(&mut plan.formula_cells));
        State {
            cells: TileStore::from_plan(plan),
            formulas,
            ..Default::default()
        }
    }

    /// Replays ops that are **already** in canonical total order, without ever
    /// materializing an `OpLog`.
    ///
    /// `source` is called twice — once for the promotion pre-pass, once to
    /// apply — and must yield the identical sequence both times, ordered by
    /// `(lamport, actor, counter)`. The caller owns that precondition, which is
    /// exactly why `replay` (which sorts, and cannot be misused) stays the
    /// default constructor.
    ///
    /// This exists because some inputs are larger than the state they build: a
    /// 10M-cell import is ~1.2 GB of ops producing ~80 MB of tiles. Row 11
    /// (snapshot + op-tail recovery) and Row 12 (import) need the same shape,
    /// and the A-001 memory harness cannot be run at all without it.
    pub fn replay_sorted<F, I>(mut source: F) -> Self
    where
        F: FnMut() -> I,
        I: Iterator<Item = Op>,
    {
        let mut s = State::from_plan(tile::plan_promotions(source()));
        for op in source() {
            s.apply(&op);
        }
        s
    }

    /// Applies a **tail** onto a state adopted from an image (ADR-036).
    ///
    /// This is what makes an image a snapshot body rather than a read-only
    /// cache. The tail's ops were not in the plan the image was built from, so
    /// a cell the plan proved uncontested may now be contested — and the
    /// summary path would overwrite it without keeping the loser. Each cell the
    /// tail writes is therefore promoted first, seeded with the winner stamp
    /// the sidecar carries, and only then applied.
    ///
    /// `tail` must be in canonical `(lamport, actor, counter)` order, the same
    /// precondition `replay_sorted` has (TD-11).
    ///
    /// # Errors
    /// An op canonically **at or before** the image's greatest key is refused,
    /// not applied. The image records that key precisely so this case is
    /// visible: a summary tile trusts arrival order instead of comparing
    /// stamps, so silently accepting an earlier op would overwrite a newer
    /// value with an older one and report success (D-101, ADR-036).
    pub fn apply_tail(&mut self, stamps: &WinnerStamps, tail: &[Op]) -> Result<(), TailError> {
        if let Some(greatest) = stamps.greatest() {
            if let Some(op) = tail.iter().find(|op| (op.lamport, op.id) <= greatest) {
                return Err(TailError::NotAfterImage {
                    op: op.id,
                    image_greatest: greatest.1,
                });
            }
        }
        for op in tail {
            if let Some((row, col)) = cell_of(&op.payload) {
                // Promote only where the image actually holds a winner: a cell
                // the tail creates has nothing to lose to.
                if let Some(stamp) = stamps.get(row, col) {
                    if stamp.1.actor != op.id.actor && !self.cells.is_contested(row.0, col.0) {
                        self.cells.adopt_stamp(row.0, col.0, stamp);
                    }
                }
            }
            self.apply(op);
        }
        Ok(())
    }

    /// Applies one op.
    ///
    /// Formula-vs-value at a cell is decided by the op's **stamp**, not by when
    /// it happens to be applied (TD-22, closed): the registry takes the greater
    /// of `(lamport, op id)`, so a value write that arrives after a newer
    /// formula loses, exactly as it would in a full replay.
    ///
    /// The *tile* store still requires canonical order — a summary tile keeps
    /// no per-cell stamps to compare against, which is ADR-005's memory
    /// argument. Both constructors sort, so the precondition holds here; it is
    /// the remaining barrier to a genuinely incremental apply (TD-11, TD-24).
    fn apply(&mut self, op: &Op) {
        match &op.payload {
            Payload::InsertRow { anchor } => self.rows.insert(anchor, op.id, op.lamport),
            Payload::DeleteRow { row } => self.rows.delete(row.0),
            Payload::InsertCol { anchor } => self.cols.insert(anchor, op.id, op.lamport),
            Payload::DeleteCol { col } => self.cols.delete(col.0),
            Payload::UndeleteRow { row } => self.rows.undelete(row.0),
            Payload::UndeleteCol { col } => self.cols.undelete(col.0),
            // DP-A5: an op this build cannot read is preserved, causally
            // ordered and hashed — and applied to nothing. Guessing at a
            // payload we do not understand is the one outcome worse than
            // ignoring it, and the op is still in the log for a build that
            // does understand it.
            Payload::Opaque(_) => {}
            Payload::SetCell { row, col, value } => {
                self.formulas
                    .note_value_write(*row, *col, (op.lamport, op.id));
                self.cells
                    .write(row.0, col.0, op.lamport, op.id, value.clone())
            }
            // A clear is a write of Blank, not an erasure: the cell keeps its
            // identity and its place in the causal history (DP-A1).
            Payload::ClearCell { row, col } => {
                self.formulas
                    .note_value_write(*row, *col, (op.lamport, op.id));
                self.cells
                    .write(row.0, col.0, op.lamport, op.id, Value::Blank)
            }
            Payload::SetFormula {
                row,
                col,
                source,
                bindings,
            } => {
                self.formulas.set_formula(
                    *row,
                    *col,
                    (op.lamport, op.id),
                    FormulaCell {
                        source: source.clone(),
                        bindings: bindings.clone(),
                    },
                );
            }
        }
    }

    /// Live row identities in display order.
    pub fn row_order(&self) -> Vec<RowId> {
        self.rows.live_order().into_iter().map(RowId).collect()
    }

    /// Row identities in axis order, tombstones included, each tagged live.
    /// The substrate for identity-interval references (docs/11).
    pub fn full_row_order(&self) -> Vec<(RowId, bool)> {
        self.rows
            .full_order()
            .into_iter()
            .map(|(id, live)| (RowId(id), live))
            .collect()
    }

    /// Column identities in axis order, tombstones included.
    pub fn full_col_order(&self) -> Vec<(ColId, bool)> {
        self.cols
            .full_order()
            .into_iter()
            .map(|(id, live)| (ColId(id), live))
            .collect()
    }

    /// Live column identities in display order.
    pub fn col_order(&self) -> Vec<ColId> {
        self.cols.live_order().into_iter().map(ColId).collect()
    }

    /// Current winning value of a cell, if any (Blank clears count as values).
    ///
    /// Returns an owned `Value` rather than a reference: a packed numeric tile
    /// stores bare `f64`s, so there is no `Value` in memory to borrow.
    pub fn cell(&self, row: RowId, col: ColId) -> Option<Value> {
        self.cells.get(&row.0, &col.0)
    }

    /// The winning formula at a cell, if a formula is the winning content.
    pub fn formula(&self, row: RowId, col: ColId) -> Option<&FormulaCell> {
        self.formulas.get(row, col)
    }

    /// Every formula in the workbook, keyed by cell identity, in identity
    /// order. The calc engine's source of truth for what to evaluate.
    pub fn formulas(&self) -> impl Iterator<Item = (RowId, ColId, &FormulaCell)> {
        self.formulas.iter()
    }

    /// Retained concurrent losers for conflict surfacing (ADR-006).
    pub fn conflicts(&self, row: RowId, col: ColId) -> &[(Lamport, OpId, Value)] {
        self.cells.losers(&row.0, &col.0)
    }

    /// Deterministic state hash: the determinism-gate primitive (docs/10).
    /// Hashes live axis order + winning cell values of live cells.
    ///
    /// Cells fold in tile-major order — the tile-Merkle direction docs/10
    /// specifies — which is deterministic because slot assignment is a pure
    /// function of the op set. Identities, never slots, go into the hash.
    pub fn state_hash(&self) -> blake3::Hash {
        let mut h = blake3::Hasher::new();
        let mut buf = Vec::new();
        for r in self.rows.live_order() {
            h.update(&r.actor.0.to_be_bytes());
            h.update(&r.counter.to_be_bytes());
        }
        h.update(b"|cols|");
        for c in self.cols.live_order() {
            h.update(&c.actor.0.to_be_bytes());
            h.update(&c.counter.to_be_bytes());
        }
        h.update(b"|cells|");
        let (rows, cols) = (&self.rows, &self.cols);
        self.cells.for_each(|r, c, value| {
            if rows.is_live(&r) && cols.is_live(&c) {
                h.update(&r.actor.0.to_be_bytes());
                h.update(&r.counter.to_be_bytes());
                h.update(&c.actor.0.to_be_bytes());
                h.update(&c.counter.to_be_bytes());
                buf.clear();
                value.encode_into(&mut buf);
                h.update(&buf);
            }
        });
        // Formulas join the hash only when any exist, so every pre-Row-9
        // corpus hashes exactly as before — the additive-evolution rule
        // (docs/10) applied to the hash itself. Shadowed registry entries are
        // bookkeeping, not content: `iter()` skips them, so a cell whose
        // formula lost to a later value write hashes exactly as it did when
        // the entry was deleted outright (pre-TD-22).
        if !self.formulas.has_no_formulas() {
            h.update(b"|formulas|");
            for (r, c, f) in self.formulas.iter() {
                let (r, c) = (&r.0, &c.0);
                if self.rows.is_live(r) && self.cols.is_live(c) {
                    h.update(&r.actor.0.to_be_bytes());
                    h.update(&r.counter.to_be_bytes());
                    h.update(&c.actor.0.to_be_bytes());
                    h.update(&c.counter.to_be_bytes());
                    h.update(&(f.source.len() as u32).to_be_bytes());
                    h.update(f.source.as_bytes());
                    for b in &f.bindings {
                        for id in [b.row_start, b.row_end, b.col_start, b.col_end] {
                            h.update(&id.actor.0.to_be_bytes());
                            h.update(&id.counter.to_be_bytes());
                        }
                        h.update(&[b.anchors]);
                    }
                }
            }
        }
        h.finalize()
    }

    /// The causal summary `(max lamport, sole writer)` of the tile holding this
    /// cell, or `None` if that tile is promoted (docs/14 §CRDT metadata).
    pub fn cell_summary(&self, row: RowId, col: ColId) -> Option<(Lamport, usk_types::ActorId)> {
        self.cells.causal_summary(&row.0, &col.0)
    }

    /// Whether this specific cell carries per-cell CRDT metadata (TD-09: the
    /// promoted unit is the cell, not its tile).
    pub fn is_cell_promoted(&self, row: RowId, col: ColId) -> bool {
        self.cells.is_cell_promoted(&row.0, &col.0)
    }

    /// Promotion accounting for assumption A-002 (docs/42) — the number that
    /// decides whether ADR-005's tile granularity holds up.
    pub fn promotion_stats(&self) -> PromotionStats {
        self.cells.promotion_stats()
    }

    /// Structural heap bytes held by the cell store, for the A-001 harness.
    pub fn cell_heap_bytes(&self) -> usize {
        self.cells.heap_bytes()
    }

    /// Number of live tiles backing the cell store.
    pub fn tile_count(&self) -> usize {
        self.cells.tile_count()
    }
}
