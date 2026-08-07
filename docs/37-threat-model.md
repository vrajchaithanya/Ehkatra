# 37 — Threat Model
Status: Approved (fulfills docs/30's promised annex) · Normative: yes · Review: quarterly maintenance week + on any new trust boundary

Method: STRIDE per trust boundary. Each threat names its mitigation and where it's verified. Solo constraint applies: every mitigation is structural or automated — none require a security team watching dashboards.

## Boundary 1 — Untrusted file → parser (XLSX/CSV/clipboard)
| Threat | Vector | Mitigation | Verified by |
|---|---|---|---|
| Tampering/EoP | crafted zip/XML exploiting parser | sandboxed subprocess, IR-only output, schema revalidation (DP-E2) | sandbox-escape suite, fuzz |
| DoS | zip bomb, entity expansion, deep nesting | ratio/size/depth caps; DTD/external entities off | bomb corpus tests |
| Malware carriage | vbaProject/OLE/DDE in imports | quarantine class — never executed, never re-emitted by default (DP-E3) | ingest-policy tests |
| Formula injection | `=cmd\|...` in CSV cells | leading `=+-@` neutralization on import AND export | OWASP CSV cases |

## Boundary 2 — Collaborator → op applier
| Spoofing | forged actor ids | authenticated channel binds ActorId to principal (Row 10); relay rejects mismatches | protocol tests |
| Tampering | hostile ops crashing appliers | schema+bounds validation on receive (DP-E4); poison-op quarantine | adversarial-op corpus |
| DoS | tombstone/op amplification | per-actor rate+byte buckets at relay | quota tests |
| Repudiation | "I didn't make that edit" | op-level attribution + hash-chained audit | audit chain verification |

## Boundary 3 — Formula → world
Exfiltration via WEBSERVICE-class functions or remote refs: **structurally dead** — the `no_std` kernel cannot perform I/O; Tier-3 exists only at the Calculation Authority under an egress allowlist (docs/13). Verified by: kernel dependency graph CI (I1) + egress tests. Hostile-formula DoS (deep recursion, spill lattices): evaluator resource governors with attributable kill reports. |

## Boundary 4 — Agent → documents (the novel surface)
| Confused deputy | prompt injection via cell content directing an agent to exfiltrate/destroy | untrusted-data labeling on all cell-derived text (DP-E6); scoped short-lived intent-declared tokens; blast-radius policy host-enforced; evidence taint records | red-team eval suite (docs/35 §9) |
| Unattributable AI damage | "what did the agent do?" | labeled groups, auto-milestone before batches, session-scoped undo | 48 AI gates |
| Preview bypass | agent applies without preview | `preview_hash` precondition above threshold, enforced at host not tool | contract tests |

## Boundary 5 — Network → local machine (DP-S5 posture)
Local server/MCP: loopback-only, token-gated, off by default, fail-fast on occupied ports. No listening surface exists unless the user turns it on. Update channel: signature-pinned manifests, HSM keys, rollback capability (the highest-blast supply-chain surface, R-12). |

## Boundary 6 — Supply chain → build
Compromised dependency or toolchain: pinned toolchain, vendored+vetted deps (cargo-deny/vet/audit in CI — must run on GH runners), dependency ceiling DP-S2 (kernel ≤5) shrinks the surface itself, reproducible builds + SLSA-3 + signed artifacts at release. |

## Boundary 7 — Operator/hoster → customer data (server plane, H2)
Standard tier: honest position — the operator can read; mitigations are access audit + encryption at rest + tenant key separation. The E2EE tiers close this boundary and remain approved-unscheduled (D-021). Never claim otherwise in product copy. |

## Explicit non-threats (scoped out, on record)
Physical access to an unlocked machine; a compromised OS/browser (we inherit the platform's integrity); a malicious *user* exporting data they legitimately can read (that's DLP-tier territory, H2+); side-channels on shared hosts (single-tenant-per-file model makes these low-value).

Residual-risk register: R-11 (agent injection — mitigations reduce, don't eliminate; the eval suite is the honest meter), R-12 (update pipeline), operator-read at Standard tier (documented, not hidden).
