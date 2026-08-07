# 30 — Security Architecture
Status: Approved · Owner: Security Architect · Normative: yes · Carved from SPEC §21–22 + kernel security design

## Trust boundaries & threat model (STRIDE per boundary; full model is a living annex)
Untrusted file → parser sandbox (docs/24) · clipboard → same sandbox · collaborator → op applier (schema+bounds validation on receive; adversarial-op corpus) · formula → engine (**no ambient I/O by construction** — the `no_std` kernel cannot open a socket; external fetch = Calculation Authority only, egress-allowlisted; kills WEBSERVICE-class exfiltration structurally) · plugin → host (WASM capabilities, quotas) · agent → documents (scoped short-lived intent-declared tokens; untrusted-data labeling; blast-radius policy; taint records) · tenant → tenant (server: per-tenant keys, partitioned storage, quota lanes) · operator → customer data (audit-chained access, break-glass quorum — H2 with Managed-E2EE) · supply chain → build (docs/34).

## Desktop platform hardening (new, desktop-first)
Windows: signed MSIX, ASLR/CFG/CET enabled, parser child in restricted-token job object, secrets in DPAPI+TPM-backed store. macOS: hardened runtime + notarization, sandbox-exec'd parser child, Keychain/Secure Enclave for secrets, TCC-respecting file access. Both: no auto-elevation, updater verifies signatures against pinned keys (docs/34), local API/MCP endpoints are loopback-only, off by default, consent-gated, token-authenticated.

## Data protection tiers
H1: **Standard** — TLS 1.3 in transit; at rest: server envelope encryption (KMS/HSM), desktop container encrypted via OS full-disk + optional per-file passphrase (Argon2id → XChaCha20-Poly1305). H2 approved-unscheduled: Managed-E2EE (MLS groups, HSM-quorum Compliance Principal, published capability matrix). The op framing is already encryption-agnostic — the one irreversible prerequisite, paid now.

## Enterprise controls (H1)
OIDC SSO; RBAC roles + ABAC (CEL) policies enforced at relay admission, API gateway, and query planner (row/range-level reads — a query/pivot result can never contain data its reader couldn't read directly; adversarially tested); range classification flows into exports (redaction unless policy clears — data doesn't launder through XLSX); append-only hash-chained audit log, externally anchored (RFC 3161), SIEM-streamable (OCSF); protection (docs/11) is UX, ACLs are security — the docs say so explicitly to prevent the classic confusion.

## Assurance
cargo-deny/vet + vendored deps + SLSA-3 provenance + Sigstore + CycloneDX SBOM (docs/34); continuous fuzz (parsers, op applier, formula parser, query planner); pen test per major release; VDP + bounty at public beta; single audited crypto implementation (aws-lc-rs; FIPS build flag), no custom crypto ever.
