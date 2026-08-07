# 06 — Design Principles: The Complete Rule Set
Status: Approved · Owner: Chief Architect · Normative: yes — **this document is the single authority for rules; every generated artifact (code, docs, APIs, tests) must comply or carry a recorded exception in docs/43**

The standard: top-1% engineering discipline. A principle without an enforcement mechanism is a poster; every rule below names its checker. Rules are grouped: **A** architecture invariants, **B** engineering method, **C** code rules, **D** API/data rules, **E** security rules, **F** process/autonomy rules. Each has an ID for traceability (docs/49) and for citing in reviews ("violates DP-A2").

## A — Architecture invariants (violating these = wrong even if tests pass)

| ID | Rule | Enforced by |
|---|---|---|
| DP-A1 | **Ops are the only truth.** All mutation flows through the op log — UI, API, MCP, import, AI, admin repair, no exceptions. State mutators are `pub(crate)` to the applier alone. | Rust visibility + lint; code review |
| DP-A2 | **Determinism is sacred.** Identical op logs ⇒ bit-identical state hash on every platform forever. No ambient time, randomness, hash-order iteration, locale, or FP-contraction in kernel paths; entropy and wall-time are injected data. | differential-replay CI (native+wasm32); reducer lint fence |
| DP-A3 | **One kernel, everywhere.** Kernel crates are `no_std + alloc`, zero OS deps; all platform access via the 10 PAL traits; adding a trait requires an ADR. | `no_std` CI build; cargo-deny graph rules |
| DP-A4 | **One canonical encoding per op** (deterministic bytes, fixed field order); BLAKE3 Merkle state hashing; op semantics immutable once shipped — new behavior is a new op type. | encode round-trip + hash-stability tests |
| DP-A5 | **Forward preservation.** Unknown op types are preserved, causally ordered, hashed opaque, retransmitted; `Cosmetic` unknowns keep editing, `Structural` unknowns force read-only. Files written today open in 20 years. | archived-corpus CI gate (permanent) |
| DP-A6 | **Identity over position.** Rows/cols/sheets carry permanent identities; A1 is a computed view; references are identity intervals; cells are intersections, not objects. | domain-model conformance tests |
| DP-A7 | **Commands compile to ops once, at the author**, by a pure versioned reducer; remote replicas never see Commands. | protocol schema; reducer purity lint |
| DP-A8 | **Convergence with honesty.** CRDT merge is commutative (property-tested + TLA+-modeled); concurrent register losers are retained 30 days and surfaced — never silently discarded. | property suite; model checking; ADR-006 tests |
| DP-A9 | **Caches are watermarked folds.** No independently mutable cache state anywhere; invalidation = dirty-interval intersection vs the log. | cache-coherence property tests |
| DP-A10 | **Errors are values with origin traces**; evaluation never panics or throws across a boundary. | zero-panic FFI tests; error-provenance coverage |
| DP-A11 | **Everything is explainable** (P4): any value/format/change answers "why am I this way" — op, actor, formula, rule, or AI action with taint record. | explain-API coverage conformance test |
| DP-A12 | **Undo is selective and other-preserving**: inverse vs current state; structural undo narrows rather than destroys others' work; agent sessions are undo scopes. | undo-law property tests |

## B — Engineering method (how we work; the top-1% discipline)

| ID | Rule | Enforced by |
|---|---|---|
| DP-B1 | **Evidence before assertion.** Every quantitative claim carries `measured(link)` or `target(assumption-id)`; assumptions have validation dates; work may not build on an assumption past its date. | MEASUREMENTS.md convention; docs/42 review |
| DP-B2 | **Irreversible decisions get rigor first** (proof, spike, or ADR with alternatives); reversible decisions are made fast and recorded. The five frozen irreversibles: op algebra, identity model, tile granularity, PAL seam, value lattice. | ARB review; docs/43 |
| DP-B3 | **Walking skeleton over specification.** Ship the thinnest end-to-end slice first; the spec follows the code's evidence, not the reverse. | roadmap gates |
| DP-B4 | **The oracle is the binary, not the docs.** Excel conformance = captured vectors from real Excel; fidelity and conformance are published numbers per release. | oracle corpus CI |
| DP-B5 | **Formal methods where bugs are eternal**: TLA+ models for merge semantics and sync handshake; mutation-testing floor (≥85%) on op applier, reducer, evaluator, codecs. | CI model checking; mutation gates |
| DP-B6 | **Budgets before code.** Performance budgets are CI gates from day one; a breach blocks release; regressions >5% need sign-off + debt entry. | bench gates (docs/31) |
| DP-B7 | **Live ADRs.** Decisions are recorded when made, with the losing alternative and why; post-hoc rationalization is flagged as debt. | docs/43 rule; review |
| DP-B8 | **Scope displaces, never extends.** Capacity is fixed; anything entering a horizon displaces equal size; slips >2 weeks trigger displacement, not schedule extension. | docs/02 rule; ARB |
| DP-B9 | **Buy boring, build differentiating.** Never build what a proven component provides (SQLite, DataFusion, icu4x, accesskit, blake3, rustybuzz); never outsource the op algebra, reducer, or merge semantics. No custom crypto, ever. | dependency review; docs/43 |
| DP-B10 | **Debt is priced at birth**: chosen debt gets a register entry with interest and a repayment trigger; unpriced debt found later is a process defect. | docs/44 |

## C — Code rules (bind all generated code, including AI-generated)

| ID | Rule | Enforced by |
|---|---|---|
| DP-C1 | `cargo fmt` + `clippy -D warnings` clean; no `unwrap()`/`expect()` outside tests; every `unsafe` justified in a comment (target zero in kernel). | CI |
| DP-C2 | Crate layering is acyclic and downward-only per docs/10; `#[cfg(target_os)]` only in `shell/` and `pal/` trees. | placement lint |
| DP-C3 | Public items carry doc-comments explaining *why*, citing the owning doc/ADR (e.g. "ADR-006"). Tests are named for the behavior they prove. | review checklist |
| DP-C4 | Every merge is green on the full gate set (build, tests, no_std, replay, property, fmt, clippy); no unverified layer is built upon (small verified increments). | merge queue |
| DP-C5 | Test code follows the same rules as product code; a flaky test is a sev-2 defect, not an annoyance. | CI quarantine policy |
| DP-C6 | Commits are conventional-format, milestone-scoped; PROGRESS.md updated with evidence at every milestone. | hook + review |

## D — API & data rules

| ID | Rule | Enforced by |
|---|---|---|
| DP-D1 | **UI ≡ API ≡ MCP**: one Command vocabulary; the UI holds no capability the API lacks; schemas are generated from the vocabulary. | codegen; UI-command diff CI |
| DP-D2 | **Three layers, no privileged layer**: L1 1:1 Commands, L2 composes L1/L3, L3 changes transport never semantics; ACL/audit/undo/version preconditions identical at all layers. | contract tests |
| DP-D3 | Writes carry `Idempotency-Key` + optimistic `If-Match` watermark; 409s return intervening ops; batches are atomic labeled undo groups. | API contract tests |
| DP-D4 | Additive evolution only within a major; deprecation = 2 LTS cycles + <0.1% telemetry + migration note. | schema-diff CI |
| DP-D5 | Locale never enters storage or evaluation — parsing/formatting/collation are display-layer; collation choices are recorded in descriptors for reproducibility. | kernel locale lint |

## E — Security rules (structural, not remedial)

| ID | Rule | Enforced by |
|---|---|---|
| DP-E1 | Formulas have **no ambient I/O** — external fetch only via the Calculation Authority under egress allowlist. | no_std construction; egress tests |
| DP-E2 | All untrusted bytes (files, clipboard, network payloads) parse in a sandbox emitting schema-revalidated IR only; resource caps; zip/XML bomb defenses. | sandbox-escape suite; fuzz |
| DP-E3 | Active content (macros, OLE, DDE) is quarantined — never executed, never re-emitted by default. | ingest-policy tests |
| DP-E4 | Ops are validated (schema+bounds) on receive — a permitted collaborator is still an untrusted input source. | adversarial-op corpus |
| DP-E5 | Agent tokens are scoped, short-lived, intent-declared; agent mutations are previewed above blast-radius, auto-milestoned, session-undoable, taint-recorded; guardrails enforced at the host/relay, never by tool etiquette. | relay enforcement tests; red-team evals |
| DP-E6 | Cell-derived text is labeled untrusted data in every AI/tool response (injection posture). | MCP contract tests |
| DP-E7 | Telemetry and crash reports are content-free by type-level construction; support bundles user-inspectable. | content-taint lint |
| DP-E8 | Supply chain: pinned toolchain, vendored+vetted deps, reproducible builds, SLSA-3, signed artifacts, SBOM per release. | release pipeline |

## F — Process & autonomy rules (bind human and AI builders equally)

| ID | Rule | Enforced by |
|---|---|---|
| DP-F1 | Autonomous sessions never block on questions: adopt the recorded decision, else best-judgment default + register entry, and continue; stop only on true external blockers. | CLAUDE.md contract |
| DP-F2 | Every milestone leaves the repo shippable: green gates, updated PROGRESS.md, honest MEASUREMENTS.md. | CI + review |
| DP-F3 | Docs are code: a behavior change PRs the owning doc or states why not; conflicting docs are defects; terms follow the glossary. | review rule |
| DP-F4 | Conservative self-scoring: nothing above 9 without strong evidence; Evidence Maturity caps the composite while numbers are targets. | docs/47 cadence |
| DP-F5 | Failures are surfaced, never smoothed: a failed assumption executes its named consequence; a missed gate blocks; "mostly works" is not a status. | docs/42/48 |

## Compliance map — artifacts generated so far

| Artifact | Compliant with | Gaps (tracked) |
|---|---|---|
| docs/ repository (41 docs) | DP-B*, DP-F3 | interface IDL pending (scorecard) |
| Milestone-1 code (usk-types/oplog/state, CLI) | DP-A1 (applier-private mutation) · DP-A2 partial (deterministic replay proven natively) · DP-A3 (`no_std` attrs) · DP-A4 (canonical encode + hash tests) · DP-A6 (identity axes, A1-free core) · DP-A8 (convergence suite + retained losers) · DP-C1–C6 (fmt/clippy/tests/conventional commits) | **DP-A2: wasm32 replay gate not yet run** (next session, A-005) · DP-A3: dedicated no_std CI job pending · DP-B5: TLA+ models pending (Q1 W4–6) · DP-A5/A7/A9–A12: land with their milestones (rows 4–9) |
| CLAUDE.md / BOOTSTRAP.md | DP-F1, DP-F2, DP-B1 | — |

Rule of precedence: this document > module docs > code comments. A generated artifact that cannot cite its governing rule is presumed non-compliant until reviewed.
