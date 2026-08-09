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
use usk_types::{ActorId, Decimal, Lamport, OpId, Value};

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
    /// The currency fast path: 18 bytes of payload per cell (padded to 32),
    /// still far below the tagged union and, unlike it, exact.
    Decimals(Vec<Decimal>),
    /// Mixed-type tile.
    Tagged(Vec<Value>),
}

impl CellPack {
    /// The packed variant this value can live in, if any.
    fn kind_of(value: &Value) -> Option<Kind> {
        match value {
            Value::Number(_) => Some(Kind::Numbers),
            Value::Decimal(_) => Some(Kind::Decimals),
            _ => None,
        }
    }

    fn empty(kind: Kind) -> CellPack {
        match kind {
            Kind::Numbers => CellPack::Numbers(Vec::new()),
            Kind::Decimals => CellPack::Decimals(Vec::new()),
            Kind::Tagged => CellPack::Tagged(Vec::new()),
        }
    }

    fn len(&self) -> usize {
        match self {
            CellPack::Numbers(v) => v.len(),
            CellPack::Decimals(v) => v.len(),
            CellPack::Tagged(v) => v.len(),
        }
    }

    fn get(&self, rank: usize) -> Value {
        match self {
            CellPack::Numbers(v) => Value::Number(v[rank]),
            CellPack::Decimals(v) => Value::Decimal(v[rank]),
            CellPack::Tagged(v) => v[rank].clone(),
        }
    }

    /// Widens a packed tile to a tagged one. One-way in v0.1: a tile that has
    /// ever held a foreign type stays tagged, because narrowing back would need
    /// a full scan on every write for no measured benefit.
    ///
    /// Note a `Number` landing in a `Decimals` tile widens it rather than
    /// converting: `f64` → decimal is only exact for some values, and a storage
    /// layer must never make that judgement silently. `Profile::to_decimal`
    /// is where that decision belongs, in front of the store.
    fn widen(&mut self) {
        let tagged: Vec<Value> = match self {
            CellPack::Tagged(_) => return,
            CellPack::Numbers(v) => v.iter().map(|n| Value::Number(*n)).collect(),
            CellPack::Decimals(v) => v.iter().map(|d| Value::Decimal(*d)).collect(),
        };
        *self = CellPack::Tagged(tagged);
    }

    fn insert(&mut self, rank: usize, value: Value) {
        match (&mut *self, &value) {
            (CellPack::Numbers(v), Value::Number(n)) => return v.insert(rank, *n),
            (CellPack::Decimals(v), Value::Decimal(d)) => return v.insert(rank, *d),
            _ => {}
        }
        self.widen();
        // `widen` guarantees Tagged; doing nothing otherwise is deliberate,
        // because the kernel never panics across a boundary (DP-A10).
        if let CellPack::Tagged(v) = self {
            v.insert(rank, value);
        }
    }

    fn replace(&mut self, rank: usize, value: Value) {
        match (&mut *self, &value) {
            (CellPack::Numbers(v), Value::Number(n)) => {
                v[rank] = *n;
                return;
            }
            (CellPack::Decimals(v), Value::Decimal(d)) => {
                v[rank] = *d;
                return;
            }
            _ => {}
        }
        self.widen();
        if let CellPack::Tagged(v) = self {
            v[rank] = value;
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            CellPack::Numbers(v) => v.capacity() * size_of::<f64>(),
            CellPack::Decimals(v) => v.capacity() * size_of::<Decimal>(),
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

/// Which packed layout a tile is using.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Numbers,
    Decimals,
    Tagged,
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
    /// Some cells in this tile are contested. **Only those cells** carry
    /// stamps; the rest stay on the summary path, exactly as if they were in a
    /// wholly uncontested tile.
    ///
    /// This is TD-09's fix. Promoting the whole tile was measured turning 0.1%
    /// contested cells into 100% promoted cells and 8.4 → 74.5 B/cell, because
    /// a tile is 16,384 cells and one conflict condemned all of them. The unit
    /// of promotion is now the cell; the tile is only where the stamps live.
    Mixed {
        max_lamport: Lamport,
        writer: ActorId,
        stamps: BTreeMap<u16, CellMeta>,
    },
}

#[derive(Clone)]
struct Tile {
    presence: Presence,
    payload: CellPack,
    meta: Meta,
}

impl Tile {
    /// A tile adopts the packed layout of its first value, so a numeric or
    /// currency column never pays the tagged-union price.
    fn new(promoted: bool, kind: Kind) -> Self {
        Tile {
            presence: Presence::default(),
            payload: CellPack::empty(kind),
            meta: if promoted {
                Meta::Mixed {
                    max_lamport: 0,
                    writer: ActorId(0),
                    stamps: BTreeMap::new(),
                }
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
            Meta::Mixed { stamps: m, .. } => m
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
    /// Per tile, a bitmap of the cell indices the pre-pass proved contested.
    /// Fixed before the first write lands, which is what makes promotion
    /// lossless — see `plan_promotions`.
    contested: BTreeMap<TileKey, [u64; PRESENCE_WORDS]>,
}

/// Result of the replay pre-pass.
///
/// One traversal answers every question that must be settled *before the first
/// write lands*: which slot each identity gets, which cells are contested (so
/// their tile starts stamped), and which cells a formula ever names (so the
/// formula registry can stamp value writes there — TD-22, see `formula.rs`).
/// They share a pass because they share that timing constraint, and because a
/// second traversal of a 10M-cell import is not free.
pub struct Plan {
    pub rows: SlotMap,
    pub cols: SlotMap,
    /// Contested cell indices per tile.
    pub contested: BTreeMap<TileKey, [u64; PRESENCE_WORDS]>,
    /// Cells named by any `SetFormula` op — the formula registry's seed set.
    pub formula_cells: BTreeSet<(OpId, OpId)>,
}

impl TileStore {
    /// Builds an empty store that will honour `plan`'s promotion decisions.
    pub fn from_plan(plan: Plan) -> Self {
        TileStore {
            rows: plan.rows,
            cols: plan.cols,
            tiles: BTreeMap::new(),
            contested: plan.contested,
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
        let tile_contested = self.contested.get(&key);
        let promoted = tile_contested.is_some();
        // Per-cell routing: a contested cell takes the stamped path, its
        // neighbours in the same tile do not.
        let cell_contested =
            tile_contested.is_some_and(|bm| bm[idx as usize / 64] & (1u64 << (idx % 64)) != 0);
        let tile = self.tiles.entry(key).or_insert_with(|| {
            Box::new(Tile::new(
                promoted,
                CellPack::kind_of(&value).unwrap_or(Kind::Tagged),
            ))
        });

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
            Meta::Mixed {
                max_lamport,
                writer,
                stamps,
            } if !cell_contested => {
                // An uncontested cell inside a mixed tile: single writer,
                // canonical arrival order, so the newest write wins with no
                // stamp — identical to the pure-summary path.
                debug_assert!(
                    lamport >= *max_lamport,
                    "mixed tile saw an out-of-order write; replay must feed \
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
            Meta::Mixed {
                max_lamport,
                writer,
                stamps: metas,
            } => {
                // The frontier covers every write to the tile, contested or
                // not: anti-entropy diffs on it (docs/15), so a stamped write
                // that did not advance it would make the tile look stale.
                if lamport >= *max_lamport {
                    *max_lamport = lamport;
                    *writer = id.actor;
                }
                match metas.get_mut(&idx) {
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
                }
            }
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
                Meta::Mixed { stamps, .. } => {
                    stamps.get(&idx).map(|c| c.losers.as_slice()).unwrap_or(&[])
                }
                Meta::Summary { .. } => &[],
            },
            None => &[],
        }
    }

    /// The tile's ~24-byte causal frontier `(max lamport, latest writer)`.
    ///
    /// Present for every tile now, including tiles holding contested cells:
    /// the frontier is what anti-entropy diffs on at Row 10 (docs/15), and a
    /// tile does not stop having one just because a few of its cells carry
    /// stamps.
    pub fn causal_summary(&self, row: &OpId, col: &OpId) -> Option<(Lamport, ActorId)> {
        match self.locate(row, col)?.0.meta {
            Meta::Summary {
                max_lamport,
                writer,
            }
            | Meta::Mixed {
                max_lamport,
                writer,
                ..
            } => Some((max_lamport, writer)),
        }
    }

    /// Whether this specific cell carries per-cell CRDT metadata.
    pub fn is_cell_promoted(&self, row: &OpId, col: &OpId) -> bool {
        let Some(row_slot) = self.rows.slot_of(row) else {
            return false;
        };
        let Some(col_slot) = self.cols.slot_of(col) else {
            return false;
        };
        let idx = cell_index(row_slot, col_slot);
        self.contested
            .get(&TileKey::of(row_slot, col_slot))
            .is_some_and(|bm| bm[idx as usize / 64] & (1u64 << (idx % 64)) != 0)
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
            s.tiles += 1;
            s.cells += tile.presence.count as usize;
            if let Some(bitmap) = self.contested.get(key) {
                s.promoted_tiles += 1;
                // Only the cells that are actually present AND contested carry
                // metadata — the number A-002 is about.
                for (w, word) in bitmap.iter().enumerate() {
                    let present = tile.presence.words[w] & *word;
                    s.promoted_cells += present.count_ones() as usize;
                }
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
            + self.contested.len() * (size_of::<TileKey>() + PRESENCE_WORDS * 8)
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

    /// Bitmap of the cells written by two or more different actors, or `None`
    /// when the tile has no contested cell at all.
    ///
    /// The pairwise AND is the whole point: a cell is contested only if two
    /// actors both wrote *it*, not if two actors both wrote somewhere in the
    /// tile. Returning the map rather than a boolean is what lets promotion be
    /// per cell (TD-09).
    fn contested_cells(&self) -> Option<[u64; PRESENCE_WORDS]> {
        let mut out = [0u64; PRESENCE_WORDS];
        let mut any = false;
        for (i, (_, a)) in self.by_actor.iter().enumerate() {
            for (_, b) in &self.by_actor[i + 1..] {
                for w in 0..PRESENCE_WORDS {
                    let both = a[w] & b[w];
                    if both != 0 {
                        out[w] |= both;
                        any = true;
                    }
                }
            }
        }
        if any {
            Some(out)
        } else {
            None
        }
    }
}

/// Pre-pass over the canonically ordered ops: assigns slots and decides which
/// tiles must start promoted.
///
/// Promotion predicate: **a cell written by two or more distinct actors is
/// promoted, and only that cell.** This still over-approximates true concurrency
/// — two actors writing one cell at causally ordered times are not concurrent —
/// and it over-approximates deliberately, in the safe direction: a promoted tile
/// is merely larger, whereas a wrongly-summarised tile would silently drop a
/// concurrent loser and violate ADR-006. Narrowing to *true* concurrency needs
/// the causal `deps` delta docs/10 specifies for `Op`, which v0.1 does not yet
/// carry; tracked as debt in docs/44.
///
/// TD-09 removed the amplification this predicate used to carry: promotion was
/// per *tile*, so one contested cell condemned 16,384 of them. Promoted cells
/// now equal contested cells exactly — an amplification factor of 1, which is
/// the floor for any implementation that must retain a concurrent loser.
pub fn plan_promotions<B: Borrow<Op>, I: Iterator<Item = B>>(ops: I) -> Plan {
    let mut rows = SlotMap::default();
    let mut cols = SlotMap::default();
    let mut writers: BTreeMap<TileKey, TileWriters> = BTreeMap::new();
    let mut formula_cells: BTreeSet<(OpId, OpId)> = BTreeSet::new();

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
            // Formulas live in the flat registry, not in tiles, so they claim
            // no slot and contest no cell — but the registry must know which
            // cells they name before any value write reaches one (TD-22).
            Payload::SetFormula { row, col, .. } => {
                formula_cells.insert((row.0, col.0));
            }
            // Undeletes touch axis order only. An opaque op (DP-A5) applies to
            // nothing, so it interns no identity and contests no cell — a
            // preserved op must not be able to promote a tile.
            Payload::DeleteRow { .. }
            | Payload::DeleteCol { .. }
            | Payload::UndeleteRow { .. }
            | Payload::UndeleteCol { .. }
            | Payload::Opaque(_) => {}
        }
    }

    let contested = writers
        .into_iter()
        .filter_map(|(k, w)| w.contested_cells().map(|bm| (k, bm)))
        .collect();
    Plan {
        rows,
        cols,
        contested,
        formula_cells,
    }
}

// ---------------------------------------------------------------- tile image
//
// The serialised form of a tile (docs/16's "tile image"). It lives here rather
// than in `image.rs` because it needs the private layout, and because a tile's
// bytes and a tile's meaning should be defined in one place — the invariant
// `payload.len() == presence.count` is enforced on read, so an image cannot
// produce a tile whose `rank` would index past its payload.

use crate::image::{ImageError, Reader, Writer, MAX_LOSERS_PER_CELL};
// --------------------------------------------------- tile store (crate-internal)

impl TileStore {
    pub(crate) fn write_image(&self, w: &mut Writer) {
        // Slot maps: the `to_id` vector is the whole truth, `to_slot` is its
        // inverse and is rebuilt rather than stored.
        w.len(self.rows.ids().len());
        for id in self.rows.ids() {
            w.opid(id);
        }
        w.len(self.cols.ids().len());
        for id in self.cols.ids() {
            w.opid(id);
        }

        w.len(self.tiles.len());
        for (key, tile) in &self.tiles {
            let start = w.out.len();
            w.u32(key.row_band);
            w.u32(key.col_band);
            tile.write_image(w);
            // The contested bitmap belongs to the tile's chunk: it is fixed
            // before the first write lands and is part of what the tile means.
            match self.contested.get(key) {
                None => w.u8(0),
                Some(bits) => {
                    w.u8(1);
                    for word in bits {
                        w.u64(*word);
                    }
                }
            }
            w.tiles.push((*key, start, w.out.len()));
        }
    }

    pub(crate) fn read_image(r: &mut Reader) -> Result<TileStore, ImageError> {
        let mut store = TileStore::default();
        let n = r.count(crate::image::MAX_AXIS_ENTRIES, "row slots")?;
        for _ in 0..n {
            store.rows.intern(r.opid()?);
        }
        let n = r.count(crate::image::MAX_AXIS_ENTRIES, "col slots")?;
        for _ in 0..n {
            store.cols.intern(r.opid()?);
        }

        let n = r.count(crate::image::MAX_TILES, "tiles")?;
        for _ in 0..n {
            let key = TileKey {
                row_band: r.u32()?,
                col_band: r.u32()?,
            };
            let tile = Tile::read_image(r)?;
            match r.u8()? {
                0 => {}
                1 => {
                    let mut bits = [0u64; PRESENCE_WORDS];
                    for word in bits.iter_mut() {
                        *word = r.u64()?;
                    }
                    store.contested.insert(key, bits);
                }
                _ => return Err(ImageError::Malformed("contested tag")),
            }
            store.tiles.insert(key, Box::new(tile));
        }

        // **Every present cell must name a slot the slot maps actually have.**
        // Without this a corrupted image produces a tile whose band points past
        // the slot map, and the first read of it indexes out of bounds — which
        // is precisely what the image fuzz test found on its first run. The
        // check is O(present cells), the same order as having read them.
        // Arithmetic in `u64`: a corrupted band is an arbitrary `u32`, and
        // `band * TILE_ROWS` overflows in a debug build long before it produces
        // a wrong answer. The fuzz test found that too, one fix later.
        let (rows, cols) = (store.rows.ids().len() as u64, store.cols.ids().len() as u64);
        for (key, tile) in &store.tiles {
            for idx in tile.presence.indices() {
                let row_slot =
                    key.row_band as u64 * TILE_ROWS as u64 + idx as u64 / TILE_COLS as u64;
                let col_slot =
                    key.col_band as u64 * TILE_COLS as u64 + idx as u64 % TILE_COLS as u64;
                if row_slot >= rows || col_slot >= cols {
                    return Err(ImageError::Malformed(
                        "a tile holds a cell outside the slot maps",
                    ));
                }
            }
        }
        Ok(store)
    }
}

impl Presence {
    /// Rebuilds `count` and `max_set` from the bitmap. They are **derived, not
    /// stored**, so an image cannot disagree with itself about which cells
    /// exist — and that agreement is what makes the dense payload's `rank`
    /// valid.
    pub(crate) fn from_words(words: [u64; PRESENCE_WORDS]) -> Presence {
        let mut count = 0u32;
        let mut max_set = None;
        for (w, word) in words.iter().enumerate() {
            if *word != 0 {
                count += word.count_ones();
                let highest = 63 - word.leading_zeros();
                max_set = Some((w * 64 + highest as usize) as u16);
            }
        }
        Presence {
            words,
            count,
            max_set,
        }
    }

    pub(crate) fn write_image(&self, w: &mut Writer) {
        for word in &self.words {
            w.u64(*word);
        }
    }

    pub(crate) fn read_image(r: &mut Reader) -> Result<Presence, ImageError> {
        let mut words = [0u64; PRESENCE_WORDS];
        for word in words.iter_mut() {
            *word = r.u64()?;
        }
        Ok(Presence::from_words(words))
    }
}

impl SlotMap {
    /// The identities in slot order — the whole truth of a slot map, since
    /// `to_slot` is its inverse and is rebuilt by re-interning.
    pub(crate) fn ids(&self) -> &[OpId] {
        &self.to_id
    }
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Kind::Numbers => 0,
            Kind::Decimals => 1,
            Kind::Tagged => 2,
        }
    }

    fn of_tag(tag: u8) -> Result<Kind, ImageError> {
        Ok(match tag {
            0 => Kind::Numbers,
            1 => Kind::Decimals,
            2 => Kind::Tagged,
            _ => return Err(ImageError::Malformed("cell pack kind")),
        })
    }
}

impl CellPack {
    fn kind(&self) -> Kind {
        match self {
            CellPack::Numbers(_) => Kind::Numbers,
            CellPack::Decimals(_) => Kind::Decimals,
            CellPack::Tagged(_) => Kind::Tagged,
        }
    }

    fn write_image(&self, w: &mut Writer) {
        w.u8(self.kind().tag());
        match self {
            // The numeric fast path stays a run of f64 bits — no tags, no
            // lengths. This is where the image's size advantage over the op set
            // comes from: 8 bytes per cell against an op's 24-byte identity
            // plus its payload.
            CellPack::Numbers(v) => {
                w.len(v.len());
                for n in v {
                    w.f64(*n);
                }
            }
            CellPack::Decimals(v) => {
                w.len(v.len());
                for d in v {
                    w.i128(d.coefficient());
                    w.i16(d.exponent());
                }
            }
            CellPack::Tagged(v) => {
                w.len(v.len());
                for value in v {
                    w.value(value);
                }
            }
        }
    }

    fn read_image(r: &mut Reader) -> Result<CellPack, ImageError> {
        let kind = Kind::of_tag(r.u8()?)?;
        let n = r.count(TILE_CELLS, "packed cells")?;
        Ok(match kind {
            Kind::Numbers => {
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(f64::from_bits(r.u64()?));
                }
                CellPack::Numbers(v)
            }
            Kind::Decimals => {
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let coefficient = r.i128()?;
                    let exponent = r.i16()?;
                    v.push(Decimal::new(coefficient, exponent));
                }
                CellPack::Decimals(v)
            }
            Kind::Tagged => {
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(r.value()?);
                }
                CellPack::Tagged(v)
            }
        })
    }
}

impl Tile {
    pub(crate) fn write_image(&self, w: &mut Writer) {
        self.presence.write_image(w);
        self.payload.write_image(w);
        match &self.meta {
            Meta::Summary {
                max_lamport,
                writer,
            } => {
                w.u8(0);
                w.u64(*max_lamport);
                w.out.extend_from_slice(&writer.0.to_le_bytes());
            }
            Meta::Mixed {
                max_lamport,
                writer,
                stamps,
            } => {
                w.u8(1);
                w.u64(*max_lamport);
                w.out.extend_from_slice(&writer.0.to_le_bytes());
                w.len(stamps.len());
                for (idx, cell) in stamps {
                    w.u16(*idx);
                    w.u64(cell.lamport);
                    w.opid(&cell.id);
                    // Retained losers are content, not bookkeeping (ADR-006,
                    // DP-A8): dropping them here would make an image a lossy
                    // projection of the state it claims to be.
                    w.len(cell.losers.len());
                    for (lamport, id, value) in &cell.losers {
                        w.u64(*lamport);
                        w.opid(id);
                        w.value(value);
                    }
                }
            }
        }
    }

    pub(crate) fn read_image(r: &mut Reader) -> Result<Tile, ImageError> {
        let presence = Presence::read_image(r)?;
        let payload = CellPack::read_image(r)?;
        let meta = match r.u8()? {
            0 => Meta::Summary {
                max_lamport: r.u64()?,
                writer: read_actor(r)?,
            },
            1 => {
                let max_lamport = r.u64()?;
                let writer = read_actor(r)?;
                let n = r.count(TILE_CELLS, "stamps")?;
                let mut stamps = BTreeMap::new();
                for _ in 0..n {
                    let idx = r.u16()?;
                    let lamport = r.u64()?;
                    let id = r.opid()?;
                    let losers_len = r.count(MAX_LOSERS_PER_CELL, "losers")?;
                    let mut losers = Vec::with_capacity(losers_len.min(64));
                    for _ in 0..losers_len {
                        losers.push((r.u64()?, r.opid()?, r.value()?));
                    }
                    stamps.insert(
                        idx,
                        CellMeta {
                            lamport,
                            id,
                            losers,
                        },
                    );
                }
                Meta::Mixed {
                    max_lamport,
                    writer,
                    stamps,
                }
            }
            _ => return Err(ImageError::Malformed("tile meta tag")),
        };
        let tile = Tile {
            presence,
            payload,
            meta,
        };
        // The tile's core structural invariant, checked on the way in rather
        // than trusted: a payload that is not dense over exactly the present
        // cells would make every later `rank` index the wrong value, silently.
        if !tile.invariant_holds() {
            return Err(ImageError::Malformed(
                "tile payload is not dense over its presence bitmap",
            ));
        }
        Ok(tile)
    }
}

fn read_actor(r: &mut Reader) -> Result<ActorId, ImageError> {
    let mut a = [0u8; 16];
    for byte in a.iter_mut() {
        *byte = r.u8()?;
    }
    Ok(ActorId(u128::from_le_bytes(a)))
}
