# 11 — Workbook Model & Undo
Status: Approved · Owner: Principal Engineer · Normative: yes · Carved from SPEC §6, §13, §18

## Grid model
Unbounded virtual grid; Excel bounds (1,048,576 × 16,384) are a per-workbook compatibility viewport (`grid_bounds: Excel | Unbounded`, default Excel for round-trip safety — desktop-first users live in XLSX). Rows/cols are identities; A1/R1C1 are views computed through a per-axis order-statistic tree (O(log n) id↔ordinal↔pixel, shared by render, parse, calc). References are identity intervals with `AnchorMode`; Excel's insert/delete/shift semantics fall out structurally; endpoint deletion re-anchors inward; empty interval → `#REF!`; 3-D refs are intervals over sheet order; copy/fill rewrites anchors in the reducer.

## Feature objects (model → projection → ops pattern)
Tables (column map; structured refs survive reorder; calculated columns auto-extend), names, validation rules (formula groups producing booleans; reject-severity enforced at reducer), conditional formatting (formula groups producing style deltas; scales use incremental range statistics), styles/themes (interned flyweights; theme-referenced colors), comments (threaded rich-text, @-mentions), merged regions (anchor + span; destructive merges return would-be-lost values in the error), freeze/split (view state), sort (identity permutation op — references travel, recorded with collation descriptor), filter (per-view state; opt-in shared mode).

## Undo architecture
Selective undo: inverse synthesized against **current** state. Text/sequence → tombstone exact identities; registers → restore only if own write still wins (else no-op — others' intent preserved); structural → blocked-and-narrowed when it would destroy others' ops, with explicit user notice. Groups are range-compressed (a 100k paste stores id-interval runs). Scopes: per-user × per-workbook × per-window session; **agent sessions are first-class scopes** ("undo everything the agent did" = one action). Stack durable across restarts, bounded 200 groups/30 days, hand-off to version history is visible in UI. Redo = undo-of-undo; everything lives in the op log.
