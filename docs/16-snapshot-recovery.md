# 16 — Snapshot, Recovery & Autosave
Status: Approved · Owner: Distributed Systems Lead · Normative: yes · Carved from SPEC §14

## Three products over one log (kept deliberately separate — ADR-015)
**Undo** (docs/11): seconds→days, per-user, group-granular. **Version history**: named versions (immutable refs), auto-milestones (session boundaries, imports, **always before agent batches** — "restore to before the AI" is guaranteed one-click), branches (fork of log at a version; merge = ordinary CRDT replay + preview diff; serves what-if and agent sandboxes), blame/diff views computed from the log. **Recovery**: snapshots + op tail.

## Snapshots
Content-addressed, structurally shared via tile Merkle identity (cost O(dirty), so minutes-granular point-in-time is cheap). Cadence: every N ops / T minutes, adaptive. Compaction rewrites deprecated op types (docs/10) and collects tombstones past watermark.

## Retention (normative — added 2026-08-08, architect ruling on TD-30)
The salvage path below promises "the **last valid** snapshot", which presupposes that more than one exists. It is a promise about the *retention policy*, not about the salvage code, and it was never written down. It is now:

> **Keep the last 3 snapshots, plus ALL ops since the oldest retained snapshot.**
>
> **Compaction may never leave the container in a state where a single corrupt snapshot loses acknowledged ops.**

Three consequences bind implementations:
1. **Three, not one.** A container holding one snapshot has no fallback: corrupting it destroys everything it compacted, and the salvage report says `lost_data` with *zero* quarantined bytes — nothing damaged, because nothing was left. That is what W-OPEN-1M measured (1,002,000 ops gone), and it is the failure this rule removes.
2. **The op floor is the *oldest* retained snapshot, not the newest.** Retaining only the ops the newest snapshot fails to cover re-creates the same hole one level down: falling back to snapshot 2 would then land on a tail that starts after snapshot 1. Ops are pruned to the oldest retained watermark or not at all.
3. **A snapshot may only authorise the deletion of ops it has *proven* it contains.** The floor is the oldest retained snapshot **that verifies** — body decodes, replay reproduces the recorded state hash, watermark matches — checked at compaction time, immediately before the ops it covers are dropped. An unverified snapshot authorising deletion is precisely how one corruption becomes total loss.
4. **No pruning below two verified snapshots.** If fewer than two retained snapshots verify, compaction drops no ops at all. This is the invariant stated operationally: with a single snapshot the tail is empty by construction, so corrupting that one snapshot loses the workbook — the exact failure being removed. With two or more verified snapshots the guarantee is structural: snapshots are nested (each covers everything the older ones cover), so whichever single snapshot is corrupt, the newest surviving one covers at least the floor and the retained tail covers the rest.

Cost, stated rather than hidden: while a snapshot body is the compacted op set (the v0.1 format), three snapshots hold three copies of the covered history, so the container is larger. In the designed tile-Merkle body the additional snapshots are O(dirty) and this cost disappears; until then it is a measured tradeoff of file size against recoverability, and recoverability wins. Recoverability is a property of this policy, not of the salvage code — the same salvage code recovered everything or nothing depending only on what the policy above it had left behind.

## Recovery objectives (desktop + server, distinct)
Desktop crash: RPO = last local fsync ≤ 250 ms of typing (SQLite WAL, batched commit); RTO < 3 s to interactive (snapshot decode + tail replay), including after power loss mid-write (WAL guarantees; crash-injection tested every release). Server: RPO ≤ 1 s acked ops (relay durable-ack after replicated write); RTO < 5 s for 100 MB workbook. Corrupted container: salvage path = last valid snapshot + readable tail + quarantined remainder, with an honest user report — silent partial restore is forbidden.

## Autosave
There is no Save and no dirty bit. Ops are durable locally within 250 ms; ⌘S/Ctrl-S creates a *named version* with a toast explaining continuous save (the 40-year habit gets a bridge, not a lecture). "Save As" = branch or export; export is explicitly a projection, not saving.

## Drills
Automated restore verification runs continuously against production-shaped data (server) and in CI crash-injection harnesses (desktop). An untested backup is a hope, not a design.
