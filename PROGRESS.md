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

## Session 7 — Row 8: identity references · DONE

**Row 8 is built and proven** — `crates/usk-calc/src/refs.rs` plus `full_row_order`/`full_col_order` on `State`, with 11 tests in `crates/usk-calc/tests/identity_refs.rs`. **96 tests total**, all gates green.

### What shipped
- **`IdRange`** — a reference is a pair of endpoint *identities* with an `AnchorMode`. There is no position anywhere in the type; that is the point (DP-A6).
- **`Binder`** — A1 view ordinals become identities once, at bind time (docs/04 invariant 3).
- **`Axis`** — the axis order **including tombstones**, which is what makes "re-anchor inward" answerable.
- **`StateGrid`** — reads a `State` through the formula engine's `Grid` port, identity-first.

### THE canonical test passes
`concurrent_row_insert_against_sum_converges`: Alice inserts a row inside the span of `SUM(A1:A10)` while Bob overwrites cells in it, concurrently, and the four ops arrive at two replicas in opposite orders. Both replicas reach the **same state hash**, the reference resolves to the **same eleven rows** on both, and the sum agrees (1145). Nothing rewrote the formula.

The five docs/11 shift rules each have their own test, and all five fall out of one resolution rule rather than being implemented separately: insert above leaves the span · insert inside extends it · insert below stays outside · delete inside shrinks it · delete an endpoint re-anchors inward · everything deleted is `#REF!`.

### Property coverage
- `resolution_is_always_a_contiguous_live_run` — 200 seeded insert/delete sequences; a resolved reference is always a contiguous run of live rows in axis order, never containing a tombstone.
- `reference_resolution_is_arrival_order_independent` — 60 shuffled arrival orders of the same op set, identical resolution and identical answer every time.

### One thing I got wrong and fixed
The first `Axis` carried the endpoints' **bind-time ordinals** as a hint for re-anchoring. That is wrong: ordinals shift under later edits, so the hint quietly decays into a lie. Exposing the tombstoned order from `usk-state` removed the hint entirely — the deleted endpoint still marks where the interval reached, and the live order alone has forgotten it.

### DECISIONS (docs/43)
D-051 references resolve against the tombstone-retaining order (with the rejected ordinal-hint approach) · D-052 seeded LCG sweeps instead of `proptest`, recorded as a deviation from a stack decision with its reasoning and revisit trigger · D-053 `usk-calc`'s ordinal `Sheet` coexists with the identity path for now.

### DEBT (docs/44)
TD-21 two addressing models in one crate — resolve at Row 9.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 96/96 ✓** · no_std wasm32 kernel build ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓

Both replay hashes unchanged (`77e5b1bf…`, `e6cc2757…`) — Row 8 added a read path and an accessor, and moved no op encoding.

### NEXT (unchanged BOOTSTRAP order)
**Row 9: reducer + commands** — `set_value`/`set_formula`/`insert`/`delete` rows-cols/`clear`/`undo`/`redo`, with per-actor labeled undo groups. Proof: undo-law tests (undo∘do = id on the actor's own scope).

Carry into Row 9:
- The reducer is `reduce_vN(Command, &Snapshot) → Vec<Op>`, **pure and versioned**, and remote replicas never see Commands (ADR-001, DP-A7). It is also where copy/fill rewrites relative anchors — `AnchorMode` is already carried on `IdRange` for exactly that.
- Selective undo is *inverse against current state*, not a stack replay (docs/11): registers restore only if the actor's own write still wins, and structural undo narrows rather than destroys others' work (DP-A12).
- This is the row that closes **TD-21** (two addressing models) and **TD-18** (no incremental regrouping) — both are waiting on edits becoming ops.
- TD-09 (tile promotion granularity, A-002's failure) is **still open** and must land before Q2.

## Session 8 — Owner corrections applied (D-054), then Row 9

Five corrections from the owner, applied before starting Row 9:

1. **TD-09 reprioritised** — A-002's consequence had drifted four rows past its failure, violating DP-F5. TD-09 is now the next work unit after Row 9, and **Row 10 may not start until TD-09 is closed with a re-run A-002 measurement**. Registers updated (docs/42, docs/44).
2. **Row 9 exit criteria hardened** — Row 9 is not done until `usk-calc`'s ordinal `Sheet`/`Rect` addressing path is deleted (TD-21) and the regrouping trigger is wired (TD-18).
3. **Supply-chain gate** — verified the cargo-deny/cargo-audit job has been in `.github/workflows/ci.yml` since session 3. The reason it has never run: **this repository has no remote and has never been pushed**; all of CI is equally dead, not just this job. Remote now configured (`origin` → the CLAUDE.md repo URL). **Standing instruction: on the first push, check the Actions run and record the supply-chain job's first green here.** The push is the owner's call — nothing has been pushed.
4. **TD-17 trigger** — PAL `Compute` + rayon bench only when a real workload breaches 200 ms single-threaded; A-003 stays "confirmed (single-threaded path)".
5. **D-052 ratified** — LCG sweeps stay; proptest is not added. CLAUDE.md stack list updated to match (DP-F3).

## Session 8 (cont.) — Row 9 core built · Row 9 IN-PROGRESS (exit criteria pending)

**Built and proven** — op-log extension + new kernel crate `usk-reduce`, 13 tests in `crates/usk-reduce/tests/undo.rs`. **109 tests total**, all gates green, both replay hashes unchanged (tags 0x16–0x18 additive, proven).

### What shipped
- **Ops**: `SetFormula { source, bindings }` (identity bindings bound once at the author, D-055), `UndeleteRow`/`UndeleteCol` (selective undo of deletes, DP-A5's new-behavior-new-op rule).
- **State**: formula registry (flat identity-keyed map per docs/14), `formula()` accessor, undelete, hash extended only-when-formulas-exist.
- **`usk-reduce`**: `Command` vocabulary (DP-D1) → pure versioned `reduce_v1` (DP-A7) → ops; `Session` with per-actor undo/redo stacks.
- **Selective undo (DP-A12)**, all proven: value/formula restore only while my actor's write still wins; insert-undo blocked when others wrote into the row (`ApplyReport::blocked` surfaces it); delete-undo resurrects the row *with its cells*; redo is undo-of-undo through one synthesis path; undo∘do = id on the projection for every command kind.

### Fixed while building
The winner check first compared the group's specific op id and wrongly blocked undoing older groups after a newer undo restored through a fresh (still mine) op id. docs/11's "own write still wins" is actor-scoped; with LIFO undo that is exactly correct (D-057).

### ROW 9 IS NOT DONE — exit criteria per D-054 remain
1. **TD-21**: delete `usk-calc`'s ordinal addressing (`Sheet`/`Rect` path); engine reads formulas from `State`'s registry, members keyed by cell identity, geometry derived from the `Binder` per rebuild (A1 = computed view, DP-A6). `parse::A1`+`RangeBinding` substitution at eval time via the resolution machinery from Row 8.
2. **TD-18**: regroup on structural/formula ops arriving (value ops → incremental `recalc_after` as today).
3. Re-run `tools/calc-bench` after the rewrite — A-003's numbers must be re-measured over the identity path, not assumed to carry over. Then update this file + MEASUREMENTS.md and declare Row 9 done.

### THEN, per D-054 (hard order)
**TD-09** (tile promotion granularity, the A-002 failure) is the next work unit after Row 9 — **Row 10 may not start until TD-09 closes with a re-run A-002 measurement.**

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 109/109 ✓** · no_std wasm32 kernel build (incl. usk-reduce) ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓

## Session 9 — new specs absorbed · **Row 9 DONE** (all exit criteria met)

Re-read docs/00 and the eight new normative docs (08, 25, 26, 27, 28, 29, 37, 38) before resuming. Gates verified green first, per protocol.

### Deltas found against the new specs
- **docs/29 violated — and worse than reported.** The rule "a new op type joins the replay-check generator" had been broken by Row 9, and auditing found `ClearCell` and `Value::Decimal` were *never* covered. The DP-A2 gate had been green over 4 of 9 payload variants. Fixed; corpus now covers all nine. **Reference hashes changed** — `oplog:ef7933e8…`, `state:5dbb01c2…`, native==wasm32. Old values recorded beside them.
- **docs/38 applied**: MEASUREMENTS.md now carries a reference-machine block (M1) and `W-*` workload ids; pre-docs/38 numbers are marked *unspecified workload* and are not quotable.
- **docs/27 §3**: generation mark added (`Engine::generation()`), tested.
- **docs/27 §5**: undo-machine transition-coverage + forbidden-transition tests added.
- **docs/26, docs/28**: no delta — identity encodings and error-domain separation already match.

### ROW 9 EXIT CRITERIA — all met
1. **TD-21 CLOSED.** `Sheet`/`Cell`/`CellRef` deleted; `Engine` works over `State`, addressing cells by `(RowId, ColId)`. References are **rebound from stored identity bindings on every rebuild**, so structural edits never rewrite formula text. `Rect` survives only as derived index geometry, rebuilt per regroup and documented as such (D-058).
2. **TD-18 CLOSED.** `Engine::observe(state, ops)` routes structural/formula ops to a regroup and value ops to the incremental path; `Session` feeds it every batch (D-059).
3. **W-CHAIN-100K re-measured over the identity path** — and it did *not* carry over: full recalc **53.0 → 92.6 ms (+75%)**, single edit 0.191 → 0.328 ms. Both budgets still pass (200 ms / 8 ms). Cause measured: identity-keyed `BTreeMap` vs `Vec` indexing. Filed as **TD-23** per docs/38's regression policy.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 118/118 ✓** · no_std wasm32 kernel build ✓ · dep budget 1/5, 10/12, 10/40 ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓

### NEXT — TD-09, per D-054 (hard gate)
**TD-09 is the next work unit, and Row 10 may not start until it closes with A-002 re-measured under docs/38's W-TILE-10M.** W-TILE-10M: 10M numeric cells written by 1 actor (import), then a 3-actor storm at 1% cell overlap (collab), then 50% (adversarial). Measures RSS at load, bytes/cell, promotion rate per pattern, compaction ratio. **A-002 pass bar: <1% promotion at the collab pattern.**

## Session 9 (cont.) — **TD-09 CLOSED**; Row 10's gate is satisfied

**The fix (D-061).** Promotion is per contested **cell**, not per tile. `Meta::Promoted(tile)` became `Meta::Mixed { frontier, stamps }`, and the pre-pass returns a per-tile *bitmap* of contested indices rather than a boolean — so an uncontested cell inside a mixed tile stays on the summary path. Chose per-cell over sub-tile blocks because a block size only moves the amplification constant; cells remove it.

**Measured under W-TILE-10M** (docs/38), 10M cells, M1:

| Pattern | B/cell | Promoted | RSS |
|---|---|---|---|
| import (1 actor) | 8.43 | 0.000% | 90.0 MB |
| collab (3 actors, 1% overlap) | **11.09** | **1.000%** | **123.6 MB** |
| adversarial (3 actors, 50%) | 137.56 | 50.000% | 1,709.0 MB |

- **Amplification 16,384× → 1×.** Promoted cells now equal contested cells exactly.
- **A-001 restored under collaboration**: 123.6 MB against a 400 MB budget, where the same load previously extrapolated to ~745 MB and failed.
- **A-001 fails at the adversarial pattern** (1.7 GB). That is 5M genuinely contested cells, not amplification; docs/38 sets no bar there. Recorded, not smoothed.
- Fixed along the way: the tile's causal frontier was not advancing on contested writes, which would have made a tile look stale to anti-entropy (docs/15).

### ⚠ A-002's bar cannot be met as written — OWNER DECISION NEEDED (D-062)
docs/38 says *<1% promotion at the collab pattern*; the collab pattern contests 1% of cells **by definition**; a contested cell must carry metadata (ADR-006). So promoted ≥ contested = 1% for *any* correct implementation, and the measured 1.000% is the **floor, not a near-miss**.

The intent (no amplification) is met and measured. The bar should be restated in amplification terms — e.g. "promoted ≤ contested", or "<1% promotion at ≤0.1% contested". **I did not change docs/38**: it is normative, and loosening a bar I just measured myself against is not the implementer's call.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 118/118 ✓** · no_std wasm32 kernel build ✓ · dep budget ✓ · differential replay native==wasm ✓ · purity + host-isolation greps ✓

### NEXT — Row 10 (sync), now unblocked
D-054's hard gate is satisfied: TD-09 is closed with A-002 re-measured under W-TILE-10M.

Row 10 must implement **docs/27 §1's replica-sync state machine exactly**, including its forbidden-transition tests: no OPS before HelloAck; remote ops failing schema/bounds validation are rejected-and-reported while staying LIVE; queued local ops are never dropped in any transition. Workload **W-SYNC-RELAY** (docs/38): 2 and 50 replicas, one relay, 10 ops/s each for 60 s, 1% packet loss; measures propagation p95, convergence time after last op, queued-op durability across a mid-run kill.

Carry into Row 10:
- **TD-22 first**: formula-vs-value LWW currently rides on full-replay order. Sync brings incremental apply, which needs per-entry stamps in the formula registry. Do this *before* the first incremental apply path exists, not after.
- docs/37 boundary 2 (collaborator → op applier) binds: schema+bounds validation on receive (DP-E4), poison-op quarantine, per-actor rate/byte buckets at the relay.
- `Session::integrate` is the seam; it currently re-replays the whole log (v0.1 cost, noted at Row 9).

## Session 10 — D-062 closed · **TD-22 closed** · **Row 10 (sync) DONE**

Opened by verifying session 9's claims from scratch: `tools/gates.ps1` green,
118/118 tests, replay hashes `ef7933e8…` / `5dbb01c2…` identical native and
wasm32 over the extended 9-variant corpus. Nothing needed repair.

### D-062 closed (owner ruling absorbed)
docs/38 now states A-002's bar in amplification terms (*promoted ÷ contested ≤
1.5*, **and** collab RSS ≤ 400 MB); docs/42 records A-002 **Confirmed**. The
measured implementation is 1.0× and 123.6 MB — passes both halves. **No code
changed**: the ruling restated the bar, not the requirement, and the requirement
was already met. D-062 carries the closure note; the escalation itself stays on
record as the precedent (surface the conflict, do not self-amend the bar).

### TD-22 closed — the formula registry is stamped (D-063)
`crates/usk-state/src/formula.rs`. Every mutation is `max` over
`(lamport, op id)` with the winner's payload kept, so the registry is a function
of the op *set*, not of arrival order. Proven over **all 120 permutations** of a
five-write mixed history (`registry_is_order_independent`), plus idempotence
under redelivery and lamport-tie determinism.

The design problem worth knowing: order-independence needs a *value* write to
leave a stamp, and a stamp per written cell is exactly the per-cell metadata
ADR-005 exists to avoid (~480 MB at 10M cells). So entries are **seeded by the
replay pre-pass** — the same traversal that decides tile promotion now also
collects the cells any `SetFormula` names. One pass, one timing constraint, the
promotion argument applied twice. Shadowed entries are skipped by `iter()`, so
the state hash is byte-identical to the pre-TD-22 behaviour, which the unchanged
replay hashes prove.

**What TD-22 did not close, and is now the only remaining barrier:** the tile
store and the axis tombstone set are still order-dependent, so `State` is not
incrementally appliable and `Session::refresh` still re-folds the whole log.
Filed as **TD-24** with a measured cost (below).

### Row 10 — sync
New kernel crate **`usk-sync`** (`machine.rs`, `queue.rs`, `clock.rs`,
`validate.rs`, `relay.rs`) and new shell crate **`ehkatra-relay`**
(`frame.rs`, `replica.rs`, `endpoint.rs`, `bus.rs`, relay + peer binaries).
`usk-sync` is `no_std`, performs **no I/O and reads no clock** — time arrives as
an event, jitter as an injected seed, and the shell does I/O with the `Action`s
the machine returns. That split is what makes partitions, loss, reordering,
hostile ops and mid-run kills ordinary tests.

**docs/27 §1 implemented exactly.** Every listed transition is exercised by name
in `sync_machine_covers_every_listed_transition`, and each forbidden line has
its own test:
1. *no OPS before HelloAck* — `no_ops_reach_the_wire_before_hello_ack` drives
   all five pre-ack states and asserts nothing reaches the wire and everything
   is queued; `offline_edits_flush_exactly_when_the_session_goes_live` proves
   the backlog releases on the SYNCING→LIVE edge and not one transition earlier.
2. *never apply an unvalidated remote op* —
   `hostile_remote_ops_are_quarantined_and_the_session_stays_live` refuses six
   distinct attacks (zero counter, zero lamport, saturating lamport, oversize
   formula, oversize text, malformed binding), applies the honest op in the same
   batch, and asserts the session stays LIVE.
3. *never drop a queued local op* — `queued_local_ops_survive_every_transition`
   drives a script visiting every state and checks all three ops after each
   step; `only_an_acknowledgement_empties_the_queue` proves the converse.
An unlisted pair trips the `debug_assert` docs/27 asks for, proven by a
`#[should_panic]` test.

**Four spec gaps found and filed, not invented (D-064):** transport loss during
HELLO_SENT or BACKOFF, a duplicate `InSync` after LIVE, `Give` during LIVE, and
`Ack` outside LIVE/SYNCING are pairs docs/27 §1 does not define but a real link
produces. The machine implements only what is listed; the **shell** refuses to
hand it an undefined pair. Recommendation to the owner: docs/27 §1 should gain
those four edges.

**Also shipped:** `Op::decode` (D-066) — BOOTSTRAP row 2's proof line claimed
"encode/decode round-trip tests" and there was no decoder. Now total (every
malformed input is a named error, never a panic), with every truncation of every
variant tested and non-canonical decimals repaired on decode so a hostile peer
cannot smuggle a second spelling past DP-A4.

**Two-terminal demo, real TCP:** `demo/collab.ps1` and `demo/collab.sh`. Relay
on loopback 7423 (fail-fast if taken), two peers, 13 + 6 concurrent edits
including a row inserted into the span another replica is writing. Both reach
19 ops and the **same state hash** —
`2f51c7f3c193b4908e76e27ae36a59ecd9c72514ee324185d02cb25d336c71c8`.

### Three defects found by measurement, not by tests
1. **Recovery re-minted spent op ids (D-067).** A replica rebuilt from its
   durable log kept counting from 0; two ops sharing one `(actor, counter)` made
   dedup discard the second, and the replicas diverged. Found by the *first*
   W-SYNC-RELAY run while seven convergence tests passed — only the benchmark
   restarts a replica that has already authored work.
2. **A replica that lost its link mid-handshake wedged forever (D-064
   addendum).** The 50-replica run diverged and its mid-run kill delivered
   **1 of 117** queued ops. docs/27 §1 defines no transition for loss during
   HELLO_SENT or BACKOFF; the shell's first answer was to do nothing, with a
   comment claiming an existing backoff timer would drive the next attempt.
   There is no such timer once a retry has fired, so with 4,051 dropped frames
   replicas piled up in HELLO_SENT with nothing to wake them.
   `Replica::hard_reset` now builds the fresh DISCONNECTED session that comment
   had promised — carrying the durable log and every unacknowledged op across —
   and `Replica::resume` reconnects by asking the state which transition is
   listed. **The lesson is about the comment:** a documented remedy no test
   exercises is a claim, and this one was false until the protocol was pushed
   hard enough to need it.
3. **The demo overstated itself.** Bob's edits were all silently refused
   (`OutOfRange` — he had no grid yet) while the output still printed "6 local
   edits applied", and the hashes matched because both replicas held only
   Alice's ops. The peer now waits for the structure it needs and reports
   *applied* vs *refused*. A demo that cannot fail is not evidence.

All three share a shape: **scale and duration are test inputs, not just
performance inputs.** Seven convergence tests passed through every one of them.

### MEASURED — W-SYNC-RELAY (MEASUREMENTS.md)
2 replicas, 1,200 ops, 1% loss: propagation p50 **200** / p95 **1,600** bus-ms;
convergence after last op **10** bus-ms; **all replicas equal**; mid-run kill
**32 ops queued at death, 32 delivered after recovery**; 0 quarantined.

50 replicas, 30,000 ops, 4,558 dropped frames / 2,085 reconnects — **passes**.
Four runs; only the last is quotable, because it is the only one produced by the
code in the tree:

| | Run 1 (pre-fix) | Run 2 (old harness) | Run 3 (real partition) | **Run 4 (authoritative)** |
|---|---|---|---|---|
| All replicas equal | **NO — DIVERGED** | YES | YES | **YES** |
| Convergence after last op | 100,000 bus-ms † | 2,530 | 2,140 | **2,140 bus-ms** |
| Propagation p50 / p95 | 1,000 / 7,500 | 1,600 / 5,900 | 1,600 / 5,800 | **800 / 3,700 bus-ms** ¶ |
| Mid-run kill | 117 queued, **1 delivered** | 2 / 2 ‡ | 45 / 45 | **45 queued, 45 delivered** |
| Wall time | 32 min | 83 min | 120 min | **6.8 min** |

† the settle budget running out, not a measurement — it never converged.
‡ hollow: the victim rejoined before the kill (the harness defect above).
¶ a corrected measurement, not a speed-up — see TD-24 below.

## TD-24 — **largely paid** (D-071, D-072)

**120 min → 6.8 min, 17.6×**, with dropped frames, reconnects, convergence, the
mid-run kill and **every state hash bit-identical** to run 3. That identity is
the proof the change is a scheduling change, not a semantic one.

**The fix was not the obvious one.** "Make `State` incrementally appliable" is
genuinely hard — a summary tile keeps no per-cell stamps to compare against,
promotion must be decided before the first write lands, and the only known route
(a resident per-tile per-actor writer index) costs 2 KiB per (tile, actor) and
would be ruinous on a sparse workbook. The cheaper observation is that the
problem was never the fold, it was the **frequency**: DP-A9 already says caches
are watermarked folds, and `Session` was eagerly re-folding on every append.
Appends now record the batch; `settle()` takes the fold on read. That is why
`state()`, `value()` and `engine()` take `&mut self` — the signature is the
point, saying "reading may cost you a fold", and making stale reads impossible
rather than merely discouraged. Under sync, reads are ~50× rarer than appends
(600 local edits vs ~30,000 delivered batches per replica), and that ratio *is*
the speed-up.

**Half the old number was the harness measuring itself (D-072).** `Bus` called
`Replica::log()` — which deep-clones every op — once per delivered frame. At
30,000 ops the instrumentation cost more than the system under test, so the
pre-fix wall-clock figures **overstated the product's cost**. Kept in the table
rather than quietly replaced.

**And it corrected a measurement, not just a speed.** Propagation was counted by
scanning the receiving replica's log, so an op held in the causal-gap buffer —
arrived, not yet applied — was counted again on redelivery with a later
timestamp, inflating the tail. p50 1,600 → 800 and p95 5,800 → 3,700 are that
correction.

**Residual, honestly:** a read after N appends is still O(N). Sync is
append-heavy so it wins; a UI repainting once per op would not. Row 11's
snapshot remains the named fix.

**A fourth defect, this one in the harness.** The mid-run kill caught only 2
queued ops here against 32 at 2 replicas, because "offline" was modelled as one
transport loss plus a long retry timer — which under 1% loss is not offline: the
next dropped frame ran the teardown-and-reconnect path, armed a fresh 500 ms
timer, and the victim rejoined and drained before the kill. **The durability row
measured nothing while looking like a result**, which is the worst failure mode a
measurement has. The bus now has a real `partition`/`heal` pair: no frames cross
in either direction, no timer brings the replica back. Pinned by
`a_partitioned_replica_stays_offline_and_keeps_its_queue`, deliberately run at 5%
loss to reproduce the old failure. Numbers below are from the fixed harness.

Writing `heal` produced a *fifth* instance of the same class: it called
`connect`, which is only listed from DISCONNECTED, on a replica sitting in
BACKOFF. The machine's `debug_assert` caught it on the first run of the new test.
That assert has now caught two shell bugs, which is a reasonable argument for
transcribing a specification exactly rather than widening it until nothing is
unlisted (D-064).

### DECISIONS (docs/43)
D-062 closed · D-063 stamped formula registry seeded by the pre-pass ·
D-064 docs/27 §1's four undefined edges, implemented exactly and filed ·
D-065 loopback TCP frames instead of WebSocket, with the dependency-budget
arithmetic · D-066 `Op::decode` and the overstated Row-2 proof line ·
D-067 recovery adopts its own durable counter.

### DEBT (docs/44)
TD-22 **PAID**. New: TD-24 `State` is not incrementally appliable (with the
measured cost) · TD-25 unknown op tags cannot be preserved-opaque without a
framed encoding, so DP-A5 is unimplementable on the wire until then — **must be
paid before the first wire-version bump** · TD-26 anti-entropy is queue/retention
based, not Merkle-guided · TD-27 transport is TCP, not WebSocket.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 164/164 ✓** · no_std wasm32 kernel build
(now including `usk-sync`) ✓ · dep budget 1/5, 10/12, **10/40** ✓ (the two new
crates added **zero** dependencies) · differential replay native==wasm ✓ ·
purity + host-isolation greps ✓ · **new gate: loopback-only listeners (DP-S5)**,
added because Row 10 introduced the project's first listening socket.

Both replay hashes unchanged (`ef7933e8…`, `5dbb01c2…`) — Row 10 added two
crates, a decoder and a stamped registry, and moved no op encoding.

### ROW 11 — feasibility checked first, and it found a blocker (D-068)

Before writing any Row 11 code I tried to build the ADR-031 container. **SQLite
cannot be compiled on this host.** `rusqlite`'s only viable route is `bundled`,
which compiles SQLite from C; the pinned toolchain's
`self-contained/x86_64-w64-mingw32-gcc.exe` is a **link-only driver whose `cc1`
backend is absent**, and `libsqlite3-sys` 0.30 has dropped the `winsqlite3`
feature that would have linked Windows' own DLL without compiling. Installing a
toolchain is the global install DP-S5 forbids.

**Correcting session 3's note:** it said this host "has no `dlltool.exe`". It
has one, in `…/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained/` — it is
just not on `PATH`. The real gap is narrower and more permanent: no C
*compiler*, only a linker driver. The old note would have sent someone hunting
for the wrong thing.

**Row 11 therefore splits at the storage seam** (D-068, TD-28):
* **Unblocked — start here:** snapshot format (content-addressed, BLAKE3
  state-hash verified on load), docs/27 §2's document-lifecycle machine with its
  forbidden transitions, docs/16's SALVAGE algorithm (last valid snapshot +
  readable tail + quarantined remainder + honest report). All pure logic above
  the seam, `no_std`, provable against deliberately corrupted byte inputs — the
  same shape that made `usk-sync` fully provable without a network.
* **Blocked:** docs/26 schema verbatim, WAL fsync cadence, atomic-rename
  compaction, kill −9 against a real file.

The blocked half is **deliberately not written blind**: DP-C4 forbids stacking
an unverified layer, and SQL that has never executed is exactly that. The probe
crate was removed rather than left as a broken workspace member.

## Row 11 (unblocked half) — **DONE**: `usk-recover`

New kernel crate `crates/usk-recover` (`snapshot.rs`, `salvage.rs`,
`machine.rs`), **15 tests**, all gates green. `no_std`, no I/O, no clock — so
corruption is a byte array, a crash is a truncated slice, and a torn write is a
test input.

### Snapshots that prove themselves
A v0.1 snapshot is **the compacted op set in canonical encoding, plus the state
hash it must produce**. Nothing new was invented: docs/26 says "the file IS the
wire format at rest", and this takes that literally. A tile image (docs/16's
Merkle-shared body) is the Row-12+ format and swaps in behind `verify`.

Verification is a **replay, not a checksum**. A checksum proves the bytes
survived; replaying them and comparing `State::state_hash()` proves the bytes
still *mean* what they meant — the stronger check, free because of DP-A2, and
the thing that makes docs/26's migration rule executable ("a migration that
changes the state hash is by definition wrong").

### docs/27 §2 implemented exactly, forbidden lines included
- **"writing to the old file during COMPACTING"** — `may_write_container()` is
  false there, and ops arriving mid-compaction are **deferred, not written and
  not dropped**, then flushed onto the new file after the atomic rename. Both
  forbidden lines are live at once in that arm; either alone is easy and the
  pair is the actual requirement.
- **"opening READY without hash-verifying the loaded snapshot"** — made
  *unrepresentable*: `Event::Recovered` carries a `VerifiedSnapshot`, whose only
  constructor is `Snapshot::verify`. The test proves the property that makes the
  transition unreachable rather than asserting an error path that cannot fire —
  the technique D-060 established for the undo machine.
- **"any transition that loses acked ops"** — `acked_ops` is monotonic, checked
  after every step of a script that visits every state.

### docs/16 SALVAGE — honest by construction
`recover(snapshots, tail_bytes)` walks snapshots newest→oldest, takes the first
that proves itself, reads the tail until the first byte it cannot decode, and
**quarantines the remainder verbatim rather than deleting it**. The report names
what was used, what was rejected and why, how much was lost. `is_clean()` and
`lost_data()` are separate questions on purpose: an older-but-valid snapshot
with a readable tail loses nothing yet is still not a clean open, and docs/16
forbids letting the user believe otherwise.

Proven: clean open · corrupt newest snapshot falls back *and says so* · torn
final write recovers up to the tear · **every snapshot corrupt still rebuilds
from the op tail alone** (ops are the truth) · forged state hash refused · lying
watermark refused.

### What Row 11 still owes
docs/26's schema verbatim, WAL fsync cadence, atomic-rename compaction, the
kill −9 file test, and **W-OPEN-1M** — all blocked on TD-28. BOOTSTRAP row 11 is
therefore **not** closed; its logic half is.

## Session 11 — architect rulings applied · **TD-28 unblocked** · Row 11 container half **DONE**

Gates verified first: 164 tests, hashes `ef7933e8…`/`5dbb01c2…`, native == wasm32.

### Rulings applied
1. **docs/27 §1 edges RATIFIED** (D-064). The spec gains
   `HELLO_SENT|BACKOFF ──transport loss──► DISCONNECTED ──timer──► HELLO_SENT`
   plus a new §1a covering the three late-arrival cases, written exactly as the
   shell resolves them. The `debug_assert` stays, and §1a says why: it has
   caught two shell defects, and widening the machine would have turned both
   into silent behaviour changes.
2. **BOOTSTRAP proptest references fixed.** Row 8 as directed — and **row 3
   carried the identical contradiction**, so it was fixed with it. DP-F3 makes
   a conflicting doc a defect wherever it appears; leaving the twin would have
   re-raised the same question next session.
3. **TD-28 unblocked (D-073).** WinLibs MinGW-w64 GCC 16.1.0 **msvcrt** build,
   in `.toolchain\`, gitignored, URL + SHA-256 in docs/43. Verified on
   `hello.c`, then on SQLite: `sqlite_version()` = **3.46.0**.
   - **msvcrt, not the newer UCRT default**: Rust's `x86_64-pc-windows-gnu`
     links msvcrt, and a UCRT `sqlite3.o` would put C objects and Rust std on
     two C runtimes — two heaps, one `sqlite3_free`. That failure looks like
     memory corruption, not a build misconfiguration.
   - **DP-S5 intact**: no global install, no PATH edit, no registry, nothing
     outside the folder. `tools\cc-env.ps1` exports the *target-suffixed*
     `CC_x86_64_pc_windows_gnu` for one process, so it cannot leak into a
     cross-compile.
   - **The bill**: workspace dep closure **10 → 29 of 40**. ADR-031 is frozen
     and this is its price, but one dependency took nineteen of thirty
     remaining slots. Kernel closure untouched at 10/12.

### Row 11 container half — `ehkatra-store`
docs/26's schema **verbatim** (the SQL is copied, not paraphrased, so drift is
visible), `user_version` + `application_id`, WAL always, `synchronous = FULL`.
**13 new tests.**

- **Row 11 exit criterion passes**: `save_then_reload_preserves_the_state_hash`,
  plus the same invariant through a snapshot + tail.
- **kill −9 is a real kill**: `crash-writer` is a separate binary that prints
  `COMMITTED <n>` per durability point; the test reads those, terminates the
  process with no unwinding, reopens, and asserts every acknowledged op
  survived and replays to the right hash. It asserts *only* docs/16's promise —
  ops after the last acknowledgement are not claimed.
- **SALVAGE against real corruption**: a randomised snapshot body, and a
  payload truncated mid-op (the torn write a power cut leaves).
- **Compaction** writes a new file and renames, driving `usk-recover`'s
  COMPACTING machine against the real file; the deferred-ops rule proven in
  logic now runs on disk.
- **Migration check implemented, not described**: `migrate::run` captures the
  state hash, refuses to commit if it moved, and rolls back. The registry is
  empty at v1 on purpose — the *mechanism* is tested now rather than letting
  the first real migration be its own first test.

### Two defects the logic half could not have found (D-074)
1. `NoValidSnapshot` fired on a container that had **never been snapshotted** —
   every kernel test supplied a snapshot, so "none present" and "none valid"
   were one branch. A young workbook is not in trouble; it is new.
2. **The autosave cadence never fired.** `append_ops` stamped the batch from the
   system clock while `maybe_commit` compared an injected one. Two ends of one
   interval, two clocks. `append_ops` now takes `now_ms`.

### MEASURED — W-OPEN-1M (MEASUREMENTS.md)
Cold open to READY **2.10 s**; SALVAGE with a corrupted final page **657 ms**;
container 108 MB for 1.1M ops. Against docs/31's <1.5 s budget: that budget is
for *skeleton + viewport*, and this is a full replay — **neither a pass nor a
breach**, recorded as what it is.

### The finding that matters most (D-075, TD-30)
The salvage run reported `lost_data = true` with **zero quarantined bytes**.
Nothing was damaged because nothing was left: the container keeps **one**
snapshot and stores only the uncovered tail in `ops`, so corrupting the snapshot
destroyed the 1,002,000 ops it had compacted. docs/16 says "the *last valid*
snapshot", which presupposes more than one exists — **the salvage path is
correct and the retention policy above it was never written.** The smaller test,
which keeps every op, recovers the whole workbook from the same corruption.
Same code, opposite outcome: **recoverability is a property of the retention
policy, not of the salvage code.** Filed for docs/16 to decide.

### TD-24's residual did NOT close here (D-076) — correcting the plan
The handoff expected the snapshot to make `settle()` cheap. It cannot: a v0.1
snapshot body *is* the compacted op set (D-069) and `verify` proves it by
replaying it, so opening costs the same as replaying the whole log — which is
what the 2.10 s measures. The residual closes only when the body becomes a
materialised state image (docs/16's tile image). That is real work, not wiring.

### GATES STATUS
fmt ✓ · clippy 0 warnings ✓ · **tests 177/177 ✓** · no_std wasm32 kernel build ✓
· dep budget 1/5, 10/12, **29/40** ✓ · differential replay native==wasm ✓ ·
purity + host-isolation + loopback greps ✓. Both replay hashes unchanged.

### NEXT
- **TD-30 first** — a container that cannot survive one corrupt snapshot should
  not meet real data. It is a policy decision plus a small change.
- Row 11 leftovers: none blocking. TD-25 (per-op length framing) is still worth
  paying before the first wire-version bump; the container and the wire want the
  same prefix.
- Then **Row 12 (CSV)**: streaming reader, strict-mode inference report before
  commit, formula-injection neutralisation on import *and* export (docs/24),
  sandboxed subprocess from the first line.

### SESSION END
Row 10 is complete. Row 11's blocker is recorded, and its unblocked half is
built and proven. **164 tests, all gates green, working tree uncommitted.**

### NEXT SESSION — start here
1. **TD-28 first, and check it rather than assume it.** If `cc` exists (or CI is
   the target), write `ehkatra-store`: docs/26's schema verbatim over
   `usk-recover`'s already-proven logic. The port is deliberately narrow —
   `Snapshot`, `recover()`, and `Document` are the whole interface, so the
   adapter is `INSERT`/`SELECT` plus fsync discipline, not new semantics.
2. Then BOOTSTRAP row 11's kill −9 file test and **W-OPEN-1M**.
3. **TD-24's residual gets cheap here.** The frequency half is paid (D-071);
   what remains is that a read after N appends is O(N). A snapshot gives the
   fold a checkpoint to start from. Land the container, then re-measure
   W-SYNC-RELAY — do not build a separate incremental applier first, and do not
   build the resident writer index (2 KiB per tile-actor, ruinous on sparse
   workbooks).
4. **TD-25 is worth paying at the same time**: the container and the wire want
   the same per-op length prefix, and DP-A5 forward preservation is
   unimplementable without it. Pay it before the first wire-version bump.

The 50-replica benchmark was still running when this was written. Re-run it with
`cargo build --release -p sync-bench; ./target/release/sync-bench` (or
`--quick` for the 2-replica case, which takes 262 ms); if it is still the wrong
shape to wait on, that *is* TD-24's evidence and the entry already says so.

`.checkpoints/01-td22/` holds the pre-TD-22 copies of `usk-state`'s `lib.rs` and
`tile.rs`.

### NEXT — Row 11 (snapshots + recovery)
docs/26's container schema **verbatim**, docs/16's SALVAGE machine, and
docs/27 §2's document lifecycle with its forbidden transitions. Proof: BOOTSTRAP
row 11's kill −9 mid-write test, and **W-OPEN-1M** (docs/38: 1M-cell workbook +
100k-op tail; cold open to READY, and SALVAGE path time with a corrupted final
page).

Carry into Row 11:
- **Row 11 is where TD-24 gets cheap.** A snapshot gives the fold a checkpoint
  to start from, which is most of the incremental-apply problem. Do not build a
  separate incremental applier first — land the snapshot and re-measure.
- The durability half of never-drop is Row 11's to prove. Session 10 proved the
  *protocol* half (recovery re-offers unacknowledged ops) against an in-memory
  log and said so explicitly; the fsync contract is not yet claimed anywhere.
- `Op::decode` now exists, so the container can store canonical op bytes and
  read them back. TD-25 (framing for forward preservation) is worth paying
  *here*, because the container format and the wire format want the same
  per-op length prefix.
- TD-19 (699 ms graph build, dominated by parsing) is repaid by persisting
  parsed formulas in the snapshot — Row 11 is its named trigger.

