# 14 — Storage Engine
Status: Approved · Owner: Principal Engineer · Normative: yes · Carved from SPEC §7

## Tile store
Unit of storage/sync/cache/render-fetch: tile = 256 rows × 64 cols, keyed `(SheetId, RowBand, ColBand)` in identity space. Layout: compressed presence bitmap; homogeneous packed payloads (`f64 | Decimal | StrId | Bool`) when type-uniform, tagged union otherwise — columnar behavior is emergent, never assumed. Formulas reference the FormulaGroup table; formats are style-run refs; each tile carries a BLAKE3 node.

## CRDT metadata (the feasibility decision — ADR-005)
Per-tile causal summary (~24 B) by default; **promotion** to per-cell metadata only on true concurrency. Budget (10M numeric cells): values 80 MB + bitmaps 1.2 MB + summaries ~15 KB ≈ **81 MB** vs 201 MB naive. Assumption A-002 (promotion <1% under real collaboration) is Q1-measured and permanently telemetered — a rising fleet promotion rate is the early warning that tile granularity needs revisiting.

## Tiered memory
T0 hot decoded (LRU-with-pin: viewport, dirty, calc closure) → T1 warm compressed in memory (~10×) → T2 cold in PAL BlockStore (desktop: single-file store, see below) → T3 remote (server doc store). Eviction against host-negotiated `MemoryBudget {soft, hard}`; deterministic shed order under pressure; full rebuild from T2 + op tail after process kill.

## Desktop file strategy (new for desktop-first)
A workbook on disk is a **single-file container** (SQLite database: ops, snapshots, blobs, index — chosen for crash-safety evidence, single-file user mental model, and 20-year archaeology; ADR-031). `.xlsx` remains an *interchange* format at import/export boundaries — never the working store (fidelity loss and no op history). File association, iCloud/OneDrive-folder tolerance (advisory locking + safe-copy semantics on sync-managed folders), and atomic save via SQLite WAL are 33/34 concerns.

## Caching law
Every cache platform-wide is a fold over the op log with a recorded watermark; invalidation = dirty-interval intersection. No independently mutable cache state exists. This single rule is why aggressive caching and correctness coexist.

## Interning
Strings: per-workbook interner, refcount GC at compaction. Styles: interned flyweights. Tombstones: collected at compaction past the 180-day watermark (docs/10 rules).
