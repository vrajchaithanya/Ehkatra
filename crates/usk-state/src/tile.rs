//! Tile store — the cell storage substrate (docs/14 §Tile store, ADR-005).
//!
//! A tile is 256 rows × 64 cols of *identity space*, holding a presence bitmap,
//! a payload packed dense over present cells (`f64` when the tile is type-uniform,
//! a tagged union otherwise), and CRDT metadata that is a ~24-byte causal summary
//! by default and only **promotes** to per-cell metadata where concurrency
//! actually occurred. That promotion rule is the whole feasibility argument for
//! ADR-005: it is what turns 201 MB of naive per-cell CRDT metadata into ~81 MB
//! for a 10M-cell numeric workbook (assumption A-001/A-002, docs/42).
//!
//! # Why slots exist (ADR-034)
//! Rows and columns are permanent identities, not integers (DP-A6), but a tile
//! key needs an ordinal. Each identity is interned to a **slot** the first time
//! replay observes it, in the canonical total order `(lamport, actor, counter)`.
//! Slots are stable for the life of an identity: inserting a row never re-keys
//! an existing tile. Slots are a *storage layout* concern only — no hash, op, or
//! convergence property depends on the value of a slot, only on its determinism.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::borrow::Borrow;
use core::mem::size_of;
use usk_oplog::{Op, Payload};
use usk_types::{ActorId, Lamport, OpId, Value};

/// Rows per tile (docs/14; frozen by ADR-005 as part of tile granularity).
pub const TILE_ROWS: u32 = 256;
/// Columns per tile (docs/14; frozen by ADR-005).
pub const TILE_COLS: u32 = 64;
/// Cells per tile.
pub const TILE_CELLS: usize = (TILE_ROWS * TILE_COLS) as usize;

const PRESENCE_WORDS: usize = TILE_CELLS / 64;

/// Interns axis identities to stable, densely-packed slots (ADR-034).
#[derive(Default, Clone)]
pub struct SlotMap {
    to_slot: BTreeMap<OpId, u32>,
    to_id: Vec<OpId>,
}

impl SlotMap {
    /// Returns the identity's slot, assigning the next free one if unseen.
    /// Idempotent, so the pre-pass and the apply pass agree by construction.
    pub fn intern(&mut self, id: OpId) -> u32 {
        if let Some(s) = self.to_slot.get(&id) {
            return *s;
        }
        let slot = self.to_id.len() as u32;
        self.to_slot.insert(id, slot);
        self.to_id.push(id);
        slot
    }

    pub fn slot_of(&self, id: &OpId) -> Option<u32> {
        self.to_slot.get(id).copied()
    }

    /// Reverse lookup — needed because hashing and iteration must speak in
    /// identities, never in slots (DP-A6).
    fn id_of(&self, slot: u32) -> OpId {
        self.to_id[slot as usize]
    }

    fn heap_bytes(&self) -> usize {
        // BTreeMap node overhead is not observable from `alloc`; the entry
        // payload is counted honestly and the caller labels the number as
        // structural (MEASUREMENTS.md says so explicitly).
        self.to_slot.len() * (size_of::<OpId>() + size_of::<u32>())
            + self.to_id.capacity() * size_of::<OpId>()
    }
}

/// Address of a tile in identity space: `(row_slot / 256, col_slot / 64)`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TileKey {
    pub row_band: u32,
    pub col_band: u32,
}

impl TileKey {
    fn of(row_slot: u32, col_slot: u32) -> Self {
        TileKey {
            row_band: row_slot / TILE_ROWS,
            col_band: col_slot / TILE_COLS,
        }
    }
}

/// Index of a cell inside its tile, row-major. Always < `TILE_CELLS`.
fn cell_index(row_slot: u32, col_slot: u32) -> u16 {
    ((row_slot % TILE_ROWS) * TILE_COLS + (col_slot % TILE_COLS)) as u16
}

/// Compressed presence bitmap: one bit per cell, 2 KiB per tile (docs/14's
/// budget line "bitmaps 1.2 MB" for a 10M-cell workbook assumes exactly this).
#[derive(Clone)]
struct Presence {
    words: [u64; PRESENCE_WORDS],
    count: u32,
    /// Highest set index, enabling the O(1) append path that dominates bulk
    /// loads (import, fill-down); without it every insert costs a rank scan.
    max_set: Option<u16>,
}

impl Default for Presence {
    fn default() -> Self {
        Presence {
            words: [0; PRESENCE_WORDS],
            count: 0,
            max_set: None,
        }
    }
}

impl Presence {
    fn contains(&self, idx: u16) -> bool {
        self.words[idx as usize / 64] & (1u64 << (idx % 64)) != 0
    }

    /// Number of present cells strictly before `idx` — the cell's position in
    /// the dense payload.
    fn rank(&self, idx: u16) -> usize {
        let w = idx as usize / 64;
        let mut n = 0usize;
        for word in &self.words[..w] {
            n += word.count_ones() as usize;
        }
        let mask = (1u64 << (idx % 64)) - 1;
        n + (self.words[w] & mask).count_ones() as usize
    }

    /// Marks `idx` present. Returns `true` if it was newly inserted.
    fn set(&mut self, idx: u16) -> bool {
        let bit = 1u64 << (idx % 64);
        let word = &mut self.words[idx as usize / 64];
        if *word & bit != 0 {
            return false;
        }
        *word |= bit;
        self.count += 1;
        self.max_set = Some(match self.max_set {
            Some(m) if m >= idx => m,
            _ => idx,
        });
        true
    }

    fn indices(&self) -> impl Iterator<Item = u16> + '_ {
        self.words.iter().enumerate().flat_map(|(w, word)| {
            let mut bits = *word;
            core::iter::from_fn(move || {
                if bits == 0 {
                    None
                } else {
                    let b = bits.trailing_zeros();
                    bits &= bits - 1;
                    Some((w * 64 + b as usize) as u16)
                }
            })
        })
    }
}

/// Tile payload, dense over *present* cells only (docs/14: "homogeneous packed
/// payloads when type-uniform, tagged union otherwise — columnar behavior is
/// emergent, never assumed").
#[derive(Clone)]
enum CellPack {
    /// The numeric fast path: 8 bytes per cell, no tag, no indirection.
    Numbers(Vec<f64>),
    /// Mixed-type tile: 32 bytes per cell.
    Tagged(Vec<Value>),
}

impl CellPack {
    fn len(&self) -> usize {
        match self {
            CellPack::Numbers(v) => v.len(),
            CellPack::Tagged(v) => v.len(),
        }
    }

    fn get(&self, rank: usize) -> Value {
        match self {
            CellPack::Numbers(v) => Value::Number(v[rank]),
            CellPack::Tagged(v) => v[rank].clone(),
        }
    }

    /// Widens a packed numeric tile to a tagged one. One-way in v0.1: a tile
    /// that has ever held a non-number stays tagged, because narrowing back
    /// would need a full scan on every write for no measured benefit.
    fn widen(&mut self) {
        if let CellPack::Numbers(v) = self {
            let mut tagged = Vec::with_capacity(v.capacity());
            for n in v.iter() {
                tagged.push(Value::Number(*n));
            }
            *self = CellPack::Tagged(tagged);
        }
    }

    fn insert(&mut self, rank: usize, value: Value) {
        if let (CellPack::Numbers(v), Value::Number(n)) = (&mut *self, &value) {
            v.insert(rank, *n);
            return;
        }
        self.widen();
        match self {
            CellPack::Tagged(v) => v.insert(rank, value),
            // Unreachable: `widen` guarantees Tagged, and the Numbers/Number
            // pair returned above. Written as a no-op rather than a panic
            // because the kernel never panics across a boundary (DP-A10).
            CellPack::Numbers(_) => {}
        }
    }

    fn replace(&mut self, rank: usize, value: Value) {
        if let (CellPack::Numbers(v), Value::Number(n)) = (&mut *self, &value) {
            v[rank] = *n;
            return;
        }
        self.widen();
        match self {
            CellPack::Tagged(v) => v[rank] = value,
            CellPack::Numbers(_) => {}
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            CellPack::Numbers(v) => v.capacity() * size_of::<f64>(),
            CellPack::Tagged(v) => {
                let mut n = v.capacity() * size_of::<Value>();
                // Interned strings arrive with compaction (docs/14 §Interning);
                // until then text bytes live in the cell and must be counted.
                for value in v {
                    if let Value::Text(s) = value {
                        n += s.capacity();
                    }
                }
                n
            }
        }
    }
}

/// Reduces a cell's retained losers to **one value per competing actor**, and
/// drops any from the winner's own actor.
///
/// ADR-006 retains *concurrent* alternatives so a collaborator's work is never
/// silently discarded. An author's own earlier edit is not concurrent with
/// their later one — a single actor's writes are totally ordered by its
/// counter — so surfacing it would be a false conflict, and would also make
/// `conflicts()` depend on whether the tile happened to be promoted (a summary
/// tile, being single-writer, retains nothing). This keeps the two paths
/// semantically identical.
///
/// The result is a pure function of the op set, not of arrival order: for each
/// losing actor it keeps that actor's greatest `(lamport, id)` write.
fn retain_concurrent_losers(losers: &mut Vec<(Lamport, OpId, Value)>, winner: ActorId) {
    losers.sort_by_key(|(l, id, _)| (id.actor, *l, *id));
    let mut kept: Vec<(Lamport, OpId, Value)> = Vec::with_capacity(losers.len());
    for entry in losers.drain(..) {
        if entry.1.actor == winner {
            continue;
        }
        match kept.last_mut() {
            Some(prev) if prev.1.actor == entry.1.actor => *prev = entry,
            _ => kept.push(entry),
        }
    }
    kept.sort_by_key(|(l, id, _)| (*l, *id));
    *losers = kept;
}

/// Per-cell CRDT metadata — only ever allocated inside a *promoted* tile.
#[derive(Clone)]
struct CellMeta {
    lamport: Lamport,
    id: OpId,
    /// Concurrent writes that lost. Retained, never discarded (ADR-006, DP-A8).
    losers: Vec<(Lamport, OpId, Value)>,
}

/// Tile CRDT metadata (ADR-005). `Summary` is the 24-byte common case.
#[derive(Clone)]
enum Meta {
    /// No *cell* in this tile was written by more than one actor. Each cell
    /// therefore has a single author, whose own writes are totally ordered by
    /// its counter and so are never concurrent: the newest write to a cell is
    /// its winner by construction, there is nothing to stamp per cell, and no
    /// loser can exist. 24 bytes for the whole tile.
    ///
    /// The two fields are the tile's causal *frontier* — greatest lamport
    /// applied and the actor that applied it — which is the unit anti-entropy
    /// diffs at Row 10. They are not an ownership claim: several authors may
    /// share a summarised tile, as long as they stay off each other's cells.
    Summary {
        max_lamport: Lamport,
        writer: ActorId,
    },
    /// Some cell in this tile was written by two or more actors, so ordering
    /// must be decided per cell and losers must be retained.
    Promoted(BTreeMap<u16, CellMeta>),
}

#[derive(Clone)]
struct Tile {
    presence: Presence,
    payload: CellPack,
    meta: Meta,
}

impl Tile {
    fn new(promoted: bool, first_is_number: bool) -> Self {
        Tile {
            presence: Presence::default(),
            payload: if first_is_number {
                CellPack::Numbers(Vec::new())
            } else {
                CellPack::Tagged(Vec::new())
            },
            meta: if promoted {
                Meta::Promoted(BTreeMap::new())
            } else {
                Meta::Summary {
                    max_lamport: 0,
                    writer: ActorId(0),
                }
            },
        }
    }

    /// The tile's core structural invariant: the payload is dense over exactly
    /// the present cells, so `rank` is a valid index into it.
    fn invariant_holds(&self) -> bool {
        self.payload.len() == self.presence.count as usize
    }

    fn heap_bytes(&self) -> usize {
        let meta = match &self.meta {
            Meta::Summary { .. } => 0,
            Meta::Promoted(m) => m
                .values()
                .map(|c| {
                    size_of::<u16>()
                        + size_of::<CellMeta>()
                        + c.losers.capacity() * size_of::<(Lamport, OpId, Value)>()
                })
                .sum(),
        };
        size_of::<Tile>() + self.payload.heap_bytes() + meta
    }
}

/// Where a tile's metadata sits, for the A-002 promotion harness.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromotionStats {
    pub tiles: usize,
    pub promoted_tiles: usize,
    pub cells: usize,
    pub promoted_cells: usize,
}

impl PromotionStats {
    /// Fraction of live cells whose tile carries per-cell CRDT metadata.
    /// A-002 claims this stays under 1% under realistic multi-author load.
    pub fn promoted_cell_fraction(&self) -> f64 {
        if self.cells == 0 {
            0.0
        } else {
            self.promoted_cells as f64 / self.cells as f64
        }
    }
}

/// The cell store: tiles keyed in identity space, plus the slot interners.
#[derive(Default, Clone)]
pub struct TileStore {
    pub rows: SlotMap,
    pub cols: SlotMap,
    tiles: BTreeMap<TileKey, Box<Tile>>,
    /// Tiles the pre-pass proved to have more than one writer. Fixed before the
    /// first write lands, which is what makes promotion lossless — see
    /// `plan_promotions`.
    promoted: BTreeSet<TileKey>,
}

/// Result of the replay pre-pass: the slot assignment and the set of tiles that
/// must start promoted.
pub struct Plan {
    pub rows: SlotMap,
    pub cols: SlotMap,
    pub promoted: BTreeSet<TileKey>,
}

impl TileStore {
    /// Builds an empty store that will honour `plan`'s promotion decisions.
    pub fn from_plan(plan: Plan) -> Self {
        TileStore {
            rows: plan.rows,
            cols: plan.cols,
            tiles: BTreeMap::new(),
            promoted: plan.promoted,
        }
    }

    /// Applies one cell write.
    ///
    /// Contract: the store was built from a `Plan` computed over the same op
    /// set, so a tile is already promoted whenever it will ever see a second
    /// writer. This is what lets `Summary` tiles carry no per-cell stamps
    /// without ever having to reconstruct stamps they never recorded.
    pub fn write(&mut self, row: OpId, col: OpId, lamport: Lamport, id: OpId, value: Value) {
        let row_slot = self.rows.intern(row);
        let col_slot = self.cols.intern(col);
        let key = TileKey::of(row_slot, col_slot);
        let idx = cell_index(row_slot, col_slot);
        let promoted = self.promoted.contains(&key);
        let tile = self
            .tiles
            .entry(key)
            .or_insert_with(|| Box::new(Tile::new(promoted, matches!(value, Value::Number(_)))));

        let present = tile.presence.contains(idx);
        // Append fast path: bulk loads walk row-major, so the new index is
        // almost always beyond every present one and rank == count.
        let rank = if !present && tile.presence.max_set.is_some_and(|m| idx > m) {
            tile.presence.count as usize
        } else {
            tile.presence.rank(idx)
        };

        match &mut tile.meta {
            Meta::Summary {
                max_lamport,
                writer,
            } => {
                // A summary tile overwrites without comparing stamps, which is
                // only sound because (a) `plan_promotions` proved no cell here
                // is contested, and (b) writes arrive in canonical order, so
                // the newest write to a cell is always the last one applied.
                // (b) is the half that is cheap to check, so check it.
                debug_assert!(
                    lamport >= *max_lamport,
                    "summary tile saw an out-of-order write; replay must feed \
                     ops in (lamport, actor, counter) order"
                );
                *writer = id.actor;
                *max_lamport = lamport;
                if present {
                    tile.payload.replace(rank, value);
                } else {
                    tile.presence.set(idx);
                    tile.payload.insert(rank, value);
                }
            }
            Meta::Promoted(metas) => match metas.get_mut(&idx) {
                Some(meta) if present => {
                    let winner_actor = if (lamport, id) > (meta.lamport, meta.id) {
                        let old = tile.payload.get(rank);
                        meta.losers.push((meta.lamport, meta.id, old));
                        meta.lamport = lamport;
                        meta.id = id;
                        tile.payload.replace(rank, value);
                        id.actor
                    } else {
                        meta.losers.push((lamport, id, value));
                        meta.id.actor
                    };
                    retain_concurrent_losers(&mut meta.losers, winner_actor);
                }
                _ => {
                    tile.presence.set(idx);
                    tile.payload.insert(rank, value);
                    metas.insert(
                        idx,
                        CellMeta {
                            lamport,
                            id,
                            losers: Vec::new(),
                        },
                    );
                }
            },
        }
        debug_assert!(
            tile.invariant_holds(),
            "payload is no longer dense over the presence bitmap"
        );
    }

    pub fn get(&self, row: &OpId, col: &OpId) -> Option<Value> {
        let (tile, idx) = self.locate(row, col)?;
        if !tile.presence.contains(idx) {
            return None;
        }
        Some(tile.payload.get(tile.presence.rank(idx)))
    }

    /// Retained concurrent losers (ADR-006). A summary tile has none by
    /// construction — its writes all came from one totally-ordered actor.
    pub fn losers(&self, row: &OpId, col: &OpId) -> &[(Lamport, OpId, Value)] {
        match self.locate(row, col) {
            Some((tile, idx)) => match &tile.meta {
                Meta::Promoted(m) => m.get(&idx).map(|c| c.losers.as_slice()).unwrap_or(&[]),
                Meta::Summary { .. } => &[],
            },
            None => &[],
        }
    }

    /// The tile's ~24-byte causal summary `(max lamport, sole writer)`, or
    /// `None` if the tile is promoted. This is the unit anti-entropy will diff
    /// on at Row 10 (docs/15) and the reason a summary tile needs no per-cell
    /// metadata at all.
    pub fn causal_summary(&self, row: &OpId, col: &OpId) -> Option<(Lamport, ActorId)> {
        match self.locate(row, col)?.0.meta {
            Meta::Summary {
                max_lamport,
                writer,
            } => Some((max_lamport, writer)),
            Meta::Promoted(_) => None,
        }
    }

    fn locate(&self, row: &OpId, col: &OpId) -> Option<(&Tile, u16)> {
        let row_slot = self.rows.slot_of(row)?;
        let col_slot = self.cols.slot_of(col)?;
        let tile = self.tiles.get(&TileKey::of(row_slot, col_slot))?;
        Some((tile.as_ref(), cell_index(row_slot, col_slot)))
    }

    /// Visits every present cell as `(row identity, col identity, value)` in
    /// tile-major order. This is the order the state hash folds in, which is
    /// the tile-Merkle direction docs/10 specifies.
    pub fn for_each<F: FnMut(OpId, OpId, &Value)>(&self, mut f: F) {
        for (key, tile) in &self.tiles {
            for idx in tile.presence.indices() {
                let row_slot = key.row_band * TILE_ROWS + (idx as u32) / TILE_COLS;
                let col_slot = key.col_band * TILE_COLS + (idx as u32) % TILE_COLS;
                let value = tile.payload.get(tile.presence.rank(idx));
                f(self.rows.id_of(row_slot), self.cols.id_of(col_slot), &value);
            }
        }
    }

    /// Promotion accounting for assumption A-002 (docs/42).
    pub fn promotion_stats(&self) -> PromotionStats {
        let mut s = PromotionStats::default();
        for (key, tile) in &self.tiles {
            let n = tile.presence.count as usize;
            s.tiles += 1;
            s.cells += n;
            if self.promoted.contains(key) {
                s.promoted_tiles += 1;
                s.promoted_cells += n;
            }
        }
        s
    }

    /// Structural heap bytes held by the store. Excludes allocator bookkeeping
    /// and `BTreeMap` node overhead, which `alloc` does not expose — the
    /// harness cross-checks this against OS peak working set so the gap is
    /// visible rather than assumed (DP-B1).
    pub fn heap_bytes(&self) -> usize {
        let tiles: usize = self
            .tiles
            .values()
            .map(|t| size_of::<TileKey>() + size_of::<Box<Tile>>() + t.heap_bytes())
            .sum();
        tiles
            + self.rows.heap_bytes()
            + self.cols.heap_bytes()
            + self.promoted.len() * size_of::<TileKey>()
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
}

/// Per-tile write tracking for the promotion pre-pass: one presence bitmap per
/// actor that wrote into the tile. 2 KiB per (tile, actor), transient.
///
/// A coarser "did two actors touch this tile" test was tried first and measured
/// useless: a tile is 16,384 cells, so *any* two collaborators on a sheet
/// promote essentially everything (measured 100% on every multi-author pattern,
/// MEASUREMENTS.md). Per-actor bitmaps cost a little transient memory and buy
/// the ability to ask the question that actually matters — did two actors write
/// **the same cell**.
#[derive(Default)]
struct TileWriters {
    by_actor: Vec<(ActorId, [u64; PRESENCE_WORDS])>,
}

impl TileWriters {
    fn record(&mut self, actor: ActorId, idx: u16) {
        let bit = 1u64 << (idx % 64);
        let word = idx as usize / 64;
        for (a, map) in self.by_actor.iter_mut() {
            if *a == actor {
                map[word] |= bit;
                return;
            }
        }
        let mut map = [0u64; PRESENCE_WORDS];
        map[word] |= bit;
        self.by_actor.push((actor, map));
    }

    /// True if any single cell was written by two different actors.
    fn has_contested_cell(&self) -> bool {
        for (i, (_, a)) in self.by_actor.iter().enumerate() {
            for (_, b) in &self.by_actor[i + 1..] {
                if a.iter().zip(b.iter()).any(|(x, y)| x & y != 0) {
                    return true;
                }
            }
        }
        false
    }
}

/// Pre-pass over the canonically ordered ops: assigns slots and decides which
/// tiles must start promoted.
///
/// Promotion predicate: **a tile containing a cell written by two or more
/// distinct actors is promoted.** This still over-approximates true concurrency
/// — two actors writing one cell at causally ordered times are not concurrent —
/// and it over-approximates deliberately, in the safe direction: a promoted tile
/// is merely larger, whereas a wrongly-summarised tile would silently drop a
/// concurrent loser and violate ADR-006. Narrowing to *true* concurrency needs
/// the causal `deps` delta docs/10 specifies for `Op`, which v0.1 does not yet
/// carry; tracked as debt in docs/44.
///
/// Note the amplification this predicate carries and A-002 does not account
/// for: one contested cell promotes its whole 16,384-cell tile. See
/// MEASUREMENTS.md for what that does to scattered contention.
pub fn plan_promotions<B: Borrow<Op>, I: Iterator<Item = B>>(ops: I) -> Plan {
    let mut rows = SlotMap::default();
    let mut cols = SlotMap::default();
    let mut writers: BTreeMap<TileKey, TileWriters> = BTreeMap::new();

    for op in ops {
        let op = op.borrow();
        match &op.payload {
            // Structural ops intern here, which is what makes slot order follow
            // creation order — and that is what gives tiles their locality.
            Payload::InsertRow { .. } => {
                rows.intern(op.id);
            }
            Payload::InsertCol { .. } => {
                cols.intern(op.id);
            }
            Payload::SetCell { row, col, .. } | Payload::ClearCell { row, col } => {
                let (r, c) = (rows.intern(row.0), cols.intern(col.0));
                writers
                    .entry(TileKey::of(r, c))
                    .or_default()
                    .record(op.id.actor, cell_index(r, c));
            }
            Payload::DeleteRow { .. } | Payload::DeleteCol { .. } => {}
        }
    }

    let promoted = writers
        .into_iter()
        .filter(|(_, w)| w.has_contested_cell())
        .map(|(k, _)| k)
        .collect();
    Plan {
        rows,
        cols,
        promoted,
    }
}
