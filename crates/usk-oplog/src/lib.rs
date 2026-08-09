//! usk-oplog — the op algebra, canonical encoding, and hashing.
//!
//! Ops are the ONLY mutation path (docs/03). This crate defines what an op
//! is, its single valid byte encoding, and the BLAKE3 hashing used for the
//! determinism gate (identical logs ⇒ identical hash on every platform).

#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use usk_types::{ColId, Lamport, OpId, RowId, Value};

/// Where a new row/col is placed relative to existing identities.
/// v0.1 uses neighbor-anchored insertion (Fugue-style origin); the order
/// CRDT in usk-state resolves concurrent inserts deterministically.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// Insert at the very start of the axis.
    Start,
    /// Insert immediately after the row/col created by this op id.
    After(OpId),
}

/// A formula reference bound to identity endpoints, carried inside
/// `SetFormula` ops.
///
/// Binding happens once, at the author, inside the reducer (DP-A7) — the op
/// then carries identities, so a replica never re-binds `A1` text against its
/// own (possibly different) view. This is what makes formulas converge under
/// concurrent structural edits: every replica resolves the same identities.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RangeBinding {
    pub row_start: OpId,
    pub row_end: OpId,
    pub col_start: OpId,
    pub col_end: OpId,
    /// bit 0 = row endpoints absolute, bit 1 = col endpoints absolute
    /// (`AnchorMode`, docs/11 — resolution ignores it; copy/fill reads it).
    pub anchors: u8,
}

/// The closed op payload taxonomy for model version 1 (docs/10).
/// Adding behavior later = adding a variant, never changing one.
#[derive(Clone, PartialEq, Debug)]
pub enum Payload {
    InsertRow {
        anchor: Anchor,
    },
    DeleteRow {
        row: RowId,
    },
    InsertCol {
        anchor: Anchor,
    },
    DeleteCol {
        col: ColId,
    },
    SetCell {
        row: RowId,
        col: ColId,
        value: Value,
    },
    ClearCell {
        row: RowId,
        col: ColId,
    },
    /// Stores a formula: the text as typed (so the CST is recoverable,
    /// ADR-011) plus one identity binding per reference in the AST, in
    /// traversal order.
    SetFormula {
        row: RowId,
        col: ColId,
        source: String,
        bindings: Vec<RangeBinding>,
    },
    /// Restores a deleted row — the inverse half of selective undo for
    /// `DeleteRow` (docs/11, DP-A12). A new op type rather than a change to
    /// `DeleteRow`'s meaning, per DP-A5.
    UndeleteRow {
        row: RowId,
    },
    UndeleteCol {
        col: ColId,
    },
    /// An op whose payload tag this build does not know, **kept verbatim**
    /// (DP-A5: unknown op types are preserved, causally ordered, hashed opaque
    /// and retransmitted).
    ///
    /// This is not a new op *type* — it carries no tag of its own. It is the
    /// representation of somebody else's tag, and re-encoding it reproduces the
    /// author's bytes exactly, so its hash is the hash the author computed.
    /// That byte-identity is the whole mechanism: an op we cannot interpret
    /// still participates in the op-set hash, still crosses the wire, and
    /// still comes back out of the container unchanged.
    ///
    /// It contributes **nothing to state**. A build that cannot read an op must
    /// not guess at what it meant, so the state hash of a workbook legitimately
    /// differs between a build that knows a tag and one that does not. DP-A5
    /// promises preservation, not cross-version state convergence.
    Opaque(OpaqueOp),
}

/// The payload tags model version 1 defines. Kept beside [`Op::decode`]'s match
/// and pinned to it by `every_known_tag_decodes_and_every_other_is_opaque` —
/// two lists that must agree are a defect waiting for the third person to edit
/// one of them.
pub fn is_known_tag(tag: u8) -> bool {
    matches!(tag, 0x10..=0x18)
}

/// An op body this build cannot interpret, held byte-exact.
///
/// The fields are private and the constructor refuses a *known* tag, so
/// "an opaque op secretly carrying a tag we do understand" — a second spelling
/// of a value the canonical encoding says has exactly one (DP-A4) — is
/// unrepresentable rather than merely discouraged.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpaqueOp {
    tag: u8,
    body: Vec<u8>,
}

impl OpaqueOp {
    /// `None` if `tag` is one this build knows: that op must be decoded, not
    /// preserved opaquely.
    pub fn new(tag: u8, body: Vec<u8>) -> Option<OpaqueOp> {
        if is_known_tag(tag) {
            return None;
        }
        Some(OpaqueOp { tag, body })
    }

    pub fn tag(&self) -> u8 {
        self.tag
    }

    /// Everything after the tag byte, exactly as it arrived.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// One immutable operation.
#[derive(Clone, PartialEq, Debug)]
pub struct Op {
    pub id: OpId,
    pub lamport: Lamport,
    pub payload: Payload,
}

fn encode_opid(id: &OpId, out: &mut Vec<u8>) {
    out.extend_from_slice(&id.actor.0.to_be_bytes());
    out.extend_from_slice(&id.counter.to_be_bytes());
}

fn encode_anchor(a: &Anchor, out: &mut Vec<u8>) {
    match a {
        Anchor::Start => out.push(0x00),
        Anchor::After(id) => {
            out.push(0x01);
            encode_opid(id, out);
        }
    }
}

impl Op {
    /// The single canonical encoding of this op (docs/10: one valid
    /// encoding per op, so hashing is well-defined). Deterministic by
    /// construction: fixed field order, big-endian integers, no maps.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        encode_opid(&self.id, &mut out);
        out.extend_from_slice(&self.lamport.to_be_bytes());
        match &self.payload {
            Payload::InsertRow { anchor } => {
                out.push(0x10);
                encode_anchor(anchor, &mut out);
            }
            Payload::DeleteRow { row } => {
                out.push(0x11);
                encode_opid(&row.0, &mut out);
            }
            Payload::InsertCol { anchor } => {
                out.push(0x12);
                encode_anchor(anchor, &mut out);
            }
            Payload::DeleteCol { col } => {
                out.push(0x13);
                encode_opid(&col.0, &mut out);
            }
            Payload::SetCell { row, col, value } => {
                out.push(0x14);
                encode_opid(&row.0, &mut out);
                encode_opid(&col.0, &mut out);
                value.encode_into(&mut out);
            }
            Payload::ClearCell { row, col } => {
                out.push(0x15);
                encode_opid(&row.0, &mut out);
                encode_opid(&col.0, &mut out);
            }
            // Tags 0x16-0x18 are new in Row 9; 0x10-0x15 keep their bytes,
            // which the unchanged replay-corpus hashes prove.
            Payload::SetFormula {
                row,
                col,
                source,
                bindings,
            } => {
                out.push(0x16);
                encode_opid(&row.0, &mut out);
                encode_opid(&col.0, &mut out);
                let b = source.as_bytes();
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                out.extend_from_slice(b);
                out.extend_from_slice(&(bindings.len() as u16).to_be_bytes());
                for binding in bindings {
                    encode_opid(&binding.row_start, &mut out);
                    encode_opid(&binding.row_end, &mut out);
                    encode_opid(&binding.col_start, &mut out);
                    encode_opid(&binding.col_end, &mut out);
                    out.push(binding.anchors);
                }
            }
            Payload::UndeleteRow { row } => {
                out.push(0x17);
                encode_opid(&row.0, &mut out);
            }
            Payload::UndeleteCol { col } => {
                out.push(0x18);
                encode_opid(&col.0, &mut out);
            }
            // Byte-exact re-emission. `OpaqueOp` cannot hold a known tag, so
            // this arm can never produce a second spelling of an op this build
            // could have encoded itself (DP-A4).
            Payload::Opaque(o) => {
                out.push(o.tag());
                out.extend_from_slice(o.body());
            }
        }
        out
    }

    /// The op as a **framed** stream element: `u32 big-endian length ‖ canonical
    /// op bytes` (TD-25).
    ///
    /// This is the prefix `ehkatra-relay`'s `put_ops` has always written, now
    /// shared with every place ops are concatenated without a container to
    /// delimit them — snapshot bodies and the recovery tail. One prefix, one
    /// meaning, in both directions.
    ///
    /// The prefix is deliberately *outside* [`Op::encode`] and therefore outside
    /// the hash. docs/26 requires the container's `payload` column to hold "the
    /// identical bytes that were hashed", which settles the question: the
    /// canonical encoding of an op is unframed, and framing is a property of a
    /// stream. It also means adding framing moved no hash.
    pub fn encode_framed(&self) -> Vec<u8> {
        let body = self.encode();
        let mut out = Vec::with_capacity(body.len() + 4);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Decodes one framed op from the front of `bytes`, returning it and the
    /// bytes consumed *including* the prefix.
    ///
    /// Because the frame states the op's extent, an unknown tag is no longer
    /// the end of the stream: it becomes a [`Payload::Opaque`] and reading
    /// continues. That is the whole reason TD-25 blocked DP-A5.
    pub fn decode_framed(bytes: &[u8]) -> Result<(Op, usize), DecodeError> {
        let header = bytes.get(..4).ok_or(DecodeError::Truncated)?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if len > MAX_OP_BYTES {
            return Err(DecodeError::FrameTooLarge { len });
        }
        let end = 4usize.checked_add(len).ok_or(DecodeError::Truncated)?;
        let body = bytes.get(4..end).ok_or(DecodeError::Truncated)?;
        Ok((Op::decode_exact(body)?, end))
    }

    /// Decodes one op that occupies **exactly** `bytes` — the form used wherever
    /// the extent comes from outside the encoding: a wire frame, a container
    /// column, or [`Op::decode_framed`]'s prefix.
    ///
    /// An unknown tag is preserved opaquely here and only here; [`Op::decode`],
    /// which must find the op's end by parsing it, still cannot and still says
    /// so.
    pub fn decode_exact(bytes: &[u8]) -> Result<Op, DecodeError> {
        let mut r = Reader { bytes, at: 0 };
        let id = r.opid()?;
        let lamport = r.u64()?;
        let tag = r.u8()?;
        if !is_known_tag(tag) {
            let opaque =
                OpaqueOp::new(tag, bytes[r.at..].to_vec()).ok_or(DecodeError::UnknownTag(tag))?;
            return Ok(Op {
                id,
                lamport,
                payload: Payload::Opaque(opaque),
            });
        }
        let (op, used) = Op::decode(bytes)?;
        if used != bytes.len() {
            return Err(DecodeError::TrailingBytes {
                used,
                len: bytes.len(),
            });
        }
        Ok(op)
    }

    /// Content hash of a single op.
    pub fn hash(&self) -> blake3::Hash {
        blake3::hash(&self.encode())
    }

    /// Decodes one op from the front of `bytes`, returning it and the number of
    /// bytes consumed.
    ///
    /// The inverse of [`Op::encode`], and the reason Row 10 can put ops on a
    /// socket at all. Decoding is **total**: every malformed input yields a
    /// `DecodeError`, never a panic and never a partially-built op (DP-A10).
    /// Unknown tags are reported rather than skipped — forward preservation
    /// (DP-A5) needs the op's length to retain it opaquely, and this encoding
    /// does not yet carry one, so the honest answer today is a named error.
    /// Recorded as TD-25.
    pub fn decode(bytes: &[u8]) -> Result<(Op, usize), DecodeError> {
        let mut r = Reader { bytes, at: 0 };
        let id = r.opid()?;
        let lamport = r.u64()?;
        let payload = match r.u8()? {
            0x10 => Payload::InsertRow {
                anchor: r.anchor()?,
            },
            0x11 => Payload::DeleteRow {
                row: RowId(r.opid()?),
            },
            0x12 => Payload::InsertCol {
                anchor: r.anchor()?,
            },
            0x13 => Payload::DeleteCol {
                col: ColId(r.opid()?),
            },
            0x14 => Payload::SetCell {
                row: RowId(r.opid()?),
                col: ColId(r.opid()?),
                value: r.value()?,
            },
            0x15 => Payload::ClearCell {
                row: RowId(r.opid()?),
                col: ColId(r.opid()?),
            },
            0x16 => {
                let row = RowId(r.opid()?);
                let col = ColId(r.opid()?);
                let len = r.u32()? as usize;
                let source = r.utf8(len)?;
                let count = r.u16()? as usize;
                let mut bindings = Vec::with_capacity(count.min(1024));
                for _ in 0..count {
                    bindings.push(RangeBinding {
                        row_start: r.opid()?,
                        row_end: r.opid()?,
                        col_start: r.opid()?,
                        col_end: r.opid()?,
                        anchors: r.u8()?,
                    });
                }
                Payload::SetFormula {
                    row,
                    col,
                    source,
                    bindings,
                }
            }
            0x17 => Payload::UndeleteRow {
                row: RowId(r.opid()?),
            },
            0x18 => Payload::UndeleteCol {
                col: ColId(r.opid()?),
            },
            tag => return Err(DecodeError::UnknownTag(tag)),
        };
        Ok((
            Op {
                id,
                lamport,
                payload,
            },
            r.at,
        ))
    }
}

/// The largest single framed op this build will allocate for. An admission
/// bound, not a format limit (docs/37 boundary 2): a corrupt or hostile length
/// prefix must not be able to ask for memory.
pub const MAX_OP_BYTES: usize = 16 << 20;

/// Why a byte string is not an op. Errors are values here too (DP-A10).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    /// The input ended inside a field.
    Truncated,
    /// A payload tag this build does not know, reached through [`Op::decode`],
    /// which has no frame to tell it where the op ends. The framed readers
    /// preserve it instead (TD-25, DP-A5).
    UnknownTag(u8),
    /// A frame claims a length past [`MAX_OP_BYTES`].
    FrameTooLarge { len: usize },
    /// An op decoded successfully but did not fill the extent it was given.
    TrailingBytes { used: usize, len: usize },
    /// A value tag this build does not know.
    UnknownValueTag(u8),
    /// Text that is not valid UTF-8.
    BadUtf8,
    /// An enum discriminant outside its defined range.
    BadDiscriminant,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    fn u128(&mut self) -> Result<u128, DecodeError> {
        let b = self.take(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(b);
        Ok(u128::from_be_bytes(a))
    }

    fn i128(&mut self) -> Result<i128, DecodeError> {
        Ok(self.u128()? as i128)
    }

    fn opid(&mut self) -> Result<OpId, DecodeError> {
        Ok(OpId {
            actor: usk_types::ActorId(self.u128()?),
            counter: self.u64()?,
        })
    }

    fn utf8(&mut self, len: usize) -> Result<String, DecodeError> {
        let b = self.take(len)?;
        core::str::from_utf8(b)
            .map(alloc::borrow::ToOwned::to_owned)
            .map_err(|_| DecodeError::BadUtf8)
    }

    fn anchor(&mut self) -> Result<Anchor, DecodeError> {
        match self.u8()? {
            0x00 => Ok(Anchor::Start),
            0x01 => Ok(Anchor::After(self.opid()?)),
            _ => Err(DecodeError::BadDiscriminant),
        }
    }

    fn value(&mut self) -> Result<Value, DecodeError> {
        match self.u8()? {
            0x00 => Ok(Value::Blank),
            0x01 => Ok(Value::Bool(false)),
            0x02 => Ok(Value::Bool(true)),
            0x03 => Ok(Value::Number(f64::from_bits(self.u64()?))),
            0x04 => {
                let len = self.u32()? as usize;
                Ok(Value::Text(self.utf8(len)?))
            }
            0x05 => {
                let kind = match self.u8()? {
                    0 => usk_types::ErrorKind::Div0,
                    1 => usk_types::ErrorKind::Value,
                    2 => usk_types::ErrorKind::Ref,
                    3 => usk_types::ErrorKind::Name,
                    4 => usk_types::ErrorKind::Num,
                    5 => usk_types::ErrorKind::Na,
                    6 => usk_types::ErrorKind::Circ,
                    7 => usk_types::ErrorKind::Spill,
                    _ => return Err(DecodeError::BadDiscriminant),
                };
                Ok(Value::Error(usk_types::CellError {
                    kind,
                    origin: self.origin()?,
                }))
            }
            0x06 => {
                let coefficient = self.i128()?;
                let b = self.take(2)?;
                let exponent = i16::from_be_bytes([b[0], b[1]]);
                // `new` canonicalises, which is a no-op on bytes the encoder
                // produced (they were canonical already) and a *repair* on
                // bytes from anywhere else — so a hostile peer cannot smuggle
                // a second representation of one value past the DP-A4 rule.
                Ok(Value::Decimal(usk_types::Decimal::new(
                    coefficient,
                    exponent,
                )))
            }
            tag => Err(DecodeError::UnknownValueTag(tag)),
        }
    }

    fn origin(&mut self) -> Result<usk_types::Origin, DecodeError> {
        match self.u8()? {
            0x00 => Ok(usk_types::Origin::Authored),
            0x01 => Ok(usk_types::Origin::Coercion {
                from: self.type_tag()?,
                to: self.type_tag()?,
            }),
            0x02 => Ok(usk_types::Origin::Arithmetic { op: self.arith()? }),
            0x03 => Ok(usk_types::Origin::Propagated),
            _ => Err(DecodeError::BadDiscriminant),
        }
    }

    fn type_tag(&mut self) -> Result<usk_types::TypeTag, DecodeError> {
        use usk_types::TypeTag::*;
        Ok(match self.u8()? {
            0 => Blank,
            1 => Bool,
            2 => Number,
            3 => Decimal,
            4 => Text,
            5 => Error,
            _ => return Err(DecodeError::BadDiscriminant),
        })
    }

    fn arith(&mut self) -> Result<usk_types::ArithOp, DecodeError> {
        use usk_types::ArithOp::*;
        Ok(match self.u8()? {
            0 => Add,
            1 => Sub,
            2 => Mul,
            3 => Div,
            _ => return Err(DecodeError::BadDiscriminant),
        })
    }
}

/// A causally-ordered set of ops. v0.1 keeps the log as a simple grow-only
/// vec per replica; `merged_hash` defines the canonical hash over a set of
/// ops *independent of arrival order* — the determinism-gate primitive.
#[derive(Default, Clone)]
pub struct OpLog {
    ops: Vec<Op>,
}

impl OpLog {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn append(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn merge_from(&mut self, other: &OpLog) {
        for op in &other.ops {
            if !self.ops.iter().any(|o| o.id == op.id) {
                self.ops.push(op.clone());
            }
        }
    }

    /// Canonical hash over the op *set*: ops are sorted by the total order
    /// (lamport, actor, counter) and chain-hashed. Two replicas holding the
    /// same set in any arrival order produce the same hash.
    pub fn canonical_hash(&self) -> blake3::Hash {
        let mut sorted: Vec<&Op> = self.ops.iter().collect();
        sorted.sort_by_key(|o| (o.lamport, o.id.actor, o.id.counter));
        let mut hasher = blake3::Hasher::new();
        for op in sorted {
            hasher.update(op.hash().as_bytes());
        }
        hasher.finalize()
    }
}
