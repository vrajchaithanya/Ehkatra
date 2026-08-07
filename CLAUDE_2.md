# CLAUDE.md — Ehkatra Project Memory

Repo: https://github.com/vrajchaithanya/Ehkatra — AI-native, web-first, CRDT-collaborative spreadsheet platform.
Architecture authority: `docs/` (41 governed documents). Read `docs/00-INDEX.md` first; `BOOTSTRAP.md` defines the current build phase. Conflicts between code intent and docs are defects — follow the docs or record an ADR.

## Autonomous operating contract (this session runs unattended)
1. **Do not ask the user questions.** For every choice, adopt the recommended option already recorded in `docs/43-decision-register.md`; if a decision is genuinely new, pick the best-engineering-judgment default, record it as a dated entry in `docs/43` (ADR if irreversible), and continue.
2. **Stop only on true external blockers** (missing credential, network-dead registry). Everything else has a default — use it.
3. Work in **small verified increments**: after every component, run `cargo build && cargo test` (and gates below). Never stack two unverified layers.
4. **Commit after every green milestone** with conventional-commit messages; push at phase boundaries.
5. Keep `PROGRESS.md` current: what's done (with test evidence), what's next, decisions taken. This file is the user's return-view — write it for a human catching up.

## Non-negotiable invariants (from docs/10; violating these = wrong, even if tests pass)
- Ops are the only mutation path; state mutators stay `pub(crate)` to the applier.
- Kernel crates are `no_std + alloc`; no `std::{fs,net,time,thread}`, no ambient time/randomness — inject via PAL traits.
- Reducers are pure and versioned; Commands compile to Ops once, at the author.
- Canonical CBOR encoding (one valid encoding per op); BLAKE3 Merkle state hash.
- Determinism gate: identical op logs ⇒ identical state hash on native and wasm32 — CI-enforced from week one.
- Errors are values with origin traces; evaluation never panics across the FFI boundary.

## Stack decisions (already made — do not relitigate)
Rust stable (pinned in `rust-toolchain.toml`) · Cargo workspace monorepo · web-first: wasm32 target + WebGPU/Canvas2D later, headless-first now · SQL via DataFusion (TD-01) · property tests via `proptest` · fuzz via `cargo-fuzz` · CBOR via `ciborium`-compatible canonical encoder (write our own canonical layer) · hashing `blake3` · parallelism `rayon` (behind PAL Compute) · server later (Q1 is kernel + CLI + local two-replica sync).

## Quality gates (all must be green before a phase is "complete")
`cargo fmt --check` · `clippy -D warnings` · `cargo test --workspace` · kernel `no_std` check-build · differential replay test (native vs wasm32 via wasmtime) hash-equal · proptest CRDT convergence suite · benches compile and run (record numbers in `MEASUREMENTS.md`, never assert unmeasured claims).

## Style
Small crates per docs/10 layering; doc-comments on public items explain *why*; no `unwrap()` outside tests; every `unsafe` block justified in a comment and minimized (target: zero in Q1).
