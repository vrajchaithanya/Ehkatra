# 41 — Risk Register
Status: Living · Review: monthly by ARB

| ID | Risk | P | I | Exposure | Mitigation | Trigger/Review |
|---|---|---|---|---|---|---|
| R-01 | CRDT promotion rate ≫1% under real collaboration → memory model collapses | M | H | **Critical** | Q1 harness w/ 3 collab patterns; fallback: smaller tiles / hybrid per-row metadata | MEASUREMENTS.md W13 |
| R-02 | XLSX fidelity plateaus below enterprise bar; pilots die on one bad board deck | M | H | **Critical** | corpus from W1 of Q2; published number; import advisories; target doc classes w/ low fidelity risk first | fidelity number trend |
| R-03 | Excel-grade desktop text/IME/a11y polish underestimated (the classic from-scratch-editor killer) | M | H | High | native IME overlay (never reimplement); accesskit; Q2 exit gates include IME + screen-reader evidence | Q2 exit |
| R-04 | Fugue-family implementation subtly wrong despite tests | L | H | High | TLA+ model + reference-vector testing + differential replay | Q1 W6 |
| R-05 | Two viable wedges, one team: agent-native + enterprise-trust focus splits | M | M | High | agent wedge leads GTM; enterprise controls ride GA anyway (they're needed for any sale) | design-partner signal Q3 |
| R-06 | Oracle capture legally/practically constrained (Excel licensing, COM instability) | M | M | Med | capture on licensed seats, store *derived vectors* not Excel bits; counsel review Q1; fallback: documented-behavior suite + user-reported divergence pipeline | Q1 W6 |
| R-07 | Performance budgets miss on reference hw (esp. keystroke path w/ a11y tree updates) | M | M | Med | budget-first dev; a11y tree virtualization; W13 + Q2 gates | each release |
| R-08 | Key-person concentration: CRDT + formula-engine expertise scarce | H | M | High | hire 2 specialists before Q2; ADRs + formal models reduce bus factor; pairing rotation | hiring board |
| R-09 | Scope re-expansion (suite ambitions, E2EE, distributed calc creep back into H1) | H | M | High | scope-change rule (docs/02); ARB monthly review; this register names the pattern | each ARB |
| R-10 | Sync-managed folders (OneDrive/iCloud) corrupt containers via naive syncing | M | M | Med | safe-copy + advisory locks + concurrent-open detection (docs/33); crash-injection incl. sync-race cases | Q2 |
| R-11 | Agent injection: hostile cell content induces harmful previewed-and-approved edits | M | M | Med | untrusted labeling, blast-radius policy, taint records; red-team evals in docs/35 §9 | pen test |
| R-12 | Update/signing pipeline compromise (highest-blast supply-chain surface) | L | H | Med | HSM keys, SLSA-3, signature-pinned manifests, ring rollout, rollback | annual audit |

Retired risks move to an archive section with outcome notes; a risk without a named trigger date is a defect of this register.
