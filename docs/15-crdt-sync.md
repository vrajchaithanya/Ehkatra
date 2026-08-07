# 15 — CRDT & Synchronization
Status: Approved · Owner: Distributed Systems Lead · Normative: yes · Carved from SPEC §4.2, §12

## Merge semantics (summary; algebra in docs/10)
Five CRDT layers with proven/tested merge rules. Register conflicts retain losers 30 days and surface asynchronously ("Bob's simultaneous edit — restore?"); order CRDT is interleaving-free (Fugue-family); structural ops are the property-test focus because that is where spreadsheet semantics are subtle (concurrent row-insert vs. range formula is the canonical case and a permanent regression test).

## Protocol
Replica ↔ relay over WS (desktop: native WS; QUIC evaluated H2). Messages: `HELLO` (auth, vector-clock summary, wire/model/capability negotiation, N−2 support), `OPS` (canonical CBOR, zstd batches, causally contiguous per actor), `NEED/GIVE` (anti-entropy: Merkle comparison localizes divergent tiles in O(log n)), `SNAP` (compacted snapshot + watermark: fresh joins; >180-day replicas rebase). The relay is fanout + retention + admission control — never a merge authority; it validates op schema/bounds and enforces per-actor token buckets (rate + bytes) and quotas.

## Presence
Ephemeral gossip channel (not in op log): cursors, selections, viewport, typing, calc state; 30 s TTL; identity-anchored selections survive concurrent structural edits.

## Offline (desktop is offline-first by construction)
Ops queue in the local container file; reconnect = ordinary CRDT merge with causal metadata; conflicts surface asynchronously, never modally. Published contract: 180-day staleness window; past it, snapshot-rebase; local unsynced ops are **never silently dropped** — unrebasable ops export to a user-visible "unmerged changes" ledger with cell-level content.

## Failure drills (tested, docs/35 §simulation)
Partition during structural edit storm; relay crash mid-batch (client redelivery via causal gaps); clock skew irrelevance (Lamport only); actor-counter collision impossibility (u128 actor ids, device-scoped); malicious-op injection from a permitted collaborator (schema+bounds validation on receive; adversarial corpus).
