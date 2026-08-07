# Changelog — Architecture Repository

## 2026-08-07 (session 5) — BOOTSTRAP Row 6: formula engine
- New kernel crate **`usk-formula`**: `text → lexer → Pratt parser → lossless CST → AST → evaluator` (docs/12).
- **Lossless CST (ADR-011)**: `Cst::text()` reproduces the input byte for byte — whitespace, unterminated strings and unparseable garbage included — with spans retained for error carets.
- **Excel precedence including its quirks**: unary minus binds tighter than `^` (`-2^2` = 4), `^` right-associative, postfix `%`.
- **69 functions** (row 6 asks for 60): aggregation, rounding, logical, error/type predicates, text, lookup, conditional aggregation, date core. `SUM` stays in the exact decimal domain when every addend is exact.
- Evaluation is total — malformed input, unknown names, bad references and undefined arithmetic are all error *values* carrying their origin.
- Excel's 1900 leap-year fiction reproduced under `compat` and not under `strict`; volatiles (`TODAY`/`NOW`) injected via the evaluation context, never read from a clock (DP-A2, ADR-009).
- New decisions: D-043 (dates as serials for now, with a proven non-breaking path to `Value::Date`), D-044 (approximate lookup refused rather than guessed), D-045 (`match` dispatch instead of docs/12's declarative registry, deferred until it carries load), D-046 (in-crate `powf`/`sqrt` rather than libm, for bit-identical results).
- New debt: TD-14 exact-match-only lookup, TD-15 fractional-exponent accuracy, TD-16 implicit intersection pending Row 7.
- 33 new tests (71 total); both replay hashes unchanged.

## 2026-08-07 (session 4) — BOOTSTRAP Row 5: value lattice
- **`Decimal`**: exact base-10 currency arithmetic — 128-bit coefficient + base-10 exponent, canonically normalised, 38 significant digits, exact `+ − ×` and comparison, half-even division, no float path, no panics. `0.1 + 0.2` is exactly `0.3`; 100 cents is exactly 1.
- **Error provenance**: `Value::Error` now carries `CellError { kind, origin }`, where `Origin` distinguishes an authored error from a refused coercion, undefined arithmetic, or propagation — and the origin survives propagation through arithmetic.
- **`Profile::{Compat, Strict}`**: Excel's coercion rules including the gene-symbol mangling (`"1E2"` → `100`) versus no-silent-conversion; `Number`/`Decimal` promotion that only promotes when lossless; both Excel 15-digit rules (`compat_round_15` for display, `compat_final_adjust` for cancellation).
- **Packed decimal tiles** (`CellPack::Decimals`): 32.5 B/cell against 56.1 for the tagged fallback.
- Encoding stayed additive: tags `0x00`–`0x05` are byte-identical and both replay-corpus hashes are unchanged. `size_of::<Value>()` 32 → 48 B, on the tagged path only; A-001's numeric figure did not move.
- New decisions: ADR-035 (`Decimal` is a scaled integer, explicitly not IEEE 754-2008 decimal128, with alternatives), D-041 (Excel's "15-digit quirk" is two rules; the cancellation threshold is documentation-derived, not oracle-captured), D-042 (Row 5 ships six lattice variants; the rest land with the rows needing them).
- New debt: TD-12 not-IEEE-decimal128, TD-13 unvalidated compat threshold.
- 24 new tests (38 total), all gates green.

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
