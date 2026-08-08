//! The container schema — **docs/26, verbatim** (ADR-031).
//!
//! The SQL below is copied from docs/26 §"The container file" rather than
//! paraphrased, because that document is normative and a schema that has
//! drifted from its specification is a defect nobody notices until a migration.
//! If these strings and docs/26 ever disagree, the document wins.

/// `PRAGMA user_version` — the schema version (docs/26).
pub const USER_VERSION: i32 = 1;

/// `PRAGMA application_id` — "so `file` identifies Ehkatra containers"
/// (docs/26). ASCII "EHKA" big-endian, which is what `file`'s magic table and a
/// hex dump both show.
pub const APPLICATION_ID: i32 = 0x4548_4B41;

/// Schema v1, exactly as docs/26 specifies it.
pub const SCHEMA_V1: &str = "
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
";

/// The `snapshots.body` column is specified as a "zstd tile+object image".
///
/// v0.1 stores the body **uncompressed**, and says so rather than implying
/// otherwise: the body format is D-069's compacted op set, zstd is a fourth
/// dependency stack on top of the nineteen `rusqlite` already cost (D-073), and
/// compression changes no property this row proves — the state hash is verified
/// over the *decoded* ops either way. Filed as TD-29.
pub const BODY_IS_COMPRESSED: bool = false;
