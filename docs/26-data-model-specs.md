# 26 — Data Model & Container Schema Specs
Status: Approved · Owner: you · Normative: yes (the "database.md" of this project — but the database is SQLite files Ehkatra creates; DP-S5: never the host's Postgres)

## The container file (one workbook = one SQLite file, ADR-031)
`PRAGMA user_version` carries the schema version; WAL mode always; `application_id` set to a registered magic so `file` identifies Ehkatra containers.

```sql
-- schema v1 (Row 11 lands this; spec'd now so Rows 9–10 build toward it)
CREATE TABLE meta      (key TEXT PRIMARY KEY, value BLOB);          -- doc_id, model_version, created, actor registry
CREATE TABLE ops       (actor BLOB NOT NULL,                        -- u128 BE
                        counter INTEGER NOT NULL,
                        lamport INTEGER NOT NULL,
                        payload BLOB NOT NULL,                      -- canonical encoding (DP-A4), verbatim
                        PRIMARY KEY (actor, counter)) WITHOUT ROWID;
CREATE INDEX ops_total ON ops (lamport, actor, counter);            -- replay order
CREATE TABLE snapshots (watermark BLOB PRIMARY KEY,                 -- vector-clock summary, canonical encoding
                        state_hash BLOB NOT NULL,                   -- BLAKE3, verify on load
                        body BLOB NOT NULL,                         -- zstd tile+object image
                        created INTEGER NOT NULL);
CREATE TABLE blobs     (hash BLOB PRIMARY KEY, body BLOB NOT NULL); -- content-addressed (images, later)
CREATE TABLE undo      (actor BLOB, session BLOB, seq INTEGER,      -- durable undo stack (docs/11)
                        group_id BLOB, label TEXT,
                        PRIMARY KEY (actor, session, seq));
```

Rules: ops are immutable — INSERT only, never UPDATE/DELETE except compaction (which rewrites via a new file + atomic rename, never in place); `state_hash` verifies on every snapshot load (corruption = salvage path, docs/16, honest report); the payload column stores the *identical bytes* that were hashed — the file IS the wire format at rest (one encoding, DP-A4/13).

## Migration rules
Additive-only within a `user_version`; version bump = migration function registered in an ordered table of `(from, to, fn)`, run inside one transaction, verified by replaying to the same state hash afterward (a migration that changes the state hash is by definition wrong — the strongest migration test that exists, free because of DP-A2). Downgrade: older code opens newer files read-only via forward-preservation (DP-A5); it never writes a schema it doesn't know.

## Server-side (H2+, same shapes)
The compact server stores the same tables per workbook (SQLite) plus `tenants/principals/grants/audit` in a separate control DB — also SQLite until measured need says otherwise (DP-S1: one storage engine). The audit table is hash-chained: each row carries `prev_hash`; the chain head is externally anchorable.

## Identity encodings (canonical, everywhere)
ActorId u128 BE (16 bytes) · Counter u64 BE · OpId = actor‖counter (24 bytes) · RowId/ColId = the creating OpId · watermark = sorted (actor, max-counter) pairs, canonical encoding. Any new encoding needs an ADR — encodings are forever.
