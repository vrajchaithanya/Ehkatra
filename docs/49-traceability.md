# 49 — Traceability Matrix
Status: Living · Requirement → design → verification. A row missing any column is a defect.

| Requirement (source) | Design (doc) | Verification (docs/35 layer) |
|---|---|---|
| Deterministic cross-platform calc (45-Reliability) | 10 kernel P2, 13 determinism tiers | L4 differential replay; L3 TLA+ |
| No silent divergence (45) | 10 Merkle state hash | L4 gate + anti-entropy localization test |
| No acked-op loss / never-drop offline edits (45) | 15 offline contract, 16 recovery | L6 simulation; crash-injection |
| Concurrent structural edits converge (vision) | 10 op algebra, 11 references | L2 property (weighted), canonical row-insert-vs-formula regression |
| 10M cells < 400 MB (31) | 14 tiles + promotion | memory harness (A-001/2) |
| Recalc 100k < 200 ms (31) | 13 groups + level-parallel | recalc bench (A-003) |
| Keystroke < 16 ms (31) | 31 renderer, 13 viewport-priority | Q2 bench (A-004) |
| XLSX semantic round-trip (45-Compat) | 24 preservation + sandbox | L1 corpus gate, published number |
| Function conformance ≥99.5% (45) | 12 catalog + profiles | L1 oracle corpus |
| Formula cannot exfiltrate (30) | 13 T3 Authority, 10 no_std | architecture invariant I1 build + egress test |
| Untrusted files cannot escape parser (30) | 24 sandbox, IR revalidation | L5 fuzz + sandbox-escape suite |
| Agent edits previewable/reversible/attributed (vision, 22) | 21 guardrails, 16 auto-milestone, 11 agent undo scope | L8 contract tests + 48 AI gates |
| Query/pivot cannot leak past reader ACL (30) | 30 planner enforcement | adversarial ACL suite |
| Undo never destroys others' work silently (11) | 11 selective undo | L2 undo laws + scripted collab E2E |
| IME native-grade (45-Usability) | 33 IME contract, 31 editor overlay | typist validation protocol |
| Screen-reader operable grid (45-A11y) | 33 a11y contract, 16 tree | accesskit assertions + scripted SR runs |
| Crash RPO/RTO (45) | 16 WAL + snapshots | crash/power-fail injection |
| 10-year file durability (45) | 10 forward-preservation, 14 container | archived-corpus CI gate (permanent) |
| Content-free telemetry (45-Privacy) | 36 taint lint | compile-time lint + bundle audit |
| Reproducible builds + provenance (30) | 34 | artifact verification in release pipeline |
