//! Writing a ZIP container — STORED entries only (D-121, session 29).
//!
//! The reader half of this crate exists because a container format is a thing
//! to be *wrong about*; the writer exists because XLSX write (docs/24 format
//! matrix) needs a container to put parts in. It writes method 0 — no
//! compression — on purpose:
//!
//! * **Zero new dependencies and zero new parser surface.** A DEFLATE
//!   *compressor* is a second nontrivial codec to get wrong, and unlike the
//!   inflater it is not security-critical — nothing hostile flows through a
//!   writer — so it would be complexity spent where the threat is not.
//! * **STORED is fully spec-compliant.** Every ZIP reader ever shipped reads
//!   it, including Excel's.
//! * The cost is size: XML compresses ~5–20:1, so a stored container is that
//!   much larger than Excel would write. Measured and published in
//!   MEASUREMENTS.md (W-XLSX-WRITE) rather than hidden; a deflate writer is
//!   filed as debt (TD-72), triggered by a real file-size complaint.
//!
//! Determinism (DP-A2): the same entries in the same order produce the same
//! bytes. The DOS timestamp is fixed at the epoch ZIP can express
//! (1980-01-01 00:00:00) because a writer that stamps wall-clock time produces
//! a different file every run, and byte-identical output is what lets a
//! round-trip test speak in hashes.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{crc32, ZipError, MAX_ENTRIES, MAX_NAME_BYTES};

/// 1980-01-01, 00:00:00 — the earliest instant a DOS timestamp can name.
const DOS_DATE: u16 = 0x0021; // year 0 (=1980), month 1, day 1
const DOS_TIME: u16 = 0x0000;
/// "Version needed to extract" for stored entries: 1.0.
const VERSION_NEEDED: u16 = 10;
/// "Version made by": 2.0, DOS attribute mapping.
const VERSION_MADE_BY: u16 = 20;

/// Builds a ZIP container holding `entries` as STORED (uncompressed) data, in
/// the order given.
///
/// The names must satisfy the same rules the reader enforces — under
/// [`MAX_NAME_BYTES`], no traversal, UTF-8 (guaranteed by `&str`) — because a
/// writer whose output its own reader refuses is not a writer. Sizes are
/// bounded by the u32 fields of the classic ZIP format; ZIP64 is out of scope
/// for the same reason deflate is (an XLSX part near 4 GB is not a spreadsheet).
pub fn build_stored(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, ZipError> {
    if entries.len() > MAX_ENTRIES {
        return Err(ZipError::CapExceeded {
            cap: "MAX_ENTRIES",
            name: String::new(),
        });
    }

    let mut out = Vec::new();
    let mut central: Vec<(u32, u32, usize)> = Vec::with_capacity(entries.len()); // (crc, offset, len)

    for (name, data) in entries {
        if name.len() > MAX_NAME_BYTES {
            return Err(ZipError::CapExceeded {
                cap: "MAX_NAME_BYTES",
                name: String::from(*name),
            });
        }
        if !crate::is_safe_name(name) {
            return Err(ZipError::UnsafeName {
                name: String::from(*name),
            });
        }
        if data.len() > u32::MAX as usize || out.len() > u32::MAX as usize {
            return Err(ZipError::CapExceeded {
                cap: "u32 field",
                name: String::from(*name),
            });
        }
        let crc = crc32(data);
        let offset = out.len() as u32;
        central.push((crc, offset, data.len()));

        // Local file header.
        put_u32(&mut out, crate::LOCAL_SIGNATURE);
        put_u16(&mut out, VERSION_NEEDED);
        put_u16(&mut out, 0); // flags: not encrypted, sizes known
        put_u16(&mut out, 0); // method 0 = stored
        put_u16(&mut out, DOS_TIME);
        put_u16(&mut out, DOS_DATE);
        put_u32(&mut out, crc);
        put_u32(&mut out, data.len() as u32); // compressed
        put_u32(&mut out, data.len() as u32); // uncompressed
        put_u16(&mut out, name.len() as u16);
        put_u16(&mut out, 0); // extra
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
    }

    let central_offset = out.len();
    for ((name, _), (crc, offset, len)) in entries.iter().zip(&central) {
        put_u32(&mut out, crate::CENTRAL_SIGNATURE);
        put_u16(&mut out, VERSION_MADE_BY);
        put_u16(&mut out, VERSION_NEEDED);
        put_u16(&mut out, 0); // flags
        put_u16(&mut out, 0); // method
        put_u16(&mut out, DOS_TIME);
        put_u16(&mut out, DOS_DATE);
        put_u32(&mut out, *crc);
        put_u32(&mut out, *len as u32);
        put_u32(&mut out, *len as u32);
        put_u16(&mut out, name.len() as u16);
        put_u16(&mut out, 0); // extra
        put_u16(&mut out, 0); // comment
        put_u16(&mut out, 0); // disk number
        put_u16(&mut out, 0); // internal attributes
        put_u32(&mut out, 0); // external attributes
        put_u32(&mut out, *offset);
        out.extend_from_slice(name.as_bytes());
    }
    let central_size = out.len() - central_offset;

    if central_offset > u32::MAX as usize {
        return Err(ZipError::CapExceeded {
            cap: "u32 field",
            name: String::new(),
        });
    }

    // End of central directory.
    put_u32(&mut out, crate::EOCD_SIGNATURE);
    put_u16(&mut out, 0); // this disk
    put_u16(&mut out, 0); // central directory disk
    put_u16(&mut out, entries.len() as u16);
    put_u16(&mut out, entries.len() as u16);
    put_u32(&mut out, central_size as u32);
    put_u32(&mut out, central_offset as u32);
    put_u16(&mut out, 0); // comment length

    Ok(out)
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
