# 21 — MCP Architecture
Status: Approved · Owner: AI Platform Architect · Normative: yes · Carved from SPEC §25

## Design law
Schemas and answers, never grids; preview before mutation; everything attributable and reversible. Agents default to this layer (their tokens are scoped to it unless L1/L3 scopes are explicitly granted).

## Tool catalog (versioned JSON-Schema I/O)
**Orient:** `describe_workbook` · `describe_sheet` (per-column type/null/cardinality stats + 5 sample rows — bounded at any scale) · `summarize` · `search`. **Analyze:** `query` (read-only SQL; relations = tables/sheets; ACL-planned; capped with explicit truncation) · `read_range` (capped escape hatch) · `explain_formula` (CST-anchored substrate feed) · `trace_error` (origin + path + probes) · `find_dependencies` / `find_dependents` · `impact_analysis` (hypothetical edits → affected ranges/errors/objects, no mutation) · `validate`. **Mutate:** `preview_edits` (scratch branch → impact report + preview_hash) · `apply_edits` (atomic labeled group; `expected_version`; auto-milestone; preview_hash required above blast-radius threshold) · `generate_formula` (candidates + sample-row test evals; never writes) · `suggest_cleanup` (profiled issues, each with previewable fix batch) · `undo`/`redo` (agent-session-scoped). **Lifecycle:** `import`/`export` (async jobs) · `stream_changes` (op-tail as resource updates).

## Deployment shapes (desktop-first addition)
1. **Server MCP** — the shared, governed endpoint (org policies, SIEM-audited).
2. **Desktop-local MCP** — the running app hosts loopback MCP (consent + token, off by default): Claude/other agents drive the user's open workbook *with the user watching*, previews rendering live in the UI before approval. This is a differentiator no competitor has: agent proposals as first-class UI objects (a preview renders as a pending-change overlay the user can inspect cell-by-cell and accept/reject in-app).

## Guardrails (relay/host-enforced, not tool etiquette)
Cell-derived text labeled untrusted in every response (injection posture); short-lived intent-declared workbook/range-scoped tokens; org-configurable blast-radius policy (>N cells → preview required; >M → human approval resource); every agent group auto-milestoned + one-click reversible; evidence taint record (what the agent read) stored per action (docs/22).
