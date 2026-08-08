# BOOTSTRAP.md — Ehkatra Build Order & MVP

## What Ehkatra is (one paragraph)
A web-first spreadsheet platform whose kernel treats every change as an operation: one op log feeds state, real-time CRDT collaboration, offline-first sync, per-user selective undo, version history, audit, and an AI/MCP surface where every agent edit is previewed, attributed, and reversible as a unit. Excel compatibility is a measured profile; AI-natives and enterprises are the wedge.

## The 10 differentiators (vs Excel / Google Sheets)
1. **Agent-native, human-sovereign** — semantic MCP tools (`describe → query → preview_edits → apply_edits → undo`); agents never dump grids into context; every agent session is one-click reversible. Neither incumbent has this.
2. **Preview-before-apply with impact reports** — "this changes 4,213 downstream cells, introduces 2 #REF!" *before* mutation; blast-radius policies enforced at the host, not by etiquette.
3. **Conflict honesty** — CRDT co-editing that never silently discards a concurrent edit (retain-losers surfacing); Excel and Sheets both discard.
4. **Offline-first web** — full function disconnected, 180-day window, never-drop merge contract.
5. **Selective per-user undo + branches** — undo your own work without clobbering collaborators; branch/merge workbooks like code.
6. **Error provenance** — every `#VALUE!` answers "where did you come from"; Excel cannot.
7. **Exact currency math** — `Decimal128` alongside f64; strict mode ends silent coercion (the gene-name bug class) while `compat` profile keeps Excel's quirks for imported files.
8. **One vocabulary: UI ≡ REST ≡ MCP** — API parity guaranteed by construction, not by catalog chasing (Graph API covers a fraction of Excel; ours covers everything, always).
9. **Op-level audit** — hash-chained, actor-attributed, cell-granular; "who changed this and why" is a query.
10. **Structural security** — formulas have no ambient network (kills WEBSERVICE-class exfiltration); parsers sandboxed; macros quarantined, never executed.

## MVP (v0.1 walking skeleton — this is what THIS code session builds)
| # | Deliverable | Proof |
|---|---|---|
| 1 | Cargo workspace: `usk-types`, `usk-oplog`, `usk-state`, `usk-formula`, `usk-calc`, `usk-reduce`, `ehkatra-cli` | builds, `no_std` check green |
| 2 | Op log: canonical CBOR, causal order, BLAKE3 Merkle state hash | encode/decode round-trip tests; hash stability test |
| 3 | Order CRDT (rows/cols): insert/delete/tombstones, A1-as-view via order-statistic index | seeded-LCG sweep: convergence over randomized interleavings (D-052) |
| 4 | Tile store: 256×64, presence bitmap, packed f64/tagged payloads, per-tile causal summary + promotion | memory harness reports bytes/cell into `MEASUREMENTS.md` |
| 5 | Values: Blank/Bool/Number/Decimal128/Text/Error(+origin) ; compat/strict coercion | unit vectors incl. Excel-quirk cases |
| 6 | Formula engine: lexer→Pratt parser→CST→AST→binder; **60 functions** (arith, logical, text, date core, SUM/AVERAGE/COUNT/MIN/MAX/IF/AND/OR/NOT/CONCAT/LEFT/RIGHT/MID/LEN/TRIM/UPPER/LOWER/ROUND family, VLOOKUP/XLOOKUP/INDEX/MATCH, SUMIF/COUNTIF/SUMIFS/COUNTIFS, IFERROR, TODAY/NOW as materialized volatiles) | function conformance vectors; error-propagation tests |
| 7 | Dependency graph: formula groups, range edges via interval index; incremental dirty→topo→parallel recalc | 100k-cell recalc bench recorded |
| 8 | Identity references: insert/delete rows shifts ranges correctly; the canonical test — concurrent row-insert vs `SUM(A1:A10)` converges | dedicated regression + seeded-LCG sweep (D-052) |
| 9 | Reducer + Commands: set_value/set_formula/insert/delete rows-cols/clear/undo/redo with per-actor labeled undo groups | undo-law tests (undo∘do = id on own scope) |
| 10 | Two-replica sync: in-process + WebSocket relay binary; ops exchange, anti-entropy by Merkle diff | two-terminal demo script `demo/collab.sh`; divergence test = hash equality |
| 11 | Snapshots + recovery: content-addressed snapshot, op-tail replay | kill −9 mid-write test recovers |
| 12 | CSV import/export + XLSX **read** (values+formulas) in a sandboxed subprocess | round-trip corpus starter (20 files) |
| 13 | Differential replay CI: native vs wasm32(wasmtime) hash-equal | GitHub Actions workflow, required check |
| 14 | MCP server (stdio): `describe_workbook`, `describe_sheet`, `query` (DataFusion), `read_range`, `preview_edits`, `apply_edits`, `undo` | contract tests; manual: Claude drives full loop |
| 15 | `MEASUREMENTS.md` + `PROGRESS.md` | every numeric claim tagged measured(link) |

Build strictly in this order — each row depends on the ones above it.

## Explicitly NOT in v0.1
UI/rendering · charts · pivots · styles beyond number format · comments · merge cells · auth beyond a bearer token · server persistence beyond local files · E2EE · i18n beyond UTF-8 · XLSX write (values-only stretch goal, last).

## Definition of done (v0.1)
All 15 rows proven · all quality gates in CLAUDE.md green · `demo/collab.sh` shows two replicas converging through concurrent structural edits · an MCP client completes describe→query→preview→apply→undo · repo pushed with tagged release `v0.1.0` and honest `MEASUREMENTS.md`.
