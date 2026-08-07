# 23 — Plugin SDK
Status: Draft (H2 GA; contract frozen H1) · Owner: Principal Engineer · Normative: interface yes, tooling draft

## Model
One mechanism, five extension points, all WASM (Component Model) with capability manifests; first-party charts and connectors ride the same rails (ADR-019 — SDK credibility through dogfooding): **functions** (deterministic-mode default; declared-volatile → Calculation Authority), **connectors** (server-side; egress per manifest), **renderers** (chart types/cell renderers producing scene-graph vectors — the scene contract is the sandbox; no raw pixels/DOM), **panels & commands** (declarative UI schema rendered host-native; webview escape hatch behind org allowlist), **automation** (on-edit/on-open/on-schedule/on-webhook scripts — the macro replacement: capability-scoped, previewable, undoable labeled groups; the VBA malware model is not reproduced).

## Why the contract freezes in H1 even though GA is H2
The extension points constrain kernel interfaces (scene-graph stability, function registration ABI, Command visibility). Freezing the *contract* now costs little; retrofitting extension points later costs a rewrite. H1 ships the contract + in-repo first-party plugins; the public SDK, marketplace, signing, and review pipeline are H2.

## SDK surface
TypeScript-first (componentize-js) + Rust; local dev harness = headless kernel + hot reload; typed `usk-sdk` mirroring Command/query (plugins cannot bypass it — kernel visibility rule applies). Runtime quotas: CPU/memory/egress per invocation. Versioning: plugins pin SDK major; two-major support with deprecation telemetry. Desktop distribution: org allowlist + signed packages; sideloading is a developer-mode setting with explicit user consent.
