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
HELLO_SENT|BACKOFF ──transport loss──► DISCONNECTED (session torn down and rebuilt) ──timer──► HELLO_SENT
LIVE ──StalenessExceeded(180d watermark gap)──► REBASE_REQUIRED ──snapshot+migrate_ops──► SYNCING
any ──auth revoked──► DISCONNECTED (queued ops retained; never dropped)
```
Forbidden: sending OPS before HelloAck; applying remote ops that fail schema/bounds validation (reject + report, stay LIVE); dropping queued local ops in any transition (DP: never-drop, docs/15).

### 1a. Loop and late-arrival edges (ratified 2026-08-08; see docs/43 D-064 and both addenda)
Row 10 implemented §1 transition-for-transition and found four (state, event) pairs a real link produces that the table above did not define. They are now part of the specification, stated as the implementation resolves them.

**Transport loss during HELLO_SENT or BACKOFF** — the row added above. There is no live session to move to BACKOFF and, after a retry has already fired, no timer to wait on; the first implementation therefore did nothing and a replica that lost its link mid-handshake wedged in HELLO_SENT permanently (found by W-SYNC-RELAY at 50 replicas: the run diverged and a mid-run kill delivered 1 of 117 queued ops). The session is **torn down and rebuilt from DISCONNECTED**, carrying the durable op log and **every unacknowledged op** across — never-drop survives a teardown for the same reason it survives a process death — and armed with a jittered delay before reconnecting. Reconnection asks the current state which edge is listed (`DISCONNECTED → connect`, `BACKOFF → timer`) rather than assuming one.

**Late or duplicated protocol messages.** These are transport facts, not state changes, and the resolution is uniform: *route by the state that defines the message, discard otherwise.* Discarding a **protocol** message is safe because the next anti-entropy round re-derives whatever it carried; a **local op** is never what gets discarded, which is the never-drop queue's job.
* `InSync` arriving in LIVE (the session already converged) — **discard**, it is a duplicate.
* `GIVE` in LIVE, or `OPS` in SYNCING — **one payload, two names**: route to whichever of the `Give`/`RemoteOps` edges the current state defines.
* `ACK` outside LIVE or SYNCING — **discard**; there is no queue to release in those states.

**The adaptation belongs in the transport shell, not in the machine** (docs/20: L3 changes transport, never semantics). The machine stays closed, and the `debug_assert` below stays with it: keeping it has caught two distinct shell defects — the wedge above, and a reconnect helper that called `connect` from BACKOFF — where widening the machine to "accept anything from anywhere" would have turned both into silent behaviour changes. **That record is the argument: a closed machine moves the cost from debugging a divergence to fixing a caller.**

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
