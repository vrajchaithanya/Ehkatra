# 07 — Solo Operating Model & Full Review
Status: Approved · Normative: yes · **Supersedes every team-shaped assumption in docs/ — where another doc names a role, committee, or rotation, this doc's automation replaces it**

## 1. The constraint, taken seriously
One engineer, enterprise-grade output, years of maintenance. The design rule that follows: **every recurring human function must become either (a) a CI gate, (b) a generated artifact, or (c) an explicit non-goal.** A process that requires a second person is a defect. The architecture already had the right bones for this — determinism makes bugs reproducible without a support team, ops-as-truth makes audit free, docs-as-code makes governance greppable. What follows redesigns the parts that still assumed teams.

## 2. Team-shaped assumptions found → solo redesign

| Was (doc) | Team assumption | Solo redesign |
|---|---|---|
| ARB monthly review (41/43) | committee | **Automated consistency checks** (doc-link checker, glossary-term lint, budget single-source check) + a quarterly solo review checklist; ADRs remain, written at decision time |
| On-call rotation, game days (36) | SRE org | Self-hosted-first ship order; SaaS deferred until revenue funds it. Compact server = one binary + SQLite + Litestream continuous backup; **users run it, you don't**. Status = automated health endpoint + uptime robot |
| Oracle capture via licensed Excel fleet (32) | compat team + lab | One Windows VM + scheduled COM harness runs, results committed as data; corpus grows incrementally; conformance number computed in CI |
| Specialist hires before Q2 (41 R-08) | hiring | Risk re-scoped: TLA+ models + reference vectors + this repo's docs ARE the bus-factor mitigation; complexity budget rule below caps what one person maintains |
| Pen test per release, external audit (30) | security org | Automated: cargo-audit/deny in CI, OSV scanning, scheduled ZAP baseline scan against the API, `cargo fuzz` in nightly CI, OWASP ASVS L2 as a checklist file with per-item evidence links; paid external pen test once, before first paying enterprise customer, not per release |
| Design partners program (40) | GTM team | Public beta + GitHub issues + replay bundles (bug reports arrive as deterministic reproductions — support cost ≈ 0 by architecture) |
| Release manager, rings (34) | release org | `cargo-dist`-style tag-driven releases: tag → CI builds/signs/publishes artifacts + SBOM + changelog from conventional commits; rollback = re-tag previous |
| VPAT/a11y manual runs (16/48) | QA team | axe-core automated a11y tests in E2E; manual screen-reader pass quarterly by you with a scripted 20-minute protocol; VPAT generated from test evidence |
| Mutation testing floors, 10⁷ nightly interleavings (35) | QA infra | Kept — these are compute, not people. Nightly GitHub Actions on schedule; failures open issues automatically |

## 3. The complexity budget (the most important solo rule)
**DP-S1:** the platform may contain at most **one** of each hard thing: one language (Rust; TS only in the thin web shell), one storage engine (SQLite everywhere — container, server, cache), one wire format (the canonical op encoding + JSON at the REST edge), one deploy artifact (single binary), one UI framework (chosen once at Q2, never churned). Every "second something" needs an ADR proving the first can't stretch.
**DP-S2:** dependency ceiling: kernel ≤ 5 external crates (currently 1: blake3), workspace ≤ 40. Every addition is a future maintenance debt you personally pay.
**DP-S3:** no service you must babysit: nothing in the architecture may require a daemon that pages you. Server components must be restart-safe, single-binary, and customer-operable.

## 4. Gap register (brutal pass, solo lens) — each with impact and disposition

| Gap | Why it matters / impact | Disposition |
|---|---|---|
| No CI existed | every gate was manual = solo death by toil | **FIXED this session**: `.github/workflows/ci.yml` — fmt, clippy, tests, differential replay (native+wasm32), kernel-purity greps |
| wasm32 determinism unproven (A-005/DP-A2) | the web-first bet rested on it | **FIXED**: replay-check binary, 5000-op corpus, bit-identical hashes measured |
| No dependency/supply-chain scanning | solo devs get owned via deps | **Session 3: `deny.toml` + a `supply-chain` CI job (cargo-deny + cargo-audit) added — but not yet executed.** cargo-deny will not build on the windows-gnu dev host (no `dlltool.exe`); it first runs for real on CI. Licences hand-verified in the meantime (all 10 deps within the allow list). Status: *added, unproven* — not green until CI says so (DP-F5). |
| No release automation | manual releases don't happen reliably solo | ADD at v0.2: tag-driven release workflow + SBOM |
| No secrets story for server | leaked tokens end the company | Design decided: env-injected, never in files; documented in 30 at server milestone |
| No browser-matrix testing | web-first without it is faith | ADD at Q2 shell: Playwright against Chrome/Edge/Firefox/WebKit (free, automated); budget gates run headless-Chromium + WebKit |
| No error/crash reporting pipeline | you can't fix what you can't see | Design: opt-in, content-free (DP-E7), self-hosted Sentry-compatible endpoint; lands with shell |
| No migration story for the container schema | breaks year-2 you | Covered by DP-A5 + SQLite user_version migrations; add a migration test harness at Row 11 |
| No CLI design doc | the CLI is the solo dev's own DX | `ehkatra` binary grows subcommands (open/apply/query/export/serve); document at Row 14 (MCP shares the plumbing) |
| No prompt-engineering standards for AI features | AI behavior drift | Versioned prompt files in-repo + eval sets gate changes (35 §9) — already designed, now explicitly solo-runnable (evals are CI) |
| UX architecture doc missing | scorecard noted it; solo means UX debt compounds silently | Write at Q2 start: one doc, Excel-keymap parity + command palette + the 5 core screens; benchmark: Linear's velocity-first UX, Apple HIG spacing |
| No uninstall/data-export path | enterprise checklist + user trust | Container is SQLite + documented schema; `ehkatra export` covers it; document at GA |

Benchmarks applied where they earn their keep: Stripe (API ergonomics: idempotency, versioning — already in docs/20), Linear (solo-friendly velocity: small releases, changelog culture), SQLite itself (the patron saint of solo-maintained enterprise software: tests ≫ code, zero deps, decades of compatibility — Ehkatra's kernel explicitly imitates this posture), Twelve-Factor (config via env, stateless server), OWASP ASVS L2 (checklist-driven, automatable). Rejected as templates: ServiceNow/Salesforce-class platform sprawl — anti-goals for a solo product.

## 5. Scores (conservative; will not inflate to please the mandate)

| Dimension | /10 | Below 9.5 because → improvement made → still required |
|---|---|---|
| Architecture | 9 | Coherent, invariant-driven, now solo-adapted. Remaining: interface IDL generation not yet built. |
| Security | 7 | Structural design is strong (no-ambient-IO, sandboxing, quarantine). Made: CI purity gates. Required: dep-scanning in CI (next session), ASVS checklist file, sandbox actually implemented at Row 12 (import), real authn at Row 10+. Paper security is not security. |
| Scalability | 7.5 | Kernel design scales by construction (tiles, groups); server plane is deliberately deferred. Required: measured A-001/A-003 at Rows 4/7. |
| Maintainability | 9 | no_std purity, 1-dep kernel, complexity budget, docs-as-code, deterministic repro. Required: keep DP-S2 as the codebase grows. |
| Performance | 6.5 | Only micro-evidence exists (1.6 ms demo, replay corpus). Made: measurement discipline. Required: Rows 4–7 benches against docs/31 budgets. |
| Developer experience | 8.5 | One-command build/test/demo; CLAUDE.md makes any Claude session productive; CI mirrors local gates. Required: CLI subcommands, dev-harness docs. |
| UX | 4 | No UI exists; keymap/interaction architecture designed but unbuilt. This is honest: UX cannot score on paper. Q2's job. |
| AI readiness | 8.5 | MCP-first design, preview/undo/audit contracts, semantic layer designed; guardrails specified as host-enforced. Required: implement Rows 14 + evals. |
| Enterprise readiness | 6 | Audit/RBAC/SSO designed, none implemented; self-host-first path fits solo GTM. Required: Rows 10+ auth, audit chain, ASVS evidence. |
| **Production readiness** | **5.5** | Milestone 1+2 of 15 done with real gates. The number rises row by row; anything higher today would be marketing. |

**Cycle status:** improvements implementable *now* are implemented (CI, wasm gate, solo redesigns, this doc). Remaining sub-9.5 gaps are bound to specific BOOTSTRAP rows with their evidence criteria — the review-improve-implement loop continues by building rows, which is the only honest way scores rise. Next iteration: Row 4 (tile store + A-001/A-002 measurement) and the dep-scanning CI job.

## 6. Host-machine isolation (DP-S5) — Ehkatra is a guest, never a tenant
The dev machine runs other production software (e.g., Postgres, an ITIL tool). Ehkatra must be fully isolated from everything it did not create:

- **No shared services.** Ehkatra never uses, connects to, configures, or migrates the host's Postgres (or any existing database/service). All Ehkatra storage is **SQLite files created by Ehkatra, inside the Ehkatra folder or its own per-user app-data directory** — nothing else. A future connector *reading* a user's Postgres is an explicit, credentialed, opt-in feature — never a dependency.
- **No default-port squatting.** Any future local server/MCP endpoint binds **loopback only**, on a configurable, uncommon default port (7423; MCP 7424), and **fails fast with a clear message if the port is taken** — it never auto-claims alternatives or touches 5432/8080/3000-class ports others likely use.
- **No global mutation.** No Windows services, no registry beyond normal per-user installer conventions (and none at all during development), no PATH edits, no admin elevation, no system Python/Node/lib changes. The Rust toolchain lives in the user profile (`%USERPROFILE%\.cargo`); builds stay in `target\` inside the repo folder.
- **No background residents.** Nothing Ehkatra installs keeps running when Ehkatra isn't (DP-S3 applied to the dev box too).
- **Uninstall = delete the folder** (plus per-user app-data dir). That property is tested, not assumed.

CI enforcement where possible: a grep gate rejects `postgres://`, `5432`, and service-install APIs from the codebase; the port defaults live in one config module.

## 7. Sustainability rule for the years ahead
**DP-S4:** every quarter, one week is maintenance-only: dependency updates, corpus growth, debt register, backup-restore drill of your own infra. Scheduled, non-negotiable, automated where possible. Solo products die of deferred maintenance, not of missing features.
