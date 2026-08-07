# 13 — Calculation Engine
Status: Approved · Owner: Principal Engineer · Normative: yes · Carved from SPEC §10–11

## Dependency graph
Nodes = formula groups (shared R1C1 pattern over a region — 1M cells ≈ hundreds of nodes), single formulas, names, tables, volatile/external bindings, spill extents. Edges = range-granular via per-sheet interval index (R-tree over identity space): "who reads what I wrote" is a log-time stab, never materialized cell edges (~0.1 MB vs ~96 MB at 1M formulas).

## Incremental evaluation
Edit → dirty intervals → stab → transitive dirty marking **with early cutoff** (unchanged-by-hash stops propagation) → incremental topo levels (Pearce-Kelly, maintained not recomputed) → level-parallel evaluation (rayon; cost-model-steered splitting). Interruptible (ops arriving mid-calc checkpoint the frontier; undirtied cells always show last-consistent values — UI never blocks), resumable, deterministic (Neumaier order-pinned reductions, no FMA contraction, row-major identity-order traversal, no locale in engine). Viewport-dirty cells evaluate first (user-perceived latency beats throughput).

## Determinism tiers (the convergence contract)
**T1 pure** — every replica evaluates identically. **T2 volatile** (`NOW/TODAY/RAND…`) — never ambiently evaluated; each call site holds a materialized `VolatileBinding {value, computed_at, computed_by}` in the CRDT; recalculation is an explicit, attributed, seeded event. Replicas converge because they read, not compute. **T3 external** (connectors, `WEBSERVICE`-class) — executed only by the server-side Calculation Authority under egress allowlist; results materialize with provenance. Iterative/circular: opt-in, Authority-only, pinned iteration/epsilon. Authority election: server when available; else lease to lowest connected ActorId; split-brain resolves as ordinary register merge — convergent under all partitions.

## Consistency model (published)
Eventual calc consistency with monotone convergence: after op quiescence all replicas reach the same fixpoint. In-flight cells carry a generation mark (subtle UI affordance). API reads carry `calc_watermark`; callers may demand `at_least(v)` or `converged`.

## Cycles & pathology
Tarjan SCC on dirty subgraph → `#CIRC!`; evaluator resource governors (depth, cell budget, time slice) kill hostile patterns with attributable reports; workbook health report (docs/36) surfaces SCC clusters and volatile density to users.

## Scale posture (desktop-first)
One node = one workbook engine, parallel within. Distributed partitioned calc (min-cut fleet, frontier exchange) is Horizon 3 — designed (SPEC §11 archived), not scheduled. Desktop budget: 10M cells in <400 MB working set with tiered eviction (docs/14).
