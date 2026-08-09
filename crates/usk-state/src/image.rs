//! The **tile image**: a materialised `State`, serialised (docs/16, docs/26).
//!
//! > docs/16: *Snapshots ... content-addressed, structurally shared via tile
//! > Merkle identity (cost O(dirty))*
//! > docs/26: *`snapshots.body` — zstd tile+object image*
//!
//! # Why this exists
//! A v0.1 snapshot body was the compacted **op set** (D-069), and `verify`
//! proved it by replaying it — so opening a workbook cost a full replay and
//! three retained snapshots cost three copies of the whole history. That is one
//! design decision showing up as three debt entries: TD-45 (cold open 7.86 s at
//! 1M cells), TD-31 (container 307 MB), and TD-24's residual (state is not
//! adoptable without folding the log).
//!
//! An image is the fold *itself*, written down. Loading is a decode, not a
//! replay.
//!
//! # What makes this safe
//! DP-A1 says ops are the only mutation path and state mutators stay private to
//! the applier — and this module does **not** weaken that. An image is a
//! materialised cache of a fold, which is exactly what DP-A9 says a cache is,
//! and it is verified the way a cache must be: [`State::from_image`] rebuilds,
//! `state_hash()` is recomputed, and `Snapshot::verify` refuses the image if it
//! does not match what the snapshot recorded. An image that hashes correctly
//! *is* the state the ops would have produced; one that does not is refused
//! before anything can read it.
//!
//! # Format
//! Little-endian, length-prefixed, no padding. Sections in a fixed order:
//! magic · version · rows axis · cols axis · slot maps · tiles · formulas.
//! Every count is bounded on read, because an image is an untrusted input the
//! moment a container is a file somebody can hand you (docs/37).
//!
//! Chunking for structural sharing is deliberately **not** here: this produces
//! one self-contained image, and [`State::write_image_parts`] exposes the
//! per-tile byte runs a container can content-address into `blobs`. Keeping the
//! layout in one place means the shared and unshared forms cannot disagree
//! about what a tile is.
//!
//! # Not yet the snapshot body
//! docs/16 and docs/26 both specify this as `snapshots.body`, and it is not
//! wired in — see TD-46 and `usk_recover::snapshot`'s module docs. The
//! blocker is not the format: it is that adopting an image and applying a tail
//! onto it loses the identity of a retained loser at any cell first written
//! inside the image, which ADR-006 and DP-A8 promise to keep.

use alloc::string::String;
use alloc::vec::Vec;
use usk_oplog::RangeBinding;
use usk_types::{ActorId, ColId, Decimal, OpId, RowId, Value};

use crate::formula::{FormulaCell, FormulaRegistry};
use crate::tile::{TileKey, TileStore};
use crate::{AxisSeq, State};

const MAGIC: &[u8; 8] = b"EHKIMG\0\0";
/// Image format version. A reader refuses anything it does not know rather
/// than guessing — an image is a *cache*, so refusing costs a replay from the
/// op tail and never costs data.
pub const IMAGE_VERSION: u16 = 1;

/// Bounds on an untrusted image (docs/37). Each is above anything a real
/// workbook produces and below anything that could exhaust memory.
pub(crate) const MAX_AXIS_ENTRIES: usize = 1 << 26;
pub(crate) const MAX_TILES: usize = 1 << 22;
const MAX_FORMULAS: usize = 1 << 24;
const MAX_LOSERS: usize = 1 << 16;
const MAX_TEXT_BYTES: usize = 1 << 26;

/// Why a byte string is not an image this build can load. Errors are values
/// (DP-A10); nothing here panics on any input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageError {
    NotAnImage,
    /// Written by a build with a different image format. The caller falls back
    /// to replaying ops, which is why this is recoverable rather than fatal.
    UnsupportedVersion(u16),
    Truncated,
    /// A count or tag outside its defined range.
    Malformed(&'static str),
    /// A length that would allocate more than the bound allows.
    CapExceeded(&'static str),
}

// ------------------------------------------------------------------ writing

pub(crate) struct Writer {
    pub(crate) out: Vec<u8>,
    /// Byte ranges of each tile, in write order, for [`State::write_image_parts`].
    pub(crate) tiles: Vec<(TileKey, usize, usize)>,
}

impl Writer {
    pub(crate) fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    pub(crate) fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i128(&mut self, v: i128) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i16(&mut self, v: i16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn len(&mut self, v: usize) {
        self.u32(v as u32);
    }
    pub(crate) fn opid(&mut self, id: &OpId) {
        self.out.extend_from_slice(&id.actor.0.to_le_bytes());
        self.u64(id.counter);
    }
    pub(crate) fn text(&mut self, s: &str) {
        self.len(s.len());
        self.out.extend_from_slice(s.as_bytes());
    }

    /// A cell value, in the **canonical** encoding `usk-types` already defines.
    /// Reusing it means an image and an op cannot disagree about what a value
    /// is, which is the only way the state hash can survive the round trip.
    pub(crate) fn value(&mut self, value: &Value) {
        value.encode_into(&mut self.out);
    }

    pub(crate) fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
}

impl State {
    /// Serialises this state as a self-contained tile image.
    pub fn write_image(&self) -> Vec<u8> {
        self.write_image_parts().0
    }

    /// The image, plus the byte range of each tile inside it.
    ///
    /// A container content-addresses those ranges into `blobs` so that
    /// retained snapshots share every tile they have in common — docs/16's
    /// "structurally shared via tile Merkle identity", and the reason three
    /// snapshots need not cost three copies.
    pub fn write_image_parts(&self) -> (Vec<u8>, Vec<(TileKey, usize, usize)>) {
        let mut w = Writer {
            out: Vec::with_capacity(1 << 16),
            tiles: Vec::new(),
        };
        w.out.extend_from_slice(MAGIC);
        w.u16(IMAGE_VERSION);

        write_axis(&mut w, &self.rows);
        write_axis(&mut w, &self.cols);
        self.cells.write_image(&mut w);
        write_formulas(&mut w, &self.formulas);

        let tiles = core::mem::take(&mut w.tiles);
        (w.out, tiles)
    }

    /// Rebuilds a state from an image.
    ///
    /// **Not a mutation path** (DP-A1): the result is trusted only once its
    /// `state_hash()` matches what the snapshot recorded, which is
    /// `Snapshot::verify`'s job and the only way a caller obtains one.
    pub fn from_image(bytes: &[u8]) -> Result<State, ImageError> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.take(MAGIC.len())? != MAGIC {
            return Err(ImageError::NotAnImage);
        }
        let version = r.u16()?;
        if version != IMAGE_VERSION {
            return Err(ImageError::UnsupportedVersion(version));
        }
        let rows = read_axis(&mut r)?;
        let cols = read_axis(&mut r)?;
        let cells = TileStore::read_image(&mut r)?;
        let formulas = read_formulas(&mut r)?;
        if r.at != bytes.len() {
            return Err(ImageError::Malformed("trailing bytes after the image"));
        }
        Ok(State {
            rows,
            cols,
            cells,
            formulas,
        })
    }
}

fn write_axis(w: &mut Writer, axis: &AxisSeq) {
    // The insertion *tree*, not the flattened order: a restored workbook keeps
    // being edited, and a later `InsertRow` anchors to an existing id. A
    // flattened order would load and then resolve the next concurrent insert
    // differently from a replica that never restarted.
    w.len(axis.children.len());
    for (anchor, kids) in &axis.children {
        match anchor {
            None => w.u8(0),
            Some(id) => {
                w.u8(1);
                w.opid(id);
            }
        }
        w.len(kids.len());
        for (lamport, id) in kids {
            w.u64(*lamport);
            w.opid(id);
        }
    }
    w.len(axis.tombstones.len());
    for id in axis.tombstones.keys() {
        w.opid(id);
    }
}

fn read_axis(r: &mut Reader) -> Result<AxisSeq, ImageError> {
    let mut axis = AxisSeq::default();
    let groups = r.count(MAX_AXIS_ENTRIES, "axis groups")?;
    for _ in 0..groups {
        let anchor = match r.u8()? {
            0 => None,
            1 => Some(r.opid()?),
            _ => return Err(ImageError::Malformed("axis anchor tag")),
        };
        let n = r.count(MAX_AXIS_ENTRIES, "axis children")?;
        let mut kids = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let lamport = r.u64()?;
            kids.push((lamport, r.opid()?));
        }
        axis.children.insert(anchor, kids);
    }
    let n = r.count(MAX_AXIS_ENTRIES, "tombstones")?;
    for _ in 0..n {
        let id = r.opid()?;
        axis.tombstones.insert(id, ());
    }
    Ok(axis)
}

fn write_formulas(w: &mut Writer, registry: &FormulaRegistry) {
    let entries = registry.image_entries();
    w.len(entries.len());
    for (row, col, stamp, formula) in entries {
        w.opid(&row.0);
        w.opid(&col.0);
        w.u64(stamp.0);
        w.opid(&stamp.1);
        match formula {
            None => w.u8(0),
            Some(f) => {
                w.u8(1);
                w.text(&f.source);
                w.len(f.bindings.len());
                for b in &f.bindings {
                    w.opid(&b.row_start);
                    w.opid(&b.row_end);
                    w.opid(&b.col_start);
                    w.opid(&b.col_end);
                    w.u8(b.anchors);
                }
            }
        }
    }
}

fn read_formulas(r: &mut Reader) -> Result<FormulaRegistry, ImageError> {
    let n = r.count(MAX_FORMULAS, "formula entries")?;
    let mut entries = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let row = RowId(r.opid()?);
        let col = ColId(r.opid()?);
        let lamport = r.u64()?;
        let id = r.opid()?;
        let formula = match r.u8()? {
            0 => None,
            1 => {
                let source = r.text(MAX_TEXT_BYTES)?;
                let count = r.count(MAX_FORMULAS, "bindings")?;
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
                Some(FormulaCell { source, bindings })
            }
            _ => return Err(ImageError::Malformed("formula presence tag")),
        };
        entries.push((row, col, (lamport, id), formula));
    }
    Ok(FormulaRegistry::from_image(entries))
}

// ------------------------------------------------------------------ reading

pub(crate) struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ImageError> {
        let end = self.at.checked_add(n).ok_or(ImageError::Truncated)?;
        let slice = self.b.get(self.at..end).ok_or(ImageError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ImageError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, ImageError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, ImageError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, ImageError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    pub(crate) fn i16(&mut self) -> Result<i16, ImageError> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn i128(&mut self) -> Result<i128, ImageError> {
        let b = self.take(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(b);
        Ok(i128::from_le_bytes(a))
    }

    fn u32_be(&mut self) -> Result<u32, ImageError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64_be(&mut self) -> Result<u64, ImageError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    fn i16_be(&mut self) -> Result<i16, ImageError> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    fn i128_be(&mut self) -> Result<i128, ImageError> {
        let b = self.take(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(b);
        Ok(i128::from_be_bytes(a))
    }

    pub(crate) fn opid(&mut self) -> Result<OpId, ImageError> {
        let b = self.take(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(b);
        Ok(OpId {
            actor: ActorId(u128::from_le_bytes(a)),
            counter: self.u64()?,
        })
    }

    /// A length, checked against its bound *before* it is used to reserve.
    /// The whole point: a hostile image must not be able to ask for memory.
    pub(crate) fn count(&mut self, cap: usize, what: &'static str) -> Result<usize, ImageError> {
        let n = self.u32()? as usize;
        if n > cap {
            return Err(ImageError::CapExceeded(what));
        }
        Ok(n)
    }

    pub(crate) fn text(&mut self, cap: usize) -> Result<String, ImageError> {
        let n = self.count(cap, "text")?;
        let b = self.take(n)?;
        core::str::from_utf8(b)
            .map(String::from)
            .map_err(|_| ImageError::Malformed("text is not UTF-8"))
    }

    /// A cell value, in the **canonical** encoding.
    ///
    /// Big-endian, unlike everything else in this format, because it is not
    /// this format's encoding: `Value::encode_into` defines it and ops use it
    /// too. Reusing it means a value has one byte string everywhere (DP-A4) —
    /// and the mismatch between the two conventions is exactly the bug the
    /// round-trip test caught on its first run, which is the argument for
    /// having one encoding rather than two that look alike.
    pub(crate) fn value(&mut self) -> Result<Value, ImageError> {
        match self.u8()? {
            0 => Ok(Value::Blank),
            1 => Ok(Value::Bool(false)),
            2 => Ok(Value::Bool(true)),
            3 => Ok(Value::Number(f64::from_bits(self.u64_be()?))),
            4 => {
                let n = self.u32_be()? as usize;
                if n > MAX_TEXT_BYTES {
                    return Err(ImageError::CapExceeded("text"));
                }
                let b = self.take(n)?;
                core::str::from_utf8(b)
                    .map(|s| Value::Text(String::from(s)))
                    .map_err(|_| ImageError::Malformed("text is not UTF-8"))
            }
            5 => {
                let kind = match self.u8()? {
                    0 => usk_types::ErrorKind::Div0,
                    1 => usk_types::ErrorKind::Value,
                    2 => usk_types::ErrorKind::Ref,
                    3 => usk_types::ErrorKind::Name,
                    4 => usk_types::ErrorKind::Num,
                    5 => usk_types::ErrorKind::Na,
                    6 => usk_types::ErrorKind::Circ,
                    7 => usk_types::ErrorKind::Spill,
                    _ => return Err(ImageError::Malformed("error kind")),
                };
                let origin = match self.u8()? {
                    0 => usk_types::Origin::Authored,
                    1 => usk_types::Origin::Coercion {
                        from: self.type_tag()?,
                        to: self.type_tag()?,
                    },
                    2 => usk_types::Origin::Arithmetic {
                        op: match self.u8()? {
                            0 => usk_types::ArithOp::Add,
                            1 => usk_types::ArithOp::Sub,
                            2 => usk_types::ArithOp::Mul,
                            3 => usk_types::ArithOp::Div,
                            _ => return Err(ImageError::Malformed("arith op")),
                        },
                    },
                    3 => usk_types::Origin::Propagated,
                    _ => return Err(ImageError::Malformed("origin")),
                };
                Ok(Value::Error(usk_types::CellError::new(kind, origin)))
            }
            6 => {
                let coefficient = self.i128_be()?;
                let exponent = self.i16_be()?;
                // `new` canonicalises. An image is written by us, so this is a
                // no-op on our own bytes — and a *repair* on anyone else's,
                // which is what stops a second spelling of one value entering
                // the state hash (DP-A4).
                Ok(Value::Decimal(Decimal::new(coefficient, exponent)))
            }
            _ => Err(ImageError::Malformed("value tag")),
        }
    }

    fn type_tag(&mut self) -> Result<usk_types::TypeTag, ImageError> {
        use usk_types::TypeTag::*;
        Ok(match self.u8()? {
            0 => Blank,
            1 => Bool,
            2 => Number,
            3 => Decimal,
            4 => Text,
            5 => Error,
            _ => return Err(ImageError::Malformed("type tag")),
        })
    }
}

/// The BLAKE3 hash of each tile chunk in an image.
///
/// Two snapshots of a workbook that differed in one tile produce identical
/// hashes for every other tile, which is what makes retaining three snapshots
/// cost O(dirty) rather than 3x (docs/16's "structurally shared via tile
/// Merkle identity").
pub fn chunk_hashes(
    image: &[u8],
    tiles: &[(TileKey, usize, usize)],
) -> Vec<((u32, u32), [u8; 32])> {
    tiles
        .iter()
        .map(|(key, start, end)| {
            (
                (key.row_band, key.col_band),
                *blake3::hash(&image[*start..*end]).as_bytes(),
            )
        })
        .collect()
}

/// Bounds shared with the tile module.
pub(crate) const MAX_LOSERS_PER_CELL: usize = MAX_LOSERS;
