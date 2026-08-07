# 25 — UX Architecture
Status: Approved (closes the scorecard's "UX doc missing" gap) · Owner: you · Normative: yes for Q2 shell work

## The UX thesis
Ehkatra's UX bet mirrors its engineering bet: **muscle-memory compatibility outside, honesty inside.** Excel users keep their hands (keymap, F2-edit, fill-drag, Ctrl+arrows); what changes is that the product never lies — no silent data mangling, no mystery recalc states, no lost concurrent edits, no un-attributable changes. Benchmarks: Linear (velocity, command palette, keyboard-first), Excel (grid interaction grammar — the compatibility target, not the ceiling), Apple HIG / Material only for platform conventions.

## The five core surfaces (v1 web shell — nothing else exists until these are excellent)
1. **The grid** — canvas-rendered, identity-anchored scroll, in-cell editor overlay (native IME), selection/fill/drag grammar per Excel, presence cursors, conflict chips (ADR-006 surfacing: subtle, dismissible, never modal).
2. **Formula bar + autocomplete** — token-aware, signature help, error carets from the CST, `explain` affordance on every error (origin-trace hover — the differentiator, docs/12).
3. **Command palette (Ctrl+K)** — every Command reachable by name; the palette IS the discoverability strategy (no ribbon in v1); doubles as the agent-transparency surface: agent actions appear in the same vocabulary a user would use.
4. **History & versions panel** — op-level blame per range, named versions, one-click restore, per-agent-session grouping with "undo everything this agent did."
5. **Agent preview overlay** — a pending `preview_edits` renders as an inspectable diff layer over the grid (changed cells highlighted, downstream impact count, new errors flagged) with accept/reject. This surface is the product's whole thesis made visible; it gets designed first, not last.

## Interaction rules
Keyboard: Excel-parity default keymap (remappable); every action reachable without a mouse (a11y + power users are the same requirement). Latency: keystroke-echo <16 ms is a UX rule, not just a perf budget — typing must never feel mediated. States: every async state is visible and honest (cells in recalc get a generation shimmer; syncing/offline is a persistent, calm indicator — never a blocking spinner). Errors: never a bare `#VALUE!` — always one hover away from "where it came from." Undo: always safe, always visible what it will undo (the structural-undo narrowing notice, docs/11).

## Visual system
One design-token file (spacing, type scale, color roles incl. semantic error/conflict/agent-pending colors, light+dark from day one, forced-colors honored). Density default = comfortable-compact (data tool, not marketing site). No custom iconography in v1 — Lucide set. Dark mode is a first-class theme transform (docs/18.6 rule: theme-derived colors remap, explicit colors preserved).

## UX debt rule
Any interaction shipped below this doc's bar gets a docs/44 entry like any other debt — UX debt compounds silently for a solo builder because no one complains until they churn.
