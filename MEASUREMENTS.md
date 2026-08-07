# MEASUREMENTS.md — every number here is measured, or it isn't here

| Claim | Measured | Where | Notes |
|---|---|---|---|
| Convergence: 200 random interleavings, 15-op history | 200/200 identical hashes | `randomized_interleavings_converge` | scale to 10^5 interleavings + larger histories in CI next session |
| Demo end-to-end (build state ×2 replicas + hash ×2, release) | 1.6 ms | timed run ×5, min | trivial workload; real benches arrive with Row 7 |
| Test suite wall time | <0.1 s | cargo test | |
| Differential replay: 5,000-op corpus, x86_64 vs wasm32-wasip1 | identical (oplog `77e5b1bf…`, state `4516044e…`) | `tools/replay-check` + `tools/run-wasi.mjs` | DP-A2 gate, CI-enforced |
| **Determinism across host + compiler**: same 5,000-op corpus on x86_64-unknown-linux-gnu / rustc "stable" (session 2, cloud) vs x86_64-pc-windows-gnu / rustc 1.97.1 (session 3, Windows host) | byte-identical: oplog `77e5b1bf2489a7a5e964e1284ad7dcc867b01af93a39d59663c9df7ce2ac5089`, state `4516044ed95e844c01b86b0693ea1f5509d970dde1e12856f85dfd2ac8438639` | `tools/gates.ps1` session-3 run vs PROGRESS session-2 record | Stronger than the CI gate: different OS, different libc, different linker (MinGW vs GNU ld), different compiler version — hashes unchanged. This is the first evidence that DP-A2 survives toolchain drift, not just target drift. |
| Kernel direct dependencies (DP-S2 budget 5) | 1 (`blake3`) | `tools/dep-budget.mjs` | |
| Kernel dependency **closure** incl. build scripts (D-035 budget 12) | 10 | `tools/dep-budget.mjs` | `blake3` pulls 9 transitively (`arrayref, arrayvec, cc, cfg-if, constant_time_eq, cpufeatures, find-msvc-tools, libc, shlex`). docs/07 §3's "currently 1" counted direct edges only — see D-035. |
| Workspace dependency closure (DP-S2 budget 40) | 10 | `tools/dep-budget.mjs` | |
| Full local gate set wall time (cold-cached workspace, warm deps) | ~35 s | `pwsh -File tools/gates.ps1` | fmt + clippy + tests + no_std wasm build + dep budget + differential replay + purity greps |

## Row 4 — tile store (`tools/tile-bench`, release, this machine)

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

### Determinism after the tile refactor
| | |
|---|---|
| oplog hash (5,000-op corpus) | `77e5b1bf…` — **unchanged** from sessions 1–2, proving the op algebra did not move |
| state hash | `e6cc2757e42581c6cacd47cfb0420c3364e5ae76e98d0a081bd7a8efb03f2957` — **new**, superseding `4516044e…`: cells now fold in tile-major order (the tile-Merkle direction docs/10 specifies) instead of flat identity order |
| native vs wasm32-wasip1 | identical |

## Not yet measured (targets remain targets — docs/42)
A-001 memory/10M cells · A-002 promotion rate · A-003 recalc 100k · A-005 wasm32 **in a real browser / Safari** (WASI-under-Node is not a browser and must not be reported as one) · all docs/31 budget rows.
