# CLAUDE.md — Ehkatra Project Memory

Repo: https://github.com/vrajchaithanya/Ehkatra — AI-native, web-first, CRDT-collaborative spreadsheet platform.
Authority order: `docs/06-design-principles.md` (the rule set) > module docs (`docs/10–24`, `30–36`) > code comments. `docs/07-solo-operating-model.md` binds the solo/automation constraints incl. DP-S5 host isolation. `BOOTSTRAP.md` defines the build order; `PROGRESS.md` is the live state.

## Session start protocol (EVERY session, fresh or resumed)
1. Read, in order: this file → `PROGRESS.md` → `BOOTSTRAP.md` → `docs/06-design-principles.md` → `docs/07-solo-operating-model.md` → the module doc(s) for the row you're about to build.
2. **Trust but verify PROGRESS.md**: run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. If a claimed-done row fails its gate, fix it BEFORE new work and note the repair in PROGRESS.md.
3. Continue from the first unproven row in BOOTSTRAP's MVP table. Never re-design what a doc already decides; never rebuild what a gate already proves.

## Session end / context-limit protocol (the handoff contract)
You WILL run out of context on long builds. Hand off cleanly so the next session (which remembers nothing) continues seamlessly:
- **Checkpoint early, not at the cliff**: when a work unit completes (or you sense a long stretch ahead), stop at a green state — compiles, tests pass — never mid-refactor.
- Update `PROGRESS.md`: what's DONE (with the test/bench that proves it), what's IN-PROGRESS (exact next action, file paths, the failing test if any), decisions taken (mirror durable ones into `docs/43-decision-register.md`), gates status.
- Update `MEASUREMENTS.md` for any new number (measured-with-link or it isn't stated).
- `git add -A && git commit` (conventional message) — commit even partial-but-green work.
- The repo + these two files ARE the memory. If it isn't written there, the next session doesn't know it.

## Autonomous operating contract
1. **Never ask the user questions.** Every choice: adopt the recorded decision in `docs/43`; if genuinely new, take the best-engineering-judgment recommended option, record it (ADR if irreversible), continue. The user's standing instruction: go with the top-1% recommended choice.
2. Stop only on true external blockers (missing credential, dead network). Everything else has a default.
3. Small verified increments: build+test after every component; never stack two unverified layers (DP-C4).
4. Report completion in PROGRESS.md, not by waiting for approval.

## Non-negotiable invariants (docs/06 §A; violating = wrong even if tests pass)
- Ops are the only mutation path; state mutators stay `pub(crate)` to the applier (DP-A1).
- Kernel crates `no_std + alloc`; no std fs/net/time/thread; entropy/time injected via PAL (DP-A2/A3).
- One canonical encoding per op; BLAKE3 state hash; op semantics immutable — new behavior = new op type (DP-A4/A5).
- Determinism gate: `tools/replay-check` native vs wasm32 hashes identical — run it after touching oplog/state/formula code.
- Errors are values with origin traces; no panics across boundaries (DP-A10). No `unwrap()` outside tests (DP-C1).
- **DP-S5 host isolation**: never touch the user's Postgres or any existing service; SQLite files in-repo/app-dir only; future servers loopback-only on ports 7423/7424, fail-fast if taken; no Windows services, no admin, no global installs.

## Stack decisions (made — do not relitigate; docs/43 has rationale)
Rust stable (pinned) · Cargo workspace · web-first (wasm32 gate permanent) · SQLite for all storage (ADR-031) · DataFusion for Q1 SQL (TD-01) · proptest · cargo-fuzz · blake3 · rayon behind PAL · custom canonical encoding behind `Op::encode` (CBOR wrapper later) · complexity budget DP-S1/S2: kernel ≤5 deps, workspace ≤40, one of each hard thing.

## Quality gates (green before any row is "done")
`cargo fmt --check` · `clippy -D warnings` · `cargo test --workspace` · replay-check native==wasm32 (`cargo build --release -p replay-check --target wasm32-wasip1`, run via Node WASI, diff hashes) · no `use std::` in `crates/*/src` · no `cfg(target_os)` outside `shell/`/`pal/` · new numbers land in MEASUREMENTS.md with evidence.

## Style
Doc-comments on public items explain *why* and cite the governing doc/ADR ("ADR-006"). Tests named for the behavior they prove. Conventional commits. Keep PROGRESS.md written for a human returning after days away.
