//! usk-types — core identity and value types for the Ehkatra kernel.
//!
//! `no_std + alloc`: this crate must never touch the OS (docs/10, invariant I1).
//! All identity is permanent: ids are never reused, renumbered, or recycled.

#![no_std]
extern crate alloc;

pub mod coerce;
pub mod decimal;

use alloc::string::String;
use alloc::vec::Vec;
pub use decimal::Decimal;

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

impl ErrorKind {
    /// The name a user sees. Storage is canonical and display is localized
    /// (SPEC §i18n); this is the canonical spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Num => "#NUM!",
            ErrorKind::Na => "#N/A",
            ErrorKind::Circ => "#CIRC!",
            ErrorKind::Spill => "#SPILL!",
        }
    }
}

/// A value's type, without its payload — the vocabulary error origins use to
/// say what was being converted into what.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TypeTag {
    Blank,
    Bool,
    Number,
    Decimal,
    Text,
    Error,
}

/// The arithmetic operation that produced an error, for `Origin::Arithmetic`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// **Why this error exists** — the differentiator behind "every `#VALUE!`
/// answers where it came from" (BOOTSTRAP §10.6, DP-A11).
///
/// The origin records only what cannot be recovered from the op log. It does
/// not carry the authoring op id, because a value lives *inside* its own op and
/// so cannot name it; the log already answers "who wrote this cell".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Entered or imported as a literal error (an `#N/A` typed by a user, or
    /// an error cell read out of an XLSX).
    Authored,
    /// A conversion the `strict` profile refused to perform silently.
    Coercion { from: TypeTag, to: TypeTag },
    /// Arithmetic with no defined result: division by zero, or a magnitude
    /// past the decimal coefficient's range.
    Arithmetic { op: ArithOp },
    /// Inherited from a referenced cell.
    ///
    /// Row 6 extends this with the source cell once formulas can reference one;
    /// adding a field is additive (docs/10 evolution rule), and the trace is
    /// deliberately *not* stored per value — a full provenance chain belongs in
    /// a watermarked fold over the log (DP-A9), not inline in every cell.
    Propagated,
}

/// An error value: the kind a user sees, plus why it happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellError {
    pub kind: ErrorKind,
    pub origin: Origin,
}

impl CellError {
    pub fn new(kind: ErrorKind, origin: Origin) -> CellError {
        CellError { kind, origin }
    }

    /// The `#VALUE!` raised when `strict` refuses a silent conversion.
    pub fn refused_coercion(from: TypeTag, to: TypeTag) -> CellError {
        CellError::new(ErrorKind::Value, Origin::Coercion { from, to })
    }
}

/// The v0.1 value lattice (docs/04, BOOTSTRAP row 5).
///
/// `Date`/`DateTime`/`Duration`/`Array`/`Reference`/`Rich`/`Lambda` are in the
/// full lattice docs/04 describes but are not in this row's scope; they land
/// with the rows that need them, as additive variants (docs/10 evolution rule).
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Blank,
    Bool(bool),
    /// IEEE-754 binary64, Excel-compatible arithmetic domain.
    Number(f64),
    /// Exact base-10, for currency (ADR-010/ADR-035). See [`decimal`].
    Decimal(Decimal),
    Text(String),
    Error(CellError),
}

impl Value {
    /// This value's type, for coercion diagnostics.
    pub fn type_tag(&self) -> TypeTag {
        match self {
            Value::Blank => TypeTag::Blank,
            Value::Bool(_) => TypeTag::Bool,
            Value::Number(_) => TypeTag::Number,
            Value::Decimal(_) => TypeTag::Decimal,
            Value::Text(_) => TypeTag::Text,
            Value::Error(_) => TypeTag::Error,
        }
    }

    /// Errors propagate through everything (docs/04 invariant 5).
    pub fn as_error(&self) -> Option<CellError> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }
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
            Value::Error(e) => {
                out.push(0x05);
                out.push(e.kind as u8);
                encode_origin(&e.origin, out);
            }
            // Tag 0x06 is new in Row 5. Tags 0x00–0x05 keep their meaning and
            // their bytes, which the unchanged replay-corpus oplog hash proves.
            Value::Decimal(d) => {
                out.push(0x06);
                out.extend_from_slice(&d.coefficient().to_be_bytes());
                out.extend_from_slice(&d.exponent().to_be_bytes());
            }
        }
    }
}

/// Canonical encoding of an error origin. Fixed field order, big-endian, one
/// byte per enum discriminant — same rules as every other encoding here.
fn encode_origin(origin: &Origin, out: &mut Vec<u8>) {
    match origin {
        Origin::Authored => out.push(0x00),
        Origin::Coercion { from, to } => {
            out.push(0x01);
            out.push(*from as u8);
            out.push(*to as u8);
        }
        Origin::Arithmetic { op } => {
            out.push(0x02);
            out.push(*op as u8);
        }
        Origin::Propagated => out.push(0x03),
    }
}
