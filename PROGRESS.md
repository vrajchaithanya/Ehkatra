# PROGRESS.md — Ehkatra Build Log

## Session 1 — 2026-08-07 (cloud build by Claude)

### DONE (with evidence)
- **Row 1 (partial): workspace** — `usk-types`, `usk-oplog`, `usk-state`, `ehkatra-cli`. All `#![no_std] + alloc` (kernel crates). `cargo build/test/clippy/fmt` green.
- **Row 2: op log** — closed payload taxonomy v1 (InsertRow/DeleteRow/InsertCol/DeleteCol/SetCell/ClearCell), canonical byte encoding (one valid encoding per op; NaN normalized), BLAKE3 op hash + order-independent canonical set hash. Evidence: `order_independence_basic`, `log_merge_idempotent_commutative`.
- **Row 3 (v1): order CRDT** — neighbor-anchored axis sequences with tombstones, deterministic concurrent-insert resolution, A1-independent identity order. Evidence: `concurrent_structural_edit_converges` (THE canonical case: Alice inserts row while Bob writes cells — converges, Bob's data lands on intended rows).
- **Cell registers** — LWW by (lamport, actor, counter) with **losers retained** (ADR-006). Evidence: `concurrent_cell_write_retains_loser`.
- **Convergence sweep** — 200 randomized interleavings of a 15-op history, all converge to one state hash. Evidence: `randomized_interleavings_converge`.
- **CLI demo** — `cargo run --bin ehkatra`: builds grid via two replicas with reversed op arrival, prints grid + both hashes, asserts convergence.

### DECISIONS TAKEN (recorded per CLAUDE.md rule 1)
- v0.1 axis CRDT is RGA-style neighbor-anchored (documented as interleaving-safe for single-level v0.1 use); full Fugue tree is Row-3-final before sync milestone. Rationale: proves the model with minimal code; upgrade is internal to `usk-state`.
- Custom canonical binary encoding now, CBOR wrapper later (encoding is behind `Op::encode`, swap is localized).

### NEXT (in BOOTSTRAP order)
Row 4 tiles → Row 5 Decimal/Date values → Row 6 formula engine (60 fns) → Row 7 dep graph/recalc → Row 8 identity-reference regression (partially proven already) → Row 9 reducer/commands/undo → Row 10 WS sync → Rows 11–15.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · tests 5/5 ✓ · no_std attrs in place (dedicated no_std CI check: TODO week 2) · wasm32 differential replay: TODO (wasm32 target not yet exercised — this is A-005-critical, schedule next session)

## Session 2 — 2026-08-07 (solo-model review + determinism gate)
- **DP-A2 CLOSED**: `tools/replay-check` — 5,000-op seeded corpus (3 actors, structural+cell ops) replays to bit-identical oplog and state hashes on x86_64-native AND wasm32-wasip1 (run under Node WASI). Hashes: oplog 77e5b1bf…, state 4516044e…
- **CI added**: `.github/workflows/ci.yml` — fmt, clippy -D warnings, tests, differential-replay gate, kernel std-leak grep, cfg-placement grep. Push to GitHub and it runs on every PR.
- **Solo operating model**: docs/07 — team-shaped assumptions redesigned as automation; complexity budget rules DP-S1..S4; full gap register + conservative scores (production readiness 5.5/10, rises row by row).
- Gates: fmt ✓ clippy ✓ tests 5/5 ✓ replay native=wasm ✓
- NEXT: Row 4 tile store (+A-001/A-002 measurement) · cargo-deny/audit CI job · then Row 5.

## Session 3 — 2026-08-07 (local Windows host: verification, toolchain, budgets)

Session opened on a machine where the repo had **no git history and no working Rust toolchain**. Verifying sessions 1–2 was therefore the whole first work unit, and it found real defects.

### VERIFICATION OF CLAIMED-DONE WORK (the "trust but verify" step)
Sessions 1–2 were built in a cloud environment. Re-run here from scratch:
- **tests 5/5 ✓** — `concurrent_structural_edit_converges`, `concurrent_cell_write_retains_loser`, `log_merge_idempotent_commutative`, `order_independence_basic`, `randomized_interleavings_converge`.
- **fmt ✓**
- **clippy ✗ → FIXED.** Two lints fired on rustc/clippy 1.97.1 that did not exist on the cloud's older `stable`: `needless_range_loop` in `crates/usk-state/tests/convergence.rs`, `explicit_counter_loop` in `tools/replay-check/src/main.rs`. Both rewritten; the replay corpus is unchanged (the counter was always `i + 1`) and **the hashes prove it** — see below.
- **differential replay ✓ and stronger than claimed.** The 5,000-op corpus produces `oplog:77e5b1bf…` / `state:4516044e…` here on **x86_64-pc-windows-gnu / rustc 1.97.1**, byte-identical to the session-2 record on **linux-gnu / older stable**. DP-A2 now has evidence across OS, libc, linker *and* compiler version — not just across targets.

### DEFECTS FOUND AND FIXED (root causes, not symptoms)
1. **Unpinned toolchain** — `channel = "stable"` meant a green repo went red with no code change. Pinned to `1.97.1` + components + `wasm32-wasip1` (D-036). This is the root cause of defect (1) above; fixing the lints alone would have left the trap armed.
2. **`Cargo.lock` was git-ignored** — a determinism claim with a hole in it. Now committed (D-037).
3. **No git repository** — `git init` + baseline commit this session. The handoff contract in CLAUDE.md assumes commits exist; they did not.
4. **No MSVC linker on the host, and installing one needs admin** — switched to the self-contained `x86_64-pc-windows-gnu` toolchain in `%USERPROFILE%\.rustup`, installed `--no-modify-path`. DP-S5 intact: no admin, no PATH edit, no service, no registry (D-038).
5. **DP-S2 was unenforced and its stated number was misleading** — "kernel ≤ 5 (currently 1: blake3)" counted direct edges; the real closure is 10. Both are now gated separately (D-035).

### ADDED (closing docs/07 gap-register rows by name)
- `tools/gates.ps1` — the entire gate set as **one command**, in CI order. Solo rule: a check that isn't one command doesn't get run.
- `tools/dep-budget.mjs` — DP-S2/D-035 complexity budget as a gate.
- `tools/run-wasi.mjs` — the WASI runner, now a real file instead of a CI heredoc, so the determinism gate is reproducible locally.
- `deny.toml` + `supply-chain` CI job (cargo-deny + cargo-audit) — closes the gap register's *"No dependency/supply-chain scanning — solo devs get owned via deps"*. Structurally denies postgres/sqlx/diesel clients (DP-S5) and openssl/ring (DP-B9 no custom crypto, one hashing stack).
- CI: host-isolation grep (docs/07 §6 asked for it explicitly), `no_std` kernel build against wasm32 (DP-A3's "dedicated no_std CI job pending" — now not pending), complexity-budget gate.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · tests 5/5 ✓ · no_std wasm32 kernel build ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓ — **all via `pwsh -File tools/gates.ps1`**

## Session 3 (cont.) — Row 4: tile store · DONE, and it failed an assumption

**Row 4 is built and proven** (`crates/usk-state/src/tile.rs`, 9 new tests in `crates/usk-state/tests/tiles.rs`, harness `tools/tile-bench`). `State` no longer keeps a flat `BTreeMap` of cells; cells live in 256×64 tiles with a presence bitmap, a payload packed dense over present cells (`f64` when type-uniform, tagged otherwise), and CRDT metadata that is a 24-byte causal summary until a contested cell forces promotion.

### What proves it
- `tile_store_matches_reference_semantics` — a randomized 3-actor corpus run through **both** the tile store and an independently-written flat reference model of ADR-006 semantics; they must agree on every winner and every retained loser. This is the test that makes the refactor a proof rather than a hope.
- `tiled_state_converges_under_reordering` · `co_located_authors_do_not_promote` · `contested_cell_promotes_its_tile` · `single_author_region_never_promotes` · `inserting_a_row_never_rekeys_existing_tiles` · `numeric_tiles_pack_tighter_than_mixed_tiles` · `cells_group_into_256x64_tiles` · `out_of_order_writes_keep_the_payload_dense`.
- The oplog hash is **unchanged** (`77e5b1bf…`) across the whole refactor — the op algebra did not move. The state hash **did** change by design (`4516044e…` → `e6cc2757…`): cells now fold in tile-major order, which is docs/10's tile-Merkle direction (D-040).

### THE HEADLINE: A-002 FAILED — read this before building Row 5
- **A-001 passes, single-author**: 10M numeric cells = **84.2 MB structural / 93.1 MB OS peak**, 8.425 B/cell, against a 400 MB budget. Matches docs/14's ~81 MB prediction.
- **A-002 fails**: the claim was *promotion <1% of cells*. Measured: **0.1% of cells contested promotes 25% (clustered) to 100% (scattered) of cells**. A tile is 16,384 cells and one contested cell promotes all of them.
- The two interact: under scattered contention memory goes 8.4 → **74.5 B/cell**, which puts 10M cells at **~745 MB — 1.9× over budget**. A-001 holds only for single-author workbooks.
- I first shipped a coarser predicate (any two actors writing the same *tile*) and measured 100% promotion everywhere; the per-cell predicate in the tree now is the improved version, and it *still* fails. So this is ADR-005's tile granularity under question, not an implementation bug.
- Consequence executed per docs/42: **A-002 → Failed**, tile-granularity redesign is a Q1 gate. Filed as TD-09 (+TD-10, TD-11) with the redesign options to weigh. Do not pick one without re-running `tools/tile-bench`.

### DECISIONS (docs/43)
ADR-034 stable identity→slot band keying (with the two rejected alternatives) · D-039 per-contested-cell promotion decided in a pre-pass, and the A-002 outcome · D-040 tile-major state hash.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 14/14 ✓** · no_std wasm32 kernel build ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓ — all via `pwsh -File tools/gates.ps1`

### NEXT AFTER ROW 4 (done — see session 4 below)
Row 5: values.

### ONE GATE IS ADDED BUT NOT YET PROVEN (don't report it as green)
The `supply-chain` CI job (cargo-deny + cargo-audit) is written and `deny.toml` is committed, but it **has never been executed**. `cargo install cargo-deny` fails on this host: the windows-gnu toolchain here has no `dlltool.exe`, which cargo-deny's own dependency tree needs. Our crates are unaffected — this is a limitation of building that third-party tool locally, not of the workspace. Don't burn time retrying it on Windows; it will first run for real on CI's `ubuntu-latest`.
What *was* verified by hand, since the licence check is the likeliest failure: all 10 dependencies are `BSD-2-Clause`, `MIT OR Apache-2.0`, `CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception`, or `CC0-1.0 OR MIT-0 OR Apache-2.0` — every one satisfiable from `deny.toml`'s allow list. Sources are crates.io only, and nothing in the `bans` deny list is present.

## Session 4 — Row 5: value lattice · DONE

**Row 5 is built and proven** (`crates/usk-types/src/decimal.rs`, `crates/usk-types/src/coerce.rs`, 24 new tests in `crates/usk-types/tests/values.rs`). 38 tests total, all gates green.

### What shipped
- **`Decimal`** — exact base-10 currency arithmetic: `{i128 coefficient, i16 exponent}`, canonically normalised, 38 significant digits, exact `+ − ×`, exact comparison, half-even division. Pure integer math, no float path, no panics (overflow → `None` → `#NUM!`).
- **`CellError { kind, origin }`** — errors carry *why*. `Origin` is `Authored | Coercion{from,to} | Arithmetic{op} | Propagated`, and it survives propagation through arithmetic, which is what makes "where did this `#VALUE!` come from" answerable.
- **`Profile::{Compat, Strict}`** — input coercion, arithmetic coercion, and `Number`/`Decimal` promotion. Promotion is lossless-only: a `Number` joins the exact domain solely when its *true* binary value fits in 38 decimal digits.
- **Packed decimal tiles** — `CellPack::Decimals`, the carry-forward item from Row 4.

### Evidence worth reading
- `gene_symbol_survives_strict_and_is_mangled_by_compat` — `"1E2"` becomes `Number(100)` under compat (Excel's real behaviour, the one that made the HUGO committee rename genes) and stays `Text` under strict. This is the row's reason for existing.
- `cent_reconciliation_has_no_phantom_pennies` — 100 × `0.01` is exactly `1`, where `f64` drifts.
- `float_to_decimal_conversion_refuses_inexact_values` — `0.1_f64` does **not** convert, because it is really `0.1000000000000000055…`. My first implementation used float scaling and wrongly claimed it did; conversion now works on the float's bits.
- `existing_value_encodings_are_byte_stable`, plus the unchanged replay-corpus hashes (`77e5b1bf…`, `e6cc2757…`) — proof the two new variants were genuinely additive, not merely intended to be.

### Two things I got wrong and fixed (both worth knowing)
1. **Excel's "15-digit quirk" is two rules.** A single 15-significant-digit rounding does not produce `=0.1+0.2-0.3 → 0`; that case is catastrophic cancellation and needs the *operand magnitude*, not just the result. Split into `compat_round_15` (display) and `compat_final_adjust` (evaluation). See D-041.
2. **Scale-round-unscale is wrong at the extremes.** `f64` holds powers of ten exactly only to `10^22`, so a built-up factor rounded `1e300` to `9.999999999999978e299`. `compat_round_15` is now a format-and-reparse round trip, which is exact *and* platform-identical (`core`'s float code is pure Rust and locale-free).

### Measured (MEASUREMENTS.md)
`size_of::<Value>()` grew 32 → 48 B (`i128` alignment), landing **only** on the tagged path. Per cell: `Number` 8.5 B · `Decimal` 32.5 B · `Text` 56.1 B — giving currency its own packed layout saves 42% over the tagged fallback. **A-001 did not regress**: the 10M-cell numeric figure is byte-identical at 8.425 B/cell.

### DECISIONS (docs/43)
ADR-035 `Decimal` is a scaled integer, explicitly *not* IEEE decimal128 (with both rejected alternatives) · D-041 the two compat rules and the unvalidated threshold · D-042 six of the lattice's variants ship now, the rest with the rows that need them.

### DEBT (docs/44)
TD-12 not-IEEE-decimal128 · TD-13 `compat_final_adjust`'s threshold is documentation-derived, not oracle-captured — it belongs in the first Excel COM capture batch (ADR-024, A-007).

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 38/38 ✓** · no_std wasm32 kernel build ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓ — all via `pwsh -File tools/gates.ps1`

### NEXT (unchanged BOOTSTRAP order)
**Row 6: formula engine** — lexer → Pratt parser → CST → AST → binder, and 60 functions (arith, logical, text, date core, SUM/AVERAGE/COUNT/MIN/MAX/IF/AND/OR/NOT/CONCAT/LEFT/RIGHT/MID/LEN/TRIM/UPPER/LOWER/ROUND family, VLOOKUP/XLOOKUP/INDEX/MATCH, SUMIF/COUNTIF/SUMIFS/COUNTIFS, IFERROR, TODAY/NOW as materialized volatiles). Proof: function conformance vectors + error-propagation tests.

Carry into Row 6:
- `coerce::arith` already provides binary arithmetic with profile-driven coercion, decimal promotion and error propagation. The evaluator should call it, not re-implement it.
- Date functions need a `Date`-vs-serial decision. docs/04 lists `Date`/`DateTime`/`Duration` as real types and the archive agrees ("dates are real types, not serial numbers"), with export mapping back to serials. Excel's 1900-leap-year fiction is a `compat` concern (docs/32) and belongs in the date↔serial conversion, not in the type.
- `Origin::Propagated` is the variant to extend with the source cell once formulas can reference one (D-042).
- Every new `Value` variant costs the tagged path 48 B/cell — re-run `tools/tile-bench` after adding any.
- Volatiles (`TODAY`/`NOW`) must be *materialized*, never read from an ambient clock: DP-A2 forbids ambient time in kernel paths, and ADR-009 puts them behind the Calculation Authority.

TD-09 (tile promotion granularity, A-002's failure) is still open and still not a prerequisite for Row 6 — but it must land before Q2.

## Session 5 — Row 6: formula engine · DONE

**Row 6 is built and proven** — new kernel crate `usk-formula` (`lexer.rs`, `parse.rs`, `eval.rs`, `functions.rs`) with 33 tests in `crates/usk-formula/tests/formulas.rs`. **71 tests total**, all gates green.

### What shipped
- **Pipeline**: `text → lexer → Pratt parser → lossless CST → AST → evaluator`, per docs/12.
- **Lossless CST (ADR-011)** — `Cst::text()` reproduces the input byte for byte, whitespace and all, including inputs the parser could not understand. Spans are retained for error carets.
- **Excel precedence**, quirks included: unary minus binds tighter than `^` (so `-2^2` is `4`), `^` is right-associative, `%` is postfix.
- **69 functions** (BOOTSTRAP asks for 60) across aggregation, rounding, logical, error/type predicates, text, lookup, conditional aggregation and date core.
- Evaluation is total: every malformed input, unknown name, bad reference and undefined operation is an error *value* carrying its origin (DP-A10).

### Evidence worth reading
- `cst_round_trips_every_input` — the ADR-011 property, over 13 inputs including unterminated strings and pure garbage.
- `unary_minus_binds_tighter_than_exponent` · `exponent_is_right_associative` — Excel's precedence, not mathematics'.
- `compat_reproduces_the_1900_leap_year_fiction` — `DAY(60)` is 29 February 1900 under compat and 1 March under strict; the profiles agree before the phantom day.
- `sum_of_decimals_stays_exact` — 100 × `0.01` sums to exactly `1` through `SUM`, and mixing in an inexact float honestly drops to the float domain.
- `errors_propagate_with_origin_intact` — a refused coercion five calls deep still knows what it was.
- `if_evaluates_lazily` — `IF(A1=0,0,1/A1)` does not divide by zero.
- `catalogue_covers_the_row_6_function_list` — every catalogue name actually dispatches, so the count is not padding.

### Three bugs found by the tests, all real
1. **CST trivia handling was wrong.** Buffering whitespace in the parser and attaching it to whichever node came next silently *moved* the user's spacing when a lookahead did not match — `"=  SUM( A1 : B2 , 3 )  "` came back as `"= SUM(   A1:  B2,  3)"`. Replaced with non-consuming lookahead, so trivia is emitted exactly once by whoever consumes the token after it. This is the ADR-011 property; a lossless tree that loses bytes is not lossless.
2. **The date epoch was off by one** — the constant held 1900-01-01 where the code wanted 1899-12-31, shifting every date by a day.
3. Excel's `MOD` takes the sign of the *divisor*, unlike Rust's `%`.

### DECISIONS (docs/43)
D-043 dates are serials for now, with `Value::Date` deferred to the format layer and a proven non-breaking upgrade path · D-044 approximate lookup refused rather than guessed · D-045 `match` dispatch instead of docs/12's declarative registry, with the reasons it is not yet load-bearing · D-046 in-crate `powf`/`sqrt` rather than libm, for DP-A2.

### DEBT (docs/44)
TD-14 exact-match-only lookup (a real compat gap for imported workbooks) · TD-15 fractional-exponent accuracy ~1e-12 · TD-16 range-to-scalar takes top-left instead of implicit intersection, pending Row 7.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 71/71 ✓** · no_std wasm32 kernel build (now including usk-formula) ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓

Both replay hashes unchanged (`77e5b1bf…`, `e6cc2757…`): Row 6 added a crate but touched no op encoding.

### NEXT (unchanged BOOTSTRAP order)
**Row 7: dependency graph** — formula groups, range edges via an interval index, incremental dirty → topo → parallel recalc. Proof: a 100k-cell recalc bench recorded in MEASUREMENTS.md (assumption A-003, budget <200 ms on 8 cores).

Carry into Row 7:
- `usk-formula` deliberately does **not** depend on `usk-state`; it reads through the `eval::Grid` port. Row 7 is where a real implementation over `State` belongs, and where A1 ordinals finally become identity intervals (DP-A6). Until then `Grid` speaks in view ordinals, which is the shortcut Row 8 removes.
- TD-16 (implicit intersection) needs the evaluating cell's position, which the dep graph supplies — close it there.
- `Ast::Name` currently evaluates to `#NAME?`; docs/12 wants unresolved names to live-rebind when the name appears, which needs the name table plus dep-graph invalidation.
- Volatiles are already injected via `Context::{today, now}`; Row 7 must make them dirty-marking so a recalc re-materialises them exactly once per pass (ADR-009).

## Session 6 — Row 7: dependency graph and recalculation · DONE

**Row 7 is built and proven** — new kernel crate `usk-calc` (`sheet.rs`, `graph.rs`) with 14 tests in `crates/usk-calc/tests/recalc.rs` and the A-003 harness `tools/calc-bench`. **85 tests total**, all gates green.

### What shipped
- **Formula groups** — nodes are R1C1 patterns, not cells. Measured: **10 nodes for 100,000 formula cells**.
- **Range-granular edges** — a group records the rectangles it reads; "who reads what I wrote" is a stab against a band-bucketed index, never a materialised cell-edge set.
- **Incremental recalc** — dirty *rectangles* propagate transitively with early cutoff, then topo **levels**, then evaluation of only the dirty members.
- **Cycle detection** → `#CIRC!`, read off the level assignment rather than a separate Tarjan pass.
- Volatiles read from the engine's materialised bindings (docs/13 T2), never a clock.

### MEASURED (MEASUREMENTS.md) — A-003 passes
| | Budget | Measured |
|---|---|---|
| Full recalc, 100k dependents | <200 ms on 8 cores | **53.0 ms on 1 core** |
| Single-edit incremental | <8 ms | **0.191 ms** (10 cells of 100,000; 278× faster than full) |

**The caveat that belongs next to the number**: this is single-threaded, so it clears a budget that *allowed* eight cores — a stronger result than asked for, but it does not validate A-003's "level-parallel via rayon" half, because rayon sits behind the unbuilt PAL `Compute` trait. Recorded as TD-17, and A-003 in docs/42 says "confirmed on the single-threaded path" rather than "confirmed".

### Two measured failures on the way — the reason the final numbers mean anything
1. **Grouping collapsed completely: 100,000 groups for 100,000 cells**, and the O(groups²) edge build then hung outright (killed after 10 minutes). All ten chained columns share one R1C1 pattern, so they formed one group whose reads overlapped its own writes and split to singletons. Fixed by partitioning a self-overlapping group **by column** first (D-047) and by building edges through the range index (D-049).
2. **Incremental recalc recomputed everything: 53 ms, 100,000 cells, 1× speed-up** — i.e. no incremental behaviour at all. Dirtiness was tracked per *group*, and a group is 10,000 cells. Fixed by carrying a dirty rectangle through marking and evaluation (D-048). That left a 10.5 ms residue re-walking 10,000 member ASTs per group; precomputing a per-member read bound took it to 0.191 ms.

Neither was visible from the tests — all 14 passed throughout. The bench found both.

### DECISIONS (docs/43)
D-047 self-overlapping groups partition by column before falling back to singletons · D-048 dirtiness is a rectangle inside a group, not the group · D-049 edges come from the range index, never pairwise comparison · D-050 cycles are read off the level assignment rather than a separate Tarjan pass.

### DEBT (docs/44)
TD-17 recalc is single-threaded (A-003's 8-core half unvalidated) · TD-18 `Engine::build` regroups the whole sheet, so *formula* edits are not yet incremental — only value edits · TD-19 graph build is 699 ms per 100k formulas, dominated by parsing · TD-20 the index is band-bucketed, not the R-tree docs/13 specifies.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 85/85 ✓** · no_std wasm32 kernel build (now including usk-calc) ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓

Both replay hashes unchanged (`77e5b1bf…`, `e6cc2757…`).

### NEXT (unchanged BOOTSTRAP order)
**Row 8: identity references** — insert/delete rows shifts ranges correctly, and the canonical test: a concurrent row-insert against `SUM(A1:A10)` converges. Proof: a dedicated regression plus a proptest.

Carry into Row 8:
- **This is the row that pays off `usk-calc`'s central shortcut.** `Rect` and `CellRef` are view *ordinals*; DP-A6 says references are identity intervals and A1 is a computed view. The seam was kept deliberately narrow — those two types in `sheet.rs`, plus `parse::A1` — so Row 8 is a substitution, not a rewrite.
- Row 8 also closes **TD-16** (implicit intersection needs the evaluating cell's position, which the engine now has) and gives lookup the sortedness contract **TD-14** waits on.
- `usk-state` already has the identity-interval machinery (`SlotMap`, `TileKey`); Row 8 is where `usk-calc` starts speaking it, which also means `usk-calc` finally depends on `usk-state` rather than carrying its own `Sheet`.
- The A-002 tile-promotion redesign (TD-09) is **still open** and must land before Q2.

