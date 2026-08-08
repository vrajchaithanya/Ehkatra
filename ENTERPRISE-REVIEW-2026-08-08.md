# Independent Enterprise Architecture & Engineering Review — Ehkatra
**Date:** 2026-08-08 · **Type:** feedback/gap analysis only — no code or docs modified · **Reviewer stance:** deliberately critical, independent of the build sessions
**Evidence basis:** all 53 docs/ documents; session handoff reports 1–9+ (including measured numbers); live file inventory of `crates/` (confirms usk-sync and usk-recover now exist — the repo is ahead of the last written handoff); code snapshots of early crates. Limitation stated plainly: I have not line-read the newest crates (tile.rs, graph.rs, sync machine); where evidence is a session's own report, the Evidence column says so.

---

## 1. Executive assessment

Ehkatra today is a **small, unusually well-verified deterministic spreadsheet *kernel*** — an op log with canonical encoding and cross-architecture hash equality, an identity-addressed grid whose reference semantics survive concurrent structural edits, a 60-function formula engine with a grouped incremental dependency graph, selective multi-user undo, and (newly) a sync state machine and recovery scaffolding. Within its scope, the engineering discipline is well above typical startup practice: failures are recorded next to successes, regressions become priced debt, an unattainable bar was escalated rather than gamed, and the determinism gate found its own blind spot (4-of-9 payload coverage) and fixed it.

It is **not yet a spreadsheet application in any user-meaningful sense**, and it is **nowhere near Excel-compatible**. There is no XLSX read or write, no persistence to disk in shipping form, no API or MCP surface implemented, no UI, no styles/formatting/merged cells/named ranges/tables/multi-sheet model, no authentication, and CI has never executed because the repo has never been pushed. The single largest strategic risk is not any code defect — it is that **the Excel-compatibility oracle (the COM capture harness) has not started**, and it is the longest-lead item on the entire critical path to the product's stated identity.

The right one-line summary: **foundation quality ~9/10, product completeness ~1.5/10, and the gap between them is exactly what the roadmap says it is — provided three long-lead items start now rather than "later": the oracle corpus, XLSX I/O, and the first push that brings CI to life.**

## 2–6. Scores (0–100%)

| Dimension | Score | One-line justification |
|---|---|---|
| Spreadsheet-domain completeness | **14%** | Calculation substrate + editing ops + undo exist; nearly the entire feature surface (§7 table) is absent. |
| Excel compatibility readiness | **8%** | No XLSX I/O, no oracle corpus, compat-profile bugs (1900, 15-digit) designed but not implemented; unsupported functions do fail explicitly (#NAME?) — the one bright spot. |
| Calculation-engine maturity | **42%** | The strongest area: identity refs, grouped graph, incremental + early cutoff, measured budgets, determinism gate. Missing: multi-sheet, whole-row/col refs, volatile materialization beyond TODAY/NOW, iterative calc, parallel eval, spill. |
| Enterprise engineering readiness | **12%** | Governance artifacts are excellent; operationally almost nothing exists: no auth, no API, no observability, no deployment, no backup story beyond design, CI never run. |
| Implementation ↔ architecture alignment | **88%** | Exceptional *for what is built*: drift has been caught and either fixed (TD-21, replay coverage) or priced (TD-23). Deductions: RGA-vs-Fugue divergence from ADR-002; BOOTSTRAP/D-052 contradiction; sync/recover crates not yet reflected in any handoff doc I can verify. |

## 7. Domain review table (selected — full surface implied)

Legend: **I**mplemented / **D**esigned / Doc = documented / **T**ested / **XC** = Excel-compatible-verified / **PR** = production-ready

| Area | Status | Evidence | Gap/Risk | Sev | Impact | Recommendation |
|---|---|---|---|---|---|---|
| Op log, encoding, determinism | I+D+Doc+T | replay-check 9-variant corpus, native==wasm32, hashes recorded | none material | Low | — | maintain gate discipline |
| Identity references, insert/delete semantics | I+D+Doc+T | Row 8 canonical test; 5 shift rules each tested | whole-row/col refs (`A:A`) absent; external refs absent | Med | formulas real users write fail | schedule with function tiers |
| Order CRDT | I+T, **diverges from D** | ADR-002 says Fugue; code is RGA-style single-level (documented in-code) | multi-level interleaving anomalies possible under nested concurrent inserts | **High** | silent misordering in collab — trust-killer | upgrade to Fugue tree BEFORE sync ships to >2 real users; it's load-bearing for the whole wedge |
| Cell registers, retain-losers | I+D+Doc+T | convergence suite | loser *surfacing* (UI/API) unimplemented — retained data invisible | Med | ADR-006's value unrealized | expose via API at Row 14 |
| Formula engine (60 fns) | I+T | functions.rs 42 KB, formulas.rs tests | catalog is 60/~200 Core; per-function conformance is vs *documented* behavior, not oracle | **High** | "passes tests" ≠ Excel-correct — the mandate's own warning | start oracle capture NOW (long-lead) |
| Coercion compat/strict, Decimal128 | I+T | coerce.rs, decimal.rs, values.rs tests | Excel bug catalog (1900 leap-year, 15-digit display, date systems) NOT implemented | High | dates before Mar-1900 and display-precision comparisons will diverge from Excel | implement behind compat profile with oracle vectors |
| Dep graph, incremental recalc | I+D+Doc+T | W-CHAIN-100K measured (92.6 ms full / 0.191 ms incr.) | TD-23 regression priced; parallel path unbuilt (TD-17, trigger-gated — acceptable) | Med | budget headroom shrinking | watch TD-23 at W-WIDE-AGG |
| Circular refs | Partial | #CIRC! via level assignment (D-050) | iterative calc absent (designed T3) | Low | acceptable v0.1 | as designed |
| Volatile functions | Partial | TODAY/NOW materialized per design | RAND/OFFSET/INDIRECT family absent; recalc-event UX absent | Med | convergence design unproven beyond 2 fns | with function tiers |
| Multi-sheet workbooks | **Absent** | State is single-sheet (v0.1 scope note in code) | cross-sheet refs, 3-D refs impossible; SheetId exists in docs only | **High** | not a workbook yet; late retrofit touches op taxonomy | schedule deliberately — op types are forever (DP-A5); design the Sheet ops before Row 14 freezes the API |
| Editing: copy/paste/fill | **Absent** | no Commands for them in usk-reduce inventory | reference-rewrite-on-copy (reducer-time, per docs/11) untested design | High | core daily operations missing; agents can't fill ranges | next after Row 11 |
| Undo/redo | I+D+Doc+T | undo.rs 16 KB; undo-law + docs/27 §5 machine tests; D-057 bug found+fixed | memory efficiency vs range-compression spec unverified at 100k-paste scale | Low | — | add W-workload when paste exists |
| Sync (Row 10) | I(in progress)+D+Doc+T(machine) | usk-sync: machine/relay/queue/validate + 19 KB tests — matches docs/27 §1 | in-process relay only; no network transport; W-SYNC-RELAY not yet reported; never-drop under kill unverified in report | High | collab claim unproven end-to-end | complete per spec; report the kill test explicitly |
| Persistence/container (Row 11) | Partial | usk-recover: snapshot/salvage/machine present | **SQLite container (docs/26 schema) absent — no_std crates can't link rusqlite; the host-side container crate doesn't exist yet**; autosave, atomic save, crash recovery therefore not real | **Critical** | today a crash loses everything not manually exported; "no silent data loss" is unmet | build the std-side container crate (ehkatra-store) implementing docs/26 verbatim, WAL, crash-injection tests — before any user-facing use |
| XLSX import/export | **Absent** | no crate, no sandbox process | round-trip fidelity, preservation, corrupted-file handling: 0% | **Critical** (for product identity) | without XLSX there is no migration path and no fidelity number | start reader now; writer next; sandbox per docs/24 from the first line |
| Formatting/styles/merged/hidden/named/tables/filters/freeze/hyperlinks/comments/charts/print/metadata | Absent (D+Doc only) | docs/11,18 archive | the entire visible surface | High (aggregate) | user-perceived completeness ~0 | per roadmap; don't let kernel polish crowd these forever |
| Security: parser sandbox, zip/XML defenses | D+Doc only | docs/24/30/37 | nothing to sandbox yet, but the risk is building the importer WITHOUT it under schedule pressure | High (latent) | the #1 CVE class in this category | sandbox is part of the importer's definition-of-done, not a fast-follow |
| Formula execution safety | I (structural) | no_std kernel — no I/O possible; verified by construction | — | Low | genuine strength | none |
| Malicious collaborator ops | I+T | validate.rs in usk-sync + adversarial corpus (per handoff) | corpus breadth unknown | Med | — | grow corpus with sync completion |
| API/MCP (Rows 13–14) | Absent (D+Doc strong) | docs/20/21 | none of L1/L2/L3 exists; agents cannot touch Ehkatra today | High | the wedge is unimplemented | after Row 11; MCP-first per wedge |
| Auth/authz, multi-tenancy, audit chain | Absent (D+Doc) | docs/22/30 | bearer-token debt TD-06 still the plan | Med (H1) | fine for local; blocks any sharing | per roadmap Q3 |
| Observability, logging, correlation | Absent | — | not even structured logging in server-less mode; replay bundles designed only | Med | debugging via test rig only — works solo, fails with users | with API server |
| Backup/DR | **Absent + regressed** | no-git rule removed the only versioning; .checkpoints partial | repo itself has NO version control and NO CI ever run; product has no persistence | **Critical** (process) | one bad session or disk event loses the project | user: zip per session (manual), push to activate CI; product: Row 11 |
| Large-scale behavior | Partially measured | W-TILE-10M: 90–124 MB @10M cells; adversarial 1.7 GB recorded | `OpLog::merge_from` is O(n²) (linear scan per op — visible in code); full-replay-per-open O(n log n) until snapshots wired; 1M-row single-sheet interactive path unmeasured (W-OPEN-1M pending) | High | sync of large histories will crawl; open times unknown | index op ids (BTreeSet) at Row 10 completion; run W-OPEN-1M at Row 11 |
| Determinism (locale/tz/parallel) | I+Doc+T | locale-free kernel; no parallel eval yet | parallel determinism untested (no parallel path) — honest | Low | — | when TD-17 triggers |
| Testing breadth | Strong for scope | 118+ tests, property sweeps, machine-transition tests, mutation targets designed | fuzzing NOT running (no parser to fuzz yet, but op-applier fuzz also not evidenced); malformed-workbook tests N/A until importer | Med | — | fuzz op applier now; importer fuzz from day one |
| Invariants (save→reload, translation, idempotency) | Partial | undo laws, convergence, hash equality tested | save→reload CANNOT be tested (no save); copy-translation CANNOT (no copy) | High | two of the mandate's core invariants are untestable today | they become the acceptance tests of Rows 11 and copy/paste |

## 8. Top 15 risks/gaps (ranked)

1. **No persistence** — memory-only workbooks; crash = total loss. (Critical; Row 11 is the fix; SQLite host-side crate missing.)
2. **No repo version control + CI never executed** — the no-git rule removed the safety net and left every gate local-only. (Critical process risk; user-side mitigation required: manual zips + a push.)
3. **Oracle corpus not started** — the longest-lead dependency of the compat identity; every week of delay is a week of building against assumptions the binary will contradict. (Critical-by-schedule.)
4. **No XLSX I/O** — no migration path, no fidelity number, no product story. (Critical for identity.)
5. **RGA vs Fugue divergence from ADR-002** — interleaving anomalies under nested concurrent inserts; must close before real multi-user sync. (High.)
6. Single-sheet model — late multi-sheet retrofit risks op-taxonomy churn (ops are forever). (High.)
7. Function conformance is spec-derived, not oracle-derived — the mandate's own warning applies: passing tests ≠ Excel-correct. (High.)
8. Excel bug-compat catalog unimplemented (dates/display rounding). (High.)
9. Copy/paste/fill absent — core editing and the reference-translation invariant untestable. (High.)
10. `merge_from` O(n²) + full-replay opens — scalability cliffs waiting for sync of long histories. (High.)
11. Importer-without-sandbox temptation once XLSX starts under pressure. (High, latent.)
12. Sync never-drop and kill-durability not yet evidenced end-to-end (W-SYNC-RELAY pending). (Med-High.)
13. Retain-losers invisible — implemented value with no surfacing path yet. (Med.)
14. Whole-row/column references unsupported. (Med.)
15. Solo bus-factor with no pushed remote — the project exists on one disk. (Med process, compounding #2.)

## 9. Critical — must not be deferred
(1) Persistence/container with crash-injection (Row 11, finish properly); (2) user pushes the repo once — CI, supply-chain gate, and offsite copy all activate with one command; (3) oracle capture harness started in parallel (it needs only Windows+Excel+schedule, not kernel time); (4) Fugue upgrade before multi-user sync is called done.

## 10. High-priority (next 2–3 sessions)
XLSX reader (sandboxed from line one) → copy/paste/fill Commands with translation tests → multi-sheet op design (even if implementation staggers) → merge_from indexing → W-SYNC-RELAY + W-OPEN-1M runs → op-applier fuzz target actually running.

## 11. MVP-acceptable limitations (fine to ship v0.1 with)
60-function catalog; no formatting/styles; no charts/pivots/tables/print; single workbook per process; bearer-token auth locally; no UI (CLI+MCP is the v0.1 shape); iterative calc absent; parallel calc absent; adversarial-pattern memory (1.7 GB @50% contested) recorded rather than solved.

## 12. Missing/insufficient MD documents
**Function semantics spec** — per-function signature/coercion/error tables (the oracle will populate it, but the skeleton should exist so implementation and capture converge on one artifact). **XLSX mapping spec** — docs/24 is strategy; the part-by-part OOXML↔model mapping tables (sheetData, sharedStrings, styles.xml minimal set) are unwritten and Row-12 needs them. **Multi-sheet op design note** — SheetId exists in docs/04 but no op-taxonomy extension is specified. **Runbook for the no-git workflow** — .checkpoints discipline is one paragraph in CLAUDE.md; a session under pressure needs the restore procedure written down.

## 13. Contradictions between MDs
(a) **BOOTSTRAP.md row 8 still names proptest** while CLAUDE.md/D-052 rejects it — a resumed session reading BOOTSTRAP first could reintroduce it. (b) **CLAUDE.md quality-gates line still says "commit locally"-era phrasing in older copies** — verify the v4 on disk is the only copy (the docs/ folder briefly contained duplicates). (c) **ADR-002 (Fugue) vs implemented RGA** — documented in code comments but the ADR itself carries no "temporarily implemented as" annotation; the register should. (d) **QUARTER-PLAN.md** still describes the original W1–13 with commit/push language and proptest — mark it historical or annotate. None are dangerous individually; all are the kind of drift that misleads a fresh autonomous session, which is this project's specific failure mode.

## 14. Areas likely to require architectural refactoring
Multi-sheet introduction (op taxonomy + State shape — do the design now, refactor once); Fugue tree in usk-state (contained, planned); OpLog storage backing (Vec → indexed/paged store when the container lands — merge_from and replay both change); key-width performance (TD-23's 48-byte BTreeMap keys — may force an interned-id layer under the graph); Session/undo persistence into the container schema (designed in docs/26, not yet shaped in code).

## 15. Recommended next checkpoints
**C1 (immediate, user):** push once; zip the folder; both are 5 minutes and remove risks #2/#15. **C2 (next session exit):** Row 10 closed with W-SYNC-RELAY numbers incl. mid-run kill; merge_from indexed. **C3:** Row 11 closed = docs/26 schema on disk, crash-injection green, W-OPEN-1M measured, save→reload invariant test exists and passes. **C4 (parallel, user-assisted):** oracle harness produces its first 500 captured vectors for the 60 shipped functions — the first real Excel-compatibility number replaces this review's 8%. **C5:** XLSX reader behind sandbox reads the 20-file starter corpus. Re-run this review at C5; the five scores should read approximately 25/25/55/20/90 if the plan holds — and if they don't, the plan was wrong, which is equally worth knowing.

---
*Review complete. No code or documents were modified. Implementation may continue uninterrupted; nothing here blocks current work except the four items in §9, of which two belong to the user, not the build sessions.*
