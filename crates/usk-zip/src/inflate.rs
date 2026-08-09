//! DEFLATE decompression (RFC 1951), `no_std` and dependency-free.
//!
//! # Why this is written rather than taken
//! XLSX is a ZIP container, so Row 12 cannot read one without an inflater.
//! `flate2` brings either a C library or `miniz_oxide` plus its own tree; the
//! workspace budget stands at 29/40 after `rusqlite` took nineteen slots
//! (D-073), and DP-S1/S2 says one of each *hard* thing. DEFLATE is old,
//! completely specified in one short RFC, and — unlike a compressor — a
//! decompressor's correctness is checkable against any file the world already
//! has. Same reasoning as `usk-json` (D-083).
//!
//! Decoding follows zlib's `puff` reference: canonical Huffman codes decoded
//! bit-by-bit against per-length counts, rather than a lookup table. That is
//! slower and very much easier to be sure of, and this code reads untrusted
//! bytes.
//!
//! # Bounds
//! Every loop here is driven by attacker-controlled data, so every one of them
//! is bounded (docs/37). The output cap in particular is not an optimisation:
//! a 42 KB ZIP that expands to 4.5 PB is a documented, named attack, and
//! `MAX_OUTPUT` plus the caller's ratio check are what stop it.

use alloc::vec;
use alloc::vec::Vec;

/// The largest single stream this will produce. Excel's own limits put a
/// worksheet's XML far below this; a stream that wants more is not a
/// spreadsheet.
pub const MAX_OUTPUT: usize = 512 << 20;

/// Why a byte string is not DEFLATE. Errors are values (DP-A10); nothing here
/// panics on any input, valid or not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InflateError {
    /// The stream ended inside a block.
    Truncated,
    /// `BTYPE = 11`, which RFC 1951 reserves.
    ReservedBlockType,
    /// A stored block whose length and one's-complement disagree — the format's
    /// own integrity check.
    StoredLengthMismatch,
    /// A Huffman code that is not in the tree.
    BadCode,
    /// Code lengths that do not form a valid canonical Huffman tree. Over- or
    /// under-subscribed sets are both rejected; an under-subscribed set decodes
    /// ambiguously, which is how a decompressor gets talked into reading
    /// somebody else's memory.
    BadCodeLengths,
    /// A back-reference pointing before the start of the output.
    DistanceTooFar,
    /// The stream tried to produce more than [`MAX_OUTPUT`].
    OutputTooLarge,
}

const MAX_BITS: usize = 15;

/// Lengths 257..=285: base length and extra bits (RFC 1951 §3.2.5).
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Distances 0..=29: base distance and extra bits.
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// The order code lengths for the code-length alphabet arrive in (§3.2.7).
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct BitReader<'a> {
    bytes: &'a [u8],
    /// Byte position of the next byte to load.
    at: usize,
    /// Bit buffer, LSB-first, as DEFLATE requires.
    buffer: u32,
    count: u32,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> BitReader<'a> {
        BitReader {
            bytes,
            at: 0,
            buffer: 0,
            count: 0,
        }
    }

    fn bits(&mut self, need: u32) -> Result<u32, InflateError> {
        while self.count < need {
            let byte = *self.bytes.get(self.at).ok_or(InflateError::Truncated)?;
            self.at += 1;
            self.buffer |= (byte as u32) << self.count;
            self.count += 8;
        }
        let value = self.buffer & ((1u32 << need) - 1);
        self.buffer >>= need;
        self.count -= need;
        Ok(value)
    }

    fn bit(&mut self) -> Result<u32, InflateError> {
        self.bits(1)
    }

    /// Discards the partial byte, which is how a stored block starts.
    fn align(&mut self) {
        let whole = self.count % 8;
        self.buffer >>= whole;
        self.count -= whole;
    }

    fn take_bytes(&mut self, n: usize) -> Result<&'a [u8], InflateError> {
        // Bytes already pulled into the bit buffer have to be given back before
        // reading raw — the buffered whole bytes are still unconsumed input.
        let buffered = (self.count / 8) as usize;
        let start = self.at - buffered;
        self.buffer = 0;
        self.count = 0;
        let end = start.checked_add(n).ok_or(InflateError::Truncated)?;
        let slice = self.bytes.get(start..end).ok_or(InflateError::Truncated)?;
        self.at = end;
        Ok(slice)
    }
}

/// A canonical Huffman decoding table: how many codes of each length, and the
/// symbols in code order.
struct Huffman {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl Huffman {
    /// Builds from code lengths, refusing anything that is not a valid tree.
    fn new(lengths: &[u8]) -> Result<Huffman, InflateError> {
        Huffman::build(lengths, false)
    }

    /// Builds without the under-subscription check, for the **fixed** tables of
    /// RFC 1951 §3.2.6 only.
    ///
    /// The fixed distance table is 30 codes of 5 bits in a 32-code space — an
    /// incomplete tree by construction, because symbols 30 and 31 "will never
    /// actually occur". It is normative, so it is not the decoder's place to
    /// reject it; a dynamic tree with the same shape *is* rejected, because
    /// there the shape is the stream's choice rather than the format's.
    ///
    /// This distinction is the whole of the first bug this crate had: strict
    /// validation applied to the fixed table made **every ordinary compressed
    /// file** fail to inflate, and the corpus archive caught it on the first
    /// run. A decompressor tested only against hand-built streams would not
    /// have noticed.
    fn fixed(lengths: &[u8]) -> Result<Huffman, InflateError> {
        Huffman::build(lengths, true)
    }

    fn build(lengths: &[u8], allow_incomplete: bool) -> Result<Huffman, InflateError> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &length in lengths {
            if length as usize > MAX_BITS {
                return Err(InflateError::BadCodeLengths);
            }
            counts[length as usize] += 1;
        }
        // A tree of nothing but zero-length codes is legal (an unused alphabet).
        if counts[0] as usize == lengths.len() {
            return Ok(Huffman {
                counts,
                symbols: Vec::new(),
            });
        }

        // Kraft's inequality. `left` going negative means over-subscribed,
        // which is always malformed.
        let mut left = 1i32;
        for &count in counts.iter().take(MAX_BITS + 1).skip(1) {
            left <<= 1;
            left -= count as i32;
            if left < 0 {
                return Err(InflateError::BadCodeLengths);
            }
        }
        // An *under*-subscribed set decodes ambiguously and is refused, with
        // two exceptions RFC 1951 requires: the fixed tables (see
        // `Huffman::fixed`), and a **single-symbol** alphabet, which is how a
        // stream with exactly one distance code is encoded.
        let coded = lengths.len() - counts[0] as usize;
        if left > 0 && coded > 1 && !allow_incomplete {
            return Err(InflateError::BadCodeLengths);
        }

        let mut offsets = [0u16; MAX_BITS + 2];
        for length in 1..=MAX_BITS {
            offsets[length + 1] = offsets[length] + counts[length];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                symbols[offsets[length as usize] as usize] = symbol as u16;
                offsets[length as usize] += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    /// Decodes one symbol, walking one bit at a time (zlib `puff`'s method).
    fn decode(&self, bits: &mut BitReader) -> Result<u16, InflateError> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for length in 1..=MAX_BITS {
            code |= bits.bit()? as i32;
            let count = self.counts[length] as i32;
            if code - count < first {
                let at = (index + (code - first)) as usize;
                return self.symbols.get(at).copied().ok_or(InflateError::BadCode);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(InflateError::BadCode)
    }
}

/// Inflates a raw DEFLATE stream.
///
/// `expected` is the size the container claims, used only to pre-allocate and
/// clamped to [`MAX_OUTPUT`]. It is never trusted as a limit: a lying container
/// must not be able to make this produce a short read that looks successful.
pub fn inflate(bytes: &[u8], expected: usize) -> Result<Vec<u8>, InflateError> {
    let mut bits = BitReader::new(bytes);
    let mut out: Vec<u8> = Vec::with_capacity(expected.min(1 << 20));

    loop {
        let last = bits.bit()?;
        match bits.bits(2)? {
            0 => stored(&mut bits, &mut out)?,
            1 => {
                let (literals, distances) = fixed_tables()?;
                block(&mut bits, &mut out, &literals, &distances)?;
            }
            2 => {
                let (literals, distances) = dynamic_tables(&mut bits)?;
                block(&mut bits, &mut out, &literals, &distances)?;
            }
            _ => return Err(InflateError::ReservedBlockType),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

fn stored(bits: &mut BitReader, out: &mut Vec<u8>) -> Result<(), InflateError> {
    bits.align();
    let header = bits.take_bytes(4)?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return Err(InflateError::StoredLengthMismatch);
    }
    let data = bits.take_bytes(len as usize)?;
    if out.len() + data.len() > MAX_OUTPUT {
        return Err(InflateError::OutputTooLarge);
    }
    out.extend_from_slice(data);
    Ok(())
}

fn fixed_tables() -> Result<(Huffman, Huffman), InflateError> {
    // RFC 1951 §3.2.6, written out rather than derived: the table is normative
    // and a derivation is one off-by-one from silently decoding garbage.
    let mut lengths = [0u8; 288];
    for (symbol, length) in lengths.iter_mut().enumerate() {
        *length = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let literals = Huffman::fixed(&lengths)?;
    let distances = Huffman::fixed(&[5u8; 30])?;
    Ok((literals, distances))
}

fn dynamic_tables(bits: &mut BitReader) -> Result<(Huffman, Huffman), InflateError> {
    let hlit = bits.bits(5)? as usize + 257;
    let hdist = bits.bits(5)? as usize + 1;
    let hclen = bits.bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(InflateError::BadCodeLengths);
    }

    let mut code_lengths = [0u8; 19];
    for &position in CODE_LENGTH_ORDER.iter().take(hclen) {
        code_lengths[position] = bits.bits(3)? as u8;
    }
    let code_length_tree = Huffman::new(&code_lengths)?;

    let mut lengths = vec![0u8; hlit + hdist];
    let mut at = 0usize;
    while at < lengths.len() {
        let symbol = code_length_tree.decode(bits)?;
        match symbol {
            0..=15 => {
                lengths[at] = symbol as u8;
                at += 1;
            }
            16 => {
                // Repeat the previous length. With nothing before it there is
                // no previous length to repeat, and accepting it would read
                // index -1.
                let previous = *lengths
                    .get(at.wrapping_sub(1))
                    .ok_or(InflateError::BadCodeLengths)?;
                let repeat = 3 + bits.bits(2)? as usize;
                fill(&mut lengths, &mut at, previous, repeat)?;
            }
            17 => {
                let repeat = 3 + bits.bits(3)? as usize;
                fill(&mut lengths, &mut at, 0, repeat)?;
            }
            18 => {
                let repeat = 11 + bits.bits(7)? as usize;
                fill(&mut lengths, &mut at, 0, repeat)?;
            }
            _ => return Err(InflateError::BadCodeLengths),
        }
    }

    let literals = Huffman::new(&lengths[..hlit])?;
    let distances = Huffman::new(&lengths[hlit..])?;
    Ok((literals, distances))
}

fn fill(lengths: &mut [u8], at: &mut usize, value: u8, repeat: usize) -> Result<(), InflateError> {
    // A repeat that runs past the table is malformed, not "clamp and carry on":
    // clamping would accept a stream no compressor produces and hand it to the
    // tree builder anyway.
    if *at + repeat > lengths.len() {
        return Err(InflateError::BadCodeLengths);
    }
    for _ in 0..repeat {
        lengths[*at] = value;
        *at += 1;
    }
    Ok(())
}

fn block(
    bits: &mut BitReader,
    out: &mut Vec<u8>,
    literals: &Huffman,
    distances: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = literals.decode(bits)?;
        match symbol {
            0..=255 => {
                if out.len() >= MAX_OUTPUT {
                    return Err(InflateError::OutputTooLarge);
                }
                out.push(symbol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let index = symbol as usize - 257;
                let length =
                    LENGTH_BASE[index] as usize + bits.bits(LENGTH_EXTRA[index] as u32)? as usize;

                let distance_symbol = distances.decode(bits)? as usize;
                if distance_symbol >= DISTANCE_BASE.len() {
                    return Err(InflateError::BadCode);
                }
                let distance = DISTANCE_BASE[distance_symbol] as usize
                    + bits.bits(DISTANCE_EXTRA[distance_symbol] as u32)? as usize;
                if distance > out.len() {
                    return Err(InflateError::DistanceTooFar);
                }
                if out.len() + length > MAX_OUTPUT {
                    return Err(InflateError::OutputTooLarge);
                }
                // Byte at a time on purpose: a run may overlap itself
                // (`distance` smaller than `length` is how DEFLATE encodes a
                // repeated pattern), so a bulk copy would read the source
                // before the copy has written it.
                let start = out.len() - distance;
                for offset in 0..length {
                    let byte = out[start + offset];
                    out.push(byte);
                }
            }
            _ => return Err(InflateError::BadCode),
        }
    }
}
