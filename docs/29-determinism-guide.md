# 29 — Determinism Verification Guide
Status: Approved · Normative: yes · The DP-A2 operator's manual — read when touching oplog/state/formula/calc code, and the FIRST time hashes diverge

## The contract
Identical op sets ⇒ bit-identical state hash, on every architecture, forever. Two gates enforce it: the convergence property suite (same set, shuffled orders) and `tools/replay-check` (same binary logic, native vs wasm32, diffed output). CI runs both on every push.

## Running the gate locally
```
cargo build --release -p replay-check
cargo build --release -p replay-check --target wasm32-wasip1
./target/release/replay-check > native.txt
node --no-warnings run-wasi.mjs target/wasm32-wasip1/release/replay-check.wasm > wasm.txt
diff native.txt wasm.txt   # empty = pass
```

## The banned-constructs list (what breaks determinism, ranked by how often it actually happens)
1. **HashMap/HashSet iteration** — randomized order per process. Use BTreeMap/BTreeSet or sort before iterating. (The #1 real-world offender.)
2. **Ambient time/randomness** — `SystemTime`, `Instant`-derived values in state, uninjected RNG. All entropy comes from op-carried seeds; wall time is op metadata, never sampled.
3. **Float traps** — NaN bit patterns (normalize before encoding — already done in `Value::encode_into`); FMA contraction differences (keep `-ffp-contract=off` posture; no `mul_add` in evaluation); reduction reordering (row-major identity-order traversal, Neumaier accumulation, never parallel-reduce without a fixed tree).
4. **Pointer/address leakage** — sorting by allocation address, using `ptr as usize` in any ordering or hash.
5. **Locale/environment** — number formatting, collation, env vars influencing evaluation. Locale is display-only (DP-D5).
6. **usize width** — wasm32 is 32-bit: never encode `usize` (encode u32/u64 explicitly); watch `as usize` truncation in the other direction.
7. **Parallelism nondeterminism** — rayon results must be order-merged deterministically; work-stealing may reorder *execution*, never *combination*.

## When the gate fails: the bisection drill
1. Confirm it's real: rerun both sides twice (a flaky diff = environment problem, e.g. stale build).
2. Which line differs — `oplog:` or `state:`? oplog ⇒ encoding bug (op bytes differ per platform: check usize/NaN/enum discriminants). state ⇒ apply/eval bug (same ops, different fold: check iteration order, float paths).
3. Localize: the Merkle structure exists for this — extend replay-check to print per-tile/per-axis subtree hashes (`--verbose` when implemented); the first differing subtree names the module.
4. Shrink: reduce the corpus seed-count by halves until the diff needs <20 ops; print those ops; the culprit is usually visible by inspection.
5. Fix, then **add the shrunk case as a permanent regression test** — determinism bugs recur in the same organs.

## When adding new state or ops
New op type → extend the canonical encoding (one valid encoding; fixed field order; big-endian) + add to replay-check's generator so the corpus exercises it. New state component → include it in `state_hash` (unhashed state is unverified state) and ask: is its content a pure fold of ops? If not, it doesn't belong in state. New evaluation path → check every item on the banned list, then run the gate on both targets before committing (DP-C4).
