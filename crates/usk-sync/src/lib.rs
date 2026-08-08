//! usk-sync — the replica sync protocol (BOOTSTRAP row 10; docs/15, docs/27 §1,
//! docs/37 boundary 2).
//!
//! # What is here, and what deliberately is not
//! Everything in this crate is a **pure function of state and input**: the
//! session machine, the never-drop queue, vector clocks and the causal-gap
//! buffer, receive-side validation, and the relay's admission control. There is
//! no socket, no clock and no entropy source anywhere in it — time arrives as an
//! event, jitter arrives as an injected seed, and I/O is something the shell
//! does with the [`machine::Action`]s the machine returns (DP-A2, DP-A3).
//!
//! That split is what makes the protocol testable: partitions, packet loss,
//! reordering, hostile ops and mid-run kills are all just event sequences, so
//! the failure drills docs/15 asks for run in microseconds with no network.
//!
//! # The three rules that are not negotiable
//! * **Never drop a local op** (docs/15 §Offline). Enforced structurally in
//!   [`queue::Queue`]: acknowledgement is the only removal path.
//! * **Never apply an unvalidated remote op** (DP-E4). Enforced in
//!   [`validate`]; refused ops are quarantined and reported, and the session
//!   stays LIVE.
//! * **Never take an unlisted transition** (docs/27). Enforced in
//!   [`machine::SyncSession::step`], which logs and does nothing rather than
//!   inventing behaviour.

#![no_std]
extern crate alloc;

pub mod clock;
pub mod machine;
pub mod queue;
pub mod relay;
pub mod validate;

pub use clock::{CausalBuffer, VectorClock};
pub use machine::{Action, Event, Hello, Negotiated, SyncSession, SyncState};
pub use queue::Queue;
pub use relay::{Admission, AdmissionError, Relay};
pub use validate::{RejectReason, Rejection};
