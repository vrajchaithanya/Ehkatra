//! usk-recover — snapshots, the document lifecycle, and the SALVAGE path
//! (BOOTSTRAP row 11; docs/16, docs/26, docs/27 §2).
//!
//! # What is here, and what is deliberately elsewhere
//! Everything above the storage seam: what a snapshot *is*, how it proves
//! itself, what recovery decides when it cannot, and the lifecycle machine that
//! refuses to open a document it has not verified. All of it is `no_std` and
//! pure — corruption is a byte array, a crash is a truncated slice, and a
//! torn write is a test input.
//!
//! Everything below the seam — the SQLite container of docs/26, WAL fsync
//! cadence, atomic-rename compaction — is a shell adapter, and is **blocked on
//! this build host** (D-068, TD-28): `rusqlite` can only be built through its
//! `bundled` feature, which compiles C, and the pinned toolchain ships a
//! link-only gcc driver with no `cc1`. That half is not written blind, because
//! DP-C4 forbids building on an unverified layer and SQL that has never
//! executed is exactly that.
//!
//! The split is not a workaround. It is the same seam that let `usk-sync` prove
//! a network protocol without a network: the logic that can be wrong in
//! interesting ways is separated from the I/O that can only be wrong in boring
//! ones.

#![no_std]
extern crate alloc;

pub mod machine;
pub mod salvage;
pub mod snapshot;

pub use machine::{Action, DocState, Document, Event};
pub use salvage::{recover, SalvageReason, SalvageReport, Salvaged};
pub use snapshot::{Snapshot, SnapshotFault, VerifiedSnapshot, Watermark};
