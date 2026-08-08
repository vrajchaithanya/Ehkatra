//! The formula registry — a stamped LWW register per cell (TD-22, docs/14).
//!
//! # Why this module exists
//! A cell's winning content is whichever of `SetCell`, `ClearCell` or
//! `SetFormula` comes last in the canonical total order. Row 9 implemented that
//! by *insert on formula, remove on value*, which is correct only while every
//! op is applied in that order — the precondition D-056 recorded as TD-22.
//!
//! Sync (Row 10) brings ops that arrive in transport order, so the rule is now
//! carried by **per-entry stamps** instead of by application order. Every
//! mutation here is `max` over `(lamport, op id)` with the winner's payload
//! kept, which makes the registry commutative, associative and idempotent — a
//! textbook LWW register (docs/15 §Merge semantics). Applying the same op set
//! in any order yields the same registry, and `registry_is_order_independent`
//! proves it over every permutation of a mixed history.
//!
//! # Why entries are seeded by a pre-pass
//! Order independence needs a value write to be *comparable* with a formula
//! write at the same cell, which means the value write has to leave a stamp. A
//! stamp for every written cell is exactly the per-cell metadata the tile store
//! exists to avoid (ADR-005): 10M cells would pay ~480 MB for a feature almost
//! no cell uses.
//!
//! So the registry holds an entry only for cells that a `SetFormula` op names
//! *somewhere* in the log, and that set is computed by the same replay pre-pass
//! that decides tile promotion (`tile::plan_promotions`). A cell outside the set
//! can never hold a formula, so nothing there is ever compared and nothing needs
//! stamping. This is the promotion argument applied a second time: pay per-cell
//! costs only where per-cell decisions actually happen.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use usk_oplog::RangeBinding;
use usk_types::{ColId, Lamport, OpId, RowId};

/// The identity of a cell — `(row, col)` as op ids, the registry's key.
pub type CellKey = (OpId, OpId);

/// The write that produced a cell's current content, in the canonical total
/// order. `(0, …)` is the seeded "nothing has been written yet" floor, which
/// every real op beats because minted lamports start at 1.
type Stamp = (Lamport, OpId);

const FLOOR: Stamp = (
    0,
    OpId {
        actor: usk_types::ActorId(0),
        counter: 0,
    },
);

/// A stored formula: the source text plus its identity bindings, exactly as the
/// `SetFormula` op carried them.
#[derive(Clone, PartialEq, Debug)]
pub struct FormulaCell {
    pub source: String,
    pub bindings: Vec<RangeBinding>,
}

/// One cell's LWW register. `formula` is `Some` when the winning write at this
/// cell was a `SetFormula`, and `None` when a value write won — including the
/// seeded state, where nothing has been written at all.
#[derive(Clone)]
struct Entry {
    stamp: Stamp,
    formula: Option<FormulaCell>,
}

/// Formulas keyed by cell identity — a flat map rather than tile payload,
/// because docs/14 has formulas *reference* the group table, never packed among
/// values.
#[derive(Default, Clone)]
pub struct FormulaRegistry {
    entries: BTreeMap<CellKey, Entry>,
}

impl FormulaRegistry {
    /// Seeds the registry with every cell the log's `SetFormula` ops name, so a
    /// value write to one of them has somewhere to leave its stamp *before* the
    /// formula op is applied — which is what makes arrival order irrelevant.
    pub fn seeded(cells: BTreeSet<CellKey>) -> Self {
        FormulaRegistry {
            entries: cells
                .into_iter()
                .map(|k| {
                    (
                        k,
                        Entry {
                            stamp: FLOOR,
                            formula: None,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Applies a `SetFormula`. Wins only if its stamp is the greatest seen at
    /// this cell.
    ///
    /// A formula op for a cell the seed did not name cannot happen when the
    /// registry was built from the same op set. If it does — a caller merged a
    /// log the pre-pass never saw — the entry is created rather than dropped:
    /// losing a user's formula is a worse failure than a missing stamp, and the
    /// `debug_assert` names the real fault (DP-A10: never panic across a
    /// boundary, but never hide the defect either).
    pub fn set_formula(
        &mut self,
        row: RowId,
        col: ColId,
        stamp: (Lamport, OpId),
        cell: FormulaCell,
    ) {
        let key = (row.0, col.0);
        match self.entries.get_mut(&key) {
            Some(entry) => {
                if stamp > entry.stamp {
                    entry.stamp = stamp;
                    entry.formula = Some(cell);
                }
            }
            None => {
                debug_assert!(
                    false,
                    "formula written to a cell the replay pre-pass did not seed"
                );
                self.entries.insert(
                    key,
                    Entry {
                        stamp,
                        formula: Some(cell),
                    },
                );
            }
        }
    }

    /// Applies a `SetCell`/`ClearCell`. A value write shadows the formula only
    /// when it is the later write; an *earlier* one loses and leaves the formula
    /// standing, which is the case full-replay order used to make unreachable.
    ///
    /// Cells the seed did not name are ignored on purpose: no formula op names
    /// them anywhere in the log, so no comparison can ever be asked of them and
    /// a stamp would be pure cost (see the module note).
    pub fn note_value_write(&mut self, row: RowId, col: ColId, stamp: (Lamport, OpId)) {
        if let Some(entry) = self.entries.get_mut(&(row.0, col.0)) {
            if stamp > entry.stamp {
                entry.stamp = stamp;
                entry.formula = None;
            }
        }
    }

    /// The winning formula at a cell, if a formula is the winning content.
    pub fn get(&self, row: RowId, col: ColId) -> Option<&FormulaCell> {
        self.entries.get(&(row.0, col.0))?.formula.as_ref()
    }

    /// Every *winning* formula, in identity order — the calc engine's source of
    /// truth for what to evaluate, and the order the state hash folds in.
    /// Shadowed entries are skipped: they are bookkeeping, not content.
    pub fn iter(&self) -> impl Iterator<Item = (RowId, ColId, &FormulaCell)> {
        self.entries
            .iter()
            .filter_map(|((r, c), e)| e.formula.as_ref().map(|f| (RowId(*r), ColId(*c), f)))
    }

    /// True when no cell's winning content is a formula. The state hash adds a
    /// formulas section only when this is false, so every pre-Row-9 corpus —
    /// and every workbook whose formulas were all overwritten — hashes exactly
    /// as it did before formulas existed.
    pub fn has_no_formulas(&self) -> bool {
        self.entries.values().all(|e| e.formula.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use usk_types::ActorId;

    fn cell(n: u64) -> (RowId, ColId) {
        let id = |c| OpId {
            actor: ActorId(0),
            counter: c,
        };
        (RowId(id(n)), ColId(id(n + 100)))
    }

    fn stamp(lamport: Lamport, actor: u128, counter: u64) -> (Lamport, OpId) {
        (
            lamport,
            OpId {
                actor: ActorId(actor),
                counter,
            },
        )
    }

    fn formula(src: &str) -> FormulaCell {
        FormulaCell {
            source: String::from(src),
            bindings: Vec::new(),
        }
    }

    /// The write that arrives *second* is not automatically the winner — the
    /// one with the greater stamp is. This is the whole of TD-22.
    #[test]
    fn an_earlier_value_write_does_not_shadow_a_later_formula() {
        let (r, c) = cell(1);
        let mut reg = FormulaRegistry::seeded([(r.0, c.0)].into_iter().collect());
        reg.set_formula(r, c, stamp(9, 1, 1), formula("=A1+1"));
        reg.note_value_write(r, c, stamp(4, 2, 1));
        assert_eq!(reg.get(r, c).map(|f| f.source.as_str()), Some("=A1+1"));
    }

    #[test]
    fn a_later_value_write_shadows_the_formula() {
        let (r, c) = cell(1);
        let mut reg = FormulaRegistry::seeded([(r.0, c.0)].into_iter().collect());
        reg.set_formula(r, c, stamp(4, 1, 1), formula("=A1+1"));
        reg.note_value_write(r, c, stamp(9, 2, 1));
        assert_eq!(reg.get(r, c), None);
        assert!(reg.has_no_formulas());
    }

    #[test]
    fn ties_break_on_op_id_so_two_replicas_cannot_disagree() {
        let (r, c) = cell(1);
        // Same lamport, different actors: the total order still decides.
        let mut a = FormulaRegistry::seeded([(r.0, c.0)].into_iter().collect());
        a.set_formula(r, c, stamp(7, 1, 1), formula("=SUM(A1:A2)"));
        a.note_value_write(r, c, stamp(7, 2, 1));
        let mut b = FormulaRegistry::seeded([(r.0, c.0)].into_iter().collect());
        b.note_value_write(r, c, stamp(7, 2, 1));
        b.set_formula(r, c, stamp(7, 1, 1), formula("=SUM(A1:A2)"));
        assert_eq!(a.get(r, c), None, "actor 2 wins the lamport tie");
        assert_eq!(b.get(r, c), None);
    }

    /// The property the sync path depends on: the registry is a function of the
    /// op *set*, never of arrival order. Proven over every permutation, not a
    /// sample — the history is small enough that exhaustive is affordable.
    #[test]
    fn registry_is_order_independent() {
        #[derive(Clone)]
        enum W {
            Value((Lamport, OpId)),
            Formula((Lamport, OpId), &'static str),
        }
        let (r, c) = cell(1);
        let writes = [
            W::Formula(stamp(2, 1, 1), "=A1"),
            W::Value(stamp(5, 2, 1)),
            W::Formula(stamp(5, 1, 2), "=B1"),
            W::Value(stamp(3, 3, 1)),
            W::Formula(stamp(1, 2, 2), "=C1"),
        ];

        let run = |order: &[usize]| {
            let mut reg = FormulaRegistry::seeded([(r.0, c.0)].into_iter().collect());
            for &i in order {
                match &writes[i] {
                    W::Value(s) => reg.note_value_write(r, c, *s),
                    W::Formula(s, src) => reg.set_formula(r, c, *s, formula(src)),
                }
            }
            reg.get(r, c).map(|f| f.source.clone())
        };

        // Canonical order's answer: greatest stamp is (5, actor 2) — a value.
        let expected = run(&[0, 1, 2, 3, 4]);
        assert_eq!(expected, None);

        let mut order = [0usize, 1, 2, 3, 4];
        let mut permutations = 0usize;
        permute(&mut order, 0, &mut |o| {
            permutations += 1;
            assert_eq!(run(o), expected, "arrival order {o:?} disagreed");
        });
        assert_eq!(permutations, 120, "all 5! orders were exercised");
    }

    /// Idempotence: a relay redelivering an op must not change the answer.
    #[test]
    fn reapplying_the_same_write_changes_nothing() {
        let (r, c) = cell(1);
        let mut reg = FormulaRegistry::seeded([(r.0, c.0)].into_iter().collect());
        reg.set_formula(r, c, stamp(3, 1, 1), formula("=A1"));
        let before = reg.get(r, c).cloned();
        for _ in 0..3 {
            reg.set_formula(r, c, stamp(3, 1, 1), formula("=A1"));
            reg.note_value_write(r, c, stamp(2, 1, 2));
        }
        assert_eq!(reg.get(r, c).cloned(), before);
    }

    /// Cells no `SetFormula` names are deliberately untracked — the memory
    /// argument in the module note, made executable.
    #[test]
    fn unseeded_cells_hold_no_entry() {
        let (r, c) = cell(1);
        let mut reg = FormulaRegistry::seeded(BTreeSet::new());
        reg.note_value_write(r, c, stamp(4, 1, 1));
        assert_eq!(reg.get(r, c), None);
        assert_eq!(reg.iter().count(), 0);
        assert!(reg.has_no_formulas());
    }

    fn permute<F: FnMut(&[usize])>(items: &mut [usize], k: usize, visit: &mut F) {
        if k == items.len() {
            visit(items);
            return;
        }
        for i in k..items.len() {
            items.swap(k, i);
            permute(items, k + 1, visit);
            items.swap(k, i);
        }
    }
}
