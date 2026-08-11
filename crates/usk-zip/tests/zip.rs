//! ZIP and DEFLATE against real archives and hostile ones (docs/24).
//!
//! The corpus archives are produced by Python's `zipfile`, i.e. by an
//! implementation that is not ours — which is the point. A decompressor tested
//! only against its own compressor proves that two bugs agree.

use usk_zip::inflate::{inflate, InflateError, MAX_OUTPUT};
use usk_zip::{crc32, Archive, ZipError, MAX_RATIO};

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("corpus {name}: {e}"))
}

// --------------------------------------------------------------- the archive

/// The end-to-end shape: read the central directory, decompress each entry,
/// and verify every CRC. The archive holds deflated text, a *stored* entry, an
/// empty entry, a nested path and incompressible bytes — five shapes that take
/// different paths through the inflater.
#[test]
fn a_real_archive_decompresses_and_every_crc_verifies() {
    let bytes = corpus("01-mixed.zip");
    let archive = Archive::open(&bytes).expect("opens");
    assert_eq!(archive.entries().len(), 5);

    let hello = archive
        .read_named("hello.txt")
        .expect("present")
        .expect("reads");
    assert_eq!(hello, "hello world\n".repeat(200).into_bytes());

    let stored = archive
        .read_named("stored.bin")
        .expect("present")
        .expect("reads");
    assert_eq!(stored, (0u8..=255).collect::<Vec<u8>>());

    let empty = archive
        .read_named("empty.txt")
        .expect("present")
        .expect("reads");
    assert!(empty.is_empty());

    let sheet = archive
        .read_named("xl/worksheets/sheet1.xml")
        .expect("present")
        .expect("reads");
    assert!(sheet.starts_with(b"<x>"));

    // Incompressible data: the compressor will have emitted stored blocks
    // inside the deflate stream, which is a distinct code path.
    let random = archive
        .read_named("random.bin")
        .expect("present")
        .expect("reads");
    assert_eq!(random.len(), 5000);
    assert_eq!(random[7], ((7 * 7919) % 256) as u8);

    // Every entry, so no path is left unexercised by accident.
    for entry in archive.entries() {
        let data = archive.read(entry).expect("reads");
        assert_eq!(crc32(&data), entry.crc32, "{}", entry.name);
        assert_eq!(data.len(), entry.uncompressed_size, "{}", entry.name);
    }
}

#[test]
fn listing_an_archive_decompresses_nothing() {
    let bytes = corpus("01-mixed.zip");
    let archive = Archive::open(&bytes).expect("opens");
    // The declared total is available before any entry is read, so a caller can
    // size the work — and refuse it — without paying for it first.
    assert!(archive.declared_output() > 0);
    assert!(archive.find("hello.txt").is_some());
    assert!(archive.find("nope.txt").is_none());
}

#[test]
fn a_file_that_is_not_a_zip_is_refused() {
    assert_eq!(Archive::open(b"").err(), Some(ZipError::NotAZip));
    assert_eq!(
        Archive::open(b"not a zip file at all").err(),
        Some(ZipError::NotAZip)
    );
    // A plausible-looking prefix with no EOCD is still not a ZIP.
    let mut fake = b"PK\x03\x04".to_vec();
    fake.extend_from_slice(&[0u8; 64]);
    assert_eq!(Archive::open(&fake).err(), Some(ZipError::NotAZip));
}

/// Truncating a real archive at every byte must produce a named error or a
/// consistent read — never a panic. This is the shape a partial download has.
#[test]
fn every_truncation_of_a_real_archive_is_handled() {
    let bytes = corpus("01-mixed.zip");
    for cut in 0..bytes.len() {
        match Archive::open(&bytes[..cut]) {
            Err(_) => {}
            Ok(archive) => {
                // If it opened, every read must still be total.
                for entry in archive.entries() {
                    let _ = archive.read(entry);
                }
            }
        }
    }
}

/// Corrupting one byte of compressed data must be caught. The CRC is the only
/// end-to-end statement the format makes about the data, which is why it is
/// checked rather than trusted.
#[test]
fn a_corrupted_entry_fails_its_crc_or_its_inflate() {
    let original = corpus("01-mixed.zip");
    let mut caught = 0usize;
    for offset in (40..original.len().min(400)).step_by(7) {
        let mut bytes = original.clone();
        bytes[offset] ^= 0xFF;
        let Ok(archive) = Archive::open(&bytes) else {
            caught += 1;
            continue;
        };
        for entry in archive.entries() {
            match archive.read(entry) {
                Ok(data) => assert_eq!(
                    crc32(&data),
                    entry.crc32,
                    "a successful read must have verified its CRC"
                ),
                Err(_) => caught += 1,
            }
        }
    }
    assert!(caught > 0, "flipping bytes in the data must be noticed");
}

// ------------------------------------------------------------------ the caps

/// docs/24's ratio cap, against a hand-built archive whose central directory
/// claims a 4 GB entry from 1 KB of data — the zip-bomb shape, refused before
/// a single byte is decompressed.
#[test]
fn a_bomb_is_refused_from_the_directory_alone() {
    // Just under 2^32 — `4 << 30` truncates to zero in the u32 field and
    // forges a *harmless* archive, which is how this test first passed while
    // proving nothing.
    let bomb = forge_archive("bomb.bin", 1024, 4_000_000_000);
    match Archive::open(&bomb) {
        Err(ZipError::CapExceeded { cap, .. }) => {
            assert!(
                cap == "MAX_RATIO" || cap == "MAX_OUTPUT" || cap == "MAX_TOTAL_OUTPUT",
                "unexpected cap {cap}"
            );
        }
        other => panic!("expected a cap, got {other:?}", other = other.map(|_| ())),
    }
}

#[test]
fn the_ratio_cap_is_where_docs_24_says() {
    // Just inside: 100:1 exactly.
    let ok = forge_archive("fine.bin", 1024, 1024 * MAX_RATIO as usize);
    assert!(Archive::open(&ok).is_ok());
    // Just outside.
    let bad = forge_archive("bad.bin", 1024, 1024 * MAX_RATIO as usize + 1024);
    assert!(matches!(
        Archive::open(&bad),
        Err(ZipError::CapExceeded {
            cap: "MAX_RATIO",
            ..
        })
    ));
}

/// Nothing here writes to disk, so a traversing name cannot currently be
/// exploited — it is refused so that the day someone adds extraction, the check
/// they forget already exists.
#[test]
fn a_name_that_escapes_the_container_is_refused() {
    for name in ["../evil", "a/../../evil", "/abs", "C:\\evil", "a\\..\\b"] {
        let archive = forge_archive(name, 16, 16);
        assert!(
            matches!(Archive::open(&archive), Err(ZipError::UnsafeName { .. })),
            "{name} was accepted"
        );
    }
    for name in ["xl/worksheets/sheet1.xml", "a..b", "..hidden"] {
        let archive = forge_archive(name, 16, 16);
        assert!(Archive::open(&archive).is_ok(), "{name} was refused");
    }
}

// ------------------------------------------------------------------- inflate

#[test]
fn a_stored_block_round_trips() {
    // BFINAL=1, BTYPE=00, then LEN/NLEN and the raw bytes.
    let mut stream = vec![0x01, 0x05, 0x00, 0xFA, 0xFF];
    stream.extend_from_slice(b"hello");
    assert_eq!(inflate(&stream, 5).expect("inflates"), b"hello");
}

#[test]
fn a_stored_block_with_a_broken_length_check_is_refused() {
    let mut stream = vec![0x01, 0x05, 0x00, 0x00, 0x00];
    stream.extend_from_slice(b"hello");
    assert_eq!(inflate(&stream, 5), Err(InflateError::StoredLengthMismatch));
}

#[test]
fn malformed_streams_are_named_errors_not_panics() {
    assert_eq!(inflate(&[], 0), Err(InflateError::Truncated));
    // BTYPE=11 is reserved.
    assert_eq!(inflate(&[0x07], 0), Err(InflateError::ReservedBlockType));
    // A back-reference before the start of the output.
    // (Fixed Huffman, a length symbol with nothing behind it.)
    let empty_backref = [0x63u8, 0x00, 0x00];
    match inflate(&empty_backref, 0) {
        Ok(_) | Err(_) => {} // either is fine; the requirement is totality
    }
    // Every truncation of a valid stream.
    let mut valid = vec![0x01, 0x05, 0x00, 0xFA, 0xFF];
    valid.extend_from_slice(b"hello");
    for cut in 0..valid.len() {
        assert!(
            inflate(&valid[..cut], 5).is_err(),
            "a truncated stream must not succeed at {cut}"
        );
    }
}

/// Fuzzing the inflater directly: DEFLATE is mostly a bit stream, so random
/// bytes reach deep into the Huffman path where a container-level test never
/// would.
#[test]
fn random_bytes_never_break_the_inflater() {
    let mut seed = 0x1234_5678_9ABC_DEF0u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };
    for _ in 0..20_000 {
        let len = (next() % 64) as usize + 1;
        let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xFF) as u8).collect();
        // An error is a fine outcome — most random bytes are not DEFLATE. What
        // must never happen is a panic, or a success that produced more than
        // the cap.
        if let Ok(out) = inflate(&bytes, 0) {
            assert!(out.len() <= MAX_OUTPUT);
        }
    }
}

/// An **over-subscribed** tree must be refused: more codes than the code space
/// holds means at least two symbols share a code, and a decompressor that
/// accepts one decodes attacker-chosen symbols.
///
/// Hand-assembled bit by bit, because the interesting streams are ones no
/// compressor emits. BFINAL=1, BTYPE=10 (dynamic), HLIT=0, HDIST=0, HCLEN=0
/// (four code-length entries), then all four given length 1 — a code space of
/// 2 asked to hold 4 codes.
#[test]
fn an_over_subscribed_tree_is_refused() {
    let stream = [0x05u8, 0x00, 0x92, 0x04];
    assert_eq!(inflate(&stream, 0), Err(InflateError::BadCodeLengths));
}

/// The exception RFC 1951 requires, pinned so the strictness above is never
/// tightened back over it: a **single-symbol** alphabet is under-subscribed and
/// legal, and it is how a stream with exactly one distance code is encoded.
/// Every ordinary compressed file in the corpus depends on this.
#[test]
fn a_single_code_alphabet_is_accepted_because_real_files_use_it() {
    let bytes = corpus("01-mixed.zip");
    let archive = Archive::open(&bytes).expect("opens");
    let hello = archive
        .read_named("hello.txt")
        .expect("present")
        .expect("a single distance code must inflate");
    assert_eq!(
        hello.len(),
        "hello world
"
        .len()
            * 200
    );
}

// ------------------------------------------------------------------ forging

/// Builds a minimal single-entry archive whose directory *claims* the given
/// sizes. Used to reach the caps without materialising a real bomb — the point
/// is that the caps fire from the directory alone, before anything is read.
fn forge_archive(name: &str, compressed: usize, uncompressed: usize) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut out = Vec::new();

    // Local file header, followed by `compressed` bytes of nothing in
    // particular — the directory is what the caps are read from.
    let local_offset = out.len();
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 2]); // version
    out.extend_from_slice(&[0u8; 2]); // flags
    out.extend_from_slice(&8u16.to_le_bytes()); // method: deflate
    out.extend_from_slice(&[0u8; 4]); // time, date
    out.extend_from_slice(&[0u8; 4]); // crc
    out.extend_from_slice(&(compressed as u32).to_le_bytes());
    out.extend_from_slice(&(uncompressed as u32).to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // extra
    out.extend_from_slice(name_bytes);
    out.extend(core::iter::repeat_n(0u8, compressed));

    let central_offset = out.len();
    out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 2]); // version made by
    out.extend_from_slice(&[0u8; 2]); // version needed
    out.extend_from_slice(&[0u8; 2]); // flags
    out.extend_from_slice(&8u16.to_le_bytes()); // method
    out.extend_from_slice(&[0u8; 4]); // time, date
    out.extend_from_slice(&[0u8; 4]); // crc
    out.extend_from_slice(&(compressed as u32).to_le_bytes());
    out.extend_from_slice(&(uncompressed as u32).to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // extra
    out.extend_from_slice(&0u16.to_le_bytes()); // comment
    out.extend_from_slice(&[0u8; 2]); // disk
    out.extend_from_slice(&[0u8; 2]); // internal attrs
    out.extend_from_slice(&[0u8; 4]); // external attrs
    out.extend_from_slice(&(local_offset as u32).to_le_bytes());
    out.extend_from_slice(name_bytes);
    let central_size = out.len() - central_offset;

    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // disk numbers
    out.extend_from_slice(&1u16.to_le_bytes()); // entries on this disk
    out.extend_from_slice(&1u16.to_le_bytes()); // entries total
    out.extend_from_slice(&(central_size as u32).to_le_bytes());
    out.extend_from_slice(&(central_offset as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

// --------------------------------------------------------------- the writer

/// The writer's output goes back through this crate's own reader — every
/// header field the reader checks, every CRC, byte-for-byte content. Stored
/// entries only (D-121): the round trip is exact.
#[test]
fn a_stored_container_round_trips_through_the_reader() {
    let entries: [(&str, &[u8]); 3] = [
        ("a.txt", b"hello"),
        ("dir/b.bin", &[0u8, 1, 2, 255]),
        ("empty.txt", b""),
    ];
    let bytes = usk_zip::write::build_stored(&entries).expect("writes");
    let archive = Archive::open(&bytes).expect("our own output must open");
    assert_eq!(archive.entries().len(), 3);
    for (name, data) in entries {
        let back = archive
            .read_named(name)
            .expect("present")
            .expect("reads with a verified CRC");
        assert_eq!(back, data, "{name}");
    }
}

/// DP-A2: the same entries are the same bytes — no clock reaches the
/// container, which is why the timestamp is fixed at the DOS epoch.
#[test]
fn the_stored_writer_is_deterministic() {
    let entries: [(&str, &[u8]); 2] = [("x", b"one"), ("y", b"two")];
    assert_eq!(
        usk_zip::write::build_stored(&entries).expect("writes"),
        usk_zip::write::build_stored(&entries).expect("writes")
    );
}

/// The writer enforces the same name rules the reader does: a container this
/// crate would refuse to read must not be produced by it either.
#[test]
fn the_writer_refuses_names_its_own_reader_would() {
    for name in ["../escape", "/absolute", "C:\\drive"] {
        let entries: [(&str, &[u8]); 1] = [(name, b"x")];
        assert!(
            matches!(
                usk_zip::write::build_stored(&entries),
                Err(ZipError::UnsafeName { .. })
            ),
            "{name} was accepted"
        );
    }
}
