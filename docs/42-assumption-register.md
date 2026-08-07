# 42 — Assumption Register
Status: Living · Rule: no work may build on an assumption past its validation date

| ID | Assumption | Load-bearing for | Validation | Date | Status |
|---|---|---|---|---|---|
| A-001 | 10M-cell workbook fits <400 MB working set with tiered store | desktop scale story | memory harness | Q1 W5 | **Confirmed, single-author only** — 84.2 MB structural / 93.1 MB RSS at 10M cells (MEASUREMENTS.md, `tools/tile-bench`). Under A-002's measured contention it becomes ~745 MB and **fails**; re-validate after the A-002 redesign. |
| A-002 | CRDT promotion <1% cells under realistic multi-author load | ADR-005, memory model | promotion harness, 3 patterns | Q1 W5 | **FAILED** — 0.1% contested cells promote 25% (clustered) to 100% (scattered) of cells, because one contested cell promotes its whole 16,384-cell tile. Consequence executes: tile-granularity redesign is a Q1 gate (docs/44 D-04, docs/43 D-039). |
| A-003 | 100k-dep recalc <200 ms on 8-core via level-parallel groups | budget table | recalc bench | Q1 W7 | **Confirmed on the single-threaded path** — 53.0 ms for 100k dependents on **1** core, and 0.191 ms for a single edit against an 8 ms budget (`tools/calc-bench`, MEASUREMENTS.md). The *level-parallel* half is unvalidated: rayon is behind the PAL `Compute` trait, which is unbuilt. Re-run on a wide model once it exists. |
| A-004 | Keystroke→paint <16 ms incl. a11y tree diff on ref hw | flagship UX claim | Q2 renderer bench | Q2 | Open |
| A-005 | wasm32 holds a 1M-cell working set in Safari (web-viewer future) | H2 web decision | wasm harness | Q1 W6 | Open |
| A-006 | Formula groups collapse real-world workbooks ≥100:1 (graph ≪ data) | dep-graph memory | corpus analysis | Q1 W7 | Open |
| A-007 | Excel COM capture is licensable + stable enough for the oracle | conformance strategy | counsel + harness pilot | Q1 W6 | Open |
| A-008 | rustybuzz-everywhere shaping cost is recoverable via cache (>95% hit) | determinism-over-speed bet | shaping bench | Q2 | Open |
| A-009 | SQLite container survives sync-managed-folder races with safe-copy discipline | desktop file strategy | sync-race injection suite | Q2 | Open |
| A-010 | Agents + preview UX produces net trust (users approve, not alarm) | wedge #1 | design-partner studies | Q3 | Open |
| A-011 | Retain-losers conflict surfacing delights rather than annoys | ADR-006 UX | alpha telemetry + interviews | Q3 | Open |
| A-012 | 2–4 engineers suffice for Q1 skeleton scope | roadmap | weekly burn vs plan | Q1 W4 | Open |

On validation: status → Confirmed (evidence link) / Revised (spec delta + ADR) / **Failed (named consequence executes — e.g., A-002 fail ⇒ tile granularity redesign before Q2 begins)**.
