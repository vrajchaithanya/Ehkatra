# Changelog — Architecture Repository

## 2026-08-07 (later) — Platform reversal: web-first restored (ADR-033)
- Directive reverses ADR-027/028; PWA + WASM primary, Tauri wrapper for desktop. Kernel/module docs unaffected (PAL payoff). docs/33 to be revised; A-005 (Safari/wasm32) becomes launch-blocking. New risk R-13: platform-strategy churn.

## 2026-08-07 — Repository establishment (this change)
- ARB review of all prior documents; consolidation memo 001 issued (contradictions C1–C4 resolved, drift D1–D3 recorded).
- **Platform pivot:** desktop-first Windows/macOS (ADR-027/028); web demoted to future target under permanent wasm32 gate.
- Monolithic GRID-ARCHITECTURE-SPEC carved into docs/10–24 + 30–36 (ADR-029); archived as SPEC-ARCHIVE.
- Scope descoped by evidence rule: embedded target → discipline only; distributed calc → H3; E2EE → approved-unscheduled (ADR-030).
- New decisions: SQLite single-file container (ADR-031); DataFusion-for-Q1 SQL (ADR-032, debt TD-01).
- Registers established: risk (12), assumptions (12, all dated), decisions (32 ADRs), debt (8, all priced).
- Governance artifacts: NFRs, glossary, production-readiness checklist, traceability matrix, scorecard.

## Earlier (pre-repository)
- 2026-08-07: QUARTER-PLAN (Q1 skeleton + proof rig) — now roadmap Q1.
- 2026-08-07: GRID-ARCHITECTURE-SPEC 1.0-RC (32 sections, 26 ADRs) — now carve source.
- 2026-08-07: DOC-GRID-DESIGN — superseded same day by the spec rewrite.
- 2026-08-07: DESIGN-V2-HARD-PROBLEMS (suite kernel) — grid-relevant decisions imported; suite remainder archived.
- 2026-08-07: ARCHITECTURE-REVIEW (suite multi-role review) — judgments imported to registers.
