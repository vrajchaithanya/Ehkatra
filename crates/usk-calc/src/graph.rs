//! The dependency graph: formula groups, range-granular edges, and incremental
//! level-ordered recalculation (docs/13).

use crate::sheet::{Cell, CellRef, Rect, Sheet};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use usk_formula::eval::{eval, Context};
use usk_formula::parse::{parse, Ast, BinOp, UnOp, A1};
use usk_types::coerce::Profile;
use usk_types::{CellError, ErrorKind, Origin, Value};

/// Rows per index band. Matches the tile band height (docs/14) so a dirty tile
/// maps to one bucket.
const BAND: u32 = 256;

/// A set of cells sharing one R1C1 pattern — a single node in the graph.
///
/// This is the structure docs/13 relies on for "1M cells ≈ hundreds of nodes":
/// a filled column is one group whose read set is one rectangle, not 1M edges.
pub struct Group {
    /// The R1C1 rendering all members share. Two cells are in the same group
    /// iff this string matches.
    pub pattern: String,
    /// Members, in row-major identity order — a deterministic traversal is a
    /// convergence requirement (docs/13).
    pub cells: Vec<CellRef>,
    /// Per-member parsed formula. Held because members differ by their relative
    /// references even though the pattern is shared.
    asts: Vec<Ast>,
    /// Rectangles this group reads, unioned across members.
    pub reads: Vec<Rect>,
    /// Bounding rectangle of *each member's* reads, in the same order as
    /// `cells`.
    ///
    /// Precomputed because the incremental path asks "which members read this
    /// rectangle?" on every edit, and re-walking 10,000 member ASTs to answer
    /// it measured at 10.5 ms against docs/31's 8 ms single-edit budget. A
    /// bounding rectangle is a safe over-approximation: it can only add an
    /// evaluation, never miss one.
    member_bounds: Vec<Rect>,
    /// Bounding rectangle of the members — what the group writes.
    pub writes: Rect,
}

/// What one recalculation actually did — the numbers the bench records.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecalcStats {
    /// Groups marked dirty, before evaluation.
    pub dirty_groups: usize,
    /// Groups actually evaluated.
    pub evaluated_groups: usize,
    /// Cells whose value was recomputed.
    pub evaluated_cells: usize,
    /// Groups skipped because an upstream group's values did not change —
    /// docs/13's early cutoff.
    pub cut_off_groups: usize,
    /// Topological levels; every group in one level is independent, so this is
    /// the width available to a parallel evaluator.
    pub levels: usize,
    /// Groups found to be in a cycle and set to `#CIRC!`.
    pub circular_groups: usize,
}

/// A band-bucketed rectangle index.
///
/// docs/13 specifies an R-tree over identity space. This is the cheaper
/// structure with the same asymptotic *shape* for the access pattern that
/// matters — a stab lands in one bucket and scans only the groups whose read
/// rectangles cross that band, rather than every group in the workbook. It is
/// honestly not an R-tree; when profiles show band scanning dominating, or when
/// Row 8 moves rectangles into identity space, replace it behind this
/// interface.
#[derive(Default)]
struct BandIndex {
    /// band → group ids whose read rectangles intersect the band.
    bands: BTreeMap<u32, Vec<u32>>,
}

impl BandIndex {
    fn insert(&mut self, group: u32, rect: &Rect) {
        let first = rect.r0 / BAND;
        let last = rect.r1 / BAND;
        for band in first..=last {
            let slot = self.bands.entry(band).or_default();
            if !slot.contains(&group) {
                slot.push(group);
            }
        }
    }

    /// Groups that might read any cell inside `rect`.
    fn stab(&self, rect: &Rect, groups: &[Group], out: &mut BTreeSet<u32>) {
        let first = rect.r0 / BAND;
        let last = rect.r1 / BAND;
        for band in first..=last {
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

/// The calculation engine over one sheet.
pub struct Engine {
    pub sheet: Sheet,
    groups: Vec<Group>,
    index: BandIndex,
    profile: Profile,
    /// Materialised volatiles (docs/13 T2): read, never computed ambiently, so
    /// replicas converge because they read the same recorded value.
    pub today: i32,
    pub now: f64,
}

impl Engine {
    /// Builds the graph over a sheet's current formulas.
    ///
    /// Grouping is a full pass, not incremental: Row 7's claim is about the
    /// *graph's* size and the recalc's cost, not about edit-time regrouping,
    /// which arrives with the reducer in Row 9.
    pub fn build(mut sheet: Sheet, profile: Profile) -> Engine {
        let mut by_pattern: BTreeMap<String, Vec<(CellRef, Ast)>> = BTreeMap::new();

        for cell in sheet.formula_cells() {
            let Some(source) = sheet.formula_source(cell) else {
                continue;
            };
            let ast = parse(source).ast;
            let pattern = r1c1(&ast, cell);
            by_pattern.entry(pattern).or_default().push((cell, ast));
        }

        let mut groups: Vec<Group> = Vec::new();
        let mut index = BandIndex::default();

        for (pattern, members) in by_pattern {
            let id = groups.len() as u32;
            let mut cells = Vec::with_capacity(members.len());
            let mut asts = Vec::with_capacity(members.len());
            let mut reads: Vec<Rect> = Vec::new();
            let mut writes: Option<Rect> = None;

            for (cell, ast) in members {
                let mut member_reads = Vec::new();
                collect_reads(&ast, &mut member_reads);
                for r in member_reads {
                    // Union into an existing rect when they touch, so a filled
                    // column collapses to one rectangle instead of N.
                    match reads.iter_mut().find(|e| rects_mergeable(e, &r)) {
                        Some(existing) => *existing = existing.union(&r),
                        None => reads.push(r),
                    }
                }
                let w = Rect::single(cell);
                writes = Some(match writes {
                    None => w,
                    Some(prev) => prev.union(&w),
                });
                cells.push(cell);
                asts.push(ast);
            }

            let writes = writes.unwrap_or(Rect {
                r0: 0,
                r1: 0,
                c0: 0,
                c1: 0,
            });

            // A group whose unioned read set overlaps its own write set may
            // contain members that depend on each other — `=A1+1` in B1 and
            // `=B1+1` in C1 share one R1C1 pattern but form a chain. Evaluating
            // such members together against one snapshot would silently compute
            // stale answers, so the group is split into singletons and the
            // ordinary graph machinery orders them. The grouping win is given up
            // exactly where grouping cannot be applied, and nowhere else.
            //
            // After splitting, a read set that still overlaps its own single
            // cell is a true self-reference, which `evaluate` reports as
            // `#CIRC!`.
            let self_referential = reads.iter().any(|r| r.overlaps(&writes));
            if self_referential && cells.len() > 1 {
                // Splitting straight to singletons here would be correct but
                // catastrophic: a chain of derived columns (`=A1+1`, `=B1+1`, …)
                // shares one pattern, so the *whole* 100k-cell region would
                // become 100k nodes and the grouping win would vanish in the
                // most ordinary model shape there is. Measured: 100,000 groups
                // for 100,000 cells.
                //
                // Partition by column first. A horizontal chain then becomes one
                // group per column, each reading only the column to its left —
                // no self-overlap, and the fill stays a single node. Only a
                // partition that *still* overlaps itself (a genuine vertical
                // running total, which is inherently serial) falls back to
                // singletons.
                let mut by_column: BTreeMap<u32, Vec<(CellRef, Ast)>> = BTreeMap::new();
                for (cell, ast) in cells.into_iter().zip(asts) {
                    by_column.entry(cell.col).or_default().push((cell, ast));
                }
                for (_, members) in by_column {
                    let (part_reads, part_writes) = extent_of(&members);
                    if part_reads.iter().any(|r| r.overlaps(&part_writes)) {
                        for (cell, ast) in members {
                            let mut member_reads = Vec::new();
                            collect_reads(&ast, &mut member_reads);
                            push_group(
                                &mut groups,
                                &mut index,
                                &mut sheet,
                                pattern.clone(),
                                alloc::vec![cell],
                                alloc::vec![ast],
                                member_reads,
                                Rect::single(cell),
                            );
                        }
                    } else {
                        let (pc, pa): (Vec<CellRef>, Vec<Ast>) = members.into_iter().unzip();
                        push_group(
                            &mut groups,
                            &mut index,
                            &mut sheet,
                            pattern.clone(),
                            pc,
                            pa,
                            part_reads,
                            part_writes,
                        );
                    }
                }
                continue;
            }

            for cell in &cells {
                sheet.assign_group(*cell, id);
            }
            for r in &reads {
                index.insert(id, r);
            }
            let member_bounds = member_bounds_of(&asts);
            groups.push(Group {
                pattern,
                cells,
                asts,
                reads,
                member_bounds,
                writes,
            });
        }

        Engine {
            sheet,
            groups,
            index,
            profile,
            today: 0,
            now: 0.0,
        }
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Recalculates everything. Used for a cold open and by the bench.
    pub fn recalc_all(&mut self) -> RecalcStats {
        let seeds: BTreeMap<u32, Rect> = self
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| (i as u32, g.writes))
            .collect();
        self.evaluate(seeds, false)
    }

    /// Recalculates only what a write to `changed` can affect.
    ///
    /// The unit of dirtiness is a **rectangle inside a group**, not the group.
    /// A group can be 10,000 cells; marking the whole of it because one input
    /// cell moved recomputes the entire column, which measured at 53 ms for a
    /// single edit against docs/31's 8 ms budget. Carrying the rectangle
    /// through marking and evaluation is what makes "incremental" mean it.
    pub fn recalc_after(&mut self, changed: &[CellRef]) -> RecalcStats {
        let mut seeds: BTreeMap<u32, Rect> = BTreeMap::new();
        for cell in changed {
            let source = Rect::single(*cell);
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
        self.evaluate(seeds, true)
    }

    /// Marks `seeds` and everything transitively downstream, orders by level,
    /// and evaluates only the dirty members of each group.
    fn evaluate(&mut self, seeds: BTreeMap<u32, Rect>, cutoff: bool) -> RecalcStats {
        let mut stats = RecalcStats::default();

        // Transitive marking, carrying the dirty rectangle. A group is
        // re-visited only when its dirty rectangle actually grows, which is
        // what terminates the walk.
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
        // A group that reads its own output is a cycle of one. Group splitting
        // in `build` leaves only genuine self-references here.
        let mut self_cyclic: BTreeSet<u32> = BTreeSet::new();

        // Edges come from the index, not from comparing every pair. The nested
        // loop is O(groups^2) and was measured hanging outright once a chain of
        // derived columns produced 100k nodes; the index answers "who reads this
        // rectangle" directly, which is the whole reason it exists.
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
            // Sorted so the level's contents are a pure function of the graph,
            // never of hash iteration order (DP-A2).
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

        // Anything the level assignment could not place is in a cycle: the same
        // fact Tarjan would report, obtained from work already done.
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
                        self.sheet.store_result(
                            cell,
                            Value::Error(CellError::new(ErrorKind::Circ, Origin::Propagated)),
                        );
                        stats.evaluated_cells += 1;
                    }
                }
            }
        }

        // Rectangles whose values actually changed this pass, per group. This is
        // both the early-cutoff signal (docs/13) and the precise dirty region
        // handed to downstream groups.
        let mut changed: BTreeMap<u32, Rect> = BTreeMap::new();

        for level in &levels {
            for id in level {
                // Recompute the dirty region precisely from what upstream
                // actually changed, rather than from the conservative marking.
                // A group whose inputs all came back identical has no region at
                // all and is skipped — docs/13's early cutoff.
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
                // Only the members inside the dirty region are evaluated.
                let members: Vec<(CellRef, Ast)> = group
                    .cells
                    .iter()
                    .zip(group.asts.iter())
                    .filter(|(c, _)| region.contains(**c))
                    .map(|(c, a)| (*c, a.clone()))
                    .collect();
                if members.is_empty() {
                    stats.cut_off_groups += 1;
                    continue;
                }

                // Compute first, then write: a group's members must all see the
                // same upstream state.
                let mut results = Vec::with_capacity(members.len());
                {
                    let ctx = Context {
                        grid: &self.sheet,
                        profile: self.profile,
                        today: self.today,
                        now: self.now,
                    };
                    for (_, ast) in &members {
                        results.push(eval(ast, &ctx));
                    }
                }

                let mut changed_rect: Option<Rect> = None;
                for ((cell, _), value) in members.iter().zip(results.iter()) {
                    let previous = match self.sheet.cell(*cell) {
                        Some(Cell::Formula { cached, .. }) => Some(cached.clone()),
                        _ => None,
                    };
                    if previous.as_ref() != Some(value) {
                        let r = Rect::single(*cell);
                        changed_rect = Some(match changed_rect {
                            None => r,
                            Some(prev) => prev.union(&r),
                        });
                    }
                }
                if let Some(r) = changed_rect {
                    changed.insert(*id, r);
                }

                for ((cell, _), value) in members.iter().zip(results) {
                    self.sheet.store_result(*cell, value);
                    stats.evaluated_cells += 1;
                }
                stats.evaluated_groups += 1;
            }
        }

        stats
    }
}

/// Bounding rectangle of the members of `group` whose reads intersect `source`.
///
/// This is what turns a dirty *group* into a dirty *region*: in a filled column
/// only the member on the edited row reads the edited cell, so a one-cell edit
/// produces a one-cell region instead of the whole column.
fn affected_members(group: &Group, source: &Rect) -> Option<Rect> {
    // Cheap rejection first: if the group's unioned read set misses entirely,
    // no member can be affected.
    if !group.reads.iter().any(|r| r.overlaps(source)) {
        return None;
    }
    let mut out: Option<Rect> = None;
    for (cell, bound) in group.cells.iter().zip(group.member_bounds.iter()) {
        if bound.overlaps(source) {
            let w = Rect::single(*cell);
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

/// Read and write extents of a candidate group, with reads merged.
fn extent_of(members: &[(CellRef, Ast)]) -> (Vec<Rect>, Rect) {
    let mut reads: Vec<Rect> = Vec::new();
    let mut writes: Option<Rect> = None;
    for (cell, ast) in members {
        let mut member_reads = Vec::new();
        collect_reads(ast, &mut member_reads);
        for r in member_reads {
            match reads.iter_mut().find(|e| rects_mergeable(e, &r)) {
                Some(existing) => *existing = existing.union(&r),
                None => reads.push(r),
            }
        }
        let w = Rect::single(*cell);
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

#[allow(clippy::too_many_arguments)]
fn push_group(
    groups: &mut Vec<Group>,
    index: &mut BandIndex,
    sheet: &mut Sheet,
    pattern: String,
    cells: Vec<CellRef>,
    asts: Vec<Ast>,
    reads: Vec<Rect>,
    writes: Rect,
) {
    let id = groups.len() as u32;
    for cell in &cells {
        sheet.assign_group(*cell, id);
    }
    for r in &reads {
        index.insert(id, r);
    }
    let member_bounds = member_bounds_of(&asts);
    groups.push(Group {
        pattern,
        cells,
        asts,
        reads,
        member_bounds,
        writes,
    });
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

/// True when two rectangles are close enough to merge without pulling in a
/// meaningful amount of unrelated area.
///
/// Merging is what collapses a filled column's 100,000 single-cell reads into
/// one rectangle. Over-merging is *safe* — a larger read rectangle can only
/// cause extra recalculation, never a missed one — so the rule is deliberately
/// generous along a shared row or column band.
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

/// Every rectangle an AST reads.
fn collect_reads(ast: &Ast, out: &mut Vec<Rect>) {
    match ast {
        Ast::Reference(r) => out.push(Rect {
            r0: r.row,
            r1: r.row,
            c0: r.col,
            c1: r.col,
        }),
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
        Ast::Unary(_, inner) | Ast::Percent(inner) => collect_reads(inner, out),
        Ast::Binary(_, l, r) => {
            collect_reads(l, out);
            collect_reads(r, out);
        }
        Ast::Literal(_) | Ast::Name(_) | Ast::Invalid(_) => {}
    }
}

/// Renders an AST in R1C1 form relative to `anchor`.
///
/// This string *is* the grouping key: two cells belong to one group exactly
/// when their formulas render identically here. `=A1*2` in B1 and `=A2*2` in B2
/// both render as `R[0]C[-1]*2`, so a filled column becomes one node.
/// Absolute references render as fixed coordinates, so `$A$1` never merges with
/// a relative reference that happens to point at the same cell.
pub fn r1c1(ast: &Ast, anchor: CellRef) -> String {
    let mut out = String::new();
    write_r1c1(ast, anchor, &mut out);
    out
}

fn write_r1c1(ast: &Ast, anchor: CellRef, out: &mut String) {
    use core::fmt::Write;
    match ast {
        Ast::Literal(v) => {
            let _ = write!(out, "{v:?}");
        }
        Ast::Reference(r) => write_ref(r, anchor, out),
        Ast::Range(a, b) => {
            write_ref(a, anchor, out);
            out.push(':');
            write_ref(b, anchor, out);
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
                write_r1c1(a, anchor, out);
            }
            out.push(')');
        }
        Ast::Unary(op, inner) => {
            out.push(match op {
                UnOp::Neg => '-',
                UnOp::Plus => '+',
            });
            write_r1c1(inner, anchor, out);
        }
        Ast::Percent(inner) => {
            write_r1c1(inner, anchor, out);
            out.push('%');
        }
        Ast::Binary(op, l, r) => {
            out.push('(');
            write_r1c1(l, anchor, out);
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
            write_r1c1(r, anchor, out);
            out.push(')');
        }
        Ast::Invalid(k) => {
            let _ = write!(out, "ERR({k:?})");
        }
    }
}

fn write_ref(r: &A1, anchor: CellRef, out: &mut String) {
    use core::fmt::Write;
    if r.row_absolute {
        let _ = write!(out, "R{}", r.row);
    } else {
        let _ = write!(out, "R[{}]", r.row as i64 - anchor.row as i64);
    }
    if r.col_absolute {
        let _ = write!(out, "C{}", r.col);
    } else {
        let _ = write!(out, "C[{}]", r.col as i64 - anchor.col as i64);
    }
}
