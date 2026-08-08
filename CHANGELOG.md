# Changelog — Architecture Repository

## 2026-08-08 (session 11) — rulings applied · TD-28 unblocked · Row 11 container half DONE
- **docs/27 §1's four undefined edges RATIFIED** (D-064): the spec gains the `HELLO_SENT|BACKOFF ──transport loss──► DISCONNECTED` row and a new §1a covering duplicate `InSync`, `GIVE`-in-LIVE / `OPS`-in-SYNCING, and out-of-state `ACK`, written as the shell resolves them. The `debug_assert` stays, and §1a records the argument: it has caught two shell defects that widening the machine would have made silent.
- **BOOTSTRAP proptest references fixed** — row 8 as directed, and **row 3**, which carried the identical contradiction (DP-F3: a conflicting doc is a defect wherever it appears).
- **TD-28 UNBLOCKED** (D-073): WinLibs MinGW-w64 GCC 16.1.0 **msvcrt** build in `.toolchain\`, gitignored, URL + SHA-256 recorded. Chose msvcrt over the newer UCRT default because Rust's `x86_64-pc-windows-gnu` links msvcrt and mixing CRTs gives two heaps and one `sqlite3_free` — a failure that reads as memory corruption, not a build mistake. DP-S5 intact: no global install, no PATH edit, no registry, nothing outside the folder. **Cost: workspace dep closure 10 → 29 of 40.**
- **Row 11 container half DONE** — `ehkatra-store`: docs/26's schema verbatim, WAL + `synchronous = FULL`, 250 ms batched autosave, atomic-rename compaction driving `usk-recover`'s COMPACTING machine against a real file, and a migration check that *implements* docs/26's rule (capture state hash, refuse and roll back if it moved) with an empty v1 registry so the mechanism is tested before the first real migration depends on it. **Row 11 exit criterion passes**: `save_then_reload_preserves_the_state_hash`.
- **kill −9 is a real kill**: `crash-writer` is a separate binary; the test reads its `COMMITTED <n>` acknowledgements, terminates it with no unwinding, reopens, and asserts every acknowledged op survived and replays to the right hash — asserting only what docs/16 promises.
- **Two defects the logic half could not have found** (D-074): `NoValidSnapshot` fired on containers that had simply never been snapshotted (a young workbook is not in trouble, it is new); and the autosave cadence never fired because `append_ops` used the system clock while `maybe_commit` used an injected one.
- **W-OPEN-1M measured**: cold open to READY **2.10 s**, SALVAGE with a corrupted final page **657 ms**, 108 MB container for 1.1M ops. Against docs/31's <1.5 s: that budget is for *skeleton + viewport* and this is a full replay — neither a pass nor a breach, recorded as what it is.
- **TD-30 filed, and it is the important one** (D-075): the salvage run reported `lost_data = true` with **zero quarantined bytes** — the container keeps one snapshot and stores only the uncovered tail, so corrupting the snapshot destroyed the 1,002,000 ops it compacted. docs/16 says "the *last valid* snapshot", presupposing more than one. **The salvage path is correct; the retention policy above it was never written.** A second test that keeps every op recovers the whole workbook from the same corruption — recoverability is a property of retention, not of salvage code.
- **TD-24's residual did NOT close** (D-076), correcting the plan: a v0.1 snapshot body *is* the compacted op set and `verify` replays it, so a snapshot is not a fold checkpoint until its body is a materialised state image.
- New debt: TD-29 (snapshot body uncompressed vs docs/26's zstd), TD-30 (snapshot retention).
- 177 tests total; both replay hashes unchanged; kernel dep closure untouched at 10/12.

## 2026-08-08 (session 10) — D-062 closed · TD-22 CLOSED · Row 10 (sync) DONE
- **D-062 closed** by the owner's ruling: docs/38 restates A-002's bar in amplification terms (promoted ÷ contested ≤ 1.5, plus collab RSS ≤ 400 MB) and docs/42 records A-002 **Confirmed**. Measured 1.0× / 123.6 MB passes both halves; **no code changed** — the ruling restated the bar, not the requirement.
- **TD-22 CLOSED** (D-063): the formula registry is a stamped LWW register — every mutation is `max` over `(lamport, op id)`, so it is a function of the op *set*, not of arrival order. Proven over **all 120 permutations** of a mixed five-write history. Entries are seeded by the replay pre-pass rather than stamping every written cell, which would have cost ~480 MB at 10M cells — the ADR-005 promotion argument applied a second time. State hashes unchanged.
- **Row 10 DONE**: new kernel crate **`usk-sync`** (docs/27 §1 machine, never-drop queue, vector clocks + causal-gap buffer, DP-E4 validation, relay admission control) and shell crate **`ehkatra-relay`** (framing, replica composition, relay/peer binaries, deterministic in-process bus). `usk-sync` is `no_std` and performs no I/O and reads no clock.
- **docs/27 §1 implemented exactly**: every listed transition exercised by name; all three forbidden lines proven rejected (no OPS before HelloAck across five pre-ack states; six distinct hostile ops quarantined while the session stays LIVE; queued local ops survive a script visiting every state). An unlisted pair trips the `debug_assert` docs/27 asks for.
- **Four docs/27 §1 gaps filed, not invented** (D-064): transport loss during HELLO_SENT/BACKOFF, duplicate `InSync` after LIVE, `Give` during LIVE, `Ack` outside LIVE/SYNCING. The machine implements only what is listed; the shell refuses to hand it an undefined pair.
- **`Op::decode` added** (D-066) — BOOTSTRAP row 2's "encode/decode round-trip tests" claim had no decoder behind it. Decoding is total, every truncation of every variant is tested, and non-canonical decimals are repaired on decode.
- **Two-terminal demo over real loopback TCP** (`demo/collab.ps1`, `demo/collab.sh`): two peers, 19 ops, identical state hash `2f51c7f3…`.
- **Three defects found by measurement, not tests** — seven convergence tests passed through all of them, because scale and duration are test inputs and not just performance inputs: (1) recovery re-minted spent op counters and diverged the replicas (D-067); (2) a replica that lost its link mid-handshake wedged in HELLO_SENT forever, so the 50-replica run diverged and its mid-run kill delivered 1 of 117 queued ops — the D-064 teardown remedy had been *documented in a comment and never built*, and is now `Replica::hard_reset`/`resume`; (3) the demo overstated itself by counting refused edits as applied.
- **W-SYNC-RELAY passes at both sizes.** 2 replicas: propagation p50 200 / p95 1,800 bus-ms, convergence 10 bus-ms, all replicas equal, **32 ops queued at a mid-run kill and 32 delivered after recovery**. 50 replicas: 30,000 ops through 4,558 dropped frames and 2,085 reconnects, p50 800 / p95 3,700 bus-ms, convergence 2,140 bus-ms, **all replicas hash-equal**, and **45 ops queued at a mid-run kill with 45 delivered after recovery** — where the same run before the D-064 fix diverged outright and never converged at all.
- **TD-24 largely PAID** (D-071): state is folded when it is **read**, not when the log grows — DP-A9's "caches are watermarked folds", which `Session` had been violating by re-folding on every append. W-SYNC-RELAY at 50 replicas: **120 min → 6.8 min (17.6×)** with dropped frames, reconnects, convergence, the mid-run kill and every state hash **bit-identical**, which is the evidence it is a scheduling change and not a semantic one. `state()`/`value()`/`engine()` now take `&mut self`: the signature says "reading may cost you a fold" and makes stale reads impossible. The hard route — making `State` incrementally appliable — was rejected: it needs a resident per-tile per-actor writer index costing 2 KiB per (tile, actor), ruinous on sparse workbooks. Residual: a read after N appends is still O(N); Row 11's snapshot remains the named fix.
- **The benchmark harness was measuring itself** (D-072): `Bus` deep-cloned the entire op log once per delivered frame, so at 30,000 ops the instrumentation cost more than the system under test — the pre-fix wall-clock numbers overstated the product's cost. It also *corrected* a measurement: propagation was counted by scanning the receiver's log, double-counting ops held in the causal-gap buffer on redelivery, so p50 1,600 → 800 and p95 5,800 → 3,700 bus-ms are an honesty fix, not a speed-up.
- New debt: TD-24 (`State` not incrementally appliable, with measured cost), TD-25 (unknown op tags cannot be preserved-opaque without a framed encoding — **pay before the first wire-version bump**), TD-26 (anti-entropy is queue-based, not Merkle-guided), TD-27 (TCP, not WebSocket — D-065, with the dependency-budget arithmetic).
- New gate: **loopback-only listeners (DP-S5)**, added because Row 10 introduced the project's first listening socket.
- Each defect became a test: `a_replica_that_loses_its_link_mid_handshake_recovers` fails on the old shell; `a_partitioned_replica_stays_offline_and_keeps_its_queue` (run at 5% loss on purpose) pins the partition semantics the kill harness depends on.
- **A fourth defect, in the harness itself**: the mid-run kill modelled "offline" as a transport loss plus a long timer, which under loss is not offline — the victim rejoined and drained before the kill, so the 50-replica durability row measured nothing while looking like a result. The bus now has a real `partition`/`heal`. Writing `heal` produced a fifth instance of the same class (calling `connect` from BACKOFF), caught immediately by the machine's `debug_assert` — which has now caught two shell bugs and is the concrete argument for transcribing a specification exactly rather than widening it.
- **Row 11 (unblocked half) DONE** — new kernel crate **`usk-recover`**: snapshots that prove themselves by *replay* rather than checksum (docs/26's `state_hash` check, which also makes its migration rule executable), docs/27 §2's document-lifecycle machine, and docs/16's SALVAGE algorithm. 15 tests.
  - docs/27 §2's three forbidden lines: ops during COMPACTING are **deferred, not written to the old file and not dropped**, then flushed after the atomic rename (both forbidden lines live at once); "opening READY without hash-verifying" is made *unrepresentable* — `Event::Recovered` carries a `VerifiedSnapshot` whose only constructor is `Snapshot::verify`; `acked_ops` is monotonic across a script visiting every state.
  - SALVAGE is honest by construction: last valid snapshot, tail read to the first unreadable byte, **remainder quarantined verbatim rather than deleted**, and a report naming what was used, rejected and lost. A workbook whose every snapshot is corrupt still rebuilds from its op tail — ops are the truth.
- **Row 11's container half is BLOCKED (D-068, TD-28)**: `rusqlite` can only build via `bundled`, which compiles C, and the pinned toolchain ships a link-only gcc driver with no `cc1`; `libsqlite3-sys` 0.30 dropped the `winsqlite3` feature. Not written blind — DP-C4 forbids stacking an unverified layer. *Corrects session 3's note*: `dlltool.exe` does exist, in the toolchain's `self-contained` directory; the real gap is the compiler, not the linker.
- 164 tests total; both replay hashes unchanged; dependency closure still **10/40** — the three new crates added zero dependencies.

## 2026-08-07 (session 9) — new specs absorbed · Row 9 DONE · TD-09 CLOSED
- **docs/29 violation fixed**: replay-check's generator covered 4 of 9 payload variants — `ClearCell` and `Value::Decimal` had *never* been in the corpus, and Row 9's three op types were missing. The DP-A2 gate had been silently green. Corpus now covers all nine; reference hashes changed (`ef7933e8…` / `5dbb01c2…`), native==wasm32.
- **docs/38 applied**: MEASUREMENTS.md carries a reference-machine block and `W-*` workload ids; pre-docs/38 numbers marked *unspecified workload* and declared unquotable.
- **Row 9 DONE**: TD-21 closed (ordinal `Sheet`/`Rect` addressing deleted; engine works over `State`, rebinding references from identity bindings each rebuild) and TD-18 closed (`Engine::observe` routes structural/formula ops to a regroup, value ops to incremental). W-CHAIN-100K re-measured over the identity path: 53.0 → 92.6 ms (+75%), both budgets still passing, filed as TD-23.
- **TD-09 CLOSED**: promotion is per contested **cell**; amplification 16,384× → 1×. W-TILE-10M: import 8.43 B/cell / 0% promoted / 90 MB; collab 11.09 B/cell / **1.000%** / **123.6 MB**; adversarial 137.56 B/cell / 50% / 1.7 GB. **A-001 restored under collaboration**; fails at the adversarial pattern (real conflict metadata, no bar set).
- **A-002's bar is unachievable as written** (D-062): <1% promotion at a pattern that contests 1% of cells is impossible for any implementation that retains losers. Intent met; bar left for the owner to restate.
- docs/27 §3 generation mark and §5 undo-machine transition/forbidden tests added.
- 118 tests total.

## 2026-08-07 (session 8) — Owner corrections (D-054) + Row 9 core
- **D-054 corrections applied**: TD-09 is the next work unit after Row 9 with Row 10 hard-gated on its closure; TD-21/TD-18 are Row 9 exit criteria; supply-chain gate verified present in ci.yml with its real blocker named (no remote, never pushed — remote now configured, push is the owner's); TD-17 trigger made explicit; D-052 ratified (no proptest).
- **Row 9 core** (Row 9 remains IN-PROGRESS until the TD-21/TD-18 exit criteria close): new ops `SetFormula` (identity bindings bound once at the author) and `UndeleteRow`/`UndeleteCol`; `State` formula registry; new kernel crate **`usk-reduce`** — `Command` vocabulary, pure versioned `reduce_v1`, `Session` with per-actor undo/redo.
- **Selective undo proven** (13 tests): own-write-wins restore, blocked insert-undo when others wrote into the row, delete-undo resurrecting rows with their cells, redo as undo-of-undo, undo∘do = id on the projection for every command kind.
- New decisions D-055/D-056/D-057; new debt TD-22 (formula LWW needs stamps before incremental merge).
- 13 new tests (109 total); both replay hashes unchanged — tags 0x16–0x18 proven additive.

## 2026-08-07 (session 7) — BOOTSTRAP Row 8: identity references
- **`IdRange`**: a reference is a pair of endpoint identities with an `AnchorMode` — no position in the type at all (DP-A6, docs/04 invariant 3).
- **The canonical test passes**: Alice inserts a row inside `SUM(A1:A10)` while Bob overwrites cells in it, concurrently, ops arriving at two replicas in opposite orders. Same state hash, same eleven resolved rows, same answer, and nothing rewrote the formula.
- All five of docs/11's insert/delete shift rules fall out of one resolution rule rather than being implemented separately, each with its own test.
- `State` now exposes the axis order **including tombstones**, which is what makes "re-anchor inward" answerable; a first attempt using bind-time ordinal hints was wrong, because ordinals shift under later edits.
- Property coverage: 200 seeded insert/delete sequences and 60 shuffled arrival orders.
- New decisions: D-051 (resolve against the tombstone-retaining order), D-052 (seeded LCG sweeps instead of `proptest`, recorded as a deviation from a stack decision), D-053 (ordinal `Sheet` coexists with the identity path until Row 9).
- New debt: TD-21 two addressing models in `usk-calc`.
- 11 new tests (96 total); both replay hashes unchanged.

## 2026-08-07 (session 6) — BOOTSTRAP Row 7: dependency graph
- New kernel crate **`usk-calc`**: formula groups, range-granular edges, incremental recalculation, cycle detection (docs/13).
- **Formula groups**: nodes are R1C1 patterns, measured at **10 nodes for 100,000 formula cells**.
- **A-003 passes**: 100k-dependent full recalc in **53.0 ms single-threaded** against a <200 ms/8-core budget; single-edit incremental in **0.191 ms** against <8 ms, evaluating 10 cells of 100,000. The level-parallel half of A-003 is unvalidated — rayon is behind the unbuilt PAL `Compute` trait (TD-17).
- Two measured failures fixed on the way: grouping collapsed to 100,000 nodes and hung the O(n²) edge build; and incremental recalc recomputed all 100,000 cells for a one-cell edit. Both are recorded in MEASUREMENTS.md, because the final numbers are only meaningful against them.
- New decisions: D-047 (self-overlapping groups partition by column), D-048 (dirtiness is a rectangle inside a group), D-049 (edges from the range index, not pairwise), D-050 (cycles read off the level assignment).
- New debt: TD-17 single-threaded recalc, TD-18 no incremental regrouping, TD-19 graph-build parse cost, TD-20 band index vs R-tree.
- 14 new tests (85 total); both replay hashes unchanged.

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
