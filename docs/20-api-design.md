# 20 — API Design: Three First-Class Layers
Status: Approved · Owner: API Designer · Normative: yes · Carved from SPEC §24

## Layer model and invariants
**L1 cell-level** (developers/plugins: read/write cell/range, structure ops, formulas, styles, validation, comments, clipboard copy/cut/paste, clear, undo/redo) — each call desugars 1:1 to a Command. **L3 bulk** (data platforms: read/write_table, stream_rows/columns/changes, batch_update, import/export_dataset, execute_sql; Arrow Flight + Substrait dataframe plans in H2) — reads go direct-to-tile (never bypassing ACLs), writes compile to the same op batches L1 would produce. **L2 semantic MCP** (agents; docs/21) — implemented entirely on L1+L3, mutations as previewed labeled groups.

Binding invariants: (a) L1 ↔ Command is 1:1 auditable; (b) every L2 mutation decomposes to L1 Commands in one group; (c) L3 changes encoding/transport, never semantics; (d) ACL, audit, undo-grouping, version preconditions identical at all layers — no privileged layer. OpenAPI/protobuf generate from the Command vocabulary (docs cannot drift from engine).

## Transports (H1 scope)
REST/JSON (+NDJSON streaming) and WebSocket (op subscription, presence, live values, low-latency submission). gRPC + Arrow Flight are H2 (desktop-first trims the surface; the protos already exist from codegen). **Desktop-local API:** the app hosts a loopback HTTP+MCP endpoint (user-consented, token-gated, off by default) so local tools and agents automate the running app — the desktop equivalent of the server API, same vocabulary, same guardrails.

## Contract mechanics
`If-Match: <watermark>` optimistic concurrency (409 returns intervening ops for rebase); `Idempotency-Key` on all writes; `batch[]` envelope = one atomic labeled undo group; cursor pagination tile-aligned; errors are structured (code, target ref, remediation, trace id). Scopes: `workbook:read|write`, `range:write:{ref}`, `structure:write`, `formula:write`, `export`, `history:read` — range-scoped tokens are the agent least-privilege mechanism. Cross-layer routing: oversized L1 reads get `hint: bulk`; SDKs auto-promote; rate limits are layer-aware (calls vs bytes vs blast-radius).

## Versioning
`/v1` frozen at GA; additive evolution; deprecation = 2 LTS cycles + telemetry <0.1% + migration note. Breaking change requires `/v2` + ADR.
