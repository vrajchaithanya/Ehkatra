# 42 — Assumption Register
Status: Living · Rule: no work may build on an assumption past its validation date

| ID | Assumption | Load-bearing for | Validation | Date | Status |
|---|---|---|---|---|---|
| A-001 | 10M-cell workbook fits <400 MB working set with tiered store | desktop scale story | memory harness | Q1 W5 | Open |
| A-002 | CRDT metadata promotion amplifies ≤1.5× the truly-contested cell rate (restated per D-062; original "<1%" bar was unattainable — see docs/38 W-TILE-10M) | ADR-005, memory model | W-TILE-10M, 3 patterns | done | **Confirmed** (1.0× floor, collab RSS 123.6 MB; failed at Row 4 as 16,384× amplification, redesigned per-cell at TD-09, re-measured) |
| A-003 | 100k-dep recalc <200 ms on 8-core via level-parallel groups | budget table | recalc bench | Q1 W7 | Open |
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
