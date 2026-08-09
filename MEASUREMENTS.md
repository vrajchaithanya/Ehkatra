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

## W-TILE-10M (docs/38) — TD-09 closure · A-001 / A-002 · M1

10,000 × 1,000 = **10,000,000 numeric cells**. Actor 1 authors the whole grid
(import), then actors 2 and 3 re-write an evenly *scattered* share — scattered
because that is the shape that punished tile-granularity promotion hardest.
Reproduce: `./target/release/tile-bench 10000 1000 [import|collab|adversarial]`.

| Pattern | Tiles | Structural heap | **B/cell** | **Promoted cells** | OS peak RSS |
|---|---|---|---|---|---|
| import — 1 actor, 0% overlap | 640 | 84,265,696 | **8.43** | 0.000% | **90.0 MB** |
| collab — 3 actors, 1% overlap | 640 | 110,888,096 | **11.09** | **1.000%** | **123.6 MB** |
| adversarial — 3 actors, 50% overlap | 640 | 1,375,581,536 | 137.56 | 50.000% | 1,709.0 MB |

### What TD-09 changed
Promotion is now per **contested cell**, not per tile. The amplification factor
went from **16,384× to 1×**: promoted cells now equal contested cells exactly.

| | Before TD-09 (tile-granular) | After TD-09 (cell-granular) |
|---|---|---|
| 0.1% contested, scattered | 100% promoted, 74.5 B/cell | 0.1% promoted |
| 1% contested (collab) | ~100% promoted, ~74.5 B/cell → **~745 MB** | **1.0% promoted, 11.09 B/cell → 111 MB** |

### A-001 · **passes at import and collab; fails at adversarial**
Budget <400 MB at 10M cells. Import 90.0 MB, collab **123.6 MB** — and this is
the headline: **A-001 now holds under collaboration**, where before TD-09 the
same load extrapolated to ~745 MB and broke it.

Adversarial (50% of cells genuinely contested) reaches 1.7 GB and **fails the
budget**. That is not amplification — it is 5M cells each legitimately carrying
a stamp and a retained loser. docs/38 sets no pass bar for the adversarial
pattern, only a measurement; recorded as a fact, not smoothed. If a real
workload ever looks like this, the answer is compact stamps (TD-10's causal
`deps` would let most of them not exist at all), not a bigger budget.

### A-002 · **CONFIRMED** (bar restated by the owner per D-062, 2026-08-08)
docs/38 now states the bar in amplification terms: *promoted ÷ contested ≤ 1.5*,
**and** collab RSS ≤ 400 MB. Measured **1.0×** (the correctness floor) and
**123.6 MB** — passes on both halves. docs/42 records A-002 as Confirmed. No
code changed as a result: the ruling restated the bar, not the requirement.

The original escalation is kept below, because the reasoning is the reusable
part — an implementer who cannot meet a normative bar surfaces the conflict
rather than editing the bar.

> docs/38's original bar: *<1% promotion at the collab pattern*. Measured:
> **exactly 1.000%**.

> That is the **floor, not a near-miss**. The collab pattern contests 1% of
> cells by definition, and a contested cell *must* carry metadata — it has to
> name the winner and retain the loser (ADR-006). So promoted ≥ contested = 1%
> for **any** correct implementation, and "<1% at 1% overlap" is unachievable by
> construction, not by this design.
>
> The engineering goal behind A-002 — *promotion must not amplify* — is met, and
> measurably: 16,384× → 1×. The bar needs restating in terms of amplification
> rather than an absolute rate. **Left for the owner: docs/38 is normative and
> loosening a bar I just measured against myself is not mine to do.** Filed per
> docs/00's rule that conflicting documents are defects.

### Compaction ratio — not yet measurable
docs/38 lists it among W-TILE-10M's measures. Compaction lands with the
container at Row 11; there is nothing to measure until then. Recorded rather
than silently omitted.

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

## W-REPLAY-5K — corpus extended again for `Payload::Opaque` (session 12)

TD-25 added the `Opaque` payload variant (DP-A5 forward preservation, D-080).
docs/29's rule applies to it like any other: a payload variant absent from the
replay-check generator is a variant the determinism gate has silently stopped
covering — the trap session 9 walked into over four variants at once.

| | Session 9 (9/9 variants) | Session 12 (10/10 variants) |
|---|---|---|
| oplog hash | `ef7933e808a86335cf77e64a69db781f42b702187a69c3249596498c7279cbac` | `c79fa5335520542f1363fe32f8bc7d3df53910e4e736637f22babd9a0143afee` |
| state hash | `5dbb01c2575c5ff3976dafafe61fb7227df671c34e41572acf79d981e4120bd1` | `b58d550544c971313ad86167c05beaedddad7c13855d29c4997bcec0b2ff6215` |
| native == wasm32 | yes | **yes** |

**No canonical op encoding moved.** The `u32` length prefix TD-25 introduced sits
*outside* `Op::encode` — docs/26 requires the container's `payload` column to
hold "the identical bytes that were hashed", which settles where framing
belongs. The hashes moved only because the corpus gained an opaque op. Worth
stating precisely, because "the replay hashes changed" and "the wire format
changed" are very different sentences and only the first is true.

The new hashes are the reference from session 12 onward.

## W-SYNC-RELAY (docs/38) — Row 10 acceptance · M1

**Definition (docs/38):** 2 and 50 replicas through one relay, each replica
10 ops/s for 60 s, 1% simulated packet loss. Measures propagation p95,
convergence time after the last op, and queued-op durability across a mid-run
kill. Reproduce: `cargo build --release -p sync-bench; ./target/release/sync-bench`
(add `--quick` for the 2-replica case only).

### Units, stated before the numbers
Propagation and convergence are in **bus milliseconds** — the simulated clock of
the deterministic transport, where one link hop costs 5 ms. That is deliberate:
they are properties of the *protocol* (how many round trips a fact needs to
reach every replica), and timing them against a wall clock would report this
laptop's memcpy speed instead. **Wall time is reported separately** and is a
statement about the implementation, not the protocol. Everything is seeded
(D-052), so any run is reproducible from its seed.

### 2 replicas · **passes**
| | |
|---|---|
| Ops authored | 1,200 (600 each, 10 ops/s × 60 s) |
| Frames dropped / reconnects | 72 / 40 |
| Propagation p50 | **200 bus-ms** |
| Propagation p95 | **1,600 bus-ms** |
| Convergence after last op | **10 bus-ms** |
| All replicas equal (state hash) | **YES** |
| Mid-run kill durability | **32 ops queued at death, 32 delivered after recovery** |
| Quarantined remote ops | 0 |
| Wall time | 97 ms (was 564 ms before TD-24 was paid) |

*Frames dropped counts partitioned frames too, which is why it rose from 43 to
72 when the kill harness gained a real partition — the partition window
deliberately swallows everything crossing that link.*

**Reading the propagation numbers.** p50 is 200 bus-ms against a 10 ms
round trip, because a replica authors one op per 100 ms tick and the bus
delivers on tick boundaries — the tick, not the link, is the floor. The p95 of
1,600 ms is the reconnect tail: 1% frame loss tears the connection down, and
recovery costs a backoff (500 ms base, jittered) plus a HELLO/NEED/GIVE round
trip. That tail is the cost of docs/27's recovery path being real rather than
assumed, and it is visible precisely because loss is modelled as a broken
connection instead of a vanishing frame.

**The durability line is the one that matters.** The victim is taken offline,
edits for 30 ticks, and is then destroyed — every in-memory structure discarded,
only its op log surviving. `Replica::recover` rebuilds it, and all 32
unacknowledged ops reach the peer. Nothing was lost, which is docs/15's
published never-drop contract, measured rather than asserted.

*Caveat, stated because it limits the claim:* the "durable log" here is an
in-memory vector, not an fsync'd container file. This measures the **protocol**
half of durability — that recovery re-offers unacknowledged work — and not the
storage half, which is Row 11's to prove and is not claimed here.

### Two defects this workload found — both invisible to the test suite
**1. Recovery re-minted spent op identities (D-067).** The 2-replica run
diverged on its first execution: a replica rebuilt from its durable log kept
minting counters from 0 and re-issued identities it had already spent, so two
different ops shared one `(actor, counter)` and the log's dedup discarded the
second. Fixed in `Session::integrate_batch`.

**2. A replica that lost its link mid-handshake wedged forever (D-064).** The
50-replica run **diverged**, and the mid-run kill delivered **1 of 117** queued
ops. Cause: docs/27 §1 defines no transition for transport loss during
HELLO_SENT or BACKOFF, and the shell's first answer to that gap was to do
nothing, on the theory that an existing backoff timer would drive the next
attempt. There is no such timer once a retry has already fired — so with 4,051
dropped frames, replicas piled up in HELLO_SENT with nothing left to wake them.
Fixed by implementing the remedy D-064 had already *documented* but not built:
tear the session down and rebuild it from DISCONNECTED, carrying the durable log
and every unacknowledged op across, then reconnect via whichever transition the
current state actually lists (`Replica::hard_reset` / `resume`).

Both defects share a shape worth naming: **seven convergence tests passed
throughout**, because only the benchmark restarts a replica that has already
authored work and only the benchmark loses enough frames to exhaust a handshake.
Scale and duration are test inputs, not just performance inputs.

### 50 replicas · **passes**

Three runs, because the first two were wrong in ways worth keeping on the
record. Only the last column is quotable — it is the only one produced by the
code that is in the tree.

| | Run 1 (pre-D-064 fix) | Run 2 (fix, old harness) | Run 3 (real partition) | **Run 4 (authoritative)** |
|---|---|---|---|---|
| Ops authored | 30,000 | 30,000 | 30,000 | **30,000** |
| Frames dropped / reconnects | 4,051 / 1,542 | 4,202 / 2,066 | 4,558 / 2,085 | **4,558 / 2,085** |
| Propagation p50 | 1,000 bus-ms | 1,600 | 1,600 | **800 bus-ms** ¶ |
| Propagation p95 | 7,500 bus-ms | 5,900 | 5,800 | **3,700 bus-ms** ¶ |
| Convergence after last op | 100,000 bus-ms † | 2,530 | 2,140 | **2,140 bus-ms** |
| All replicas equal | **NO — DIVERGED** | YES | YES | **YES** |
| Mid-run kill | 117 queued, **1 delivered** | 2 / 2 ‡ | 45 / 45 | **45 queued, 45 delivered** |
| Quarantined remote ops | 0 | 0 | 0 | **0** |
| Wall time | 32 min | 83 min | 120 min | **6.8 min** |

† Not a measurement — the settle budget running out. The run never converged.
‡ Not a measurement either — see the harness defect below.
¶ A *corrected* measurement, not a speed-up — see "the propagation columns" below.

**Run 4 is run 3 with TD-24 paid (D-071) and the harness's own overhead removed
(D-072).** Every protocol figure is **bit-identical** to run 3 — dropped frames,
reconnects, convergence, the mid-run kill, and every state hash. That identity
is the evidence that lazy folding is a scheduling change and not a semantic one;
a 17.6× speed-up that also moved a convergence figure would have meant something
had broken.

**Why 120 min → 6.8 min**, separated because only one cause is about the product:
(1) `Session` re-folded the whole log on every *append*; it now folds on every
*read*, and under sync reads are ~50× rarer (600 local edits versus ~30,000
delivered batches per replica). (2) The harness called `Replica::log()` — which
deep-clones every op — once per delivered frame, so at 30,000 ops the
instrumentation cost more than the system it was instrumenting. **The pre-fix
wall-clock numbers therefore overstated the product's cost**, and are kept above
rather than quietly replaced.

**The propagation columns moved because the definition was wrong, not because
anything got faster.** Arrivals were counted by scanning the receiving replica's
log, so an op sitting in the causal-gap buffer — arrived but not yet applied —
was counted again on redelivery with a later timestamp, inflating the tail.
Run 4 counts first delivery, which is what "propagated" means.

**What each run cost to learn.**
*Run 1 diverged* because a replica that lost its link mid-handshake wedged in
HELLO_SENT forever (D-064's teardown branch was documented and never built).
*Run 2 converged but its durability row was hollow*: the harness took its victim
offline with one transport loss plus a long retry timer, which under 1% loss is
not offline — the next dropped frame ran the teardown-and-reconnect path, armed
a fresh 500 ms timer, and the victim rejoined and drained before the kill
landed. It reported 2 ops where it should have reported tens. **A durability
measurement that measures nothing while looking like a result is worse than one
that fails.** Fixed with a real `partition`/`heal` on the bus — no frames cross,
no timer returns the replica — and pinned by
`a_partitioned_replica_stays_offline_and_keeps_its_queue`, which runs at 5% loss
specifically to reproduce the old failure rather than merely avoid it.
*Run 3* is the one that means something: **45 ops queued at the kill, 45
delivered after recovery**, at 50 replicas, through 4,558 dropped frames.

**Why the numbers move in the direction they do.** Reconnects climbed across all
three runs (1,542 → 2,066 → 2,085) and wall time nearly quadrupled (32 → 120
min). Both are the fixes working: replicas that used to wedge silently now tear
down and reconnect, the partition window forces a genuine catch-up, and every
reconnect runs an anti-entropy exchange whose ops are folded into state.

**Cost, after TD-24.** 6.8 minutes of wall clock for 60 seconds of simulated
session across 50 replicas and 30,000 ops. The old finding — *the protocol is
not the bottleneck; the state fold is* — was correct and is now acted on rather
than merely recorded. The residual is honest: a read after N appends is still
O(N), which sync tolerates because it is append-heavy and a repainting UI would
not. Row 11's snapshot remains the named fix. It is why TD-24 carries a measured
justification rather than a suspicion.

## W-OPEN-1M (docs/38) — Row 11 acceptance · M1 · **unblocked and measured**

1,000,000 cells (1000 × 1000) + a 100,000-op tail = 1,102,000 ops. The snapshot
covers everything but the tail. Reproduce:
`cargo build --release -p open-bench; ./target/release/open-bench`.
The container is written, dropped, and reopened, so "cold" means from a file
this process did not hold open — the OS page cache is still warm, and that is
stated rather than worked around, because a genuinely cold figure needs a
reboot this session cannot perform.

| | |
|---|---|
| Corpus | 1,102,000 ops |
| Snapshot body | 90,115,952 B (uncompressed — TD-29) |
| Container on disk | 107,720,704 B |
| Write (snapshot + 100k tail + commit) | 5.02 s |
| **Cold open to READY** | **2.10 s** (1,002,000 snapshot ops + 100,000 tail, clean) |
| **SALVAGE, corrupted final page** | **657 ms** (1 snapshot rejected, 100,000 tail ops read) |

**Against docs/31's budget, carefully.** docs/31 budgets cold open of a 1M-cell
workbook at <1.5 s for **skeleton + viewport** — a partial load. The 2.10 s here
is a *full* replay of every op to a complete `State`, which is strictly more
work than the budgeted thing. It is therefore **neither a pass nor a breach of
that budget**, and is recorded as what it is: the cost of the total path. A
skeleton+viewport figure needs partial materialisation, which does not exist
yet.

### The salvage number is the interesting one, and not for its speed
657 ms, one snapshot rejected, 100,000 tail ops recovered — and **zero
quarantined bytes** with `lost_data = true`. Nothing was damaged, because there
was nothing left to damage: the container keeps one snapshot and stores only the
uncovered tail in `ops`, so corrupting the snapshot destroyed the 1,002,000 ops
it had compacted.

**The salvage path did exactly what docs/16 specifies. The retention policy
above it did not exist.** docs/16's phrase is "the *last valid* snapshot", which
presupposes there is more than one. Filed as **TD-30** (D-075), with the choice
— keep N ≥ 2 snapshots, or retain compacted ops until a second one verifies —
left to docs/16, since it trades file size against recoverability.

Worth putting the two results side by side: `a_corrupted_snapshot_opens_through_salvage_and_reports_it`
keeps every op in `ops` and recovers the *whole* workbook from the same
corruption. Same code, opposite outcome. **Recoverability is a property of the
retention policy, not of the salvage code.**

### What this measurement did *not* buy
TD-24's residual, which the plan expected to close here. A v0.1 snapshot body is
the compacted op set (D-069) and `verify` proves it by replaying it, so opening
costs replay(snapshot) + replay(tail) — the same work as replaying the whole
log, which is precisely what the 2.10 s is. A snapshot becomes a fold checkpoint
only when its body is a materialised state image. Corrected in D-076 rather than
reported as closed.

## W-ORACLE (docs/38) — **the first measured Excel-compatibility percentage** · session 12

**Definition (docs/38):** every case in `tools/oracle-capture/vectors/` and
`vectors-1904/`, evaluated through `usk-formula` under `Profile::Compat` and
compared against what Excel actually did. Run with
`cargo run --release -p conformance`; the full per-case divergence list lands in
`.tmp/oracle-report.md`.

**Corpus:** 1,366 cases, 80 distinct functions, captured over COM from **Excel 16.0 build
20228** (Microsoft 365), calc engine 191029, Windows 11 26200, `.` decimal /
`,` list separator. One build, one locale — a second build would be a second
data point, not a re-run.

| | Cases | Pass | Rate |
|---|---:|---:|---:|
| **Baseline, before this session's fixes** | 1,366 | 896 | **65.6%** |
| **After the two cancellation mechanisms (D-041 as amended)** | 1,366 | 975 | **71.4%** |
| **After the cheap docs/50 §7 fixes** | 1,366 | **1,014** | **74.2%** |
| — 1900 date system | 1,236 | 988 | 79.9% |
| — 1904 date system | 130 | 26 | 20.0% |
| **After the date model (TD-33, session 20)** | 1,366 | **1,140** | **83.5%** |
| — 1900 date system | 1,236 | 1,026 | 83.0% |
| — 1904 date system | 130 | 114 | **87.7%** |
| **After lookup + wildcards (TD-14, TD-35, session 20)** | 1,366 | **1,171** | **85.7%** |
| **After the criteria sub-language (TD-34, session 20)** | 1,366 | **1,189** | **87.0%** |
| **After the literal parser (TD-32, session 20)** | 1,366 | **1,205** | **88.2%** |
| **After argument coercion (TD-51, TD-54, session 20)** | 1,366 | **1,220** | **89.3%** |
| **After conditions and omitted arguments (TD-52, TD-53, session 21)** | 1,366 | **1,235** | **90.4%** |

352 fail at 74.2%, of which **12 are numerically near** (relative difference
≤ 1e-12) and are counted as fails anyway. 0 unjudged — a case the runner cannot
score is reported, never dropped. At 83.5% it is 226 fail, the same 12 near, and
still 0 unjudged.

### TD-33: +126 cases, and the 1904 corpus stops being a hole · session 20

`cargo run --release -p conformance`, same corpus, same Excel build. The five
date functions reach **100% in both corpora** — `DATE` 30/30, `DAY` 12/12,
`MONTH` 11/11, `WEEKDAY` 22/22, `YEAR` 14/14 — from 76.7 / 75.0 / 72.7 / 45.5 /
64.3% under 1900 and 26.7 / 16.7 / 54.5 / 13.6 / 14.3% under 1904.

The 1904 corpus scored 20% because the engine had **no 1904 mode at all**: every
serial was off by the constant 1,462, so nearly every case failed for a single
reason. That is why it was the cheapest 104 cases in the register — one model,
not a hundred fixes.

**What this number does not include, stated so it is not read as more than it
is.** The residual date-adjacent failures are not date arithmetic:
`__compat_1900_leap` (13/22) and `__compat_serial_boundary` (12/19) are now held
down by `TEXT()` (TD-36), `DATEVALUE` and `EOMONTH`, each of which returns
`#NAME?` because it is unimplemented. Those cases belong to TD-36 and to the
function catalogue; counting them against TD-33 would make this row look worse
than the work was, and counting them *for* it would be worse still.

### TD-14 + TD-35: +31 cases, and every lookup function reaches 100% · session 20

`VLOOKUP` 24/24 · `HLOOKUP` 12/12 · `XLOOKUP` 15/15 · `MATCH` 16/16 · `FIND`
14/14 · `SEARCH` 13/13 — from 66.7 / 75.0 / 60.0 / 56.2 / 100 / 69.2%.

| | Cases | Pass | Rate |
|---|---:|---:|---:|
| before | 1,366 | 1,140 | 83.5% |
| after | 1,366 | **1,171** | **85.7%** |

**The measurement that decided the algorithm.** Over the *unsorted* key column
`30, 10, 50, 10, (blank)`, `VLOOKUP(35, …, TRUE)` returns the row holding **10**
— Excel's binary search probes the middle, finds 50 above the needle, halves
downward and lands there. A linear "largest key ≤ 35" scan answers 30. Both are
defensible; only one is Excel. This is the clearest case in the corpus for
ADR-024's premise that the binary is the spec, and the reason TD-14's refusal
was correct until vectors existed.

**Three `INDEX` cases still diverge and are *not* counted here.** All three are
TD-16 (implicit intersection needs the calling cell's position, which arrives
with the dependency graph). `INDEX(range,0,n)` now returns the whole column
correctly — `SUM(INDEX(A1:B5,0,1))` is 150 — but collapsing that array in a
scalar context still takes the top-left where Excel intersects against the
caller.

### TD-34: +18 cases, and the rule that criteria are not lookups · session 20

`COUNTIF` 15/15 · `COUNTIFS` 8/8 · `SUMIFS` 10/10 · `SUMIF` 18/20 · `AVERAGEIF`
7/9 — the four shortfalls are TD-50 (3) and one TD-15 float `near`.

| | Cases | Pass | Rate |
|---|---:|---:|---:|
| before | 1,366 | 1,171 | 85.7% |
| after | 1,366 | **1,189** | **87.0%** |

**The measurement that split the two comparisons.** `COUNTIF(range, 7)` counts a
cell holding the *text* `"7"`; `VLOOKUP(7, …)` does not find it. Criteria coerce
across the text boundary and lookups compare within it, so the shared
`values_equal` both families used was wrong for one of them. Nothing in the
documented contract distinguishes them.

**Three cases are TD-50 and are not counted here**: `SUMIF`/`AVERAGEIF` must
extend a short sum range to the criteria range's shape (`H1` means `H1:H5`),
which needs the *reference* rather than the materialised values — the same
information TD-16 waits on.

### TD-32: +16 cases, and the truncation has to happen on the text · session 20

`__compat_literal_parser` 29/29 — **100%**, from 51.7%.

| | Cases | Pass | Rate |
|---|---:|---:|---:|
| before | 1,366 | 1,189 | 87.0% |
| after | 1,366 | **1,205** | **88.2%** |

**The measurement that decided where the rule lives.** Excel truncates a literal
to 15 significant digits *before* converting it to a double, and truncates
rather than rounds: `=9999999999999999` is **9999999999999990**, where rounding
gives `1e16`. Truncating the parsed double cannot reproduce this at all —
`9999999999999999` has no exact `f64`, so it lands on `1e16` and the digits
Excel drops are already gone. The rule is therefore textual, which is also what
makes it identical on every target: `core`'s float formatting is pure Rust and
locale-free (DP-A2).

The second measured oddity is the underflow boundary. `=1E-308` and `=1E-309`
are both **0**, but `=1E-310` is a **parse error** — the line sits at the
*written* exponent, not at anything representable, since both are perfectly good
subnormals.

### TD-51 + TD-54: +15 cases from two rules where there had been one · session 20

| | Cases | Pass | Rate |
|---|---:|---:|---:|
| before | 1,366 | 1,205 | 88.2% |
| after | 1,366 | **1,220** | **89.3%** |

`SUM`, `COUNT`, `MAX`, `MIN`, `AVERAGE`, `PRODUCT` and the `IS` predicates are
now clean apart from two `SUM` cancellation cases, which belong to D-041 rather
than here.

**The measurement.** A value written as a direct argument is coerced; the same
value inside a range is skipped. `SUM("7",1)` is **8** and `SUM(TRUE,1)` is
**2**, while a text or logical *cell* is ignored — and `SUM("abc",1)` is
**`#VALUE!`** where a text cell would simply be passed over. The documented
description of `SUM` ("ignores text and logical values") describes only the
range half, which is why one rule had been written for both.

**One case left in this area and it is not this cluster's**: `ERROR.TYPE` is
unimplemented, so `ERROR.TYPE(NA())` is `#NAME?` rather than `7`. That is the
function catalogue, not argument handling.

### TD-52 + TD-53 + `ERROR.TYPE`: **the 90% target is met** · session 21

| | Cases | Pass | Rate |
|---|---:|---:|---:|
| before | 1,366 | 1,220 | 89.3% |
| after | 1,366 | **1,235** | **90.4%** |

`IF` 20/20 · `IFERROR` 10/10 · `IFNA` 8/8 · `NA` 6/6 · `AND` 12/12 · `OR`
10/10 · `NOT` 10/10 · `XOR` 10/10 — **every logical and conditional function is
now exact**.

**The 90% bar in docs/44's table is met and passed**: 1,235 of 1,366 oracle
cases match real Excel exactly under `Profile::Compat`, up from **74.2%** two
sessions ago.

**The measurement that inverted a rule.** A condition reads text that *spells* a
logical and refuses text that merely looks numeric: `IF("TRUE",1,2)` is **1**,
`IF("true",1,2)` is **1**, and `IF("1",1,2)` is **`#VALUE!`**. The engine had
sent conditions through the ordinary text→number coercion, which accepts `"1"`
and rejects `"TRUE"` — wrong in both directions at once.

**+15 where ~10 were forecast.** The extra three came from sharing the corrected
rule: `NOT("TRUE")`, and two `XOR` cases once `XOR` was given the same
skip-non-logicals-and-require-one behaviour `AND`/`OR` already had.

**Locale is deliberately excluded.** `YEAR("2024-03-15")` is implemented;
`YEAR("15/03/2024")` is not, although the fixture for it exists. The second
form's meaning depends on the capture host's regional settings, so implementing
it from this corpus would encode one machine's locale as engine behaviour.
Filed as TD-49 rather than guessed at.

**What the 8.6-point movement was.** The cancellation work alone moved 79 cases:
the positional `+`/`-` rule (`eval_top`, and an `Ast::Paren` node so
`=(0.1+0.2-0.3)` can differ from `=0.1+0.2-0.3` at all) and the unconditional
`SUM`/`AVERAGE` rule applied at every accumulation step. The remaining 39 came
from six measured divergences in docs/50 §7 — `ROUND(2.675,2)` (rounding moved
into the decimal domain, because scaling by `10^d` destroys the digit being
asked about), `FLOOR(x,0)` → `#DIV/0!` against `CEILING(x,0)` → `0`,
`POWER(0,0)` → `#NUM!`, `POWER(-8,1/3)` → a real odd root, `TRIM` keeping a
non-breaking space, `SUM` overflow → `#NUM!` — plus `UNICHAR`/`UNICODE`/`EXP`/
`LN`, each of which the corpus had measured as a `#NAME?`.

**The 1904 corpus is the honest embarrassment**, and it is reported at full size
rather than folded into the headline: 20.0%. It is 130 date cases, and the
engine has one date epoch. That is a known, unimplemented feature showing up as
exactly what it is.

**What this number does not include**, stated because omissions inflate
percentages: `general_text` (Excel's value→text coercion, where
`compat_round_15` lives) is not asserted, so this is *value* conformance, not
display conformance. Array semantics, number-format grammar, localised formula
text and XLSX round-trip fidelity are all outside the corpus (docs/50 §Limits).

**Largest remaining divergence clusters** (ranked, and filed as debt):
date semantics ~98 cases (TD-33) · approximate-match lookup ~24 (TD-14) ·
criteria sub-language ~20 (TD-34) · literal parser ~14 (TD-32) ·
`SEARCH`/`FIND` wildcards ~6 (TD-35) · `TEXT()` ~28 (TD-36).

## W-XLSX-CORPUS — XLSX read fidelity, per file · session 14

**Definition:** every file in `crates/usk-xlsx/tests/corpus/` read through
`usk-xlsx`, reporting part coverage, cells, formulas, losses and quarantined
parts. Printed by
`cargo test -p usk-xlsx --test xlsx -- --nocapture the_per_file_fidelity_report`.
docs/24 makes fidelity "a measured product attribute", and BOOTSTRAP row 12 asks
for a 20-file starter corpus; this is both.

**Corpus:** 20 files, hand-assembled by `tests/make_corpus.py` from the
ECMA-376 shapes Excel emits — *not* by a spreadsheet library, because a reader
tested against files its own writer produced proves only that two bugs agree.
It deliberately includes the awkward cases: inline strings, cached formula
results, error cells, a custom number format, sheets crossed over in the
relationship table, a dangling style index, a shared-string index past the end,
an unmodelled chart/drawing/theme, a stored (uncompressed) container, a missing
relationship part, and a macro payload that must be quarantined.

**What `part coverage` means (D-093):** parts read ÷ parts that carry user data
and are safe to read. Quarantined active content leaves the denominator because
not reading it is the *correct* outcome; package plumbing
(`[Content_Types].xml`, `_rels/.rels`) leaves it because it carries no user
data. Charts and drawings **stay** in the denominator — those are data this
build drops.

| File | Part coverage | Cells | Formulas | Losses | Quarantined |
|---|---:|---:|---:|---:|---:|
| `01-minimal.xlsx` | 100.0% | 1 | 0 | 0 | 0 |
| `02-numbers.xlsx` | 100.0% | 6 | 0 | 0 | 0 |
| `03-shared-strings.xlsx` | 100.0% | 3 | 0 | 0 | 0 |
| `04-formulas.xlsx` | 100.0% | 4 | 2 | 0 | 0 |
| `05-errors.xlsx` | 100.0% | 5 | 1 | 0 | 0 |
| `06-booleans.xlsx` | 100.0% | 3 | 1 | 0 | 0 |
| `07-inline-strings.xlsx` | 100.0% | 2 | 1 | 0 | 0 |
| `08-number-formats.xlsx` | 100.0% | 4 | 0 | 0 | 0 |
| `09-multi-sheet.xlsx` | 100.0% | 3 | 0 | 0 | 0 |
| `10-rels-out-of-order.xlsx` | 100.0% | 2 | 0 | 0 | 0 |
| `11-sparse.xlsx` | 100.0% | 4 | 0 | 0 | 0 |
| `12-entities.xlsx` | 100.0% | 3 | 0 | 0 | 0 |
| `13-macro-enabled.xlsm` | 100.0% | 1 | 0 | 0 | 1 |
| `14-unmodelled-parts.xlsx` | **50.0%** | 1 | 0 | 0 | 0 |
| `15-stored.xlsx` | 100.0% | 1 | 0 | 0 | 0 |
| `16-dangling-style.xlsx` | 100.0% | 1 | 0 | **1** | 0 |
| `17-bad-shared-index.xlsx` | 100.0% | 1 | 0 | **1** | 0 |
| `18-odd-cells.xlsx` | 100.0% | 2 | 0 | **3** | 0 |
| `19-no-optional-parts.xlsx` | 100.0% | 1 | 0 | 0 | 0 |
| `20-missing-rels.xlsx` | 100.0% | 1 | 0 | 0 | 0 |

**17 of 20 files read with no loss; 49 cells total.** The three that lose
something are the three built to lose something — a dangling style, an
out-of-range shared string, and a cell type this build does not model — and each
loss arrives as a named reason with its cell reference rather than a silent
substitution. `14-unmodelled-parts.xlsx` reads every cell it has and still
scores 50%, because it carries a chart, a drawing and a theme that v0.1 drops
(TD-39). That is the number doing its job.

**What this corpus is not.** It is 20 synthetic files, not the "thousands of
real-world workbooks" docs/24 asks for at release. It exercises the shapes, not
the long tail of what twenty years of Excel versions actually emit. A published
fidelity percentage needs the real corpus; this is the starter BOOTSTRAP asked
for and the harness the real one will run through.

## v0.1 AUDIT RE-RUN (session 16, 2026-08-09) — every W-* workload, current code

docs/38's regression policy makes a stale number a release blocker rather than a
footnote, so every workload was re-run against the tree as it stands. Three
moved. All three are explained below rather than quietly replaced, and the two
that are regressions are filed.

| Workload | Session 11–14 | **Audit re-run** | Verdict |
|---|---|---|---|
| W-REPLAY-5K | `c79fa533…` / `b58d5505…` | **identical**, native == wasm32 | unchanged |
| W-TILE-10M | 8.43 / 11.09 / 137.56 B/cell · 0% / 1.000% / 50.000% promoted | **bit-identical** | unchanged |
| W-SYNC-RELAY (50) | p50 800 / p95 3,700 bus-ms · converge 2,140 · 45/45 kill · all equal | **bit-identical** | unchanged |
| W-ORACLE | 74.2% (1,014 / 1,366) | **74.2%** | unchanged |
| W-XLSX-CORPUS | 19/20 at 100% coverage, 17/20 lossless | **unchanged** | unchanged |
| W-CHAIN-100K full recalc | 92.6 ms | **114.0 ms** (+23%) | **regression — TD-44** |
| W-CHAIN-100K single edit | 0.328 ms | **0.618 ms** (+88%) | **regression — TD-44** |
| W-OPEN-1M cold open | 2.10 s | **7.86 s** | **workload changed — see below** |
| W-OPEN-1M salvage | 657 ms, `lost_data=true` | **6.49 s, `lost_data=false`** | **workload changed** |
| W-OPEN-1M container | 108 MB | **307 MB** | **workload changed** |

### W-CHAIN-100K: the compat cancellation rule costs 23%

Cause, measured rather than guessed: D-041's positional rule means every
top-level `+`/`-` formula now runs `compat_final_adjust` — a binade read and a
comparison — and the chain's 100,000 formulas are all `=prev + const`, i.e. all
top-level adds. It is the worst possible shape for this rule and therefore the
right one to quote.

**A larger regression was found and fixed during the audit.** The first
`eval_top` re-evaluated both operands to recover their magnitudes, which doubled
the work of every such formula: 92.6 → **145.2 ms**. Restructuring it to
evaluate each operand once took it to 114.0 ms. The remaining 23% is the rule
itself and is not removable without giving up the conformance it buys (79 oracle
cases, MEASUREMENTS W-ORACLE).

Both budgets still pass with margin — 114.0 ms against 200 ms, 0.618 ms against
8 ms — so this is cost, not breach. Filed as **TD-44** per docs/38.

### W-OPEN-1M: the workload changed, because the old one measured an
### impossible container

The harness wrote **one** snapshot and only the uncovered tail. That is the
shape whose corruption lost 1,002,000 ops and produced TD-30 — and since
session 12 it is *unreachable through the container's own API*, because
compaction refuses to prune below two verified snapshots. Measuring it would
have been measuring a state the product cannot get into.

The harness now builds what docs/16 §Retention actually produces: three
snapshots at 80% / 90% / 100% of the compacted history, plus every op since the
oldest. The numbers are therefore **not comparable** to session 11's, and the
old ones are superseded rather than regressed.

What the new shape proves, which the old one could not:

> **Corrupting the newest snapshot's final page loses nothing.**
> `snapshots rejected 1, tail 200,200 ops, quarantined 0 B, lost_data = false`

That is TD-30's guarantee at 1M cells, measured. The old harness reported
`lost_data = true` for the same operation.

The bill, stated plainly:
* **Container 108 → 307 MB (2.8×)** — three snapshot bodies of 94 MB each,
  because a v0.1 snapshot body *is* the compacted op set (D-069). In docs/16's
  designed Merkle-shared tile image the extra snapshots are O(dirty) and this
  disappears. **TD-31.**
* **Cold open 2.10 → 7.86 s** — the retained tail is now 300,400 ops rather than
  100,000, the file is 2.8× larger, and opening still replays everything
  (D-076). A `BTreeSet` → `HashSet` change for the snapshot-coverage test during
  the audit removed ~0.9 s of it; the rest is inherent to the op-set body.
  Against docs/31's 1.5 s budget — which is for *skeleton + viewport*, not a
  full replay — this remains **neither a pass nor a breach**, and it is now
  further away. **TD-45.**

Recoverability was the right side of that trade and it is not a free one. Both
entries close with the same change: the tile-image snapshot body, which is also
TD-24's residual.

## W-CHAIN-100K re-measured (docs/38) — A-003, and a number that was noise · session 21

Run: `cargo run --release -p calc-bench`. Same M1 machine, same 10,000 x 10
shape (100,000 formula cells).

| | budget | audit (session 16) | **session 21** |
|---|---|---:|---:|
| full recalc, single-threaded | 200 ms (A-003) | 114.0 ms | **105.8 ms** |
| incremental, one edit | 8 ms (docs/31) | 0.618 ms | **0.436 ms** |
| graph build | — | 699 ms (session 7) | **857 ms** median of 5, range **801–918** |

**Neither TD-17's nor TD-44's trigger is live, and this is the measurement that
says so rather than the register quoting itself.** TD-17 is gated by owner
directive (D-054) on *a real workload breaching 200 ms single-threaded*: full
recalc is at **53% of budget**. The bench also reports `max parallel width ~
1.0` for this shape — a chain has no parallel width to exploit — so building
PAL `Compute` and the rayon bench now would be speculative twice over. TD-44's
trigger is *a W-\* workload approaching its budget*: incremental is at **5.5%**.
Both stay open, unpaid, and correctly so.

### The graph-build figure, and why no debt entry was filed

A single run measured **875.9 ms** against the 699 ms on record — a 25% gap,
which docs/38 §39 would make a signed debt entry. It is **not** one, and the
process that reached that conclusion is the point.

The suspect was TD-32's literal parser: `compat_parse_15` runs on every numeric
literal, and its first implementation allocated a `String` per literal to count
significant digits. Removing that allocation is a strict reduction in work. It
moved the number to **1090 ms** — *worse* — which is impossible as a causal
result and therefore evidence about the measurement rather than the code. Five
runs then put graph build at **801–918 ms**, so both the 876 and the 1090 sit
inside ordinary variance and the original 699 ms is a **single unreplicated
sample** from session 7 that cannot be compared against a median.

Recorded as measured, with three consequences stated plainly:
* **No debt entry**, because there is no demonstrated regression — filing one
  on an unreplicated delta is the defect D-104 was written about, in the
  performance register instead of the debt register.
* **No fix claimed.** The allocation removal is kept because doing less work on
  a hot path is right regardless, and its comment says so rather than claiming
  a speed-up it did not produce.
* **The 699 ms row is superseded** by a replicated median with a range. A
  benchmark that reports one sample invites exactly this, which is why the
  recalc figures beside it are already medians of five.

## W-SCROLL (docs/38) — the first renderer number · session 21

Run: `shell/target/release/ehkatra-shell --bench 1000000`. 120 consecutive
frames, scrolled by a different amount each frame so nothing is reused between
them, over a **1,000,000-row × 40-column** document at 1280×800.

| | measured | budget (docs/31) |
|---|---:|---:|
| **CPU frame** — viewport + scene build | p50 **0.192 ms** · p99 **0.316 ms** | 8.3 ms |
| CPU + GPU incl. readback | p50 6.239 ms · p99 8.010 ms | — |
| quads per frame | **331**, in **1 draw call** | — |
| axis build (one-off) | 31.9 ms for 1M rows | — |

**The number against the budget is the CPU frame: 0.192 ms p50, about 2% of
docs/31's 8.3 ms.** That is the work a scroll actually costs — resolving the
anchor, walking the visible span, and building the instance buffer — and it is
flat in document size because virtual scrolling means 39 rows are produced
whether the sheet has a thousand or a million.

**The 8.010 ms figure is NOT the budget number, and reading it as one would be
the convenient interpretation D-062 forbids.** It includes a full
texture-to-buffer readback and a blocking map, which exist so the frame can be
written to a PNG and committed as evidence; a presenting frame does none of it.
The real GPU present cost is **unmeasured** until the windowed path exists, and
is recorded as unmeasured rather than inferred from this.

Zero-jank at p99 is likewise not yet claimed: 120 frames is enough to see a
median, not enough to characterise a tail, and there is no compositor in the
loop to jank against.

### Evidence

`demo/grid.png` — a real frame: 200,000 rows scrolled to row 2,041, 39×20
visible, cells coloured by kind from the live `State`. The blue block is the 12
value columns, green is the formula column, pink is an error cell, and the
2-of-3 row banding is the corpus's own `r % 3 == 2` gap, not a decoration.

**Two defects the first frame exposed**, both fixed and both worth recording
because a screenshot is what found them:
* Every formula cell was missing. The scene gated on `state.cell()`, and a cell
  holding only a formula has no value in the tile store — so the entire formula
  column drew as empty. Reading the image is what caught it; no test asserted
  "the formula column is visible".
* Colours were washed out. The theme was written in sRGB and handed to an
  `Rgba8UnormSrgb` target, which encodes linear→sRGB on write — so everything
  was encoded twice. Tokens are now converted sRGB→linear on the way to the GPU.

### PNG writer: 4.1 MB → 341 KB

The first encoder emitted DEFLATE **stored** blocks, making each screenshot
4,097,178 bytes — untenable when `demo/` gains a frame per feature for a
quarter. Replaced with fixed-Huffman DEFLATE matching at distance 1 (a run) and
distance `stride` (a repeated scanline), which is what a flat-shaded grid
actually contains: **341,381 bytes, 12× smaller**, same image. Proven by
round-tripping the stream through the project's *own* inflater (`usk-zip`)
rather than by inspection — a hand-written compressor that emits a plausible
stream no decoder accepts is the obvious failure, and only a round trip catches
it.

## Shell dependency closure (ADR-037) — **measured, and it moved the kernel** · session 21

ADR-037 required the shell's dependency ceiling to be measured before the first
GPU dependency was committed. It was. The ceiling is the *less* important of the
two numbers that came back.

Measured by adding `winit = "0.30"` and `wgpu = "23"` to a new `ehkatra-shell`
member and running `node tools/dep-budget.mjs` (resolution only — `cargo
metadata` does not compile):

| | before | with the shell as a workspace member |
|---|---:|---:|
| **kernel dep closure** (cap **12**, D-035) | 10 | **13 — FAIL** |
| non-shell workspace closure (cap 40) | 29 | 34 |
| shell closure | — | **196** |

**The headline is not 196.** It is that putting the shell in the same workspace
pushed the **kernel** closure over its cap. Cargo resolves one lockfile per
workspace and unifies versions globally, so pulling in a GPU stack re-resolved
crates the kernel already depended on: `cc` moved and brought `jobserver`, and
`getrandom` + `r-efi` arrived in the shared graph (with `wasip2` and
`wit-bindgen` in the wider workspace).

That cap is not decoration. The kernel closure is what keeps `usk-*` buildable
as `no_std` against `wasm32-wasip1`, which is the gate the entire determinism
argument rests on (DP-A3, and the differential-replay evidence for DP-A2). A
GUI arriving must not be able to move it, and as a workspace member it does.

**Conclusion, recorded as D-116: the shell cannot be a member of the kernel's
workspace.** Verified both ways — removing it returns the closure to exactly
10/12 and 29/40, so the contamination is the shell and not a routine lockfile
refresh.

### The separate workspace, and the ceiling set from it

`shell/` is its own workspace with its own lockfile, depending on the kernel
crates by path. Re-measured with it in place:

| | cap | measured |
|---|---:|---:|
| kernel direct deps | 5 | **1** |
| kernel dep closure | 12 | **10 — unchanged** |
| non-shell workspace closure | 40 | **29 — unchanged** |
| **shell workspace closure** | **280** | **231** (was 230; `pollster` for wgpu's async setup) |

**The kernel and workspace lines are byte-identical to what they were before the
shell existed**, which is the property the separate workspace was for: the
shell's dependency choices cannot reach the kernel's resolution graph, so
DP-S2's kernel line is structural rather than something to keep re-checking.

The 230 is `winit 0.30` + `wgpu 23` plus the **registry** closure of the kernel
crates the shell depends on (`usk-state`, `usk-calc`, `usk-reduce`,
`ehkatra-store` — which is where `rusqlite` and `libsqlite3-sys` come in). Path
dependencies are excluded by source: a registry source is a crate we did not
write.

**Ceiling 280 = 230 measured + ~50 for named, already-owed work**: accesskit for
the a11y tree, and the file-dialog and menu adapters docs/33 specifies. Nothing
else. Deliberately not more generous — DP-S2 exists to make growth visible, so
the next raise should cost an ADR; but a ceiling that the first planned adapter
breaches would make the gate a nuisance, and nuisances get raised without
thought.

## W-OPEN-1M with an image body (docs/38) — **TD-45 and TD-31 paid** · session 21

Run: `cargo run --release -p open-bench`. Same 1,000,000-cell corpus (1000 x
1000) plus a 100,000-op tail, three retained snapshots.

| | before (op-set body) | image, naive sidecar | **image, per-tile sidecar** |
|---|---:|---:|---:|
| cold open to READY | 7.86 s | 7.96 s | **1.79 s** |
| SALVAGE (corrupt page) | 6.49 s | 6.70 s | **2.24 s** |
| container | 307 MB | 318 MB | **148 MB** |
| 3 snapshot bodies | — | 265.6 MB | **95.6 MB** |

**TD-45 and TD-31 are paid: cold open 7.86 → 1.79 s (4.4×) and container
307 → 148 MB (2.1×)**, with salvage 6.49 → 2.24 s as well. docs/31 budgets 1.5 s
for *skeleton + viewport*; this is a full open of a 1M-cell workbook, so it is
now close to a budget it was 5× outside.

### The middle column is the lesson, and it is kept on purpose

The first implementation was **correct, fully tested, and bought nothing** —
7.96 s and 318 MB, marginally *worse* than the op-set body it replaced. The
correctness argument and the encoding were separable, and only the encoding was
wrong: The container half of ADR-036 is implemented,
correct and tested — `verify` decodes instead of replaying, coverage excludes
un-representable ops, DP-A5 holds — and it delivers **none** of the size or
speed the ADR priced. The measurement says why, and the arithmetic closes:

| | per cell | at 1M cells |
|---|---:|---:|
| D-102 priced (per-tile writer index + delta-varint) | **3.10 B** | 3.1 MB |
| **what was implemented** (global map, full identity keys per entry) | **66 B** | 66 MB |

Each stamp was written as `row OpId` (24 B) + `col OpId` (24 B) + lamport varint
+ actor `u128` (16 B) + counter varint. That is **the naive layout D-102
explicitly measured and rejected**, arrived at again by storing the sidecar as a
flat `BTreeMap<(RowId, ColId), Stamp>` and serialising its keys — instead of
laying it out *per tile*, positionally, where the tile already knows which cells
it holds and no identity needs storing at all.

Three images at 66 B/cell plus 2.7M covered op ids at 24 B come to ~288 MB
against a measured 265.6 MB (the two smaller snapshots hold fewer cells). The
model and the measurement agree, which is what makes this a diagnosis rather
than a guess.

**Fixed (TD-56 paid).** The section is now laid out per tile and positionally:
the tile already knows which cells it holds, so no identity is stored, and the
48 bytes of `row`/`col` `OpId` per entry disappear. Per tile it is a writer
table (usually one actor) plus a delta-varint `(lamport, counter)` run over
present cells — exactly what D-102 measured. The images now total ~30.7 MB for
three snapshots, about **10.2 B/cell** including stamps, against 8.30 B/cell for
the stamp-less image: the sidecar costs ~1.9 B/cell here, inside D-102's 3.10.

**What the meaning tests did for this.** The round-trip, loser-equivalence and
refusal tests pin what the section *means*; the encoding change rewrote how it
is spelled and not one of them needed touching. That is the whole argument for
writing them first — a 66 → 1.9 B/cell rewrite of a format landed with the
safety net already in place.

**The remaining cost is now the covered-id list**, not the image: 2.7M ids at
24 B is **64.9 of the 95.6 MB** of bodies. Filed as **TD-57** — a per-actor run
encoding would collapse a dense counter range to a pair, and every id in this
corpus comes from one actor.

## W-IMAGE-STAMPS (docs/38) — the stamp-carrying tile image vs A-001 · session 18

**The question.** `usk_state::image` round-trips a `State` to the same hash, but
it cannot be `snapshots.body` until a summary tile carries per-cell winner
stamps — without them, adopting an image and applying a tail loses the identity
of a retained loser, which ADR-006 and DP-A8 promise to keep (D-101, TD-46).
Per-cell metadata is precisely what ADR-005 exists to avoid and what TD-09
measured the cost of, so the encoding is a decision that has to be measured
against **A-001's 400 MB collab-pattern bar**.

**Run:** `cargo run --release -p image-bench -- <rows> <cols>`.

### Measured, 1M cells (1000 x 1000)

| pattern | state B/cell | image B/cell | naive stamp | writer-index stamp | **delta-varint stamp** | promoted |
|---|---:|---:|---:|---:|---:|---:|
| import (1 actor) | 8.67 | 8.30 | 32.00 | 17.00 | **3.00** | 0.00% |
| collab (3 actors, 1%) | 11.33 | 9.59 | 32.00 | 17.00 | **3.10** | 1.00% |
| adversarial (3 actors, 50%) | 137.80 | 68.43 | 32.00 | 17.00 | **7.95** | 50.00% |

At 2M cells the per-cell figures are **stable**: import 3.00, collab 3.12,
adversarial 9.00. Only the adversarial pattern drifts, and it drifts because a
50%-contested history deltas badly — it has no RSS bar (docs/38).

### The answer, projected to 10M cells at the collab pattern

Measured state RSS there is **123.6 MB** (W-TILE-10M).

| stamp encoding | sidecar | total | vs A-001's 400 MB |
|---|---:|---:|---|
| naive, as it sits in memory (32 B/cell) | +305.2 MB | **428.8 MB** | **FAIL** |
| per-tile writer index + `u64` pair (17 B/cell) | +162.1 MB | 285.7 MB | pass |
| **writer index + delta-varint (3.1 B/cell)** | **+29.6 MB** | **153.2 MB** | **pass, 2.6x headroom** |

**So the answer is encoding-dependent, and only one of the three obvious choices
fails.** The naive layout — the one TD-46 assumed when it priced this at 24 B and
called it "the memory TD-09 removed" — is the one that fails, by 7%. It is also
the one nobody would ship: within a tile, a bulk write assigns lamports and
counters that ascend almost in lockstep, so the deltas are one varint byte each
and the writer is a one-byte index into a per-tile table.

Two things make the real cost lower still, and both are stated rather than
claimed: the figures count **every** cell, but a promoted cell already carries
its stamp in `Meta::Mixed` and needs no sidecar entry; and the sidecar only costs
RSS if it is *decoded* at load — kept encoded and decoded per tile as the tail
reaches it, the load cost is the image's own bytes.

**One number worth noticing on its own:** the image is *smaller than the resident
state it encodes* — 9.59 against 11.33 B/cell at the collab pattern, and 68.43
against 137.80 at the adversarial one, where the tagged-union in-memory layout is
far heavier than the packed serialised form.

## Not yet measured (targets remain targets — docs/42)
A-001 memory/10M cells · A-002 promotion rate · A-003 recalc 100k · A-005 wasm32 **in a real browser / Safari** (WASI-under-Node is not a browser and must not be reported as one) · all docs/31 budget rows.
