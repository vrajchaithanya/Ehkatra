# 08 — Engineering Handbook
Status: Approved · Owner: you (solo) · Normative: yes for process; rules live in docs/06 (this doc never restates a rule, it operationalizes them)

## Daily workflow
Trunk-based on `main`; short-lived branches only for risky experiments. Conventional commits (`feat:`, `fix:`, `perf:`, `test:`, `docs:`, `refactor:`) — the changelog is generated from them. Every work session ends at a green state with PROGRESS.md updated (CLAUDE.md handoff protocol — it binds human sessions too). Push at least daily; CI is the reviewer that never sleeps.

## Definition of done (any unit of work)
Behavior proven by a named test · gates green (fmt, clippy, tests, replay-check if kernel-touching) · new numbers in MEASUREMENTS.md with evidence · owning doc updated or consciously not (say why in the commit) · decisions recorded in docs/43 · debt priced in docs/44 if any was taken.

## Self-review checklist (replaces the review team — run before merging anything non-trivial)
Wearing each hat for one minute beats wearing none for zero:
- **Correctness:** what input breaks this? Did I test the boundary and the concurrent case, not just the happy path?
- **Invariants:** does this touch mutation paths (DP-A1), determinism (DP-A2 — any time/rand/hash-iteration?), encoding (DP-A4)? Run replay-check if in doubt.
- **Security:** does any input cross a trust boundary here (DP-E2/E4)? Any new dependency (DP-S2 ceiling)?
- **Performance:** does this sit on a budgeted path (docs/31)? Bench before and after if yes.
- **Future-you:** will this make sense in 18 months from the doc-comment alone? Does the name say what it is?
- **Blast radius:** if this is wrong in production, what corrupts? If the answer is "user data forever," double the tests (mutation-testing modules, DP-B5).

## Release procedure (tag-driven, docs/34)
`git tag vX.Y.Z && git push --tags` → CI builds, tests, signs, publishes artifacts + SBOM + changelog. Rollback = re-point users at the previous tag; container files are forward-preserved so downgrade is safe (DP-A5). Never release with a red gate, an open fuzz crash, or an unpriced debt taken that week.

## Quarterly maintenance week (DP-S4 — scheduled, non-negotiable)
Dependency updates (one at a time, gates between) · fidelity/conformance corpus growth · debt register triage · restore-drill of your own backups · re-run the scorecard (docs/47) honestly · read PROGRESS.md end to end and prune.

## Templates
ADRs: `docs/templates/adr-template.md` — written at decision time, with the losing option (DP-B7). RFCs: `docs/templates/rfc-template.md` — for changes touching ≥2 module docs or any frozen irreversible; solo means the RFC is a thinking tool, not a meeting.

## Onboarding (for future-you, a collaborator, or a fresh agent session)
Read order: CLAUDE.md → PROGRESS.md → docs/00 → 03 → 04 → 06 → the module doc you'll touch. Then: `cargo test --workspace && cargo run --bin ehkatra` — if the demo converges, your environment works. First change: pick a `test:`-only commit to learn the gates before touching product code.
