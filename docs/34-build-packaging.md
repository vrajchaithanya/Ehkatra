# 34 — Build, Packaging & Release
Status: Approved · Owner: Build Engineer · Normative: yes

## Build architecture
Cargo workspace monorepo (kernel crates + shells + server + tools); trunk-based, merge queue, every merge green on the full gate set: `no_std` kernel build · differential replay (x86_64/aarch64/wasm32) · property suites · fuzz smoke · budget benchmarks on dedicated runners (bare-metal, pinned governors — benchmarks on shared CI are noise) · `#[cfg(target_os)]` placement lint · cargo-deny/vet. Reproducible builds (pinned toolchain, vendored deps, `--remap-path-prefix`); build provenance SLSA-3; artifacts signed via Sigstore + platform signing.

## Packaging (the desktop-first work that was missing pre-pivot)
**Windows:** MSIX (Store + enterprise sideload) and signed EXE/MSI (direct + winget); Authenticode via HSM-held EV cert; per-machine and per-user installs; silent-install + admin-template (ADMX) settings for enterprise. **macOS:** universal2 (arm64+x86_64) .app, hardened runtime, notarized, DMG + Homebrew cask + MAS build (sandboxed variant with entitlement-gated features documented); MDM-friendly (plist-managed settings). **Server:** OCI images + Helm chart + single-binary compact mode (embedded SQLite + local blobs — same code as SaaS, on-prem never forks).

## Update strategy
Desktop auto-update: staged rings (internal → beta 5% → GA), delta patches, signature-pinned manifest, rollback-capable (previous version retained; container files forward-compatible so rollback is safe by docs/10 rules); enterprise LTS channel (18-month, quarterly security backports) with deferral policy honoring managed settings. Server: rolling with N−2 wire negotiation; compaction-rewriter migrations; tested rollback path per release.

## Release discipline
Release train: 4-week cadence post-GA; a release = artifacts + SBOM (CycloneDX) + fidelity number + conformance number + benchmark deltas + changelog. Blockers: any budget gate, any open fuzz crash, any a11y conformance regression, restore-drill failure. Hotfix lane: security-only, 48 h SLA, all rings simultaneously.
