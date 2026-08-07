# 22 — AI Architecture
Status: Approved · Owner: AI Platform Architect · Normative: yes · Carved from SPEC §26

## Principle
AI-native means the substrate is AI-legible and the AI is a *citizen*, not a privileged subsystem: the AI plane holds ordinary agent tokens and uses the public MCP surface with standard guardrails (ADR-023). No second security model exists.

## Semantic layer (kernel-resident CRDT objects)
Column semantic types (inferred locally by cheap classifiers over column stats; user-confirmed; confidence explicit), table/range descriptions, workbook intent docs, formula-group intents, unit annotations. Grounds NL→SQL/formula; served by `describe_*`; versioned/previewable like all state.

## Capabilities (H1 set; each = MCP composition + UX surface)
Formula generation (candidates with test evaluations against sample rows → user picks → `apply_edits`); formula & dependency explanation (CST-anchored feed → leveled prose); error tracing (guided fix flow, previewed repairs); NL query (NL→SQL; the SQL is shown — trust through transparency); data cleaning (profile → issue list → previewed transform batches, never silent); workbook summarization/auto-documentation (living README object, diffable). H2: chart recommendation, scenario analysis over branches, impact narration.

## Model strategy
Model-pluggable per tenant/org (BYO endpoint; desktop can use local models for the semantic classifiers — no cell content leaves the machine without policy consent). Prompt templates and tool selection are versioned artifacts; capability evals (docs/35) gate model/prompt upgrades exactly like code.

## Explainability & audit contract (every AI action stores)
Intent (the ask) · evidence (taint record of ranges/queries the model saw) · proposal (previewed diff) · decision (who approved: human click / policy auto / chain) · explanation ref. Reachable from any affected cell via "why did this change." The taint record is also the exfiltration audit mechanism.

## Failure honesty
AI features degrade to absent, never to unguarded: if the preview pipeline is down, `apply_edits` above the blast threshold refuses rather than skipping preview; if the model endpoint is down, deterministic features (Flash-Fill-family synthesis, docs/11) still work — they never depended on it.
