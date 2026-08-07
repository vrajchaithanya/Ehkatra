//! Row 4 proofs: the tile store (docs/14, ADR-005, ADR-034).
//!
//! The load-bearing test is `tile_store_matches_reference_semantics`: it runs a
//! randomized multi-actor corpus through both the tile store and an
//! independently written flat reference, and demands they agree cell for cell
//! and loser for loser. Everything else here pins one specific mechanism.

use helpers::*;
use usk_oplog::{Anchor, Op, OpLog};
use usk_state::tile::{TILE_COLS, TILE_ROWS};
use usk_state::State;
use usk_types::{ActorId, CellError, ColId, ErrorKind, OpId, Origin, RowId, Value};

mod helpers {
    use std::collections::BTreeMap;
    use usk_oplog::{Anchor, Op, Payload};
    use usk_types::{ActorId, ColId, OpId, RowId, Value};

    pub fn opid(actor: u128, counter: u64) -> OpId {
        OpId {
            actor: ActorId(actor),
            counter,
        }
    }

    pub fn insert_row(actor: u128, counter: u64, lamport: u64, anchor: Anchor) -> Op {
        Op {
            id: opid(actor, counter),
            lamport,
            payload: Payload::InsertRow { anchor },
        }
    }

    pub fn insert_col(actor: u128, counter: u64, lamport: u64, anchor: Anchor) -> Op {
        Op {
            id: opid(actor, counter),
            lamport,
            payload: Payload::InsertCol { anchor },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_cell(
        actor: u128,
        counter: u64,
        lamport: u64,
        row: OpId,
        col: OpId,
        value: Value,
    ) -> Op {
        Op {
            id: opid(actor, counter),
            lamport,
            payload: Payload::SetCell {
                row: RowId(row),
                col: ColId(col),
                value,
            },
        }
    }

    /// One recorded write: `(lamport, op id, value)`.
    type Write = (u64, OpId, Value);

    /// A deliberately naive flat model of the documented semantics, written
    /// independently of the tile store so it can contradict it.
    ///
    /// Rules it encodes directly from ADR-006 / docs/10:
    ///
    /// * winner = the write with the greatest `(lamport, actor, counter)`
    /// * losers = for every *other* actor, that actor's own greatest write
    ///   (an actor's earlier writes are superseded by its later ones, because
    ///   one actor's writes are totally ordered and never concurrent)
    #[derive(Default)]
    pub struct Reference {
        writes: BTreeMap<(OpId, OpId), Vec<Write>>,
    }

    impl Reference {
        pub fn apply(&mut self, op: &Op) {
            let (row, col, value) = match &op.payload {
                Payload::SetCell { row, col, value } => (row.0, col.0, value.clone()),
                Payload::ClearCell { row, col } => (row.0, col.0, Value::Blank),
                _ => return,
            };
            self.writes
                .entry((row, col))
                .or_default()
                .push((op.lamport, op.id, value));
        }

        fn candidates(&self, key: &(OpId, OpId)) -> Vec<Write> {
            let mut per_actor: BTreeMap<ActorId, Write> = BTreeMap::new();
            for (l, id, v) in self.writes.get(key).into_iter().flatten() {
                let slot = per_actor.entry(id.actor).or_insert((*l, *id, v.clone()));
                if (*l, *id) >= (slot.0, slot.1) {
                    *slot = (*l, *id, v.clone());
                }
            }
            let mut out: Vec<_> = per_actor.into_values().collect();
            out.sort_by_key(|(l, id, _)| (*l, *id));
            out
        }

        pub fn cell(&self, row: OpId, col: OpId) -> Option<Value> {
            self.candidates(&(row, col)).pop().map(|(_, _, v)| v)
        }

        pub fn losers(&self, row: OpId, col: OpId) -> Vec<Value> {
            let mut c = self.candidates(&(row, col));
            c.pop();
            c.into_iter().map(|(_, _, v)| v).collect()
        }

        pub fn cells(&self) -> impl Iterator<Item = &(OpId, OpId)> {
            self.writes.keys()
        }
    }
}

/// Deterministic LCG — the tests must not depend on ambient randomness (DP-A2).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// Builds a corpus of `rows`×`cols` cells written by `actors` actors, with
/// deliberate same-cell collisions so both loser paths get exercised.
fn corpus(rows: usize, cols: usize, actors: u128, seed: u64) -> (OpLog, Vec<OpId>, Vec<OpId>) {
    let mut log = OpLog::new();
    let mut rng = Lcg(seed);
    let mut row_ids = Vec::new();
    let mut col_ids = Vec::new();
    let mut lamport = 1u64;

    for i in 0..rows {
        let anchor = row_ids
            .last()
            .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
        let op = insert_row(1, 1000 + i as u64, lamport, anchor);
        lamport += 1;
        row_ids.push(op.id);
        log.append(op);
    }
    for i in 0..cols {
        let anchor = col_ids
            .last()
            .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
        let op = insert_col(1, 5000 + i as u64, lamport, anchor);
        lamport += 1;
        col_ids.push(op.id);
        log.append(op);
    }

    let mut counter = 0u64;
    for r in &row_ids {
        for c in &col_ids {
            // Each cell is written 1-3 times, by varying actors, so some cells
            // are single-author and some are contested.
            let writes = 1 + (rng.next() % 3);
            for _ in 0..writes {
                counter += 1;
                let actor = 1 + (rng.next() as u128 % actors);
                let value = match rng.next() % 5 {
                    0 => Value::Text(format!("t{counter}")),
                    1 => Value::Bool(counter.is_multiple_of(2)),
                    2 => Value::Error(CellError::new(ErrorKind::Value, Origin::Authored)),
                    _ => Value::Number((rng.next() % 10_000) as f64 / 4.0),
                };
                log.append(set_cell(actor, counter, lamport, *r, *c, value));
                lamport += 1;
            }
        }
    }
    (log, row_ids, col_ids)
}

/// THE Row-4 test: the tile store and an independent flat model of the
/// documented CRDT semantics agree on every cell and every retained loser.
#[test]
fn tile_store_matches_reference_semantics() {
    let (log, _, _) = corpus(9, 9, 3, 0x51ED_C0DE_1234_5678);
    let state = State::replay(&log);

    let mut reference = Reference::default();
    let mut sorted: Vec<&Op> = log.ops().iter().collect();
    sorted.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
    for op in sorted {
        reference.apply(op);
    }

    let mut checked = 0;
    for (row, col) in reference.cells() {
        assert_eq!(
            state.cell(RowId(*row), ColId(*col)),
            reference.cell(*row, *col),
            "winner differs at {row:?}/{col:?}"
        );
        let got: Vec<Value> = state
            .conflicts(RowId(*row), ColId(*col))
            .iter()
            .map(|(_, _, v)| v.clone())
            .collect();
        assert_eq!(
            got,
            reference.losers(*row, *col),
            "retained losers differ at {row:?}/{col:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 81, "corpus should cover every cell");
}

/// Convergence still holds through the tile store: the same op set delivered in
/// two different orders yields one state hash (DP-A2, DP-A8).
#[test]
fn tiled_state_converges_under_reordering() {
    let (log, _, _) = corpus(6, 6, 3, 0xC0FFEE);
    let forward = State::replay(&log);

    let mut reversed = OpLog::new();
    for op in log.ops().iter().rev() {
        reversed.append(op.clone());
    }
    let backward = State::replay(&reversed);

    assert_eq!(forward.state_hash(), backward.state_hash());
    assert_eq!(forward.tile_count(), backward.tile_count());
    assert_eq!(forward.promotion_stats(), backward.promotion_stats());
}

/// A single author's region stays on the 24-byte causal summary: no per-cell
/// metadata, and no false conflicts from their own edit history (ADR-005).
#[test]
fn single_author_region_never_promotes() {
    let (log, rows, cols) = corpus(4, 4, 1, 0xABCD);
    let state = State::replay(&log);
    let stats = state.promotion_stats();

    assert_eq!(
        stats.promoted_tiles, 0,
        "one actor cannot conflict with self"
    );
    assert_eq!(stats.promoted_cells, 0);
    assert_eq!(stats.promoted_cell_fraction(), 0.0);
    // Overwriting your own cell is sequential, not concurrent — nothing retained.
    assert!(state.conflicts(RowId(rows[0]), ColId(cols[0])).is_empty());
}

/// Promotion tracks *contested cells*, not merely co-located authors: two
/// people working in the same region on different cells is the ordinary case
/// and must stay on the cheap summary path (ADR-005).
#[test]
fn co_located_authors_do_not_promote() {
    let r = insert_row(1, 1, 1, Anchor::Start);
    let c1 = insert_col(1, 2, 2, Anchor::Start);
    let c2 = insert_col(1, 3, 3, Anchor::After(c1.id));

    let mut log = OpLog::new();
    for op in [&r, &c1, &c2] {
        log.append(op.clone());
    }
    log.append(set_cell(1, 10, 10, r.id, c1.id, Value::Number(1.0)));
    log.append(set_cell(2, 11, 11, r.id, c2.id, Value::Number(2.0)));

    let state = State::replay(&log);
    assert_eq!(state.tile_count(), 1, "both cells share one tile");
    assert_eq!(
        state.promotion_stats().promoted_tiles,
        0,
        "different cells are not a conflict, however close together they are"
    );
    assert!(state.conflicts(RowId(r.id), ColId(c1.id)).is_empty());
}

/// **TD-09's proof.** A cell written by two actors is promoted — and *only*
/// that cell. Its neighbours in the same tile stay on the summary path.
///
/// Before TD-09 this test asserted the opposite ("one contested cell promotes
/// every cell in its tile"), which is exactly the amplification that failed
/// A-002: 0.1% contested cells became 100% promoted cells.
#[test]
fn contested_cell_is_promoted_alone() {
    let r = insert_row(1, 1, 1, Anchor::Start);
    let c1 = insert_col(1, 2, 2, Anchor::Start);
    let c2 = insert_col(1, 3, 3, Anchor::After(c1.id));

    let mut log = OpLog::new();
    for op in [&r, &c1, &c2] {
        log.append(op.clone());
    }
    // Actor 1 owns both cells: summarised, no per-cell metadata.
    log.append(set_cell(1, 10, 10, r.id, c1.id, Value::Number(1.0)));
    log.append(set_cell(1, 12, 12, r.id, c2.id, Value::Number(3.0)));
    let single = State::replay(&log);
    assert_eq!(single.promotion_stats().promoted_tiles, 0);
    assert!(single
        .cell_summary(RowId(r.id), ColId(c1.id))
        .is_some_and(|(lamport, actor)| lamport == 12 && actor == ActorId(1)));

    // Actor 2 writes a cell actor 1 already wrote — that is the conflict.
    log.append(set_cell(2, 11, 14, r.id, c1.id, Value::Number(2.0)));
    let contested = State::replay(&log);
    let stats = contested.promotion_stats();
    assert_eq!(stats.promoted_tiles, 1, "the tile holds the stamps");
    assert_eq!(
        stats.promoted_cells, 1,
        "exactly the contested cell is promoted — TD-09"
    );
    assert_eq!(stats.promoted_cell_fraction(), 0.5, "one cell of the two");
    assert!(
        contested.is_cell_promoted(RowId(r.id), ColId(c1.id)),
        "the contested cell carries a stamp"
    );
    assert!(
        !contested.is_cell_promoted(RowId(r.id), ColId(c2.id)),
        "its uncontested neighbour does not"
    );
    // A tile keeps its causal frontier even while holding stamps: anti-entropy
    // diffs on the frontier (docs/15), and a few stamped cells do not remove it.
    assert!(contested
        .cell_summary(RowId(r.id), ColId(c1.id))
        .is_some_and(|(lamport, _)| lamport == 14));
    // The contested cell resolves by (lamport, actor) and retains the loser.
    assert_eq!(
        contested.cell(RowId(r.id), ColId(c1.id)),
        Some(Value::Number(2.0))
    );
    let losers = contested.conflicts(RowId(r.id), ColId(c1.id));
    assert_eq!(losers.len(), 1);
    assert_eq!(losers[0].2, Value::Number(1.0));
    // The uncontested neighbour keeps its value and its summary path.
    assert_eq!(
        contested.cell(RowId(r.id), ColId(c2.id)),
        Some(Value::Number(3.0))
    );
}

/// Identity, not position: inserting a row above existing data must not move
/// any existing cell to a different tile (DP-A6, ADR-034).
#[test]
fn inserting_a_row_never_rekeys_existing_tiles() {
    let r1 = insert_row(1, 1, 1, Anchor::Start);
    let c1 = insert_col(1, 2, 2, Anchor::Start);
    let w = set_cell(1, 3, 3, r1.id, c1.id, Value::Number(7.0));

    let mut log = OpLog::new();
    for op in [&r1, &c1, &w] {
        log.append(op.clone());
    }
    let before = State::replay(&log);
    let before_tiles = before.tile_count();

    // A new row inserted *above* r1: in A1 terms everything below shifts down.
    log.append(insert_row(2, 1, 9, Anchor::Start));
    let after = State::replay(&log);

    assert_eq!(after.row_order().len(), 2);
    assert_eq!(after.row_order()[0].0, opid(2, 1), "new row renders first");
    assert_eq!(
        after.cell(RowId(r1.id), ColId(c1.id)),
        Some(Value::Number(7.0)),
        "the value travels with its identity"
    );
    assert_eq!(
        after.tile_count(),
        before_tiles,
        "no new tile: the existing cell kept its slot"
    );
}

/// A tile stays on the packed f64 path until a non-number lands in it, and the
/// packed path costs materially less per cell (docs/14).
#[test]
fn numeric_tiles_pack_tighter_than_mixed_tiles() {
    let build = |mixed: bool| {
        let r = insert_row(1, 1, 1, Anchor::Start);
        let mut log = OpLog::new();
        log.append(r.clone());
        let mut cols = Vec::new();
        for i in 0..60u64 {
            let anchor = cols
                .last()
                .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
            let op = insert_col(1, 100 + i, 2 + i, anchor);
            cols.push(op.id);
            log.append(op);
        }
        for (i, c) in cols.iter().enumerate() {
            let value = if mixed && i == 0 {
                Value::Text(String::from("x"))
            } else {
                Value::Number(i as f64)
            };
            log.append(set_cell(
                1,
                1000 + i as u64,
                200 + i as u64,
                r.id,
                *c,
                value,
            ));
        }
        State::replay(&log)
    };

    let packed = build(false);
    let tagged = build(true);
    assert_eq!(packed.tile_count(), 1);
    assert_eq!(tagged.tile_count(), 1);
    assert!(
        tagged.cell_heap_bytes() > packed.cell_heap_bytes(),
        "one text cell widens the whole tile: packed={} tagged={}",
        packed.cell_heap_bytes(),
        tagged.cell_heap_bytes()
    );
    // The widened tile keeps every value it already held.
    assert_eq!(
        tagged.cell(RowId(opid(1, 1)), ColId(opid(1, 159))),
        Some(Value::Number(59.0))
    );
}

/// Cells land in the tile their slots dictate, and a band boundary starts a new
/// tile — the property that makes a viewport fetch touch a bounded number of
/// tiles.
#[test]
fn cells_group_into_256x64_tiles() {
    let mut log = OpLog::new();
    let mut lamport = 1u64;
    let mut rows = Vec::new();
    // One row band boundary (256) and one col band boundary (64).
    for i in 0..(TILE_ROWS + 1) as u64 {
        let anchor = rows
            .last()
            .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
        let op = insert_row(1, i, lamport, anchor);
        lamport += 1;
        rows.push(op.id);
        log.append(op);
    }
    let mut cols = Vec::new();
    for i in 0..(TILE_COLS + 1) as u64 {
        let anchor = cols
            .last()
            .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
        let op = insert_col(1, 100_000 + i, lamport, anchor);
        lamport += 1;
        cols.push(op.id);
        log.append(op);
    }

    let corners = [
        (0usize, 0usize),
        (0, TILE_COLS as usize),
        (TILE_ROWS as usize, 0),
        (TILE_ROWS as usize, TILE_COLS as usize),
    ];
    for (n, (r, c)) in corners.iter().enumerate() {
        log.append(set_cell(
            1,
            900_000 + n as u64,
            lamport + n as u64,
            rows[*r],
            cols[*c],
            Value::Number(n as f64),
        ));
    }

    let state = State::replay(&log);
    assert_eq!(
        state.tile_count(),
        4,
        "four cells straddling both band boundaries occupy four distinct tiles"
    );
    for (n, (r, c)) in corners.iter().enumerate() {
        assert_eq!(
            state.cell(RowId(rows[*r]), ColId(cols[*c])),
            Some(Value::Number(n as f64))
        );
    }
}

/// The presence bitmap and the packed payload stay in step when writes arrive
/// out of index order — the case the append fast path does *not* cover.
#[test]
fn out_of_order_writes_keep_the_payload_dense() {
    let mut log = OpLog::new();
    let r = insert_row(1, 1, 1, Anchor::Start);
    log.append(r.clone());
    let mut cols = Vec::new();
    for i in 0..40u64 {
        let anchor = cols
            .last()
            .map_or(Anchor::Start, |id: &OpId| Anchor::After(*id));
        let op = insert_col(1, 10 + i, 2 + i, anchor);
        cols.push(op.id);
        log.append(op);
    }
    // Write columns in a scattered order (descending odds, then ascending evens).
    let order: Vec<usize> = (0..40usize)
        .rev()
        .filter(|i| !i.is_multiple_of(2))
        .chain((0..40usize).filter(|i| i.is_multiple_of(2)))
        .collect();
    for (n, i) in order.iter().enumerate() {
        log.append(set_cell(
            1,
            500 + n as u64,
            100 + n as u64,
            r.id,
            cols[*i],
            Value::Number(*i as f64),
        ));
    }

    let state = State::replay(&log);
    for (i, c) in cols.iter().enumerate() {
        assert_eq!(
            state.cell(RowId(r.id), ColId(*c)),
            Some(Value::Number(i as f64)),
            "cell {i} landed at the wrong payload rank"
        );
    }
}
