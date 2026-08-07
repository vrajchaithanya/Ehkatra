# 16 — Snapshot, Recovery & Autosave
Status: Approved · Owner: Distributed Systems Lead · Normative: yes · Carved from SPEC §14

## Three products over one log (kept deliberately separate — ADR-015)
**Undo** (docs/11): seconds→days, per-user, group-granular. **Version history**: named versions (immutable refs), auto-milestones (session boundaries, imports, **always before agent batches** — "restore to before the AI" is guaranteed one-click), branches (fork of log at a version; merge = ordinary CRDT replay + preview diff; serves what-if and agent sandboxes), blame/diff views computed from the log. **Recovery**: snapshots + op tail.

## Snapshots
Content-addressed, structurally shared via tile Merkle identity (cost O(dirty), so minutes-granular point-in-time is cheap). Cadence: every N ops / T minutes, adaptive. Compaction rewrites deprecated op types (docs/10) and collects tombstones past watermark.

## Recovery objectives (desktop + server, distinct)
Desktop crash: RPO = last local fsync ≤ 250 ms of typing (SQLite WAL, batched commit); RTO < 3 s to interactive (snapshot decode + tail replay), including after power loss mid-write (WAL guarantees; crash-injection tested every release). Server: RPO ≤ 1 s acked ops (relay durable-ack after replicated write); RTO < 5 s for 100 MB workbook. Corrupted container: salvage path = last valid snapshot + readable tail + quarantined remainder, with an honest user report — silent partial restore is forbidden.

## Autosave
There is no Save and no dirty bit. Ops are durable locally within 250 ms; ⌘S/Ctrl-S creates a *named version* with a toast explaining continuous save (the 40-year habit gets a bridge, not a lecture). "Save As" = branch or export; export is explicitly a projection, not saving.

## Drills
Automated restore verification runs continuously against production-shaped data (server) and in CI crash-injection harnesses (desktop). An untested backup is a hope, not a design.
