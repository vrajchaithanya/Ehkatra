# 35 — Testing, Benchmark & Validation Strategy
Status: Approved · Owner: QA Architect · Normative: yes · Carved from SPEC §29

## The layered strategy (unusual layers explicit)
1. **Oracle conformance** — Excel-captured vectors (docs/32) for every function × edge grid; XLSX corpus round-trip with semantic + layout diff; both produce the published numbers.
2. **Algebraic property tests** — CRDT commutation/convergence (10⁵ interleavings CI / 10⁷ nightly, structural ops weighted), undo laws (undo∘do = id on own scope), cache-watermark coherence, reference-rewrite round-trips.
3. **Formal verification** — TLA+ models of the order CRDT + retain-losers register + sync handshake, model-checked in CI (small configurations) and at depth nightly; artifacts in `formal/`, referenced by ADR-002/006. Property tests sample; the checker exhausts.
4. **Differential replay** — full op corpus, 3 architectures, bit-identical BLAKE3 hashes; the P2 gate, every merge.
5. **Continuous fuzzing** — parsers (structure-aware, grammar-based), op applier (adversarial logs), formula parser, query planner (SQLancer-style TLP/NoREC logic oracles); release-blocking on open crashes.
6. **Deterministic simulation** — FoundationDB-style: relay + N replica state machines under scripted partitions/reorders/crash-restarts with seeded schedules, replayable failures; where the distributed bugs actually get found.
7. **Desktop E2E** — golden-workbook suites per feature; multi-instance collaboration scripts; crash-injection (kill −9 mid-op-batch, power-fail simulation on the container file); platform matrices (Win 10/11, macOS N−2..N, per-monitor DPI, IME: Japanese/Chinese/Korean, screen readers: Narrator/NVDA/JAWS/VoiceOver via accesskit assertions + scripted runs).
8. **API/MCP contract tests** — every tool/endpoint × truncation/error/permission paths (agents hit these constantly); schema-fuzzed.
9. **AI capability evals** — versioned eval sets per capability (formula-gen correctness on held-out sheets, NL→SQL accuracy, cleanup precision/recall); model/prompt changes gate on evals like code gates on tests.
10. **Performance** — docs/31 gates. **11. Chaos/restore drills** — docs/16/36.

## Coverage philosophy
Mutation testing (≥85% mutants killed) on the modules where silent bugs corrupt data forever: op applier, reducer, evaluator, codecs. Coverage percentage elsewhere is informational, not a gate — gates are behavioral (oracle, property, replay).

## Validation = evidence lifecycle
Every assumption (docs/42) names the test that retires it; every budget row names its benchmark; every ADR names its verification. The traceability matrix (docs/49) binds them; a claim without a verification row is a defect in this strategy.
