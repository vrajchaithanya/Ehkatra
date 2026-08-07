//! usk-types — core identity and value types for the Ehkatra kernel.
//!
//! `no_std + alloc`: this crate must never touch the OS (docs/10, invariant I1).
//! All identity is permanent: ids are never reused, renumbered, or recycled.

#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// A replica/participant identity. 128 bits so device-scoped random ids
/// cannot collide in practice (docs/15 failure drills).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ActorId(pub u128);

/// Per-actor monotonic counter. (actor, counter) is globally unique.
pub type Counter = u64;

/// Globally unique op identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct OpId {
    pub actor: ActorId,
    pub counter: Counter,
}

/// Lamport timestamp for total-order tie-breaking (never wall-clock: docs/10).
pub type Lamport = u64;

/// Permanent row/column identities. The *order* among them lives in the
/// order CRDT (usk-state); the id itself is just identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RowId(pub OpId);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ColId(pub OpId);

/// Spreadsheet error kinds (errors are values, never panics — docs/12).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    Div0,
    Value,
    Ref,
    Name,
    Num,
    Na,
    Circ,
    Spill,
}

/// The v0.1 value lattice subset (docs/04). Decimal/Date/etc. arrive with
/// the formula milestone; adding variants is additive (docs/10 evolution rule).
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Blank,
    Bool(bool),
    /// IEEE-754 binary64, Excel-compatible arithmetic domain.
    Number(f64),
    Text(String),
    Error(ErrorKind),
}

impl Value {
    /// Canonical byte encoding of a value for hashing/encoding.
    /// Exactly one valid encoding per value (docs/10: canonical rule).
    /// NaN is normalized to a single bit pattern so hashes are stable.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Value::Blank => out.push(0x00),
            Value::Bool(false) => out.push(0x01),
            Value::Bool(true) => out.push(0x02),
            Value::Number(n) => {
                out.push(0x03);
                let bits = if n.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    n.to_bits()
                };
                out.extend_from_slice(&bits.to_be_bytes());
            }
            Value::Text(s) => {
                out.push(0x04);
                let b = s.as_bytes();
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                out.extend_from_slice(b);
            }
            Value::Error(k) => {
                out.push(0x05);
                out.push(*k as u8);
            }
        }
    }
}
