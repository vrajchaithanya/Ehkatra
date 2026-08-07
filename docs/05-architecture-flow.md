# 05 — Architecture Flow & Feature Comparison
Status: Approved (web-first per ADR-033) · Owner: Chief Architect

## End-to-end flow (web-first)

```
                                USER / DEVELOPER / AI AGENT
                                          │
        ┌─────────────────────────────────┼──────────────────────────────────┐
        │ PWA (browser)        Tauri wrapper (Win/macOS)       API clients   │
        │ WASM kernel          same web shell + native         REST · WS ·   │
        │ WebGPU renderer      menus/files/updater             MCP (L1/L2/L3)│
        └────────┬────────────────────┬─────────────────────────────┬────────┘
                 │  gesture/call      │                             │
                 ▼                    ▼                             ▼
        ┌──────────────────────────────────────────────────────────────────┐
        │                      COMMAND API  (one vocabulary)                │
        │        UI action  ≡  REST call  ≡  MCP tool  ≡  plugin call       │
        └───────────────────────────────┬──────────────────────────────────┘
                                        ▼
        ┌──────────────────────────────────────────────────────────────────┐
        │  REDUCER (pure, versioned) — compiles Command → Ops ONCE, at      │
        │  the authoring replica · validation · ACL early-deny · undo group │
        └───────────────────────────────┬──────────────────────────────────┘
                                        ▼
        ┌──────────────────────────────────────────────────────────────────┐
        │                    OP LOG  (single source of truth)               │
        │   canonical CBOR · causal order · BLAKE3 Merkle state hash        │
        └──────┬──────────┬───────────┬───────────┬───────────┬────────────┘
               ▼          ▼           ▼           ▼           ▼
        ┌──────────┐┌──────────┐┌───────────┐┌──────────┐┌───────────────┐
        │ CRDT     ││ CALC     ││ HISTORY   ││ AUDIT    ││ SYNC          │
        │ STATE    ││ ENGINE   ││ undo·     ││ CHAIN    ││ relay ⇄ peers │
        │ tiles·   ││ dep graph││ versions· ││ hash-    ││ offline queue │
        │ registers││ groups·  ││ branches· ││ chained· ││ anti-entropy  │
        │ order·   ││ parallel ││ snapshots ││ anchored ││ presence      │
        │ objects  ││ levels   ││           ││          ││               │
        └────┬─────┘└────┬─────┘└───────────┘└──────────┘└──────┬────────┘
             ▼           ▼                                      ▼
        ┌──────────────────────────┐                   ┌────────────────────┐
        │ PROJECTIONS (watermarked │                   │ SERVER PLANE       │
        │ folds): render tree ·    │                   │ same kernel headless│
        │ a11y tree · view model · │                   │ API·MCP·Calc       │
        │ query relations · MCP    │                   │ Authority·connectors│
        │ descriptions             │                   │ ·import/export     │
        └──────────────────────────┘                   │ sandbox·AI plane   │
                                                       └────────────────────┘
```

Read it as one sentence: every actor speaks Commands; the reducer turns them into ops exactly once; the op log is the only truth; and everything else — state, calculation, undo, history, audit, sync, rendering, MCP answers — is a fold over that log with a watermark.

## Feature comparison vs Microsoft Excel

Rating = our design quality for that feature area (/10, conservative; evidence-tagged designs, not shipped code). Advantage = who wins architecturally once built as designed.

| Feature area | Ours (status) | Excel | Rating | Advantage |
|---|---|---|---|---|
| Core grid & editing | Infinite virtual grid; identity addressing (H1) | 1M×16K fixed; positional | 9 | **Us** — references survive structure edits by construction |
| Formulas (breadth) | Core-200 → Extended-250 (H1→H2) | ~500 functions, decades of tail | 6 | **Excel** — breadth takes years; our order is usage-driven |
| Formula correctness | Decimal128 currency, strict mode, error provenance (H1) | f64 only, silent coercion, no provenance | 9 | **Us** — fixes Excel's most famous data-integrity sins |
| Dynamic arrays / LAMBDA | Spill-as-overlay, first-class lambdas (H1) | Mature, shipped | 8 | Par on capability; ours convergent under co-editing |
| Recalculation | Incremental, parallel, interruptible, deterministic (H1) | Mature, fast, single-authority | 8 | Par-to-us — cross-replica determinism is new ground |
| Co-authoring | CRDT, offline-first, conflict surfacing (H1) | Retrofitted, locks/merge quirks, Excel-online-biased | 9 | **Us** — structural, not bolted on |
| Offline | Full function offline, 180-day window (H1) | Desktop yes; web/co-auth degrade | 9 | **Us** on web; par on desktop |
| Undo / history | Selective per-user undo, branches, op-level blame (H1) | Linear undo; coarse version history | 9 | **Us** — agent-session undo is unique |
| Tables / structured refs | Identity-anchored, typed columns (H1) | Mature | 8 | Par |
| Pivot tables | Incremental materialized design (H2) | Deep, mature, 30 years | 5 | **Excel** until H2 ships |
| Charts | Plugin contract + core set (H1 partial → H2) | Vast gallery | 4 | **Excel** — breadth again |
| Conditional formatting / validation | As formula groups, incremental (H1) | Mature | 8 | Par; ours explainable |
| Power Query / connectors | Connector service + federation (H2) | Power Query is excellent | 4 | **Excel** until H2 |
| Automation | WASM plugins, capability-scoped, previewable (contract H1, GA H2) | VBA (powerful, dangerous) + Office Scripts | 8 | **Us** on safety; **Excel** on installed base |
| AI integration | AI-native: preview/undo/audit/taint, semantic layer, agent MCP (H1) | Copilot: assistive, limited undo/audit granularity | 9 | **Us** — the wedge |
| API surface | 3 layers, UI≡API≡MCP parity guaranteed (H1) | Graph API + JS API + VBA, partial coverage, drift | 9 | **Us** — parity by construction |
| MCP / agents | Semantic tools, SQL-first reads, blast-radius policy (H1) | Nascent | 9 | **Us** |
| XLSX fidelity | Semantic-lossless + preservation, published number (H1) | Is the format owner | 7 | **Excel** by definition; ours honest + measured |
| Performance at scale | Tiles + groups; budgets gated; 10M cells target | Decades of tuning; proven | 6 | **Excel** until MEASUREMENTS.md says otherwise |
| Security model | No ambient formula I/O, sandboxed parse, quarantined macros (H1) | Macro legacy; Protected View mitigations | 9 | **Us** — structural vs remedial |
| Audit / governance | Op-level, hash-chained, SIEM (H1) | File/tenant-level via M365 | 9 | **Us** at cell granularity |
| Accessibility | Designed-in tree, Excel keymap, VPAT gate (H1) | Mature, decades of AT work | 6 | **Excel** until we prove parity |
| Printing / page layout | H2 | Mature | 3 | **Excel** |
| Ecosystem / templates / community | — | Billions of files, 40 years | 1 | **Excel**, immovably, for years |

**Honest summary:** we win where architecture decides (collaboration, AI, API, audit, undo, correctness, security) — 9s; Excel wins where accumulated years decide (function tail, pivots, charts, Power Query, ecosystem) — and our roadmap buys those back in measured order. The strategy is exactly that shape on purpose.
