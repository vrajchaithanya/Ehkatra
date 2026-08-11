# 48 — Production Readiness Checklist (GA gate)
Status: Living · Every line is verified evidence, not assertion; owner signs with link

## Correctness
☐ Differential replay green 90 consecutive days across 3 architectures · ☐ TLA+ models checked at release configuration · ☐ Zero open fuzz crashes · ☐ Mutation floor met on op applier/reducer/evaluator/codecs · ☐ Oracle conformance ≥ 99.5% published · ☐ XLSX fidelity ≥ bar, published, with top-regression list

## Reliability & ops
☐ Crash-injection suite (incl. power-fail, sync-folder races) green · ☐ Restore drills green 30 consecutive days · ☐ Poison-op quarantine exercised in staging · ☐ Relay HA failover drill < 60 s · ☐ On-call staffed, runbooks game-dayed · ☐ Rollback executed successfully in staging for the GA build itself

## Security & privacy
☐ Pen test complete, criticals closed, report archived · ☐ SLSA-3 provenance verified on GA artifacts · ☐ SBOM published · ☐ Signing keys in HSM with dual control · ☐ Telemetry content-taint lint enforced · ☐ VDP live

## Desktop quality
☐ Budgets green on all reference hw (linked runs) · **◑ IME validated by native JP/CN/KR typists** — *mechanical half closed (D-127): the JP/CN/KR event shapes are replayed end to end with every step asserted, 29/29, W-IME-SCRIPTS session 34, frames in `demo/ime-{jp,cn,kr}.png`. Stays **open**: a native typist is the oracle and has not run it. The residue is a 7-item checklist in MEASUREMENTS.md §W-IME-SCRIPTS, and the mechanical half already found TD-83 (Chinese drawn by a Japanese face) and TD-84 (the focused clause is invisible)* · ☐ Narrator + NVDA + JAWS + VoiceOver scripted runs green; VPAT published · ☐ Per-monitor DPI matrix green · ☐ Signed/notarized installers through all rings · ☐ Update rollback verified · ☐ MSIX/ADMX + MDM enterprise deploy validated with a real design partner

## Agents & AI
☐ Blast-radius policies enforced at host (attempted bypass test) · ☐ Injection red-team suite green · ☐ Capability evals at floor · ☐ Auto-milestone-before-agent-batch verified · ☐ "Undo the agent session" verified end-to-end

## Governance
☐ All assumptions Confirmed/Revised (none Open past date) · ☐ Debt register reviewed; unpriced debt zero · ☐ Traceability matrix complete (no unverified NFR) · ☐ Scorecard re-run; no dimension < 6 without an accepted waiver
