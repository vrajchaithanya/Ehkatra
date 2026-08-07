# 38 — Benchmark Specification
Status: Approved · Normative: yes · Budgets live in docs/31 (single source); this doc specifies the *workloads* so numbers are reproducible and comparable across sessions and years

## Rules
Seeded generation only (LCG, fixed seeds committed) — a benchmark that can't reproduce its own workload measures nothing. Report: min of 5 runs for latency micro-benches; p50/p95 of 100 iterations for operation benches; RSS via OS-reported peak for memory. Machine context recorded with every number (CPU model, core count, target, opt level). Numbers land in MEASUREMENTS.md with the workload id — a number without a workload id is invalid.

## Workloads (id → definition)
**W-CHAIN-100K** (A-003, implemented Row 7): 10,000 rows × 10 columns; col A input values; cols B..K each `=prev_col + rand_const`, filled down — 100,000 formula cells in 10 chained groups. Measures: full recalc; single-edit incremental (edit one A cell, expected ~10 dirty cells). *Current: 53.0 ms full / 0.191 ms incremental, single-thread.*

**W-REPLAY-5K** (DP-A2 gate, implemented): the replay-check corpus — 5,000 seeded ops, 3 actors, mixed structural/cell. Measures: hash equality (pass/fail) + replay wall time.

**W-WIDE-AGG** (Row 7 follow-up): 100,000 rows × 5 data cols + one `SUM` per column + a grand total — the aggregation shape (few groups, huge ranges). Measures: full recalc; incremental after single data edit (expected: one partial-sum path, not 100k cells).

**W-TILE-10M** (A-001/A-002, due at TD-09 closure): 10M numeric cells written by 1 actor (import pattern), then 3-actor concurrent edit storm at 1% cell overlap (collab pattern), then 50% overlap (adversarial). Measures: RSS at load; bytes/cell; **promotion rate** per pattern; compaction ratio. *A-002 pass bar: <1% promotion at collab pattern.*

**W-SPARSE-SCATTER**: 1M cells scattered uniformly over a 10⁶×10³ identity space (0.1% density). Measures: RSS (sparse storage honesty — tiles must not materialize emptiness); point-read latency.

**W-STRUCT-STORM** (CRDT stress): 10,000 alternating row inserts/deletes from 3 actors at random anchors, 200 arrival orders. Measures: convergence (pass/fail); order-resolution wall time; tombstone count before/after compaction.

**W-EDIT-LATENCY** (A-004, Q2): scripted keystroke stream into a 10k-cell sheet with live recalc + (when shell exists) paint. Measures: keystroke→state p95; keystroke→paint p95.

**W-SYNC-RELAY** (Row 10): 2 and 50 replicas through one relay, each replica 10 ops/s for 60 s, 1% simulated packet loss. Measures: propagation p95; convergence time after last op; queued-op durability across a mid-run kill.

**W-OPEN-1M** (Row 11): container with 1M-cell workbook + 100k-op tail. Measures: cold open to READY (snapshot decode + replay); SALVAGE path time with a corrupted final page.

## Regression policy (docs/31)
Any budget breach blocks release. >5% p95 regression on any W-* workload vs the last release requires a signed-off debt entry. New feature areas must add a W-* entry here *before* their perf claims are stated anywhere — the workload spec is the license to publish a number.
