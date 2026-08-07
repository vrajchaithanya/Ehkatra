# 01 — Vision
Status: Approved · Owner: Product Architect · Normative: no

## The product
A desktop-first, enterprise-grade spreadsheet platform whose kernel treats every change as an operation, every consumer — screen, API, AI agent, audit trail — as a fold over the same log, and every AI action as previewable, attributable, and reversible.

## The two wedges (why anyone switches)
1. **Agent-native.** The first spreadsheet where AI tooling is a peer of the UI: same command vocabulary, semantic MCP surface, preview-before-apply, one-click undo of everything an agent did. Sold to teams currently gluing Python to Excel.
2. **Trustworthy at enterprise depth.** Deterministic calculation with published conformance numbers, op-level audit with actor attribution, conflict surfacing instead of silent last-writer-wins, on-prem-capable server plane. Sold where Excel co-authoring and Sheets' cloud-only model both fail.

## What we refuse to be
A web app in a window (desktop is the primary craft surface, not a port); an Excel clone (compat is a profile, not an identity); an AI demo (every AI capability rides production guardrails or doesn't ship).

## Ten-year test
Every irreversible decision (identity model, op algebra, value lattice, file compatibility posture) must still be defensible in 2036. Reversible decisions are made quickly and recorded; irreversible ones get ADRs, proofs, or spikes first.

## Success metrics (18-month)
A published Excel-conformance number ≥ 99.5% on the function corpus; p95 keystroke-to-paint < 16 ms on reference hardware; 10 design-partner organizations with agents in daily production use; zero data-loss incidents (defined: acked op lost, or silent divergence detected by state hash).
