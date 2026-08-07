# Changelog — Architecture Repository

## 2026-08-07 (session 3) — BOOTSTRAP Row 4: tile store; A-002 fails
- **Row 4 built**: `usk_state::tile` — 256×64 tiles in identity space, presence bitmap, payload packed dense over present cells (`f64` fast path / tagged union), 24-byte per-tile causal summary with promotion on contested cells. `State` no longer holds a flat cell map. 9 new tests (14 total), including a reference-model equivalence proof.
- **A-001 confirmed (single-author)**: 10M numeric cells = 84.2 MB structural / 93.1 MB OS peak, 8.425 B/cell, vs a 400 MB budget.
- **A-002 FAILED**: 0.1% contested cells promote 25–100% of cells; memory rises to 74.5 B/cell, i.e. ~745 MB at 10M cells. One contested cell promotes its whole 16,384-cell tile. ADR-005's tile granularity now needs redesign before Q2 (TD-09; docs/42 consequence executed).
- New decisions: ADR-034 (stable identity→slot band keying), D-039 (per-contested-cell promotion, decided in a replay pre-pass), D-040 (tile-major state hash; oplog hash unchanged).
- New debt: TD-09 promotion granularity, TD-10 multi-writer ≠ concurrency (needs `Op.deps`), TD-11 `replay_sorted` precondition.

## 2026-08-07 (session 3) — Repository and toolchain repair
- Repository initialised (the tree had no git history). `Cargo.lock` committed (D-037); toolchain pinned to 1.97.1 with components and targets (D-036) after an unpinned `stable` turned a green gate set red with no code change.
- Host toolchain switched to self-contained `x86_64-pc-windows-gnu` — no MSVC, no admin, no PATH edit, DP-S5 intact (D-038).
- Gates: `tools/gates.ps1` runs the whole set in one command; added supply-chain scanning (`deny.toml`, cargo-deny + cargo-audit), the DP-S5 host-isolation grep docs/07 §6 asked for, a `no_std` wasm32 kernel build, and the DP-S2 complexity budget as an executable gate (`tools/dep-budget.mjs`, D-035).
- Determinism evidence strengthened: the 5,000-op corpus hashes identically on windows-gnu/rustc 1.97.1 and on the session-2 linux-gnu build — DP-A2 survives toolchain drift, not just target drift.

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
