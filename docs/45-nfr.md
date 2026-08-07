# 45 — Non-Functional Requirements
Status: Approved · Normative: yes · Each NFR names its verification (docs/49 binds them)

**Performance** — the docs/31 budget table, verbatim, is the requirement set (single source).
**Reliability** — zero silent divergence (state-hash verified; any mismatch = sev-1); zero acked-op loss; desktop crash RPO ≤ 250 ms typing, RTO < 3 s; server RPO ≤ 1 s / RTO < 5 s per workbook; crash-free sessions ≥ 99.5%; poison-op quarantine, never crash-loop.
**Availability (server, GA)** — 99.9% API/sync monthly; error budget governs release pace; degraded modes defined (relay down ⇒ desktop fully functional offline, queued sync).
**Scalability** — H1: 10M cells/workbook desktop, 50 concurrent editors/workbook, 200 workbooks/compact-server node; H2 targets set from GA telemetry, not guessed.
**Security** — docs/30 posture; no ambient formula I/O ever; sandbox for all untrusted bytes; SLSA-3 provenance; pen test per major; zero known-critical at release.
**Privacy** — telemetry content-free by type-level construction, opt-in; crash dumps scrubbed + user-inspectable; AI evidence taint-recorded; local-model option for semantic classifiers.
**Compatibility** — published XLSX fidelity ≥ target per release (initial bar: 99% corpus semantic fidelity, raised as measured); function conformance ≥ 99.5% on oracle corpus at GA; container files forward-openable indefinitely; wire N−2.
**Accessibility** — WCAG 2.2 AA; UIA + NSAccessibility complete; Excel-parity keymap; VPAT maintained; a11y regression = release blocker.
**Usability (desktop craft bar)** — cold launch < 1.0 s; file-open double-click to interactive < 1.5 s (1M cells); IME composition indistinguishable from native apps (validated with JP/CN/KR typists); undo always available within 2 gestures.
**Maintainability** — kernel purity lints; one normative doc per concern with owner; mutation-test floor on corruption-critical modules; new-engineer first-PR-merged < 2 weeks (onboarding doc + dev harness).
**Observability** — every user-visible failure diagnosable from a support bundle without cell content; calc bugs reproducible from replay bundles.
**Internationalization** — Tier-1 scripts at GA (Latin/Cyrillic/Greek + bidi + horizontal CJK), locale-correct parsing/formatting/collation; storage locale-free.
**Durability of meaning (10-year)** — a container written at GA opens, calculates identically (state hash), and round-trips in every future release; verified by an archived-corpus gate in CI forever.
