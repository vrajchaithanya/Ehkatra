# 12 — Formula Engine
Status: Approved · Owner: Principal Engineer · Normative: yes · Carved from SPEC §8–9

## Pipeline
`text → lexer → Pratt parser → lossless CST → AST → binder → analyzer → typed plan → interpreter` (JIT slot reserved, unscheduled). The CST is retained: refactoring (rename table column rewrites formulas preserving whitespace), precise error carets, AI explanation anchoring. Binder resolves text to identities (`A1` → ids under current view; `Table1[Amt]` → through column map; unknown names → `#NAME?` thunks that live-rebind).

## Value & coercion rules
Lattice per docs/04. `Decimal128` for currency-formatted data (exact base-10). Coercion: `compat` mode = Excel's rules exactly (imported workbooks); `strict` = no silent coercion, `#VALUE!` with trace (native workbooks). Enumerated Excel bug catalog reproduced under `compat` only: 1900 leap-year, 15-digit display rounding, 1900/1904 date systems, SUM accumulation order. Number formatting is a pure display function (full Excel format-code grammar); never feeds back into values.

## Function architecture
Declarative registration: arity, per-arg type/coercion class, volatility tier, spill shape, aggregation identity, i18n block (storage = canonical English; display localizes). Catalog: **Core-200** (H1) → **Extended-250** (H2) → compat tail by measured demand. Conformance = oracle vectors captured from real Excel via COM (ADR-024): the binary is the spec. UDFs: named LAMBDAs (pure) → WASM plugin functions (capability-scoped, deterministic-mode; declared-volatile ones execute at the Calculation Authority) → connector functions (H2).

## Dynamic arrays
Spill = derived overlay (no stored cells, no ownership conflicts, `#SPILL!` computed and self-healing, convergent under concurrency). `A1#` binds to current extent via extent-edges in the dep graph; `@` implicit intersection for compat; legacy CSE arrays import under a fixed-extent flag. `LET`/`LAMBDA` + helpers (`MAP/REDUCE/SCAN/BYROW/BYCOL/MAKEARRAY`); closures capture by value snapshot (deterministic, explainable — ADR-012).
