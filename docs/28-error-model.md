# 28 — Error Handling Model
Status: Approved · Normative: yes

Three disjoint error domains. Confusing them is the classic failure; each has its own type, its own propagation, and its own audience.

## Domain 1 — Spreadsheet errors (user-visible values)
`#DIV/0! #VALUE! #REF! #NAME? #NUM! #N/A #CIRC! #SPILL!` — these are **Values** (DP-A10), not failures. They propagate through evaluation by the per-function rules in docs/12, carry an `OriginTrace` (first-producing cell + sub-expression + path), and are *correct outputs*: a workbook full of `#N/A` is a healthy engine. Never logged as errors, never retried, never panic.

## Domain 2 — Operational errors (recoverable conditions)
Rust `Result<T, E>` with per-crate error enums (`thiserror`-style by hand; kernel stays dep-lean). Rules: no `unwrap()/expect()` outside tests (DP-C1); errors carry what the *caller* needs to decide, not prose; fallible APIs never partially mutate (validate → apply, or stage → commit); I/O errors at the PAL boundary are retried by policy at the caller, never inside the kernel. API surface mapping (Row 13+): structured `{code, target, message, remediation, trace_id}`; codes are stable API (docs/20 versioning applies to them).

## Domain 3 — Invariant violations (bugs)
A violated kernel invariant (divergent hash, op applied twice, mutation outside the applier) is a **bug, never a handled condition**. Policy: `debug_assert!` liberally; in release, detect → quarantine + report, never limp: a poison op quarantines by hash (docs/36); a state-hash mismatch marks the replica divergent, stops sync send, and preserves everything for a replay bundle. **No panic crosses an FFI/WASM boundary** — the facade catches at the boundary and returns a fatal-error code with a bundle pointer. Corruption honesty rule (docs/16): a salvage is reported as a salvage, never presented as a clean open.

## Cross-cutting rules
Every error path is a tested path (the untested error branch is where data loss lives — docs/35 contract tests cover truncation/denial/failure paths explicitly). Error text never contains cell contents when it may reach logs/telemetry (DP-E7); it names *locations*, not values. Agents receive machine-actionable errors: `expected_version` conflicts return the intervening ops; refusals name the policy that refused (docs/25's honesty rule applies to machines too).
