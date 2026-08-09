//! `ehkatra-parse` — the sandboxed parser (docs/24 §Sandbox rule).
//!
//! **This binary is the untrusted half.** It is the only place a hostile
//! document's bytes are interpreted, it runs in its own process under the
//! confinement `ehkatra_io::sandbox` applies, and it can produce exactly one
//! thing: an IR document on stdout.
//!
//! Two properties are worth stating because they are structural rather than
//! promised:
//! * **It never opens a file.** The document arrives on stdin, so the parser
//!   has no reason to touch the filesystem and no path to be tricked about.
//!   The host chose the file; the child only ever sees bytes.
//! * **It links no networking code**, which is what makes docs/24's "no
//!   network" true here. Windows offers no seccomp equivalent without
//!   installation or elevation (DP-S5), so this is a structural argument rather
//!   than an enforced one — recorded honestly as TD-37 rather than claimed as a
//!   syscall filter.
//!
//! Usage: `ehkatra-parse csv [--no-header] < document.csv > ir.json`

use std::io::{self, Read, Write};

use ehkatra_io::ir::{encode, Ir};
use usk_csv::infer;
use usk_csv::reader::CsvParser;
use usk_csv::{Dialect, Record};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let header = !args.iter().any(|a| a == "--no-header");
    let format = args.first().map(String::as_str).unwrap_or("csv");
    if format != "csv" && format != "xlsx" {
        // The vocabulary is closed on purpose: an unknown mode is a host bug,
        // and guessing would be the sandbox interpreting instructions.
        eprintln!("ehkatra-parse: unsupported format {format:?}");
        std::process::exit(2);
    }

    let mut bytes = Vec::new();
    if let Err(err) = io::stdin().read_to_end(&mut bytes) {
        eprintln!("ehkatra-parse: {err}");
        std::process::exit(3);
    }

    let ir = if format == "xlsx" {
        run_xlsx(&bytes)
    } else {
        run(&bytes, header)
    };
    let out = encode(&ir);
    let mut stdout = io::stdout();
    // A write failure means the host has gone or stopped reading. There is
    // nobody to report it to, so exit rather than pretend.
    if stdout.write_all(out.as_bytes()).is_err() || stdout.flush().is_err() {
        std::process::exit(4);
    }
}

/// XLSX, through three parsers — ZIP, DEFLATE, XML — before any spreadsheet
/// semantics appear. Which is the reason docs/24 wants this in its own process.
pub fn run_xlsx(bytes: &[u8]) -> Ir {
    match usk_xlsx::read(bytes) {
        Ok(workbook) => Ir::Workbook(workbook),
        Err(err) => Ir::WorkbookFailed(format!("{err:?}")),
    }
}

/// The whole of the child's logic, as a pure function — so the sandbox's
/// behaviour is testable in-process without spawning anything, and the
/// subprocess test can then check only what the subprocess adds.
pub fn run(bytes: &[u8], header: bool) -> Ir {
    let dialect = Dialect::sniff(bytes);
    let mut parser = CsvParser::new(dialect);
    let mut records: Vec<Record> = Vec::new();

    // Chunked deliberately, even though the bytes are already in hand: the
    // streaming path is the one docs/24 requires, so it is the one that gets
    // exercised on every real document rather than only in a test.
    const CHUNK: usize = 64 << 10;
    for chunk in bytes.chunks(CHUNK) {
        if let Err(err) = parser.push(chunk, &mut records) {
            return Ir::Failed(err);
        }
    }
    if let Err(err) = parser.finish(&mut records) {
        return Ir::Failed(err);
    }

    let report = infer::analyze(&records, header);
    Ir::Parsed {
        dialect,
        records,
        report,
    }
}
