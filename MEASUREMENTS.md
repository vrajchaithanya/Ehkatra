# MEASUREMENTS.md — every number here is measured, or it isn't here

## Reference machine (docs/38 requires machine context with every number)
**M1** — Intel Core i7-10750H @ 2.60 GHz, 6 cores / 12 threads, 15.9 GB RAM,
Windows 11, target `x86_64-pc-windows-gnu`, rustc 1.97.1, `--release`
(opt-level 3). Every number below is M1 unless stated otherwise.

> **docs/38 rule:** a number without a `W-*` workload id is invalid. Rows
> predating docs/38 have been mapped to their workload id where one exists;
> where none exists the number is marked *unspecified workload* and is not
> quotable until a `W-*` entry is added.

| Claim | Measured | Where | Notes |
|---|---|---|---|
| Convergence: 200 random interleavings, 15-op history — *precursor to W-STRUCT-STORM* | 200/200 identical hashes | `randomized_interleavings_converge` | scale to 10^5 interleavings + larger histories in CI next session |
| Demo end-to-end (build state ×2 replicas + hash ×2, release) — *unspecified workload* | 1.6 ms | timed run ×5, min | trivial workload; real benches arrive with Row 7 |
| Test suite wall time | <0.1 s | cargo test | |
| **W-REPLAY-5K** — differential replay, x86_64-pc-windows-gnu vs wasm32-wasip1 | identical: oplog `ef7933e8…`, state `5dbb01c2…` | `tools/replay-check` + `tools/run-wasi.mjs` | DP-A2 gate, CI-enforced. **Hashes changed at session 9** when docs/29's rule ("a new op type joins the generator") was applied — see the W-REPLAY-5K section below. |
| **Determinism across host + compiler**: same 5,000-op corpus on x86_64-unknown-linux-gnu / rustc "stable" (session 2, cloud) vs x86_64-pc-windows-gnu / rustc 1.97.1 (session 3, Windows host) | byte-identical: oplog `77e5b1bf2489a7a5e964e1284ad7dcc867b01af93a39d59663c9df7ce2ac5089`, state `4516044ed95e844c01b86b0693ea1f5509d970dde1e12856f85dfd2ac8438639` | `tools/gates.ps1` session-3 run vs PROGRESS session-2 record | Stronger than the CI gate: different OS, different libc, different linker (MinGW vs GNU ld), different compiler version — hashes unchanged. This is the first evidence that DP-A2 survives toolchain drift, not just target drift. |
| Kernel direct dependencies (DP-S2 budget 5) | 1 (`blake3`) | `tools/dep-budget.mjs` | |
| Kernel dependency **closure** incl. build scripts (D-035 budget 12) | 10 | `tools/dep-budget.mjs` | `blake3` pulls 9 transitively (`arrayref, arrayvec, cc, cfg-if, constant_time_eq, cpufeatures, find-msvc-tools, libc, shlex`). docs/07 §3's "currently 1" counted direct edges only — see D-035. |
| Workspace dependency closure (DP-S2 budget 40) | 10 | `tools/dep-budget.mjs` | |
| Full local gate set wall time (cold-cached workspace, warm deps) — *unspecified workload* | ~35 s | `pwsh -File tools/gates.ps1` | fmt + clippy + tests + no_std wasm build + dep budget + differential replay + purity greps |

## Row 4 — tile store (`tools/tile-bench`, release, M1)

> **Workload status:** these predate docs/38. The A-001/A-002 numbers are
> superseded at TD-09 closure by **W-TILE-10M**, whose definition differs
> (3-actor storm at 1% and 50% overlap). Treat everything in this section as
> historical once the W-TILE-10M block exists.

Reproduce: `cargo build --release -p tile-bench; ./target/release/tile-bench`.
Corpus is a pure function of its shape — no clock, no RNG (DP-A2).

### A-001 — memory for 10M numeric cells · **CONFIRMED, wide margin**
| | |
|---|---|
| Budget (docs/31, A-001) | < 400 MB |
| Cells / tiles | 10,000,000 in 640 tiles (10,000 rows × 1,000 cols) |
| Structural heap (`State::cell_heap_bytes`) | 84,245,216 B = **84.2 MB**, **8.425 B/cell** |
| OS peak working set (whole process) | **93.1 MB** |
| docs/14's predicted budget | ~81 MB (80 values + 1.2 bitmaps + summaries) |

The 8.9 MB gap between structural and OS peak is allocator bookkeeping, the
binary, and transient pre-pass state — reported rather than hidden, because
"structural bytes" is a number the code can compute and RSS is the number a
user feels. Both are far inside budget. **Caveat that matters: this is the
solo, uncontested case.** See A-002.

### A-002 — CRDT promotion under multi-author load · **FAILED**
Claim (docs/42): *promotion < 1% of cells under realistic multi-author load*.
Measured on 262,144 cells / 16 tiles, one author writing the sheet and a second
re-writing some of it:

| Contention shape | Cells contested | Tiles promoted | **Cells promoted** | B/cell |
|---|---|---|---|---|
| solo, none | 0% | 0 / 16 | 0.00% | 8.5 |
| clustered in one block | 0.10% | 4 / 16 | **25.00%** | 25.0 |
| clustered in one block | 1.00% | 4 / 16 | **25.00%** | 25.6 |
| scattered evenly | 0.10% | 16 / 16 | **100.00%** | 74.5 |
| scattered evenly | 1.00% | 16 / 16 | **100.00%** | 75.1 |

The mechanism is amplification, not a coding error: a tile is 16,384 cells, and
**one contested cell promotes all of them**. 0.1% of cells contested — a very
mild collaboration load — promotes the entire sheet if those cells are spread
out, which is the normal way a reviewer edits a document.

This propagates straight into A-001: at 74.5 B/cell, a 10M-cell workbook needs
**~745 MB**, or **1.9× the 400 MB budget**. A-001 is confirmed only for
single-author workbooks; under scattered contention it breaks.

An earlier, coarser predicate (*any* two actors writing the same tile) measured
100% promotion on every multi-author pattern — useless. The per-cell predicate
shipped here is the improved version, and it still fails the assumption.
Consequence executed per docs/42: A-002 → Failed, tile-granularity redesign is
now a Q1 gate (docs/44 debt entry, docs/43 D-039).

## Row 5 — value lattice (`tools/tile-bench`, release, same machine)

### Cost per cell by stored type (1024 × 256 = 262,144 cells)
| Stored type | Tile layout | Heap bytes | **B/cell** |
|---|---|---|---|
| `Number` | `CellPack::Numbers` (packed f64) | 2,218,496 | **8.5** |
| `Decimal` | `CellPack::Decimals` (packed exact base-10) | 8,509,952 | **32.5** |
| `Text` | `CellPack::Tagged` (union + string bytes) | 14,712,528 | **56.1** |

`size_of::<Value>()` = **48 B**, up from 32 B before this row. The growth is
`i128` alignment in `Decimal`, and it lands only on the *tagged* path — the
numeric fast path is untouched, which the A-001 re-run confirms below. Giving
currency its own packed layout is what keeps it at 32.5 rather than 56.1: a 42%
saving on the column type this row exists to serve.

### A-001 re-run after Row 5 · **no regression**
| | Row 4 | Row 5 |
|---|---|---|
| 10M numeric cells, structural | 84,245,216 B (8.425 B/cell) | **identical** |
| OS peak working set | 93.1 MB | 92.7 MB |
| Grid state hash | `be64a419…` | **identical** |

Adding two variants to `Value` changed nothing on the numeric path, which is
the property that keeps A-001 valid across the row.

### Encoding stability across the row · the DP-A4 evidence
| | |
|---|---|
| replay-check oplog hash | `77e5b1bf…` — **unchanged** |
| replay-check state hash | `e6cc2757…` — **unchanged** |

Row 5 added a value variant (`Decimal`, tag `0x06`) and extended the error
payload with an origin, and the 5,000-op corpus still hashes bit-identically.
That is the proof the extension was genuinely additive rather than merely
intended to be: tags `0x00`–`0x05` produce the same bytes they did before.
Pinned independently by `existing_value_encodings_are_byte_stable`, which
asserts the literal byte sequences.

### Determinism after the tile refactor
| | |
|---|---|
| oplog hash (5,000-op corpus) | `77e5b1bf…` — **unchanged** from sessions 1–2, proving the op algebra did not move |
| state hash | `e6cc2757e42581c6cacd47cfb0420c3364e5ae76e98d0a081bd7a8efb03f2957` — **new**, superseding `4516044e…`: cells now fold in tile-major order (the tile-Merkle direction docs/10 specifies) instead of flat identity order |
| native vs wasm32-wasip1 | identical |

## Row 7 — dependency graph and recalculation · **W-CHAIN-100K** (`tools/calc-bench`, release, M1)

**W-CHAIN-100K** (docs/38): 10,000 rows x 10 chained formula columns =
**100,000 dependent formula cells**, each reading the column to its left.
Reproduce with `cargo build --release -p calc-bench; ./target/release/calc-bench`.

### A-003 — full recalculation · **passes, with a caveat that matters**
| | |
|---|---|
| Budget (docs/31, A-003) | < 200 ms for 100k dependents **on 8 cores** |
| Measured (ordinal path, superseded — see the identity-path re-run above) | **53.0 ms**, median of 5 |
| Threads used | **1** |
| Throughput | 1.89 M cells/s |
| Graph size | **10 nodes for 100,000 formula cells** (10,000 cells/node) |
| Topological levels | 10 (chain depth), parallel width ~1 |

The caveat: this is single-threaded, so it clears a budget that *allowed* eight
cores. That is a stronger result than the budget asked for, but it does not
validate the "level-parallel via rayon" half of A-003 — rayon sits behind the
PAL `Compute` trait, which does not exist (DP-A3, docs/10). The bench reports
level width so the available parallelism is visible: on this shape it is ~1,
because a chain of 10 columns is inherently 10 deep. A wide model would show
width, and that is the shape to bench once the PAL lands.

### Incremental recalculation, one edit · **passes**
| | |
|---|---|
| Budget (docs/31, single edit) | < 8 ms |
| Measured | **0.191 ms**, median of 5 |
| Cells evaluated | **10** of 100,000 |
| Speed-up vs full recalc | **278x** |

### What the numbers cost to get right
Two measured failures on the way, both recorded because the fixed numbers mean
nothing without them:

1. **Grouping collapsed entirely: 100,000 groups for 100,000 cells**, and the
   O(groups^2) edge build then hung outright (killed after 10 minutes). All ten
   chained columns share one R1C1 pattern, so they formed a single group whose
   read set overlapped its own write set, which split to singletons. Fixed by
   partitioning a self-overlapping group by column first, and by building edges
   through the range index instead of comparing every pair.
2. **Incremental recalc recomputed everything: 53 ms, 100,000 cells, 1x
   speed-up.** Dirtiness was tracked per *group*, and a group is 10,000 cells.
   Fixed by carrying a dirty *rectangle* through marking and evaluation. That
   left a 10.5 ms residue spent re-walking 10,000 member ASTs per group to
   answer "who reads this rectangle"; precomputing a per-member read bound took
   it to 0.191 ms.

### Graph construction (not a budgeted number, recorded as a watch item)
| | |
|---|---|
| Graph build, 100k formulas | 699 ms |

Dominated by parsing 100,000 formula strings. docs/31 budgets cold open of a
1M-cell workbook at <1.5 s for *skeleton + viewport*, which is not this number,
but a naive 1M-formula build would extrapolate to ~7 s. Filed as TD-19 rather
than left to be discovered at Row 11.

## W-CHAIN-100K re-run after TD-21 (session 9) — identity path

Row 9's exit criterion required A-003 to be **re-measured over the identity
path**, not assumed to carry over from the ordinal one. It did not carry over.

| | Ordinal path (session 6) | Identity path (session 9) | |
|---|---|---|---|
| Full recalc, 100k cells | 53.0 ms | **92.6 ms** | **+75% — a real regression** |
| Single edit | 0.191 ms | **0.328 ms** | +72%, still 24× under budget |
| Graph nodes | 10 | 10 | unchanged |
| Levels / width | 10 / ~1 | 10 / ~1 | unchanged |
| Budget (A-003) | <200 ms on 8 cores | **passes on 1 core** | |
| Budget (single edit) | <8 ms | **passes** | |

Cause, measured not guessed: results are now keyed by cell **identity**
(`(RowId, ColId)` = 48 bytes) in a `BTreeMap`, where the ordinal engine wrote
into a `Vec` by index. 100,000 tree inserts with 48-byte keys is the whole
difference; the evaluator and the graph are unchanged.

This is the honest price of TD-21, and docs/38's regression policy says a
>5% regression gets a signed-off debt entry rather than a footnote — filed as
**TD-23**. Both budgets still pass with margin, so the correct call was to take
the regression and record it rather than keep two addressing models alive.

Also confirmed by this run: `regrouped: false` on a value edit — the TD-18
trigger routes value ops to the incremental path and structural/formula ops to
a rebuild, without the caller deciding.

## W-REPLAY-5K — corpus extended to full payload coverage (session 9)

docs/29 mandates that a new op type joins the replay-check generator, or the
determinism gate silently stops covering it. Three Row-9 op types
(`SetFormula`, `UndeleteRow`, `UndeleteCol`) had been added without this, and
auditing the generator found `ClearCell` and `Value::Decimal` had *never* been
covered either — the gate had been green over a corpus that exercised 4 of 9
payload variants and 3 of 6 value variants.

| | Before (4/9 variants) | After (9/9 variants) |
|---|---|---|
| oplog hash | `77e5b1bf2489a7a5e964e1284ad7dcc867b01af93a39d59663c9df7ce2ac5089` | `ef7933e808a86335cf77e64a69db781f42b702187a69c3249596498c7279cbac` |
| state hash | `e6cc2757e42581c6cacd47cfb0420c3364e5ae76e98d0a081bd7a8efb03f2957` | `5dbb01c2575c5ff3976dafafe61fb7227df671c34e41572acf79d981e4120bd1` |
| native == wasm32 | yes | **yes** |

The new hashes are the reference from session 9 onward. The corpus now
exercises: InsertRow, InsertCol, DeleteRow, DeleteCol, UndeleteRow,
UndeleteCol, SetCell, ClearCell, SetFormula (with variable-length identity
bindings), across Number, Bool, Text, Decimal and Blank values.

That the extended corpus still hashes identically across targets is the
stronger result: the Row 5 decimal encoding and the Row 9 variable-length
binding vector are now proven platform-stable, not merely assumed.

## Not yet measured (targets remain targets — docs/42)
A-001 memory/10M cells · A-002 promotion rate · A-003 recalc 100k · A-005 wasm32 **in a real browser / Safari** (WASI-under-Node is not a browser and must not be reported as one) · all docs/31 budget rows.
