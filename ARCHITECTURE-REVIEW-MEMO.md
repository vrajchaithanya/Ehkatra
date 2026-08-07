# Architecture Review Board — Consolidation Memo 001

**Date:** 2026-08-07 · **Scope:** all five pre-existing documents · **Outcome:** repository restructure + 4 new ADRs

## Documents reviewed

| Document | Status after review |
|---|---|
| ARCHITECTURE-REVIEW.md (suite-level multi-role review) | **Historical.** Suite-level; its surviving judgments are imported into the risk/decision registers. |
| DESIGN-V2-HARD-PROBLEMS.md (suite kernel: 5 questions, security, text/i18n) | **Partially normative.** Kernel decisions (ops-as-truth, undo, recalc tiers, versioning, MLS, parser sandbox) imported into module docs. Text-layout/i18n sections apply to the future document product, out of scope for the grid; archived. |
| DOC-GRID-DESIGN.md | **Superseded** by GRID-ARCHITECTURE-SPEC.md (its own §1 says so). Archived. |
| GRID-ARCHITECTURE-SPEC.md | **Primary technical source.** Carved into per-module docs (docs/10–24); the monolith is retained read-only as SPEC-ARCHIVE until carving is complete, then archived. No module doc may contradict it without an ADR. |
| QUARTER-PLAN.md | **Active.** Becomes docs/40-roadmap Phase Q1; desktop-shell work added to Q2. |

## Contradictions detected and resolved

**C1 — Platform priority (material).** All prior documents assume web-first (PWA + WASM primary; Tauri shell). The new mandate is desktop-first Windows/macOS, web/mobile future. *Resolution — ADR-027:* desktop-first. The damage is smaller than it appears: the renderer was already a platform-agnostic scene graph on wgpu (not DOM), the kernel was already native-compiled, and the hybrid a11y/IME design maps directly onto UIA/NSAccessibility via accesskit. What actually changes: the shell is a native windowed app (winit + platform adapters), the browser demotes from primary target to CI-verified determinism target, PWA/OPFS storage paths demote to future work, and packaging/update/signing docs (34) are new first-class work. wasm32 stays in the differential-replay gate permanently — cheap insurance that the web target stays reachable.

**C2 — Monolith vs. repository.** A 32-section spec cannot be owned by multiple teams. *Resolution — ADR-029:* docs-as-code repo; one normative doc per concern; the spec is carved, not duplicated; every doc has an owner role and a status header.

**C3 — Ambition vs. quarter buildability.** The spec's embedded/riscv64 profile and distributed calc fleet conflict with the one-quarter evidence mandate and the simplicity principle. *Resolution — ADR-030:* embedded demotes from supported target to *preserved discipline* (the `no_std` CI check remains — it enforces kernel purity, which desktop needs anyway); distributed fleet calc moves to Horizon 3 of the roadmap; the Q1 walking skeleton + proof rig is unchanged and is the evidence engine.

**C4 — E2EE tiers vs. desktop-first enterprise scope.** MLS/E2EE design is sound but pulls a full quarter of specialist work ahead of any customer. *Resolution:* Standard tier only through GA; Managed-E2EE design retained as approved-but-unscheduled (decision register D-021), with the one irreversible prerequisite kept now: op payloads are already encryption-agnostic framed.

## Drift detected

**D1 — Asserted numbers.** §7/§28 figures are targets presented as facts. Tracked as assumptions A-001…A-006 with Q1 validation gates; the scorecard's Evidence Maturity dimension exists to keep this honest. **D2 — ADRs written post-hoc.** The 26 spec ADRs are rationalizations; accepted as baseline but the register now requires live ADRs for every future decision. **D3 — Terminology.** "OpenSuite/doc-grid/gridkernel" used inconsistently; glossary fixes: platform = **Grid Platform**, kernel = **USK**, product name TBD (D-030).

## Overlaps consolidated

Undo appeared in 3 documents (kernel design, grid design, spec §13) → docs/13 owns calc, docs/15 owns sync, docs/11 owns undo; MCP appeared in 4 → docs/21 owns it; performance budgets in 3 → docs/31 owns the single table, all others link.
