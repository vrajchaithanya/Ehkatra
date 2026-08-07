# The One-Quarter Build: Walking Skeleton + Proof Rig

**Goal:** in 13 weeks, convert the reference spec from "asserted" to "measured or refuted" on every load-bearing claim, and ship a runnable v0.1 that a developer or an AI agent can actually use.
**Team assumption:** 2–4 engineers (scales down to 1 with the stretch items dropped). Rust required; no other stack.
**Ruthless rule:** if it doesn't either (a) de-risk a frozen ADR or (b) appear in the demo script, it is not in the quarter.

---

## What v0.1 IS

A **headless spreadsheet kernel + API server**, single node, that can:

1. Create/open workbooks; read CSV and basic XLSX (values + formulas + number formats — no charts, no styles beyond number format).
2. Full identity-based grid: RowId/ColId order CRDT, A1 as a view, insert/delete rows/cols with correct reference behavior.
3. ~60 formula functions (Core catalog Tier 1), formula groups, range-edge dependency graph, incremental + parallel recalc.
4. Op log with canonical CBOR encoding, BLAKE3 Merkle state hash, snapshots, op-tail recovery.
5. **Two-replica sync over WebSocket** with CRDT merge — the demo is two terminal sessions editing one workbook concurrently, including the nasty case (concurrent row insert vs. range formula) converging correctly.
6. REST Layer-1 subset (read/write cell/range, structure ops, get/set formula, undo) + MCP Layer-2 subset (`describe_workbook`, `describe_sheet`, `query` with SQL over one-sheet relations, `preview_edits`, `apply_edits`, `explain_formula`, `trace_error`).
7. Per-actor labeled undo groups; agent edits reversible as a unit.

## What v0.1 is NOT (explicit cut list)

No UI (a read-only web viewer is the single stretch item). No E2EE/MLS (Standard tier only; auth = bearer token). No charts, pivots, validation, cond-format, comments, merge-cells. No gRPC/Arrow (REST+JSON only; Arrow is Q2). No distributed calc. No mobile/embedded builds (but the `no_std` CI check runs from day 1 — cheap now, impossible later). No i18n beyond UTF-8 correctness. XLSX *write* is values-only.

---

## The correctness rig (this is how the 7.5 → 9 happens)

Built in parallel from week 1; every item replaces an assertion in the spec with a number or a proof:

| Rig component | Spec claim it tests | Week |
|---|---|---|
| Differential replay CI (x86_64 + aarch64 + wasm32, BLAKE3 hash equality) | P2 determinism | 2 |
| CRDT property suite: 10⁵ random interleavings/run, structural ops weighted | §4.2 commutation | 3 |
| **TLA+ model of the order CRDT + retain-losers register**, model-checked | merge convergence | 4–6 |
| Memory harness: 10M-cell load, measured RSS + promotion-rate counter | §7 "81 MB / <1% promotion" | 5 |
| wasm32-in-Chromium + Safari harness: same 10M-cell load | browser feasibility | 6 |
| Recalc benchmark: 100k-dep-cell graph, measured p95 | "<200 ms" | 7 |
| Oracle capture harness v0: Excel-via-COM, capture Tier-1 function vectors | ADR-024, compat correctness | 3→ (runs continuously) |
| Fuzz: formula parser + op applier + CSV/XLSX reader (cargo-fuzz, CI) | §21 | 6 |
| Adversarial op corpus: hand-written hostile logs | malicious-collaborator threat | 9 |

**Deliverable at quarter end: a MEASUREMENTS.md** where every table from the spec (§7 memory, §28 budgets) is either confirmed with a real number, revised, or marked failed with the design change it forces. That document is what makes the spec technically correct — correctness is a property of evidence, not of prose.

---

## Week-by-week

**W1–2 — Foundations.** Repo, crate skeleton per §4.1 layering (visibility rules enforced day 1), `usk-types` + op encoding + BLAKE3 hashing, differential-replay CI green on a trivial op set. *Milestone: two platforms, one hash.*

**W3–4 — Grid + order CRDT.** Fugue-family sequence for rows/cols, tile store (presence bitmap + packed f64 + tagged union), per-tile causal summary + promotion, A1 view via order-statistic tree. Oracle harness starts capturing. *Milestone: insert/delete/write/read via CLI; property suite green.*

**W5–6 — Formulas + dep graph.** Parser→CST→AST→binder, 60 functions, formula groups, interval index, dirty propagation + topo levels + rayon evaluation. TLA+ model checking completes. Memory + wasm32 harnesses report first real numbers. *Milestone: 100k-formula workbook recalcs; measured numbers exist.*

**W7–8 — Op log lifecycle + sync.** Snapshots, recovery, undo groups, WebSocket relay (single process), two-replica convergence incl. concurrent-structural cases. *Milestone: the two-terminal demo works.*

**W9–10 — API + MCP.** Axum REST Layer-1 subset; MCP server with the 7 tools; SQL via DataFusion over sheet relations (buy, don't build, for v0.1 — replace with the tile-native engine when Arrow lands in Q2); `preview_edits` on a scratch branch. *Milestone: Claude connects over MCP and does the full describe→query→preview→apply→undo loop.*

**W11–12 — Import + hardening.** CSV (streaming, strict-mode inference report), XLSX read (values/formulas/number formats via sandboxed subprocess), fuzz triage, adversarial op corpus, perf tuning against budgets.

**W13 — Measurement week.** No features. Run everything, write MEASUREMENTS.md, revise the spec where reality disagreed, cut a tagged v0.1 with the demo script.

---

## Spec revisions this quarter forces (known in advance)

1. Every §28 number gets a "measured on <hw>" footnote or a revision.
2. §7 promotion-rate assumption becomes a graph from the harness, under three simulated collaboration patterns.
3. The wasm32 result decides ADR-005's fate honestly: if Safari can't hold the 10M-cell working set, the projection-mode threshold (§11.2) moves into the v1 requirements, and the spec says so.
4. TLA+ artifacts get checked into `formal/` and referenced from ADR-002/006 — upgrading them from "tested" to "model-checked."

## Q2 preview (so cuts don't feel like losses)

Rendering skeleton + virtual scroll, Arrow/Layer-3, tile-native query engine, styles/validation/cond-format, presence, XLSX write fidelity + corpus, the third spike (canvas a11y) — which needs the renderer, hence Q2.
