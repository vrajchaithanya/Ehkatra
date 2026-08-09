//! usk-csv — CSV/TSV parsing, type inference and serialization (docs/24,
//! BOOTSTRAP row 12).
//!
//! `no_std + alloc` and **entirely without I/O**: this crate is handed bytes and
//! returns records, a report, or an error. That is not incidental tidiness — it
//! is what lets the whole of it be proven against hostile input without a
//! filesystem, exactly as `usk-sync` is proven without a network and
//! `usk-recover` without a disk. The subprocess confinement docs/24 mandates
//! lives in `ehkatra-io`, above this crate, and cannot be weakened from here
//! because there is nothing here to weaken.
//!
//! # The three rules docs/24 states, and where each is enforced
//! * *"streaming"* — [`reader::CsvParser`] is a push state machine. The caller
//!   chooses the chunk size and drains records as they complete; nothing here
//!   requires the whole document in memory.
//! * *"type-inference **preview before commit** — the gene-name bug is a
//!   surfaced decision, never silent"* — [`infer::analyze`] returns a report and
//!   commits nothing. Producing values requires [`infer::commit`] and an
//!   explicit per-column [`infer::Decision`]. The silent path does not exist to
//!   be taken by accident.
//! * *"formula-injection neutralization on both import and export per OWASP"* —
//!   [`inject`], applied in both directions. Export is the half people forget.
//!
//! # Bounds
//! Every limit here exists because a CSV file is untrusted input (docs/37): a
//! field that never ends and a record with a million columns are both ordinary
//! hostile inputs, and both are refused by name rather than by allocation.

#![no_std]
extern crate alloc;

pub mod infer;
pub mod inject;
pub mod reader;
pub mod writer;

use alloc::string::String;
use alloc::vec::Vec;

/// One parsed record: its fields, and the 1-based line it started on.
///
/// The line number is carried because every report this crate produces has to
/// say *where* — "column 3 has mixed types" is a complaint, "column 3 is text
/// except on line 4,182" is a finding.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub fields: Vec<String>,
    pub line: usize,
}

/// The separator/quoting conventions of one file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dialect {
    pub delimiter: u8,
    pub quote: u8,
}

impl Default for Dialect {
    fn default() -> Self {
        Dialect {
            delimiter: b',',
            quote: b'"',
        }
    }
}

impl Dialect {
    pub fn tsv() -> Dialect {
        Dialect {
            delimiter: b'\t',
            ..Dialect::default()
        }
    }

    /// Guesses the delimiter from a sample.
    ///
    /// The rule is *consistency*, not frequency: the winning delimiter is the
    /// one whose per-line count varies least across the sample, with ties going
    /// to the more common candidate. Counting occurrences alone picks the comma
    /// out of a semicolon-delimited European file the moment one field contains
    /// prose, which is the common case rather than a corner one.
    ///
    /// Counting happens outside quotes, so a comma inside `"Smith, J."` does not
    /// vote.
    pub fn sniff(sample: &[u8]) -> Dialect {
        const CANDIDATES: &[u8] = b",;\t|";
        let mut best = Dialect::default();
        let mut best_score: Option<(usize, usize)> = None; // (variance, -count)

        for &candidate in CANDIDATES {
            let counts = per_line_counts(sample, candidate);
            let lines: usize = counts.len();
            if lines == 0 {
                continue;
            }
            let total: usize = counts.iter().sum();
            if total == 0 {
                continue;
            }
            let mean = total / lines;
            let variance: usize = counts.iter().map(|c| c.abs_diff(mean)).sum();
            let score = (variance, usize::MAX - total);
            if best_score.is_none_or(|b| score < b) {
                best_score = Some(score);
                best = Dialect {
                    delimiter: candidate,
                    quote: b'"',
                };
            }
        }
        best
    }
}

/// Per-line occurrences of `delimiter` outside quotes, over at most 64 lines.
fn per_line_counts(sample: &[u8], delimiter: u8) -> Vec<usize> {
    let mut counts = Vec::new();
    let mut current = 0usize;
    let mut in_quotes = false;
    for &byte in sample.iter().take(1 << 16) {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b'\n' if !in_quotes => {
                counts.push(current);
                current = 0;
                if counts.len() >= 64 {
                    return counts;
                }
            }
            b if b == delimiter && !in_quotes => current += 1,
            _ => {}
        }
    }
    if current > 0 || counts.is_empty() {
        counts.push(current);
    }
    counts
}

/// Why a byte string is not the CSV it claimed to be. Errors are values
/// (DP-A10); nothing here panics on any input.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CsvError {
    /// A field exceeded [`limits::MAX_FIELD_BYTES`].
    FieldTooLong { line: usize, bytes: usize },
    /// A record exceeded [`limits::MAX_FIELDS`].
    TooManyFields { line: usize },
    /// A quoted field ran to the end of the document without closing. Reported
    /// rather than silently completed: an unterminated quote means every
    /// delimiter after it was misread, so the records already emitted are
    /// suspect and the caller must be told.
    UnterminatedQuote { line: usize },
    /// Bytes that are not UTF-8. CSV has no encoding declaration, so this is a
    /// statement about the file, not about the parser.
    NotUtf8 { line: usize },
}

/// Bounds on untrusted input (docs/37).
pub mod limits {
    /// Excel's cell text limit. A bound that already had to exist beats
    /// inventing a security-flavoured constant, and it means a file we accept
    /// is a file that can round-trip.
    pub const MAX_FIELD_BYTES: usize = 32_767;
    /// Excel's column limit.
    pub const MAX_FIELDS: usize = 16_384;
    /// Rows examined by [`crate::infer::analyze`] before it stops sampling.
    /// The report says when it stopped, so a partial sample is never mistaken
    /// for a whole-file guarantee.
    pub const INFERENCE_SAMPLE_ROWS: usize = 10_000;
}
