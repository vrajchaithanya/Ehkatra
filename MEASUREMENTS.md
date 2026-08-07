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

## Not yet measured (targets remain targets — docs/42)
A-001 memory/10M cells · A-002 promotion rate · A-003 recalc 100k · A-005 wasm32 **in a real browser / Safari** (WASI-under-Node is not a browser and must not be reported as one) · all docs/31 budget rows.
