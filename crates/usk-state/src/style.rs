//! The style registry — interned facet values plus stamped rules over identity
//! rectangles (ADR-041, docs/11 §Feature objects, docs/04, docs/14).
//!
//! # Why there is no styled-cell map
//! The obvious model is "a style per cell", and it is wrong for the workload a
//! spreadsheet actually has: formatting a whole column is one gesture, and the
//! column is usually empty. A per-cell store would have to materialise
//! 1,048,576 entries for it — which is exactly the cost ADR-005 built the tile
//! store to avoid, arriving through a different door.
//!
//! So the unit of storage is the **rule**: one `SetStyle`/`ClearStyle` op, one
//! rule, forever. A rule names an identity rectangle, one facet slot, and the
//! `(lamport, op id)` stamp that authored it. Formatting a million-row column
//! allocates one rule and (at most) one interned value; there is no per-cell
//! path for it to take, so the property cannot regress by accident.
//!
//! # Why the register is per `(cell, facet)`
//! Resolution takes the greatest-stamped covering rule **per facet slot,
//! independently**. Two actors, one cell, one turning it bold and the other
//! filling it yellow: both survive, because they contend for different slots.
//! A single style blob per cell would let one clobber the other, and the loser
//! is a change the user watched themselves make — the outcome ADR-006 exists
//! to forbid. The displaced rule stays in the registry and is reachable
//! through [`StyleRegistry::losers`], which is retain-losers at facet
//! granularity.
//!
//! # Why this converges
//! Rules are held in ascending stamp order and inserted by binary search, and
//! `(lamport, op id)` is a total order over ops. So the registry is a pure
//! function of the op *set*: any arrival order produces the identical `Vec`,
//! and re-applying an op it already holds changes nothing (DP-A8).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use usk_oplog::{AxisSpan, StyleFacet, StyleTarget};
use usk_types::{ColId, Lamport, OpId, RowId};

/// An interned facet value's identity — docs/04's *"Style | interned flyweight
/// id | value object, deduplicated"*.
pub type FacetId = u32;

/// What a rule says about its facet slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleValue {
    Set(FacetId),
    /// Back to the workbook default. Not the absence of a rule: a clear must
    /// out-rank an *earlier* set, which only a stamped rule can do.
    Clear,
}

/// One `SetStyle`/`ClearStyle` op, as state holds it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    pub stamp: (Lamport, OpId),
    pub target: StyleTarget,
    pub slot: u8,
    pub value: RuleValue,
}

/// The interner. Values are stored once and referred to by id, which is what
/// makes a thousand cells sharing one format cost one format.
#[derive(Default, Clone)]
pub struct StyleTable {
    values: Vec<StyleFacet>,
    index: BTreeMap<StyleFacet, FacetId>,
}

impl StyleTable {
    /// Interns a value, returning the existing id if it is already held.
    /// First-use order, so the table is a pure function of the op set.
    fn intern(&mut self, facet: StyleFacet) -> FacetId {
        if let Some(id) = self.index.get(&facet) {
            return *id;
        }
        let id = self.values.len() as FacetId;
        self.index.insert(facet.clone(), id);
        self.values.push(facet);
        id
    }

    pub fn get(&self, id: FacetId) -> Option<&StyleFacet> {
        self.values.get(id as usize)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn values(&self) -> &[StyleFacet] {
        &self.values
    }
}

/// The workbook's formatting: an interner plus stamped rules.
#[derive(Default, Clone)]
pub struct StyleRegistry {
    table: StyleTable,
    /// Ascending by stamp. Insertion is a sorted insert rather than a push,
    /// which is what makes the registry order-independent (see module note).
    rules: Vec<Rule>,
}

impl StyleRegistry {
    /// Applies a `SetStyle`. Idempotent: an op already held is ignored, so a
    /// relay redelivering it changes nothing (DP-A8).
    pub(crate) fn set(&mut self, stamp: (Lamport, OpId), target: StyleTarget, facet: StyleFacet) {
        let slot = facet.slot();
        // An *unknown* facet is preserved in the log and applied to nothing
        // (DP-A5): this build cannot say what it would look like, and guessing
        // is the one outcome worse than ignoring it.
        if matches!(facet, StyleFacet::Unknown(_)) {
            return;
        }
        let value = RuleValue::Set(self.table.intern(facet));
        self.insert(Rule {
            stamp,
            target,
            slot,
            value,
        });
    }

    /// Applies a `ClearStyle`.
    pub(crate) fn clear(&mut self, stamp: (Lamport, OpId), target: StyleTarget, slot: u8) {
        self.insert(Rule {
            stamp,
            target,
            slot,
            value: RuleValue::Clear,
        });
    }

    fn insert(&mut self, rule: Rule) {
        match self.rules.binary_search_by(|r| r.stamp.cmp(&rule.stamp)) {
            Ok(_) => {}
            Err(at) => self.rules.insert(at, rule),
        }
    }

    /// Every rule, ascending by stamp — the order the state hash folds in.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn table(&self) -> &StyleTable {
        &self.table
    }

    /// True when no style op has ever been applied. The state hash adds a
    /// styles section only when this is false, so every workbook authored
    /// before styles existed hashes exactly as it did (docs/10's
    /// additive-evolution rule, applied to the hash itself).
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Structural heap bytes, for the W-STYLE-COLUMN memory claim.
    pub fn heap_bytes(&self) -> usize {
        let rules = self.rules.capacity() * core::mem::size_of::<Rule>();
        let values: usize = self
            .table
            .values
            .iter()
            .map(|v| core::mem::size_of::<StyleFacet>() + facet_payload_bytes(v))
            .sum();
        // The index holds a second copy of each key; counted rather than
        // excused, because a memory claim that omits its own bookkeeping is
        // not a memory claim.
        rules + 2 * values + self.table.values.capacity() * core::mem::size_of::<StyleFacet>()
    }

    /// Rebuilds a registry from an image's styles section.
    ///
    /// `None` when a rule names a value the table does not hold: an image is
    /// an untrusted input (docs/37), and a dangling id would resolve to the
    /// workbook default — a silently *different* document rather than a
    /// refused one. Rules are re-sorted rather than trusted, so a tampered
    /// order cannot make a restored replica resolve differently from one that
    /// never restarted.
    pub(crate) fn from_image(
        values: Vec<StyleFacet>,
        mut rules: Vec<Rule>,
    ) -> Option<StyleRegistry> {
        let mut table = StyleTable::default();
        for value in values {
            // Re-interning rather than trusting the written ids is what makes a
            // duplicated table entry impossible to smuggle in: `intern` is the
            // only way a value acquires an id, here as on the apply path.
            table.intern(value);
        }
        for rule in &rules {
            if let RuleValue::Set(id) = rule.value {
                table.get(id)?;
            }
        }
        rules.sort_by_key(|rule| rule.stamp);
        rules.dedup_by(|a, b| a.stamp == b.stamp);
        Some(StyleRegistry { table, rules })
    }

    /// The rules a cell's facet slot lost to, newest first — retain-losers
    /// (ADR-006) at facet granularity. The winner is **not** included.
    pub fn losers<'a>(
        &'a self,
        at: &'a StyleResolver,
        cell: (RowId, ColId),
        slot: u8,
    ) -> Vec<&'a Rule> {
        let mut out = Vec::new();
        let mut seen_winner = false;
        for (index, rule) in self.rules.iter().enumerate().rev() {
            if rule.slot != slot || !at.covers(index, cell) {
                continue;
            }
            if seen_winner {
                out.push(rule);
            }
            seen_winner = true;
        }
        out
    }
}

fn facet_payload_bytes(facet: &StyleFacet) -> usize {
    match facet {
        StyleFacet::NumberFormat(code) => code.len(),
        StyleFacet::Font(f) => f.name.len(),
        StyleFacet::Unknown(u) => u.body().len(),
        StyleFacet::Fill(_) | StyleFacet::Align(_) => 0,
    }
}

/// A cell's resolved formatting — one winner per facet slot, or the workbook
/// default where nothing covers it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ResolvedStyle {
    pub number_format: Option<usk_oplog::StyleFacet>,
    pub font: Option<usk_oplog::StyleFacet>,
    pub fill: Option<usk_oplog::StyleFacet>,
    pub align: Option<usk_oplog::StyleFacet>,
}

impl ResolvedStyle {
    pub fn is_default(&self) -> bool {
        self.number_format.is_none()
            && self.font.is_none()
            && self.fill.is_none()
            && self.align.is_none()
    }

    /// The number-format *code*, which is the one facet the XLSX layer has
    /// always modelled and the one every other layer asks for by name.
    pub fn number_format_code(&self) -> Option<&str> {
        match &self.number_format {
            Some(StyleFacet::NumberFormat(code)) => Some(code.as_str()),
            _ => None,
        }
    }
}

/// Style rules resolved against one axis order.
///
/// # Why this is built once rather than queried per cell
/// A rule names identities; a cell is a `(row, col)` identity pair; deciding
/// whether one covers the other means locating four endpoints in the axis
/// order and re-anchoring each inward past tombstones (docs/11's rule, the same
/// one `usk_calc::refs::Axis::resolve` implements for formula ranges). Doing
/// that per cell would repeat, for every cell of a render frame, work that
/// depends only on the rule and the axis.
///
/// So each rule is resolved **once** into a window of full-order indices, and a
/// coverage test is then two range comparisons. Build one per frame, per
/// export, or per test; it borrows nothing from the registry and goes stale the
/// moment an axis changes, which is why it is a separate value rather than a
/// cache inside `State`.
pub struct StyleResolver {
    rows: Vec<(OpId, bool)>,
    cols: Vec<(OpId, bool)>,
    row_index: BTreeMap<OpId, usize>,
    col_index: BTreeMap<OpId, usize>,
    /// Per rule, in registry order: the full-order index window it covers, or
    /// `None` when every identity it named is gone.
    windows: Vec<Option<Window>>,
}

#[derive(Clone, Copy)]
struct Window {
    row_lo: usize,
    row_hi: usize,
    col_lo: usize,
    col_hi: usize,
}

impl StyleResolver {
    pub(crate) fn build(
        registry: &StyleRegistry,
        rows: &[(OpId, bool)],
        cols: &[(OpId, bool)],
    ) -> StyleResolver {
        let row_index: BTreeMap<OpId, usize> = rows
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();
        let col_index: BTreeMap<OpId, usize> = cols
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();
        let windows = registry
            .rules()
            .iter()
            .map(|rule| {
                let (row_lo, row_hi) = resolve_span(&rule.target.rows, rows, &row_index)?;
                let (col_lo, col_hi) = resolve_span(&rule.target.cols, cols, &col_index)?;
                Some(Window {
                    row_lo,
                    row_hi,
                    col_lo,
                    col_hi,
                })
            })
            .collect();
        StyleResolver {
            rows: rows.to_vec(),
            cols: cols.to_vec(),
            row_index,
            col_index,
            windows,
        }
    }

    fn window_of(&self, target: &StyleTarget) -> Option<Window> {
        let (row_lo, row_hi) = resolve_span(&target.rows, &self.rows, &self.row_index)?;
        let (col_lo, col_hi) = resolve_span(&target.cols, &self.cols, &self.col_index)?;
        if row_lo > row_hi || col_lo > col_hi {
            return None;
        }
        Some(Window {
            row_lo,
            row_hi,
            col_lo,
            col_hi,
        })
    }

    /// The rules for one facet slot that overlap `target`, **ascending by
    /// stamp**, each clipped to the overlap.
    ///
    /// This is what makes undo exact (ADR-041 decision 5). Replaying the
    /// returned list over a cleared `target` reproduces the previous resolution
    /// cell for cell, because every returned rectangle is a subset of `target`
    /// and the list is in the order the stamps originally layered them.
    ///
    /// A clipped span stays `All` only when **both** sides were `All` —
    /// otherwise the intersection is bounded and is written as identities. That
    /// distinction is load-bearing: collapsing `All ∩ All` to today's first and
    /// last row would quietly convert a column rule into a row-range rule, and
    /// the next row inserted would come out unformatted.
    pub fn overlapping(
        &self,
        registry: &StyleRegistry,
        target: &StyleTarget,
        slot: u8,
    ) -> Vec<(StyleTarget, Option<StyleFacet>, (Lamport, OpId))> {
        let Some(mine) = self.window_of(target) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (index, rule) in registry.rules().iter().enumerate() {
            if rule.slot != slot {
                continue;
            }
            let Some(Some(w)) = self.windows.get(index) else {
                continue;
            };
            let row_lo = w.row_lo.max(mine.row_lo);
            let row_hi = w.row_hi.min(mine.row_hi);
            let col_lo = w.col_lo.max(mine.col_lo);
            let col_hi = w.col_hi.min(mine.col_hi);
            if row_lo > row_hi || col_lo > col_hi {
                continue;
            }
            let clipped = StyleTarget {
                rows: clip_span(&rule.target.rows, &target.rows, &self.rows, row_lo, row_hi),
                cols: clip_span(&rule.target.cols, &target.cols, &self.cols, col_lo, col_hi),
            };
            let value = match rule.value {
                RuleValue::Set(id) => registry.table().get(id).cloned(),
                RuleValue::Clear => None,
            };
            out.push((clipped, value, rule.stamp));
        }
        out
    }

    /// Where a cell sits in the two full orders, resolved **once** per cell.
    ///
    /// Hoisting this out of the per-rule loop is not a micro-optimisation: it
    /// is the difference between a range read costing two tree walks and
    /// costing two tree walks *per rule*. W-STYLE-COLUMN measured the inner
    /// version at **1.8–3.4 µs per cell per facet** on a 262,144-row sheet with
    /// 64 rules — 2.4 ms for one facet over a viewport, against docs/31's 8.3 ms
    /// whole-frame budget. Same shape as TD-71 in the tile store, found the same
    /// way: by measuring rather than by reasoning about it.
    fn locate(&self, cell: (RowId, ColId)) -> Option<(usize, usize)> {
        Some((
            *self.row_index.get(&cell.0 .0)?,
            *self.col_index.get(&cell.1 .0)?,
        ))
    }

    fn covers_at(&self, rule_index: usize, at: (usize, usize)) -> bool {
        let Some(Some(window)) = self.windows.get(rule_index) else {
            return false;
        };
        window.row_lo <= at.0
            && at.0 <= window.row_hi
            && window.col_lo <= at.1
            && at.1 <= window.col_hi
    }

    fn covers(&self, rule_index: usize, cell: (RowId, ColId)) -> bool {
        match self.locate(cell) {
            Some(at) => self.covers_at(rule_index, at),
            None => false,
        }
    }

    /// The winning value of one facet slot at one cell, or `None` for the
    /// workbook default.
    ///
    /// Scans newest-first and stops at the first covering rule, which is both
    /// the correct answer (greatest stamp wins) and the fast one (the rule a
    /// user just applied is at the end).
    pub fn facet<'a>(
        &self,
        registry: &'a StyleRegistry,
        row: RowId,
        col: ColId,
        slot: u8,
    ) -> Option<&'a StyleFacet> {
        let at = self.locate((row, col))?;
        for (index, rule) in registry.rules().iter().enumerate().rev() {
            if rule.slot != slot || !self.covers_at(index, at) {
                continue;
            }
            return match rule.value {
                RuleValue::Set(id) => registry.table().get(id),
                RuleValue::Clear => None,
            };
        }
        None
    }

    /// Every facet at one cell, resolved in a single scan.
    pub fn style(&self, registry: &StyleRegistry, row: RowId, col: ColId) -> ResolvedStyle {
        let mut out = ResolvedStyle::default();
        let mut decided = [false; 4];
        if registry.is_empty() {
            return out;
        }
        let Some(at) = self.locate((row, col)) else {
            return out;
        };
        for (index, rule) in registry.rules().iter().enumerate().rev() {
            let Some(slot) = (1..=4u8).position(|s| s == rule.slot) else {
                continue;
            };
            if decided[slot] || !self.covers_at(index, at) {
                continue;
            }
            decided[slot] = true;
            let value = match rule.value {
                RuleValue::Set(id) => registry.table().get(id).cloned(),
                RuleValue::Clear => None,
            };
            match slot {
                0 => out.number_format = value,
                1 => out.font = value,
                2 => out.fill = value,
                _ => out.align = value,
            }
            if decided.iter().all(|d| *d) {
                break;
            }
        }
        out
    }
}

/// The intersection of two spans, expressed in the *weakest* form that is still
/// exact. `All ∩ All` is `All`; anything else is bounded by identities, which
/// the caller has already resolved to `lo`/`hi`.
fn clip_span(a: &AxisSpan, b: &AxisSpan, order: &[(OpId, bool)], lo: usize, hi: usize) -> AxisSpan {
    if matches!(a, AxisSpan::All) && matches!(b, AxisSpan::All) {
        return AxisSpan::All;
    }
    // `lo` and `hi` are each the max/min of two *live* endpoints, so each is
    // itself one of them and is therefore live.
    AxisSpan::Between(order[lo].0, order[hi].0)
}

/// One axis of a target, resolved to a window of full-order indices.
///
/// `All` is the whole axis, including identities inserted after the rule was
/// authored — which is the entire point of the variant. `Between` locates its
/// endpoints and re-anchors each inward past tombstones, exactly as docs/11
/// specifies for a reference interval; every identity gone means the rule
/// covers nothing rather than covering everything.
fn resolve_span(
    span: &AxisSpan,
    order: &[(OpId, bool)],
    index: &BTreeMap<OpId, usize>,
) -> Option<(usize, usize)> {
    match span {
        AxisSpan::All => {
            if order.is_empty() {
                // An empty axis is covered by nothing, but the rule survives:
                // a column formatted before any row exists must format the
                // rows that arrive later.
                return Some((usize::MAX, 0));
            }
            Some((0, order.len() - 1))
        }
        AxisSpan::Between(start, end) => {
            let (a, b) = (*index.get(start)?, *index.get(end)?);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let mut low = lo;
            while low <= hi && !order[low].1 {
                low += 1;
            }
            if low > hi {
                return None;
            }
            let mut high = hi;
            while high > low && !order[high].1 {
                high -= 1;
            }
            if !order[high].1 {
                return None;
            }
            Some((low, high))
        }
    }
}
