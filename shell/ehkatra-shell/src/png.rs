//! A minimal PNG writer, so a rendered frame can be committed as evidence.
//!
//! Hand-written rather than a dependency, and the reason is the budget: the
//! shell's ceiling has ~50 crates of headroom earmarked for accesskit and the
//! platform dialogs (ADR-037), and spending five of them on an image encoder
//! this file replaces in two hundred lines would be spending it on the wrong
//! thing.
//!
//! DEFLATE with **fixed Huffman codes** and a deliberately narrow matcher: it
//! looks for repeats at distance 1 (a run of one byte) and at distance
//! `stride` (this row equals the one above). Those two cover nearly everything
//! a flat-shaded grid produces, and they are the difference between a 4 MB
//! screenshot and a committable one — which matters when `demo/` gains a frame
//! per feature for a quarter. The first version emitted stored blocks and
//! produced exactly that 4 MB file.
//!
//! Not a general compressor and not trying to be. A photograph would compress
//! badly here; nothing in `demo/` is a photograph.

/// Encodes RGBA8 rows as a PNG.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    debug_assert_eq!(rgba.len(), (width * height * 4) as usize);

    // Each scanline is prefixed with a filter byte; 0 = None. Keeping the
    // filter trivial means a row identical to the one above is *byte*
    // identical, which is precisely what the stride matcher below looks for.
    let stride = width as usize * 4 + 1;
    let mut raw = Vec::with_capacity(stride * height as usize);
    for row in 0..height as usize {
        raw.push(0);
        let start = row * width as usize * 4;
        raw.extend_from_slice(&rgba[start..start + width as usize * 4]);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, no filter, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_deflate(&raw, stride));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let mut crc_input = Vec::with_capacity(4 + body.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(body);
    // PNG's CRC-32 is ZIP's, and `usk-zip` already has a tested one.
    out.extend_from_slice(&usk_zip::crc32(&crc_input).to_be_bytes());
}

/// A zlib stream carrying one fixed-Huffman DEFLATE block.
fn zlib_deflate(data: &[u8], stride: usize) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // deflate, 32K window, no preset dictionary
    let mut bits = BitWriter::default();
    bits.push(1, 1); // BFINAL
    bits.push(1, 2); // BTYPE = 01, fixed Huffman

    let mut at = 0usize;
    while at < data.len() {
        // Two candidate back-references, which between them describe a flat
        // fill and a repeated scanline: the byte before, and the row above.
        let mut best = (0usize, 0usize); // (length, distance)
        for dist in [1usize, stride] {
            if dist == 0 || dist > at {
                continue;
            }
            let mut len = 0usize;
            while len < 258 && at + len < data.len() && data[at + len] == data[at + len - dist] {
                len += 1;
            }
            if len >= 3 && len > best.0 {
                best = (len, dist);
            }
        }
        if best.0 >= 3 {
            let (len, dist) = best;
            let (code, extra, base) = length_code(len);
            fixed_literal(&mut bits, code);
            bits.push((len - base) as u32, extra);
            let (dcode, dextra, dbase) = distance_code(dist);
            // Fixed-Huffman distance codes are 5 bits, MSB-first.
            bits.push_rev(dcode as u32, 5);
            bits.push((dist - dbase) as u32, dextra);
            at += len;
        } else {
            fixed_literal(&mut bits, data[at] as u16);
            at += 1;
        }
    }
    fixed_literal(&mut bits, 256); // end of block
    out.extend_from_slice(&bits.finish());
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Fixed-Huffman literal/length code, MSB-first as DEFLATE specifies for
/// Huffman codes. The extra bits that follow are LSB-first, which is the part
/// that catches everyone — hence two separate methods on the writer.
fn fixed_literal(bits: &mut BitWriter, sym: u16) {
    match sym {
        0..=143 => bits.push_rev(0x30 + sym as u32, 8),
        144..=255 => bits.push_rev(0x190 + sym as u32 - 144, 9),
        256..=279 => bits.push_rev(sym as u32 - 256, 7),
        _ => bits.push_rev(0xc0 + sym as u32 - 280, 8),
    }
}

/// `(code, extra bits, base length)` for a match length.
fn length_code(len: usize) -> (u16, u32, usize) {
    const BASES: [usize; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const EXTRA: [u32; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    let mut i = 0;
    while i + 1 < BASES.len() && BASES[i + 1] <= len {
        i += 1;
    }
    (257 + i as u16, EXTRA[i], BASES[i])
}

/// `(code, extra bits, base distance)` for a back-reference distance.
fn distance_code(dist: usize) -> (u16, u32, usize) {
    const BASES: [usize; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const EXTRA: [u32; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    let mut i = 0;
    while i + 1 < BASES.len() && BASES[i + 1] <= dist {
        i += 1;
    }
    (i as u16, EXTRA[i], BASES[i])
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// DEFLATE's bit order: the stream is LSB-first, but Huffman codes are packed
/// from their most significant bit. Getting that backwards produces a stream
/// that looks plausible and inflates to garbage, so the two orders are separate
/// methods rather than one method with a flag.
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    /// Value bits, LSB-first: extra bits for lengths and distances.
    fn push(&mut self, value: u32, bits: u32) {
        for i in 0..bits {
            self.bit((value >> i) & 1);
        }
    }

    /// Huffman code bits, MSB-first.
    fn push_rev(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.bit((value >> i) & 1);
        }
    }

    fn bit(&mut self, b: u32) {
        self.acc |= b << self.n;
        self.n += 1;
        if self.n == 8 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler_matches_the_standard_check_vector() {
        assert_eq!(adler32(b"123456789"), 0x091e_01de);
    }

    #[test]
    fn a_png_has_the_signature_and_the_three_chunks_in_order() {
        let png = encode_rgba(2, 2, &[255u8; 16]);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        let body = &png[8..];
        assert_eq!(&body[4..8], b"IHDR");
        let ihdr_len = u32::from_be_bytes(body[..4].try_into().unwrap()) as usize;
        let after_ihdr = &body[8 + ihdr_len + 4..];
        assert_eq!(&after_ihdr[4..8], b"IDAT");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    /// The compressed stream must **inflate back to the input**, checked with
    /// the project's own DEFLATE decoder rather than by inspection. A
    /// hand-written compressor that emits a plausible-looking stream no decoder
    /// accepts is the obvious failure here, and only a round trip catches it.
    #[test]
    fn the_deflate_stream_round_trips_through_the_real_inflater() {
        let mut two_rows: Vec<u8> = (0..64u8).collect();
        let row = two_rows.clone();
        two_rows.extend_from_slice(&row);
        two_rows.extend_from_slice(&row);

        let cases: Vec<Vec<u8>> = vec![
            vec![0u8; 1],
            vec![9u8; 300],        // one long run, exercising the length table
            (0..=255u8).collect(), // no repeats at all: pure literals
            two_rows,              // the stride case
        ];
        for data in cases {
            let z = zlib_deflate(&data, 64);
            assert_eq!(&z[..2], &[0x78, 0x01]);
            // Between the 2-byte zlib header and the 4-byte Adler trailer.
            let raw = &z[2..z.len() - 4];
            let back =
                usk_zip::inflate::inflate(raw, data.len()).expect("our own inflater accepts it");
            assert_eq!(back, data, "round trip failed for {} bytes", data.len());
            assert_eq!(
                u32::from_be_bytes(z[z.len() - 4..].try_into().unwrap()),
                adler32(&data)
            );
        }
    }

    /// The point of compressing at all: a flat-shaded frame must not cost
    /// megabytes, because `demo/` gains one per feature.
    #[test]
    fn a_flat_image_compresses_by_orders_of_magnitude() {
        let (w, h) = (256u32, 256u32);
        let rgba = vec![0xEEu8; (w * h * 4) as usize];
        let png = encode_rgba(w, h, &rgba);
        assert!(
            png.len() < rgba.len() / 50,
            "a flat image encoded to {} bytes from {}",
            png.len(),
            rgba.len()
        );
    }
}
