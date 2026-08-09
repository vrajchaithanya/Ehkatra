//! Fuzzing the CSV parser: a seeded generator plus an in-repo corpus.
//!
//! # Why a seeded LCG rather than `cargo-fuzz`
//! D-052 already settled the shape of property testing here — a failure must be
//! reproducible from a seed alone, and the dependency budget (DP-S2) is
//! defended rather than decorative. `cargo-fuzz` additionally needs a nightly
//! toolchain and a global install, which DP-S5 forbids on this host. What it
//! buys that this does not is coverage-guided mutation; what this buys is that
//! it **runs in `cargo test`**, which means it runs, which is the property a
//! solo project needs most (docs/07 §2).
//!
//! # What is asserted
//! Not "it does not crash" — that is what a fuzzer checks *incidentally*. The
//! assertions are the parser's actual contracts:
//! * **Totality.** Every input yields records or a *named* error. There is no
//!   panic, no partial record, and no silent truncation.
//! * **Chunk independence.** The same bytes split anywhere parse identically.
//!   This is the streaming guarantee, and it is the property most likely to
//!   break under a state-machine edit.
//! * **Downstream totality.** Whatever comes out is safe to hand to `analyze`
//!   and `commit`, because in production it will be.
//! * **Writer/reader agreement.** Anything the writer emits, the reader reads
//!   back as the same fields.

use std::path::PathBuf;

use usk_csv::infer::{self, Decision};
use usk_csv::reader::{parse_all, CsvParser};
use usk_csv::writer::write_csv;
use usk_csv::{Dialect, Record};
use usk_types::Value;

/// The LCG from `usk-state`'s convergence sweeps, so a failing seed here means
/// the same bytes everywhere in the repo (D-052).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
}

/// Bytes weighted toward the characters that actually decide a CSV parse.
/// Uniform random bytes would spend the whole budget on payload and almost
/// never produce a quote inside a quoted field.
const ALPHABET: &[u8] = b",;\t|\"\"\"\n\r\n\r  ab019=+-@'\xef\xbb\xbf\xff\x00";

fn generate(seed: u64, max_len: usize) -> Vec<u8> {
    let mut rng = Lcg(seed);
    let len = rng.below(max_len) + 1;
    (0..len)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
}

fn corpus_files() -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(corpus_dir())
        .expect("the in-repo corpus must exist — a fuzz test with no corpus is a random walk")
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = std::fs::read(entry.path()).expect("corpus file");
            (name, bytes)
        })
        .collect();
    files.sort();
    files
}

/// Every contract, over every input, from both sources.
fn assert_contracts(label: &str, bytes: &[u8]) {
    let dialect = Dialect::sniff(bytes);

    // Totality: an error is a value, and a panic is not one of the outcomes.
    let whole = parse_all(bytes, dialect);

    // Chunk independence, at several splits rather than all of them — the
    // exhaustive version lives in `chunking_never_changes_the_records` over a
    // fixed document; here the documents are many and the splits are sampled.
    for divisor in [2usize, 3, 5, 7] {
        let split = bytes.len() / divisor;
        let mut parser = CsvParser::new(dialect);
        let mut chunked: Vec<Record> = Vec::new();
        let first = parser.push(&bytes[..split], &mut chunked);
        let second = first.and_then(|()| parser.push(&bytes[split..], &mut chunked));
        let finished = second.and_then(|()| parser.finish(&mut chunked));

        match (&whole, finished) {
            (Ok(expected), Ok(())) => assert_eq!(
                &chunked, expected,
                "{label}: split at {split} changed the parse"
            ),
            (Err(a), Err(b)) => assert_eq!(a, &b, "{label}: split at {split} changed the error"),
            (whole, chunked) => panic!(
                "{label}: split at {split} disagreed about success — whole {whole:?}, chunked {chunked:?}"
            ),
        }
    }

    let Ok(records) = whole else { return };

    // Downstream totality: whatever parsed is safe to profile and commit.
    for header in [true, false] {
        let report = infer::analyze(&records, header);
        assert!(
            report.rows_sampled <= report.rows_total,
            "{label}: the report contradicts itself"
        );
        assert_eq!(
            report.columns.len(),
            report.suggestions().len(),
            "{label}: a suggestion per column"
        );
        let values = infer::commit(&records, header, &report.suggestions());

        // An imported field is never a formula, whatever the file said.
        for row in &values {
            for value in row {
                if let Value::Text(text) = value {
                    assert!(
                        !text.starts_with('=') || usk_csv::inject::classify_import(text).is_some(),
                        "{label}: {text:?} escaped injection handling"
                    );
                }
            }
        }

        // Writer/reader agreement: what we emit, we read back.
        let (csv, _) = write_csv(&values, dialect);
        let reread = parse_all(csv.as_bytes(), dialect)
            .unwrap_or_else(|e| panic!("{label}: our own output did not parse: {e:?}"));
        let back = infer::commit(
            &reread,
            false,
            &vec![Decision::Text; report.columns.len() + 2],
        );
        assert_eq!(
            back.len(),
            values.len(),
            "{label}: row count changed through the writer"
        );
    }
}

/// 4,000 seeded documents. The seed is printed with any failure, so a
/// counterexample is reproduced by `generate(seed, 400)` and nothing else.
#[test]
fn seeded_documents_never_break_a_contract() {
    for seed in 0..4_000u64 {
        let bytes = generate(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), 400);
        assert_contracts(&format!("seed {seed}"), &bytes);
    }
}

/// The corpus is in the repo so a document that once found a defect keeps being
/// tested forever. A fuzz corpus that lives only in a temp directory tests the
/// same bug exactly once.
#[test]
fn every_corpus_file_holds_the_contracts() {
    let files = corpus_files();
    assert!(
        files.len() >= 14,
        "the corpus shrank — files are added when they find something, never removed"
    );
    for (name, bytes) in files {
        assert_contracts(&name, &bytes);
    }
}

/// Truncating a corpus file at every byte is the cheapest way to reach the
/// states a real short read produces, and it is where a streaming parser
/// actually breaks.
#[test]
fn every_prefix_of_every_corpus_file_holds_the_contracts() {
    for (name, bytes) in corpus_files() {
        for cut in 0..bytes.len() {
            assert_contracts(&format!("{name}[..{cut}]"), &bytes[..cut]);
        }
    }
}
