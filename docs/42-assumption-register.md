# 42 — Assumption Register
Status: Living · Rule: no work may build on an assumption past its validation date

| ID | Assumption | Load-bearing for | Validation | Date | Status |
|---|---|---|---|---|---|
| A-001 | 10M-cell workbook fits <400 MB working set with tiered store | desktop scale story | W-TILE-10M | Q1 W5 | **Confirmed for import and collaboration** — 8.43 B/cell / 90.0 MB RSS (import) and 11.09 B/cell / **123.6 MB** (3-actor collab at 1% overlap) against a 400 MB budget. TD-09 is what restored the collab case, which previously extrapolated to ~745 MB and failed. **Fails at the adversarial 50%-overlap pattern** (1.7 GB) — real conflict metadata, not amplification; docs/38 sets no bar there. |
| A-002 | CRDT promotion <1% cells under realistic multi-author load | ADR-005, memory model | W-TILE-10M | Q1 W5 | **Amplification fixed (TD-09 closed); bar met at its floor and the bar itself needs restating.** Promotion is per contested *cell*: amplification 16,384× → **1×**, measured **1.000%** at the collab pattern — the floor, since a contested cell must carry metadata. "<1% at 1% overlap" is unachievable by any correct implementation; **doc defect filed for the owner to restate in docs/38**. Previously:  — 0.1% contested cells promote 25% (clustered) to 100% (scattered) of cells, because one contested cell promotes its whole 16,384-cell tile. 0.1% contested cells promoted 25–100% of cells. Consequence executed; TD-09 closed session 9 with W-TILE-10M re-measured. |
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
