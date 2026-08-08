//! ehkatra-relay — the sync **shell**: transport, framing, and the composition
//! of an editing session with a sync session (BOOTSTRAP row 10).
//!
//! Everything here is `std`, and that is the point: the kernel crates own the
//! protocol semantics and perform no I/O, while this crate owns sockets, byte
//! framing and the event loop. Swapping the transport (loopback TCP today,
//! WebSocket later — D-065) touches [`frame`] and [`main`] and nothing else.
//!
//! * [`frame`] — the wire format.
//! * [`replica`] — workbook + connection, and the `Action` → message pump.
//! * [`endpoint`] — the relay's message handling over `usk_sync::Relay`.
//! * [`bus`] — a deterministic in-process transport for tests and benchmarks.

pub mod bus;
pub mod endpoint;
pub mod frame;
pub mod replica;

pub use bus::Bus;
pub use endpoint::{RelayEndpoint, RelayOut, RelayStats};
pub use frame::{read_frame, write_frame, Message};
pub use replica::Replica;

/// Loopback port for the relay (DP-S5 names 7423/7424; the relay takes the
/// first). Never bound on a non-loopback interface, and the process fails fast
/// rather than fighting for a port that is already taken.
pub const RELAY_PORT: u16 = 7423;
