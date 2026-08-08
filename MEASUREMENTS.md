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

## Not yet measured (targets remain targets — docs/42)
A-001 memory/10M cells · A-002 promotion rate · A-003 recalc 100k · A-005 wasm32 **in a real browser / Safari** (WASI-under-Node is not a browser and must not be reported as one) · all docs/31 budget rows.
