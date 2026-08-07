# 44 — Technical Debt Register
Status: Living · Rule: debt is *chosen* and recorded at decision time with a repayment trigger, or it is a defect.

| ID | Debt | Chosen because | Interest we pay | Repayment trigger |
|---|---|---|---|---|
| TD-01 | DataFusion for Q1 SQL instead of tile-native engine | quarter scope | extra copy sheet→Arrow; ACL enforcement wrapped, not planned-in | Arrow layer lands (H2) or query p95 breach |
| TD-02 | ADRs 001–026 written post-hoc | rewrite speed | rationales may hide rejected-alternative context | live-ADR rule from now; backfill alternatives when touched |
| TD-03 | Spec budgets are targets, not measurements | design preceded rig | credibility risk; possible re-design | MEASUREMENTS.md Q1 W13 |
| TD-04 | XLSX write values-only in Q1 | scope | demo asymmetry (read ≫ write) | Q2 XLSX write + corpus |
| TD-05 | No gRPC/Arrow in H1 | desktop-first trim | bulk integrations wait; REST/NDJSON only | first data-platform design partner |
| TD-06 | Auth = bearer token in Q1 skeleton | scope | not production-safe; must not leak into Q3 | server plane alpha (Q3) replaces wholesale |
| TD-07 | Charts/pivots absent at GA-alpha boundary | scope discipline | pilot objections expected | H2 top of queue; pre-briefed to partners |
| TD-08 | Single relay process (no HA) through Q3 | ops simplicity | availability risk in pilots | GA operational-readiness gate |
| TD-09 | **Promotion granularity is the whole 16,384-cell tile** | ADR-005 as designed; the cheap summary is per tile, so the promoted unit is too | Measured: one contested cell costs ~1 MB of per-cell metadata; 0.1% scattered contention drives 8.4 → 74.5 B/cell and puts 10M cells at ~745 MB vs a 400 MB budget (A-002 FAILED, MEASUREMENTS.md) | **Q1 gate, before Q2 begins** — docs/42's named consequence for an A-002 failure. Weigh sub-tile promotion blocks vs compact per-cell stamps vs two-level summaries, and re-measure with `tools/tile-bench` |
| TD-10 | Promotion detects *multi-writer cells*, not true concurrency | v0.1 `Op` carries `{id, lamport, payload}` — docs/10's causal `deps` delta is not implemented yet, so causally-ordered cross-actor writes are indistinguishable from concurrent ones | Over-promotion: sequential hand-offs (A edits, then B edits the same cell later) pay full per-cell metadata for a conflict that never existed | `Op` gains `deps` (Row 10 sync). Re-run `tools/tile-bench`; expect the A-002 numbers to improve but **not** enough to close TD-09 on its own |
| TD-11 | `State::replay_sorted` trusts its caller to supply canonical order | The A-001 harness cannot exist otherwise: 10M cells is ~1.2 GB of ops for ~84 MB of state, so the log cannot be materialised | A caller feeding unsorted ops gets silently wrong LWW in summary tiles (a `debug_assert` catches it in dev builds only) | Row 11/12, when snapshot recovery and import become real callers — replace the precondition with a checked, streaming merge |

Paid debt moves to an archive with the date and the diff link. The register is reviewed at every release train; unpriced debt found in review is filed against the team that took it silently.
