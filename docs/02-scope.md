# 02 — Scope & Non-Goals
Status: Approved · Owner: Product Architect · Normative: yes

## In scope — Horizon 1 (GA, ~4 quarters)
Windows + macOS desktop application; headless server (Linux) for sync relay, API, MCP, calculation authority; grid kernel per docs/10–16; formula catalog Core-200; tables, named ranges, dynamic arrays, validation, conditional formatting, sort/filter, comments, freeze/split; XLSX/CSV import-export with published fidelity; three-layer API (docs/20); MCP surface (docs/21); AI capabilities per docs/22 behind the guardrail contract; real-time collaboration + offline (docs/15); Standard encryption tier; SSO (OIDC), RBAC, audit chain.

## In scope — Horizon 2 (post-GA, scheduled by evidence)
Pivot tables (design frozen: incremental materialized aggregation), charts core set, plugin SDK GA, connectors (SQL/REST/files), gRPC + Arrow bulk plane, ODS/Parquet, i18n Tier 2, Linux desktop if demand warrants, web viewer (read + light edit) reusing the wgpu renderer via WebGPU.

## In scope — Horizon 3 (approved designs, unscheduled)
Managed-E2EE tier (MLS), distributed fleet calculation, mobile, streaming connectors, Solver/scenarios, marketplace.

## Explicit non-goals
VBA execution (import quarantines macros; automation is the WASM plugin path); pixel-identical Excel rendering (semantic + layout fidelity, not raster fidelity); a web-first product before the desktop one earns its users; embedded/riscv64 as a supported target (the `no_std` discipline stays; the target does not — ADR-030); building our own solver, our own SQL dialect, or our own crypto.

## Scope-change rule
Anything moving between horizons requires: an ADR, a roadmap delta, and a scorecard re-run. Nothing enters Horizon 1 without displacing something of equal size — the quarter capacity is fixed.
