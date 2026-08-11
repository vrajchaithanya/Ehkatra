//! usk-zip — a read-only ZIP reader and DEFLATE decompressor (docs/24).
//!
//! > *Zip: streaming, entry/size/ratio caps (100:1).*
//!
//! `no_std + alloc`, no dependencies, no I/O. XLSX is a ZIP container, so this
//! is the floor Row 12's second half stands on; it is a separate crate from the
//! XLSX reader because a container format and a document format are different
//! things to be wrong about, and DP-C4 says do not stack two unverified layers.
//!
//! # What this deliberately does not do
//! **It does not compress.** The [`write`] module (session 29, D-121) produces
//! containers with STORED entries only — writing was out of v0.1's scope while
//! reading was the untested security surface, and now that a writer exists it
//! deliberately contains no DEFLATE compressor: nothing hostile flows through
//! a writer, so a second codec would be complexity without a threat. The size
//! cost is measured in MEASUREMENTS.md (W-XLSX-WRITE) and the compressor is
//! filed as TD-72.
//!
//! **It does not decrypt.** An encrypted entry is reported as unsupported
//! rather than skipped, because "we silently dropped a sheet" is exactly the
//! partial restore docs/16 forbids at the other boundary.
//!
//! # The caps, and why each one exists
//! A ZIP is the classic amplification attack: the 42 KB `42.zip` expands to
//! 4.5 PB. Three independent limits stop it, and no single one is sufficient —
//! a bomb can be one huge entry, ten thousand small ones, or a modest entry
//! with an absurd ratio.

#![no_std]
extern crate alloc;

pub mod inflate;
pub mod write;

use alloc::string::String;
use alloc::vec::Vec;

use inflate::{inflate, InflateError};

/// docs/24's stated ratio cap. Legitimate XML compresses well — 20:1 is
/// ordinary for a worksheet — so the cap is set where a real file cannot reach
/// but a bomb must.
pub const MAX_RATIO: u64 = 100;
/// Entries in one container. An XLSX with more parts than this is not a
/// spreadsheet.
pub const MAX_ENTRIES: usize = 4096;
/// Uncompressed bytes across the whole container.
pub const MAX_TOTAL_OUTPUT: usize = 1 << 30;
/// The longest entry name accepted, in bytes.
pub const MAX_NAME_BYTES: usize = 512;

pub(crate) const EOCD_SIGNATURE: u32 = 0x0605_4b50;
pub(crate) const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
pub(crate) const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const EOCD_MIN_LEN: usize = 22;

/// Why a byte string is not a ZIP we will read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ZipError {
    /// No end-of-central-directory record. Not a ZIP at all.
    NotAZip,
    Truncated,
    /// A structure whose fields point outside the file.
    Malformed(&'static str),
    /// A compression method other than store or deflate.
    UnsupportedMethod {
        name: String,
        method: u16,
    },
    /// The entry is encrypted. Reported, never skipped.
    Encrypted {
        name: String,
    },
    /// A cap was hit. Carries which one, because "the file was rejected" is not
    /// something a user can act on.
    CapExceeded {
        cap: &'static str,
        name: String,
    },
    /// An entry name that escapes the container — `../` or an absolute path.
    /// Harmless while nothing is extracted to disk, and refused anyway: this
    /// crate should not be the reason a future extractor is a vulnerability.
    UnsafeName {
        name: String,
    },
    Inflate {
        name: String,
        err: InflateError,
    },
    /// The entry's data did not match the CRC-32 the container recorded.
    CrcMismatch {
        name: String,
    },
}

/// One entry's directory record. Reading the *data* is a separate, explicit
/// step, so a caller can list a container without decompressing it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub crc32: u32,
    pub compressed_size: usize,
    pub uncompressed_size: usize,
    /// Offset of the local file header.
    pub local_header_offset: usize,
}

impl Entry {
    pub fn is_directory(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// A parsed central directory. Holds no decompressed data.
pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
    total_output: usize,
}

impl<'a> Archive<'a> {
    /// Reads the central directory and applies the container-level caps.
    ///
    /// The central directory is authoritative here, not the local headers: a
    /// ZIP with disagreeing copies of an entry's metadata is a known ambiguity
    /// attack, and picking one and saying so is the fix.
    pub fn open(bytes: &'a [u8]) -> Result<Archive<'a>, ZipError> {
        let eocd = find_eocd(bytes)?;
        let mut r = Cursor::new(bytes, eocd + 10);
        let entry_count = r.u16()? as usize;
        let _central_size = r.u32()? as usize;
        let central_offset = r.u32()? as usize;

        if entry_count > MAX_ENTRIES {
            return Err(ZipError::CapExceeded {
                cap: "MAX_ENTRIES",
                name: String::new(),
            });
        }

        let mut entries = Vec::with_capacity(entry_count.min(64));
        let mut total_output = 0usize;
        let mut at = central_offset;
        for _ in 0..entry_count {
            let (entry, next) = read_central_header(bytes, at)?;
            check_caps(&entry, &mut total_output)?;
            entries.push(entry);
            at = next;
        }

        Ok(Archive {
            bytes,
            entries,
            total_output,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Total uncompressed bytes the container *claims*. Checked against the
    /// caps at open time, so a caller can size its work before decompressing.
    pub fn declared_output(&self) -> usize {
        self.total_output
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Decompresses one entry and verifies its CRC-32.
    ///
    /// The CRC is checked because it is free and because it is the only
    /// end-to-end statement the format makes about the data — an inflater bug
    /// or a corrupt stream that happens to decode is caught here and nowhere
    /// else.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, ZipError> {
        let data = self.raw(entry)?;
        let out = match entry.method {
            0 => data.to_vec(),
            8 => inflate(data, entry.uncompressed_size).map_err(|err| ZipError::Inflate {
                name: entry.name.clone(),
                err,
            })?,
            method => {
                return Err(ZipError::UnsupportedMethod {
                    name: entry.name.clone(),
                    method,
                })
            }
        };
        if crc32(&out) != entry.crc32 {
            return Err(ZipError::CrcMismatch {
                name: entry.name.clone(),
            });
        }
        Ok(out)
    }

    /// Convenience: read by name.
    pub fn read_named(&self, name: &str) -> Option<Result<Vec<u8>, ZipError>> {
        self.find(name).map(|entry| self.read(entry))
    }

    /// The entry's compressed bytes, located through its **local** header —
    /// because that is where the data actually is, even though the central
    /// directory is authoritative about what it means.
    fn raw(&self, entry: &Entry) -> Result<&'a [u8], ZipError> {
        let mut r = Cursor::new(self.bytes, entry.local_header_offset);
        if r.u32()? != LOCAL_SIGNATURE {
            return Err(ZipError::Malformed("local header signature"));
        }
        r.skip(2)?; // version needed
        let flags = r.u16()?;
        if flags & 1 != 0 {
            return Err(ZipError::Encrypted {
                name: entry.name.clone(),
            });
        }
        r.skip(2 + 2 + 2 + 4 + 4 + 4)?; // method, time, date, crc, sizes
        let name_len = r.u16()? as usize;
        let extra_len = r.u16()? as usize;
        let start =
            r.at.checked_add(name_len)
                .and_then(|v| v.checked_add(extra_len))
                .ok_or(ZipError::Truncated)?;
        let end = start
            .checked_add(entry.compressed_size)
            .ok_or(ZipError::Truncated)?;
        self.bytes
            .get(start..end)
            .ok_or(ZipError::Malformed("entry data past end of file"))
    }
}

fn check_caps(entry: &Entry, total_output: &mut usize) -> Result<(), ZipError> {
    if entry.name.len() > MAX_NAME_BYTES {
        return Err(ZipError::CapExceeded {
            cap: "MAX_NAME_BYTES",
            name: String::new(),
        });
    }
    if !is_safe_name(&entry.name) {
        return Err(ZipError::UnsafeName {
            name: entry.name.clone(),
        });
    }
    if entry.uncompressed_size > inflate::MAX_OUTPUT {
        return Err(ZipError::CapExceeded {
            cap: "MAX_OUTPUT",
            name: entry.name.clone(),
        });
    }
    // The ratio check needs a floor: a 3-byte entry compressing to 1 byte is a
    // 3:1 ratio and means nothing. Below a few hundred bytes the ratio is noise
    // and the absolute caps are what matter.
    if entry.compressed_size >= 256 {
        let ratio = entry.uncompressed_size as u64 / entry.compressed_size.max(1) as u64;
        if ratio > MAX_RATIO {
            return Err(ZipError::CapExceeded {
                cap: "MAX_RATIO",
                name: entry.name.clone(),
            });
        }
    }
    *total_output = total_output.saturating_add(entry.uncompressed_size);
    if *total_output > MAX_TOTAL_OUTPUT {
        return Err(ZipError::CapExceeded {
            cap: "MAX_TOTAL_OUTPUT",
            name: entry.name.clone(),
        });
    }
    Ok(())
}

/// Refuses names that would escape a directory if anything ever extracted them.
/// `pub(crate)` because the writer enforces the same rule on what it emits.
///
/// Nothing here writes to disk, so this cannot currently be exploited. It is
/// enforced anyway because the day someone adds extraction, the check they
/// forget is this one — and a container format crate is the right place for it
/// to already exist.
pub(crate) fn is_safe_name(name: &str) -> bool {
    if name.starts_with('/') || name.starts_with('\\') {
        return false;
    }
    // A drive letter, e.g. `C:\evil`.
    if name.len() >= 2 && name.as_bytes()[1] == b':' {
        return false;
    }
    !name
        .split(['/', '\\'])
        .any(|segment| segment == ".." || segment.contains('\0'))
}

/// Finds the end-of-central-directory record by scanning backwards.
///
/// Backwards because the record sits at the end and its variable-length comment
/// follows it, so there is no way to seek to it. The scan is bounded by the
/// maximum comment length, which is what stops a hostile file turning this into
/// a whole-file search.
fn find_eocd(bytes: &[u8]) -> Result<usize, ZipError> {
    if bytes.len() < EOCD_MIN_LEN {
        return Err(ZipError::NotAZip);
    }
    let max_comment = 0xFFFF;
    let earliest = bytes.len().saturating_sub(EOCD_MIN_LEN + max_comment);
    let mut at = bytes.len() - EOCD_MIN_LEN;
    loop {
        let signature =
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        if signature == EOCD_SIGNATURE {
            return Ok(at);
        }
        if at == earliest {
            return Err(ZipError::NotAZip);
        }
        at -= 1;
    }
}

fn read_central_header(bytes: &[u8], at: usize) -> Result<(Entry, usize), ZipError> {
    let mut r = Cursor::new(bytes, at);
    if r.u32()? != CENTRAL_SIGNATURE {
        return Err(ZipError::Malformed("central directory signature"));
    }
    r.skip(2 + 2)?; // version made by, version needed
    let flags = r.u16()?;
    let method = r.u16()?;
    r.skip(2 + 2)?; // time, date
    let crc32 = r.u32()?;
    let compressed_size = r.u32()? as usize;
    let uncompressed_size = r.u32()? as usize;
    let name_len = r.u16()? as usize;
    let extra_len = r.u16()? as usize;
    let comment_len = r.u16()? as usize;
    r.skip(2 + 2 + 4)?; // disk number, internal attrs, external attrs
    let local_header_offset = r.u32()? as usize;
    let name_bytes = r.take(name_len)?;
    let name = core::str::from_utf8(name_bytes)
        .map_err(|_| ZipError::Malformed("entry name is not UTF-8"))?;
    let name = String::from(name);

    if flags & 1 != 0 {
        return Err(ZipError::Encrypted { name });
    }
    r.skip(extra_len + comment_len)?;

    Ok((
        Entry {
            name,
            method,
            crc32,
            compressed_size,
            uncompressed_size,
            local_header_offset,
        },
        r.at,
    ))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], at: usize) -> Cursor<'a> {
        Cursor { bytes, at }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ZipError> {
        let end = self.at.checked_add(n).ok_or(ZipError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(ZipError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<(), ZipError> {
        self.take(n).map(|_| ())
    }

    fn u16(&mut self) -> Result<u16, ZipError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, ZipError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// CRC-32 (IEEE), computed bitwise.
///
/// A 256-entry table would be faster; this runs once per entry over data that
/// has already been decompressed, so the table would optimise the wrong thing
/// and add a static to get wrong.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
