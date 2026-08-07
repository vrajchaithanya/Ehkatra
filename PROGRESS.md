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

### NEXT (unchanged BOOTSTRAP order)
**Row 4: tile store** — 256×64 tiles, presence bitmap, packed f64/tagged payloads, per-tile causal summary + promotion; memory harness reporting bytes/cell into MEASUREMENTS.md (assumptions A-001/A-002). Then Row 5 values (Decimal128/Date + compat/strict coercion), Row 6 formula engine.
