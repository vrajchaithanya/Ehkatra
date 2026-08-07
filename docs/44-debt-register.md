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

Paid debt moves to an archive with the date and the diff link. The register is reviewed at every release train; unpriced debt found in review is filed against the team that took it silently.
