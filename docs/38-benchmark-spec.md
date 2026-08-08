# 38 — Benchmark Specification
Status: Approved · Normative: yes · Budgets live in docs/31 (single source); this doc specifies the *workloads* so numbers are reproducible and comparable across sessions and years

## Rules
Seeded generation only (LCG, fixed seeds committed) — a benchmark that can't reproduce its own workload measures nothing. Report: min of 5 runs for latency micro-benches; p50/p95 of 100 iterations for operation benches; RSS via OS-reported peak for memory. Machine context recorded with every number (CPU model, core count, target, opt level). Numbers land in MEASUREMENTS.md with the workload id — a number without a workload id is invalid.

## Workloads (id → definition)
**W-CHAIN-100K** (A-003, implemented Row 7): 10,000 rows × 10 columns; col A input values; cols B..K each `=prev_col + rand_const`, filled down — 100,000 formula cells in 10 chained groups. Measures: full recalc; single-edit incremental (edit one A cell, expected ~10 dirty cells). *Current: 53.0 ms full / 0.191 ms incremental, single-thread.*

**W-REPLAY-5K** (DP-A2 gate, implemented): the replay-check corpus — 5,000 seeded ops, 3 actors, mixed structural/cell. Measures: hash equality (pass/fail) + replay wall time.

**W-WIDE-AGG** (Row 7 follow-up): 100,000 rows × 5 data cols + one `SUM` per column + a grand total — the aggregation shape (few groups, huge ranges). Measures: full recalc; incremental after single data edit (expected: one partial-sum path, not 100k cells).

**W-TILE-10M** (A-001/A-002, due at TD-09 closure): 10M numeric cells written by 1 actor (import pattern), then 3-actor concurrent edit storm at 1% cell overlap (collab pattern), then 50% overlap (adversarial). Measures: RSS at load; bytes/cell; **promotion rate** per pattern; compaction ratio. *A-002 pass bar (restated per D-062, 2026-08-08): **promotion amplification ≤ 1.5× the contested-cell rate** (promoted cells ÷ contested cells ≤ 1.5 — correctness requires ≥ 1.0, since a contested cell must carry metadata per ADR-006; the original "<1% promotion at collab" was unattainable as written because the collab pattern contests exactly 1% by definition), **and** collab-pattern RSS ≤ 400 MB (the A-001 bar). Adversarial pattern (50% contested): no RSS bar — 50% genuinely contested cells legitimately cost memory; measured and recorded, revisit only if real-world telemetry ever shows contested rates above 5%.* Measured at TD-09 closure: amplification 1.0× (floor), collab RSS 123.6 MB — **pass**.

**W-SPARSE-SCATTER**: 1M cells scattered uniformly over a 10⁶×10³ identity space (0.1% density). Measures: RSS (sparse storage honesty — tiles must not materialize emptiness); point-read latency.

**W-STRUCT-STORM** (CRDT stress): 10,000 alternating row inserts/deletes from 3 actors at random anchors, 200 arrival orders. Measures: convergence (pass/fail); order-resolution wall time; tombstone count before/after compaction.

**W-EDIT-LATENCY** (A-004, Q2): scripted keystroke stream into a 10k-cell sheet with live recalc + (when shell exists) paint. Measures: keystroke→state p95; keystroke→paint p95.

**W-SYNC-RELAY** (Row 10, implemented `tools/sync-bench`): 2 and 50 replicas through one relay, each replica 10 ops/s for 60 s, 1% simulated packet loss. Measures: propagation p95; convergence time after last op; queued-op durability across a mid-run kill. Propagation and convergence are reported in **bus milliseconds** (the deterministic transport's simulated clock, one hop = 5 ms) because they are protocol properties — round trips to reach every replica — not properties of the host; wall time is reported separately. *Measured at Row 10 closure. 2 replicas: propagation p50 200 / p95 1,600 bus-ms; convergence 10 bus-ms; all replicas hash-equal; **32 ops queued at a mid-run kill, 32 delivered after recovery**; 0 quarantined; 97 ms wall. 50 replicas: 30,000 ops, 4,558 dropped frames / 2,085 reconnects, propagation p50 800 / p95 3,700 bus-ms, convergence 2,140 bus-ms, **all replicas hash-equal**, **45 ops queued at a mid-run kill and 45 delivered after recovery**, 6.8 min wall (120 min before TD-24 was paid — D-071/D-072).*
*The workload earned its place immediately: it found three convergence defects the whole test suite missed (D-067; the D-064 teardown branch, without which the 50-replica run diverged outright) plus a hollow durability row of its own. **Two harness notes worth keeping.** "Offline" must be a real partition, not a dropped frame plus a long timer — the latter lets the victim reconnect and drain before the kill, which made the durability row read 2 ops instead of 45 while looking like a result. And propagation must be counted at first delivery, not by scanning the receiver's log: an op held in the causal-gap buffer is counted twice by the latter, inflating the tail. See MEASUREMENTS.md and PROGRESS.md session 10.*

**W-OPEN-1M** (Row 11): container with 1M-cell workbook + 100k-op tail. Measures: cold open to READY (snapshot decode + replay); SALVAGE path time with a corrupted final page.

## Regression policy (docs/31)
Any budget breach blocks release. >5% p95 regression on any W-* workload vs the last release requires a signed-off debt entry. New feature areas must add a W-* entry here *before* their perf claims are stated anywhere — the workload spec is the license to publish a number.
