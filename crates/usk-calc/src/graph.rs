//! The dependency graph: formula groups, range-granular edges, and incremental
//! level-ordered recalculation over **identities** (docs/13, docs/27 §3).
//!
//! # Addressing
//! Cells are `(RowId, ColId)`. The only positions here are the `Rect`
//! coordinates the spatial index uses, and those are *derived from the current
//! axis order on every rebuild* — A1 is a computed view (DP-A6), so a stale
//! position cannot survive a structural edit. Nothing persists an ordinal.
//!
//! # The graph is a cache
//! Every value in `results` is a fold over the op log with a watermark, not
//! independently mutable state (DP-A9). Structural or formula ops invalidate
//! the fold and force a rebuild; value ops flow through the incremental path.
//! [`Engine::observe`] is the trigger that decides which.

use crate::refs::Binder;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use usk_formula::eval::{eval, Context, Grid};
use usk_formula::functions::DateSystem;
use usk_formula::parse::{parse, Ast, BinOp, UnOp, A1};
use usk_oplog::{Op, Payload, RangeBinding};
use usk_state::State;
use usk_types::coerce::Profile;
use usk_types::{CellError, ColId, ErrorKind, Origin, RowId, Value};

/// Rows per index band. Matches the tile band height (docs/14) so a dirty tile
/// maps to one bucket.
const BAND: u32 = 256;

/// An inclusive rectangle in *derived view coordinates*.
///
/// Not an address: rebuilt from the axis order every time the graph is, and
/// never stored anywhere that outlives a rebuild. It exists because a spatial
/// index needs a total order on two axes, which identities alone do not give.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub r0: u32,
    pub r1: u32,
    pub c0: u32,
    pub c1: u32,
}

impl Rect {
    fn point(row: u32, col: u32) -> Rect {
        Rect {
            r0: row,
            r1: row,
            c0: col,
            c1: col,
        }
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.r0 <= other.r1 && other.r0 <= self.r1 && self.c0 <= other.c1 && other.c0 <= self.c1
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.r0 && row <= self.r1 && col >= self.c0 && col <= self.c1
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            r0: self.r0.min(other.r0),
            r1: self.r1.max(other.r1),
            c0: self.c0.min(other.c0),
            c1: self.c1.max(other.c1),
        }
    }

    pub fn cell_count(&self) -> u64 {
        (self.r1 - self.r0 + 1) as u64 * (self.c1 - self.c0 + 1) as u64
    }
}

/// One member of a group during construction.
type Member = (RowId, ColId, (u32, u32), Ast);

/// One member selected for evaluation: identity, derived position, formula.
type Pending = ((RowId, ColId), (u32, u32), Ast);

/// A set of cells sharing one R1C1 pattern — a single node in the graph.
pub struct Group {
    /// The R1C1 rendering all members share.
    pub pattern: String,
    /// Members by identity, in axis order.
    pub cells: Vec<(RowId, ColId)>,
    /// Each member's formula, already rebound to the current view.
    asts: Vec<Ast>,
    /// Each member's derived write position.
    positions: Vec<(u32, u32)>,
    /// Bounding rectangle of each member's reads, same order as `cells`.
    /// Precomputed because the incremental path asks "which members read this
    /// rectangle?" on every edit; re-walking member ASTs measured 10.5 ms
    /// against docs/31's 8 ms budget.
    member_bounds: Vec<Rect>,
    /// Rectangles this group reads, unioned across members.
    pub reads: Vec<Rect>,
    /// Bounding rectangle of the members — what the group writes.
    pub writes: Rect,
}

/// What one recalculation actually did.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecalcStats {
    pub dirty_groups: usize,
    pub evaluated_groups: usize,
    pub evaluated_cells: usize,
    /// Groups skipped because nothing upstream changed — docs/13's early cutoff.
    pub cut_off_groups: usize,
    /// Topological levels; every group in one level is independent, so this is
    /// the width available to a parallel evaluator.
    pub levels: usize,
    pub circular_groups: usize,
    /// True when this pass rebuilt the graph rather than reusing it.
    pub regrouped: bool,
}

/// A band-bucketed rectangle index.
///
/// docs/13 specifies an R-tree over identity space. This is the cheaper
/// structure with the right access shape — a stab lands in one bucket and scans
/// only the groups whose read rectangles cross that band. Tracked as TD-20.
#[derive(Default)]
struct BandIndex {
    bands: BTreeMap<u32, Vec<u32>>,
}

impl BandIndex {
    fn insert(&mut self, group: u32, rect: &Rect) {
        for band in (rect.r0 / BAND)..=(rect.r1 / BAND) {
            let slot = self.bands.entry(band).or_default();
            if !slot.contains(&group) {
                slot.push(group);
            }
        }
    }

    fn stab(&self, rect: &Rect, groups: &[Group], out: &mut BTreeSet<u32>) {
        for band in (rect.r0 / BAND)..=(rect.r1 / BAND) {
            let Some(candidates) = self.bands.get(&band) else {
                continue;
            };
            for id in candidates {
                let Some(g) = groups.get(*id as usize) else {
                    continue;
                };
                if g.reads.iter().any(|r| r.overlaps(rect)) {
                    out.insert(*id);
                }
            }
        }
    }
}

/// Reads the workbook as a formula sees it: computed results where a formula
/// owns the cell, stored values everywhere else.
struct EngineGrid<'a> {
    state: &'a State,
    binder: &'a Binder,
    results: &'a BTreeMap<(RowId, ColId), Value>,
}

impl Grid for EngineGrid<'_> {
    fn read(&self, row: u32, col: u32) -> Option<Value> {
        let r = self.binder.rows.at(row as usize)?;
        let c = self.binder.cols.at(col as usize)?;
        Some(match self.results.get(&(r, c)) {
            Some(v) => v.clone(),
            None => self.state.cell(r, c).unwrap_or(Value::Blank),
        })
    }

    fn extent(&self) -> (u32, u32) {
        (self.binder.rows.len() as u32, self.binder.cols.len() as u32)
    }
}

/// The calculation engine over a workbook `State`.
pub struct Engine {
    groups: Vec<Group>,
    index: BandIndex,
    binder: Binder,
    /// Computed formula results by cell identity — the watermarked fold.
    results: BTreeMap<(RowId, ColId), Value>,
    profile: Profile,
    /// Materialised volatiles (docs/13 T2): read, never computed ambiently.
    pub today: i32,
    pub now: f64,
    /// The workbook's date system (TD-33). A workbook-level property, so it
    /// belongs beside `profile` rather than being decided per formula — a
    /// 1904 workbook that recalculated under 1900 semantics would shift every
    /// date it computed by 1,462 days.
    pub dates: DateSystem,
    generation: u64,
}

impl Engine {
    /// Builds the graph over a state's current formulas.
    pub fn build(state: &State, profile: Profile) -> Engine {
        let mut engine = Engine {
            groups: Vec::new(),
            index: BandIndex::default(),
            binder: Binder::from_state(state),
            results: BTreeMap::new(),
            profile,
            today: 0,
            now: 0.0,
            dates: DateSystem::default(),
            generation: 0,
        };
        engine.regroup(state);
        engine
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// The generation mark. docs/27 §3 forbids observing a half-evaluated pass
    /// without one, so every completed pass bumps this.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The value a reader sees at a cell: computed result, else stored value.
    pub fn value(&self, state: &State, row: RowId, col: ColId) -> Option<Value> {
        match self.results.get(&(row, col)) {
            Some(v) => Some(v.clone()),
            None => state.cell(row, col),
        }
    }

    /// **The regrouping trigger** (TD-18, docs/13).
    ///
    /// Structural and formula ops change the *graph*: rows move, formulas
    /// appear, patterns regroup. Value ops change only inputs. This routes each
    /// case to the cheap path it deserves, so callers never have to remember
    /// which kind of edit they just made.
    pub fn observe(&mut self, state: &State, ops: &[Op]) -> RecalcStats {
        let mut changed_cells: Vec<(RowId, ColId)> = Vec::new();
        let mut structural = false;

        for op in ops {
            match &op.payload {
                Payload::SetCell { row, col, .. } | Payload::ClearCell { row, col } => {
                    changed_cells.push((*row, *col));
                }
                // A formula write changes the group set; an axis edit changes
                // every derived position. Both invalidate the graph.
                Payload::SetFormula { .. }
                | Payload::InsertRow { .. }
                | Payload::InsertCol { .. }
                | Payload::DeleteRow { .. }
                | Payload::DeleteCol { .. }
                | Payload::UndeleteRow { .. }
                | Payload::UndeleteCol { .. } => structural = true,
                // An opaque op (DP-A5) changed no state, so it dirties nothing.
                // Treating it as structural would let an op this build cannot
                // read force a full regroup on every arrival.
                Payload::Opaque(_) => {}
            }
        }

        if structural {
            self.regroup(state);
            let mut stats = self.recalc_all(state);
            stats.regrouped = true;
            stats
        } else {
            self.recalc_after(state, &changed_cells)
        }
    }

    /// Rebuilds the graph from the state's formulas, rebinding every reference
    /// to the current view. Positions are derived here and nowhere else.
    pub fn regroup(&mut self, state: &State) {
        self.binder = Binder::from_state(state);
        self.groups = Vec::new();
        self.index = BandIndex::default();
        // Results from the previous shape are not transferable: a regroup means
        // the graph's inputs moved.
        self.results.clear();

        let mut by_pattern: BTreeMap<String, Vec<Member>> = BTreeMap::new();

        for (row, col, formula) in state.formulas() {
            let (Some(rp), Some(cp)) = (
                self.binder.rows.position_of(&row),
                self.binder.cols.position_of(&col),
            ) else {
                // The formula's own cell was deleted: no view position, so
                // nothing to evaluate into.
                continue;
            };
            let ast = rebind(&parse(&formula.source).ast, &formula.bindings, &self.binder);
            let pattern = r1c1(&ast, rp as u32, cp as u32);
            by_pattern
                .entry(pattern)
                .or_default()
                .push((row, col, (rp as u32, cp as u32), ast));
        }

        for (pattern, members) in by_pattern {
            let (reads, writes) = extent_of(&members);
            // A group whose unioned reads overlap its own writes may contain
            // members that depend on each other (`=A1+1` in B1 and `=B1+1` in
            // C1 share a pattern). Partition by column first: a horizontal
            // chain becomes one group per column, each reading only the column
            // to its left. Splitting straight to singletons measured 100,000
            // groups for 100,000 cells and hung the edge build.
            if reads.iter().any(|r| r.overlaps(&writes)) && members.len() > 1 {
                let mut by_column: BTreeMap<u32, Vec<Member>> = BTreeMap::new();
                for m in members {
                    by_column.entry(m.2 .1).or_default().push(m);
                }
                for (_, part) in by_column {
                    let (pr, pw) = extent_of(&part);
                    if pr.iter().any(|r| r.overlaps(&pw)) {
                        // A vertical running total: inherently serial, so each
                        // member becomes its own node and the graph orders them.
                        for m in part {
                            let one = alloc::vec![m];
                            let (r, w) = extent_of(&one);
                            self.push_group(pattern.clone(), one, r, w);
                        }
                    } else {
                        self.push_group(pattern.clone(), part, pr, pw);
                    }
                }
            } else {
                self.push_group(pattern.clone(), members, reads, writes);
            }
        }
    }

    fn push_group(
        &mut self,
        pattern: String,
        members: Vec<Member>,
        reads: Vec<Rect>,
        writes: Rect,
    ) {
        let id = self.groups.len() as u32;
        let mut cells = Vec::with_capacity(members.len());
        let mut asts = Vec::with_capacity(members.len());
        let mut positions = Vec::with_capacity(members.len());
        for (row, col, pos, ast) in members {
            cells.push((row, col));
            positions.push(pos);
            asts.push(ast);
        }
        let member_bounds = member_bounds_of(&asts);
        for r in &reads {
            self.index.insert(id, r);
        }
        self.groups.push(Group {
            pattern,
            cells,
            asts,
            positions,
            member_bounds,
            reads,
            writes,
        });
    }

    /// Recalculates everything.
    pub fn recalc_all(&mut self, state: &State) -> RecalcStats {
        let seeds: BTreeMap<u32, Rect> = self
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| (i as u32, g.writes))
            .collect();
        self.evaluate(state, seeds, false)
    }

    /// Recalculates only what writes to `changed` can affect.
    ///
    /// The unit of dirtiness is a **rectangle inside a group**, not the group.
    /// A group can be 10,000 cells; marking all of it for one input cell
    /// measured 53 ms against docs/31's 8 ms budget.
    pub fn recalc_after(&mut self, state: &State, changed: &[(RowId, ColId)]) -> RecalcStats {
        let mut seeds: BTreeMap<u32, Rect> = BTreeMap::new();
        for (row, col) in changed {
            let (Some(rp), Some(cp)) = (
                self.binder.rows.position_of(row),
                self.binder.cols.position_of(col),
            ) else {
                continue;
            };
            let source = Rect::point(rp as u32, cp as u32);
            let mut readers = BTreeSet::new();
            self.index.stab(&source, &self.groups, &mut readers);
            for id in readers {
                let Some(g) = self.groups.get(id as usize) else {
                    continue;
                };
                if let Some(rect) = affected_members(g, &source) {
                    merge_rect(&mut seeds, id, rect);
                }
            }
        }
        self.evaluate(state, seeds, true)
    }

    fn evaluate(&mut self, state: &State, seeds: BTreeMap<u32, Rect>, cutoff: bool) -> RecalcStats {
        let mut stats = RecalcStats::default();

        // Transitive marking, carrying the dirty rectangle. A group is
        // re-visited only when its rectangle grows, which terminates the walk.
        let mut dirty: BTreeMap<u32, Rect> = seeds.clone();
        let mut frontier: Vec<u32> = seeds.keys().copied().collect();
        while let Some(id) = frontier.pop() {
            let Some(source) = dirty.get(&id).copied() else {
                continue;
            };
            let mut readers = BTreeSet::new();
            self.index.stab(&source, &self.groups, &mut readers);
            for r in readers {
                if r == id {
                    continue;
                }
                let Some(g) = self.groups.get(r as usize) else {
                    continue;
                };
                if let Some(rect) = affected_members(g, &source) {
                    if merge_rect(&mut dirty, r, rect) {
                        frontier.push(r);
                    }
                }
            }
        }
        stats.dirty_groups = dirty.len();

        let dirty_list: Vec<u32> = dirty.keys().copied().collect();
        let mut dependents: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut upstream: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut indegree: BTreeMap<u32, usize> = dirty_list.iter().map(|id| (*id, 0)).collect();
        let mut self_cyclic: BTreeSet<u32> = BTreeSet::new();

        // Edges come from the index, not pairwise comparison: the nested loop
        // is O(groups^2) and was measured hanging once a chain of derived
        // columns produced 100k nodes.
        for a in &dirty_list {
            let Some(ga) = self.groups.get(*a as usize) else {
                continue;
            };
            if ga.reads.iter().any(|r| r.overlaps(&ga.writes)) {
                self_cyclic.insert(*a);
            }
            let writes = ga.writes;
            let mut readers = BTreeSet::new();
            self.index.stab(&writes, &self.groups, &mut readers);
            for b in readers {
                if b == *a || !dirty.contains_key(&b) {
                    continue;
                }
                dependents.entry(*a).or_default().push(b);
                upstream.entry(b).or_default().push(*a);
                *indegree.entry(b).or_insert(0) += 1;
            }
        }

        // Kahn by levels. Every group in a level is independent of the others
        // in it, which is exactly the width a parallel evaluator would use.
        let mut levels: Vec<Vec<u32>> = Vec::new();
        let mut ready: Vec<u32> = dirty_list
            .iter()
            .copied()
            .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0 && !self_cyclic.contains(id))
            .collect();
        let mut placed = 0usize;

        while !ready.is_empty() {
            // Sorted so a level's contents are a pure function of the graph,
            // never of iteration order (DP-A2, docs/29 §1).
            ready.sort_unstable();
            let mut next: Vec<u32> = Vec::new();
            for id in &ready {
                placed += 1;
                for dep in dependents.get(id).into_iter().flatten() {
                    let e = indegree.entry(*dep).or_insert(0);
                    *e = e.saturating_sub(1);
                    if *e == 0 && !self_cyclic.contains(dep) {
                        next.push(*dep);
                    }
                }
            }
            levels.push(core::mem::take(&mut ready));
            ready = next;
        }
        stats.levels = levels.len();

        // Whatever the level assignment could not place is in a cycle — the
        // same fact Tarjan would report, from work already done (docs/27 §3).
        if placed < dirty_list.len() {
            let placed_set: BTreeSet<u32> = levels.iter().flatten().copied().collect();
            for id in &dirty_list {
                if !placed_set.contains(id) {
                    stats.circular_groups += 1;
                    let cells = self
                        .groups
                        .get(*id as usize)
                        .map(|g| g.cells.clone())
                        .unwrap_or_default();
                    for cell in cells {
                        self.results.insert(
                            cell,
                            Value::Error(CellError::new(ErrorKind::Circ, Origin::Propagated)),
                        );
                        stats.evaluated_cells += 1;
                    }
                }
            }
        }

        // Rectangles whose values actually changed this pass, per group: both
        // the early-cutoff signal and the precise dirty region for downstream.
        let mut changed: BTreeMap<u32, Rect> = BTreeMap::new();

        for level in &levels {
            for id in level {
                let mut region: Option<Rect> = if cutoff { None } else { dirty.get(id).copied() };
                if cutoff {
                    if let Some(seed) = seeds.get(id) {
                        region = Some(*seed);
                    }
                    let Some(g) = self.groups.get(*id as usize) else {
                        continue;
                    };
                    for u in upstream.get(id).into_iter().flatten() {
                        let Some(src) = changed.get(u) else { continue };
                        if let Some(rect) = affected_members(g, src) {
                            region = Some(match region {
                                None => rect,
                                Some(prev) => prev.union(&rect),
                            });
                        }
                    }
                }

                let Some(region) = region else {
                    stats.cut_off_groups += 1;
                    continue;
                };

                let Some(group) = self.groups.get(*id as usize) else {
                    continue;
                };
                let members: Vec<Pending> = group
                    .cells
                    .iter()
                    .zip(group.positions.iter())
                    .zip(group.asts.iter())
                    .filter(|((_, pos), _)| region.contains(pos.0, pos.1))
                    .map(|((cell, pos), ast)| (*cell, *pos, ast.clone()))
                    .collect();
                if members.is_empty() {
                    stats.cut_off_groups += 1;
                    continue;
                }

                // Compute first, then write: a group's members must all see the
                // same upstream state.
                let mut computed = Vec::with_capacity(members.len());
                {
                    let grid = EngineGrid {
                        state,
                        binder: &self.binder,
                        results: &self.results,
                    };
                    let ctx = Context {
                        grid: &grid,
                        profile: self.profile,
                        today: self.today,
                        now: self.now,
                        dates: self.dates,
                    };
                    for (_, _, ast) in &members {
                        computed.push(eval(ast, &ctx));
                    }
                }

                let mut changed_rect: Option<Rect> = None;
                for ((cell, pos, _), value) in members.iter().zip(computed.iter()) {
                    if self.results.get(cell) != Some(value) {
                        let r = Rect::point(pos.0, pos.1);
                        changed_rect = Some(match changed_rect {
                            None => r,
                            Some(prev) => prev.union(&r),
                        });
                    }
                }
                if let Some(r) = changed_rect {
                    changed.insert(*id, r);
                }

                for ((cell, _, _), value) in members.iter().zip(computed) {
                    self.results.insert(*cell, value);
                    stats.evaluated_cells += 1;
                }
                stats.evaluated_groups += 1;
            }
        }

        // docs/27 §3: EVALUATING → IDLE bumps the generation.
        self.generation += 1;
        stats
    }
}

/// Rewrites an AST's `A1` view coordinates from its stored identity bindings.
///
/// This is "A1 is a computed view" made executable (DP-A6): the op carries
/// identities, and the *current* ordinals are recomputed here on every rebuild.
/// A structural edit therefore needs no formula rewriting — the same bindings
/// resolve to different positions.
///
/// A binding whose interval has been entirely deleted becomes `#REF!`, which is
/// docs/11's empty-interval rule.
fn rebind(ast: &Ast, bindings: &[RangeBinding], binder: &Binder) -> Ast {
    let mut next = 0usize;
    rebind_walk(ast, bindings, binder, &mut next)
}

fn rebind_walk(ast: &Ast, bindings: &[RangeBinding], binder: &Binder, next: &mut usize) -> Ast {
    match ast {
        Ast::Reference(_) | Ast::Range(..) => {
            let Some(binding) = bindings.get(*next) else {
                // Fewer bindings than references: a malformed op, not a broken
                // workbook — report rather than guess.
                return Ast::Invalid(ErrorKind::Ref);
            };
            *next += 1;
            let rows = binder
                .rows
                .resolve(&RowId(binding.row_start), &RowId(binding.row_end));
            let cols = binder
                .cols
                .resolve(&ColId(binding.col_start), &ColId(binding.col_end));
            let (Some(r0), Some(r1), Some(c0), Some(c1)) =
                (rows.first(), rows.last(), cols.first(), cols.last())
            else {
                return Ast::Invalid(ErrorKind::Ref);
            };
            let (Some(rp0), Some(rp1), Some(cp0), Some(cp1)) = (
                binder.rows.position_of(r0),
                binder.rows.position_of(r1),
                binder.cols.position_of(c0),
                binder.cols.position_of(c1),
            ) else {
                return Ast::Invalid(ErrorKind::Ref);
            };
            let row_abs = binding.anchors & 1 != 0;
            let col_abs = binding.anchors & 2 != 0;
            let start = A1 {
                row: rp0 as u32,
                col: cp0 as u32,
                row_absolute: row_abs,
                col_absolute: col_abs,
            };
            let end = A1 {
                row: rp1 as u32,
                col: cp1 as u32,
                row_absolute: row_abs,
                col_absolute: col_abs,
            };
            if start == end {
                Ast::Reference(start)
            } else {
                Ast::Range(start, end)
            }
        }
        Ast::Call { name, args } => Ast::Call {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| rebind_walk(a, bindings, binder, next))
                .collect(),
        },
        Ast::Unary(op, inner) => {
            Ast::Unary(*op, Box::new(rebind_walk(inner, bindings, binder, next)))
        }
        Ast::Percent(inner) => Ast::Percent(Box::new(rebind_walk(inner, bindings, binder, next))),
        Ast::Binary(op, l, r) => Ast::Binary(
            *op,
            Box::new(rebind_walk(l, bindings, binder, next)),
            Box::new(rebind_walk(r, bindings, binder, next)),
        ),
        other => other.clone(),
    }
}

/// Read and write extents of a candidate group, with reads merged.
fn extent_of(members: &[Member]) -> (Vec<Rect>, Rect) {
    let mut reads: Vec<Rect> = Vec::new();
    let mut writes: Option<Rect> = None;
    for (_, _, pos, ast) in members {
        let mut member_reads = Vec::new();
        collect_reads(ast, &mut member_reads);
        for r in member_reads {
            match reads.iter_mut().find(|e| rects_mergeable(e, &r)) {
                Some(existing) => *existing = existing.union(&r),
                None => reads.push(r),
            }
        }
        let w = Rect::point(pos.0, pos.1);
        writes = Some(match writes {
            None => w,
            Some(prev) => prev.union(&w),
        });
    }
    (
        reads,
        writes.unwrap_or(Rect {
            r0: 0,
            r1: 0,
            c0: 0,
            c1: 0,
        }),
    )
}

/// Bounding rectangle of each formula's reads, one per member.
fn member_bounds_of(asts: &[Ast]) -> Vec<Rect> {
    let mut out = Vec::with_capacity(asts.len());
    let mut scratch = Vec::new();
    for ast in asts {
        scratch.clear();
        collect_reads(ast, &mut scratch);
        let mut bound: Option<Rect> = None;
        for r in &scratch {
            bound = Some(match bound {
                None => *r,
                Some(prev) => prev.union(r),
            });
        }
        // A formula with no references reads nothing; an empty rectangle at the
        // far corner never overlaps a real edit.
        out.push(bound.unwrap_or(Rect {
            r0: u32::MAX,
            r1: u32::MAX,
            c0: u32::MAX,
            c1: u32::MAX,
        }));
    }
    out
}

/// Bounding rectangle of the members whose reads intersect `source`.
///
/// This is what turns a dirty *group* into a dirty *region*: in a filled column
/// only the member on the edited row reads the edited cell.
fn affected_members(group: &Group, source: &Rect) -> Option<Rect> {
    if !group.reads.iter().any(|r| r.overlaps(source)) {
        return None;
    }
    let mut out: Option<Rect> = None;
    for (pos, bound) in group.positions.iter().zip(group.member_bounds.iter()) {
        if bound.overlaps(source) {
            let w = Rect::point(pos.0, pos.1);
            out = Some(match out {
                None => w,
                Some(prev) => prev.union(&w),
            });
        }
    }
    out
}

/// Merges `rect` into `map[id]`, returning true when the stored rectangle grew.
fn merge_rect(map: &mut BTreeMap<u32, Rect>, id: u32, rect: Rect) -> bool {
    match map.get_mut(&id) {
        None => {
            map.insert(id, rect);
            true
        }
        Some(existing) => {
            let merged = existing.union(&rect);
            if merged != *existing {
                *existing = merged;
                true
            } else {
                false
            }
        }
    }
}

/// True when two rectangles are close enough to merge without pulling in a
/// meaningful amount of unrelated area. Over-merging is safe — a larger read
/// rectangle can only cause extra recalculation, never a missed one.
fn rects_mergeable(a: &Rect, b: &Rect) -> bool {
    let same_cols = a.c0 == b.c0 && a.c1 == b.c1;
    let same_rows = a.r0 == b.r0 && a.r1 == b.r1;
    (same_cols && rows_touch(a, b)) || (same_rows && cols_touch(a, b)) || a.overlaps(b)
}

fn rows_touch(a: &Rect, b: &Rect) -> bool {
    a.r0 <= b.r1.saturating_add(1) && b.r0 <= a.r1.saturating_add(1)
}

fn cols_touch(a: &Rect, b: &Rect) -> bool {
    a.c0 <= b.c1.saturating_add(1) && b.c0 <= a.c1.saturating_add(1)
}

fn collect_reads(ast: &Ast, out: &mut Vec<Rect>) {
    match ast {
        Ast::Reference(r) => out.push(Rect::point(r.row, r.col)),
        Ast::Range(a, b) => out.push(Rect {
            r0: a.row.min(b.row),
            r1: a.row.max(b.row),
            c0: a.col.min(b.col),
            c1: a.col.max(b.col),
        }),
        Ast::Call { args, .. } => {
            for a in args {
                collect_reads(a, out);
            }
        }
        Ast::Unary(_, inner) | Ast::Percent(inner) | Ast::Paren(inner) => collect_reads(inner, out),
        Ast::Binary(_, l, r) => {
            collect_reads(l, out);
            collect_reads(r, out);
        }
        Ast::Literal(_) | Ast::Name(_) | Ast::Invalid(_) => {}
    }
}

/// Renders an AST in R1C1 form relative to a member's derived position.
///
/// This string *is* the grouping key: two cells group exactly when their
/// formulas render identically. `=A1*2` in B1 and `=A2*2` in B2 both render as
/// `R[0]C[-1]*2`, so a filled column becomes one node.
pub fn r1c1(ast: &Ast, anchor_row: u32, anchor_col: u32) -> String {
    let mut out = String::new();
    write_r1c1(ast, anchor_row, anchor_col, &mut out);
    out
}

fn write_r1c1(ast: &Ast, ar: u32, ac: u32, out: &mut String) {
    use core::fmt::Write;
    match ast {
        Ast::Literal(v) => {
            let _ = write!(out, "{v:?}");
        }
        Ast::Reference(r) => write_ref(r, ar, ac, out),
        Ast::Range(a, b) => {
            write_ref(a, ar, ac, out);
            out.push(':');
            write_ref(b, ar, ac, out);
        }
        Ast::Name(n) => {
            out.push_str("N(");
            out.push_str(n);
            out.push(')');
        }
        Ast::Call { name, args } => {
            out.push_str(name);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_r1c1(a, ar, ac, out);
            }
            out.push(')');
        }
        Ast::Unary(op, inner) => {
            out.push(match op {
                UnOp::Neg => '-',
                UnOp::Plus => '+',
            });
            write_r1c1(inner, ar, ac, out);
        }
        Ast::Percent(inner) => {
            write_r1c1(inner, ar, ac, out);
            out.push('%');
        }
        // Rendered, not elided: the R1C1 string *is* the grouping key, and
        // `=(A1+B1-C1)` and `=A1+B1-C1` evaluate differently under the compat
        // cancellation rule (docs/50 finding 2). Grouping them together would
        // give one of them the other's answer.
        Ast::Paren(inner) => {
            out.push('(');
            write_r1c1(inner, ar, ac, out);
            out.push(')');
        }
        Ast::Binary(op, l, r) => {
            out.push('(');
            write_r1c1(l, ar, ac, out);
            out.push_str(match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Pow => "^",
                BinOp::Concat => "&",
                BinOp::Eq => "=",
                BinOp::NotEq => "<>",
                BinOp::Lt => "<",
                BinOp::Gt => ">",
                BinOp::LtEq => "<=",
                BinOp::GtEq => ">=",
            });
            write_r1c1(r, ar, ac, out);
            out.push(')');
        }
        Ast::Invalid(k) => {
            let _ = write!(out, "ERR({k:?})");
        }
    }
}

fn write_ref(r: &A1, ar: u32, ac: u32, out: &mut String) {
    use core::fmt::Write;
    if r.row_absolute {
        let _ = write!(out, "R{}", r.row);
    } else {
        let _ = write!(out, "R[{}]", r.row as i64 - ar as i64);
    }
    if r.col_absolute {
        let _ = write!(out, "C{}", r.col);
    } else {
        let _ = write!(out, "C[{}]", r.col as i64 - ac as i64);
    }
}
