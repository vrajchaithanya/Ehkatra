# 40 — Roadmap & Implementation Plan
Status: Approved · Owner: Chief Architect + Product Architect

## Q1 — Walking skeleton + proof rig (the evidence quarter; detailed week-by-week in QUARTER-PLAN)
Headless kernel: op log, order CRDT, tile store, 60 functions, incremental parallel calc, two-replica WS sync, snapshots/recovery, REST L1 subset, MCP 7-tool subset (DataFusion SQL), CSV + XLSX-read in sandbox. Proof rig: differential replay CI, property suites, **TLA+ models checked**, memory/promotion harness, wasm32 harness, oracle capture start, fuzz, adversarial ops. **Exit: MEASUREMENTS.md — every §31 target confirmed/revised/failed-with-consequence; the two-terminal concurrent-edit demo; Claude doing describe→query→preview→apply→undo.**

## Q2 — Desktop foundation
*Platform authority: **ADR-037** (native winit + wgpu is the product, superseding ADR-033's web-first PWA). The shell's dependency ceiling is a separate budget line and must be **measured before the first GPU dependency lands** — see ADR-037 and MEASUREMENTS.md.*

winit+wgpu shell on Win+macOS: renderer, virtual scroll, editing surface, native IME overlay, menus/dialogs/file-association adapters, accesskit tree v1; container file format (SQLite) with crash-injection suite; styles/formats, validation, cond-format, sort/filter, tables; XLSX write + corpus v1 + first published fidelity number; presence; installer/signing/update pipeline (rings). *Exit: dogfood-daily internal alpha; canvas-a11y spike resolved with Narrator+VoiceOver evidence.*

## Q3 — Collaboration + agents in product
Server plane alpha (relay, doc store, auth, audit chain); multi-user editing at 50 concurrent; offline/reconnect + conflict surfacing UX; comments; full MCP surface + blast-radius policies + desktop-local MCP with in-UI preview overlays; AI capabilities v1 (formula gen/explain, NL query, cleanup) behind evals; charts core-6; freeze/split; find/replace. *Exit: 5 design partners, agents making previewed edits in real workbooks.*

## Q4 — Enterprise GA hardening
RBAC/ABAC + SSO + SIEM streaming; LTS channel + enterprise deployment (MSIX/ADMX, MDM); Core-200 complete + conformance number published; import advisories; perf/battery budget closure; a11y VPAT; operational readiness gate (docs/36); security pen test + fixes. *Exit: GA with published fidelity + conformance + benchmark numbers.*

## Horizon 2 (post-GA, order by evidence and demand)
Pivots → connectors + gRPC/Arrow → plugin SDK GA → ODS/Parquet → i18n T2 → web viewer → Managed-E2EE.

## Sequencing laws
Nothing builds on an unmeasured assumption past its validation date (docs/42); budgets gate from Q1 day one; any quarter slipping >2 weeks triggers scope displacement, not schedule extension.
