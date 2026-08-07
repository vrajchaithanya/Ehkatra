# 27 — State Machine Specifications
Status: Approved · Normative: yes · Needed by: Row 10 (sync), Row 11 (recovery), Q2 shell

Every stateful protocol in the platform is specified as an explicit machine: states, events, transitions, and — the part that catches bugs — **what is forbidden**. Implementations mirror these as Rust enums; a transition not listed here is a `debug_assert` + logged error, never silent.

## 1. Replica sync session (per workbook connection) — Row 10 implements this
```
DISCONNECTED ──connect──► HELLO_SENT ──HelloAck(negotiated)──► SYNCING
HELLO_SENT ──HelloReject(version)──► INCOMPATIBLE (terminal until upgrade; read-only local)
SYNCING ──vector clocks equal──► LIVE
SYNCING ──NEED/GIVE exchange──► SYNCING            (anti-entropy loop, Merkle-guided)
LIVE ──local op──► LIVE (send OPS)                 (echo suppressed by op id)
LIVE ──remote OPS──► LIVE (apply, ack watermark)
LIVE|SYNCING ──transport loss──► BACKOFF(n) ──timer──► HELLO_SENT   (jittered exp backoff, cap 60 s)
LIVE ──StalenessExceeded(180d watermark gap)──► REBASE_REQUIRED ──snapshot+migrate_ops──► SYNCING
any ──auth revoked──► DISCONNECTED (queued ops retained; never dropped)
```
Forbidden: sending OPS before HelloAck; applying remote ops that fail schema/bounds validation (reject + report, stay LIVE); dropping queued local ops in any transition (DP: never-drop, docs/15).

## 2. Document lifecycle (container, Row 11)
```
CLOSED ──open──► RECOVERING ──snapshot ok + tail replay──► READY
RECOVERING ──snapshot hash mismatch──► SALVAGE (last valid snapshot + readable tail
             + quarantined remainder + honest user report) ──user ack──► READY
READY ──ops──► READY (WAL append ≤250 ms batched fsync)
READY ──compaction trigger──► COMPACTING (new file, atomic rename) ──► READY
READY ──close──► CLOSED (final fsync; no other work permitted)
```
Forbidden: writing to the old file during COMPACTING; opening READY without hash-verifying the loaded snapshot; any transition that loses acked ops (RPO contract, docs/16).

## 3. Calc engine (already implemented in Rows 7–8; recorded here as-built)
```
IDLE ──edit/dirty──► MARKING ──topo levels assigned──► EVALUATING
EVALUATING ──interrupt (new op)──► MARKING            (frontier checkpointed; undirtied cells keep last-consistent values)
EVALUATING ──level exhausted──► IDLE (generation++)
MARKING ──cycle found──► IDLE (cells → #CIRC!)
```
Forbidden: blocking the UI thread in any state; observing half-evaluated generations without a generation mark.

## 4. Agent mutation session (Row 14, MCP)
```
ORIENTED ──preview_edits──► PREVIEWED(hash) ──apply_edits(hash, version ok)──► APPLIED(group)
PREVIEWED ──workbook advanced──► STALE ──re-preview──► PREVIEWED
ORIENTED ──apply_edits without preview, size>threshold──► REFUSED (policy, host-enforced)
APPLIED ──undo(group|session)──► REVERTED
```
Forbidden: APPLIED without auto-milestone (docs/16); any transition that skips the blast-radius policy check; treating REFUSED as retryable-without-preview.

## 5. Undo stack (per actor·session, Row 9)
States per group: `LIVE ──undo──► UNDONE ──redo──► LIVE`; structural undo entering `NARROWED(notice)` when others' ops would be destroyed. Forbidden: undoing another actor's group; a group spanning two Commands.

Testing: each machine gets a transition-coverage test (every listed edge exercised) plus a forbidden-transition test (every "forbidden" line proven to be rejected). The sync machine additionally runs under the seeded partition/reorder sweep (docs/35 §6).
