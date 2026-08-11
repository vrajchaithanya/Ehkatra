//! XLSX write, held to the round-trip property (docs/24, session 29).
//!
//! Two levels, deliberately different in what they demand:
//!
//! * **Synthetic** — a workbook built in-engine, written, re-read. 100% is the
//!   only acceptable number: any loss on our own output is a bug, not a
//!   fidelity statement.
//! * **Corpus** — every file the reader's 20-file corpus holds, read → write →
//!   re-read through the same sandboxed reader, compared cell for cell. The
//!   comparison result *is* the published write-fidelity number
//!   (MEASUREMENTS.md, W-XLSX-WRITE); the parts a round-trip drops (charts,
//!   themes, vbaProject) are named by the report, never silently gone.
//!
//! The corpus pass doubles as the fuzz-adjacent guarantee: the writer's output
//! goes back through the reader on every file, so an output that panics or
//! fails to parse fails the suite.

use std::collections::BTreeMap;
use std::path::PathBuf;

use usk_types::decimal::Decimal;
use usk_types::{CellError, ErrorKind, Origin, Value};
use usk_xlsx::write::{write, WriteError, WriteLossReason, Written};
use usk_xlsx::{read, Cell, Fidelity, Sheet, Workbook};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
}

fn file(name: &str) -> Vec<u8> {
    std::fs::read(corpus_dir().join(name)).unwrap_or_else(|e| panic!("corpus {name}: {e}"))
}

fn corpus_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(corpus_dir())
        .expect("the corpus must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".xlsx") || n.ends_with(".xlsm"))
        .collect();
    names.sort();
    names
}

/// The modelled surface of one sheet, as a comparable map. Sheet order and
/// names are compared separately.
type CellMap = BTreeMap<(u32, u32), (Value, Option<String>, Option<String>)>;

fn cell_map(sheet: &Sheet) -> CellMap {
    sheet
        .cells
        .iter()
        .map(|c| {
            (
                (c.row, c.col),
                (c.value.clone(), c.formula.clone(), c.number_format.clone()),
            )
        })
        .collect()
}

fn assert_same_model(name: &str, before: &Workbook, after: &Workbook) {
    assert_eq!(
        before.sheets.len(),
        after.sheets.len(),
        "{name}: sheet count changed"
    );
    for (b, a) in before.sheets.iter().zip(&after.sheets) {
        assert_eq!(b.name, a.name, "{name}: sheet name changed");
        let (bm, am) = (cell_map(b), cell_map(a));
        for (address, cell) in &bm {
            assert_eq!(
                Some(cell),
                am.get(address),
                "{name}/{}: cell {address:?} changed or vanished",
                b.name
            );
        }
        assert_eq!(bm.len(), am.len(), "{name}/{}: cells appeared", b.name);
    }
}

fn synthetic() -> Workbook {
    let cell = |row, col, value| Cell {
        row,
        col,
        value,
        formula: None,
        number_format: None,
    };
    let dense = Sheet {
        name: String::from("Values & <edge> \"cases\""),
        part: String::new(),
        cells: vec![
            // The numbers that stress f64 rendering.
            cell(0, 0, Value::Number(0.0)),
            cell(0, 1, Value::Number(-1.5)),
            cell(0, 2, Value::Number(0.1)),
            cell(0, 3, Value::Number(core::f64::consts::PI)),
            cell(0, 4, Value::Number(1e-17)),
            cell(0, 5, Value::Number(1234567890123456.0)),
            cell(0, 6, Value::Number(f64::MAX)),
            cell(0, 7, Value::Number(5e-324)), // smallest subnormal
            cell(1, 0, Value::Bool(true)),
            cell(1, 1, Value::Bool(false)),
            // Text that exercises escaping: entities, quotes, an astral
            // scalar, whitespace that must be preserved, the empty string.
            cell(2, 0, Value::Text(String::from("a & b < c > d"))),
            cell(2, 1, Value::Text(String::from("caf\u{e9} \u{1F600}"))),
            cell(2, 2, Value::Text(String::from("  leading and trailing  "))),
            cell(2, 3, Value::Text(String::from("line\nbreak"))),
            cell(2, 4, Value::Text(String::new())),
            cell(2, 5, Value::Text(String::from("shared"))),
            cell(3, 0, Value::Text(String::from("shared"))), // dedup path
            // Every error a plain `t="e"` cell can carry. `#SPILL!` is not
            // among them — Excel refuses a container holding one, literal or
            // formula-cached (proven via COM, session 29) — so it lives in the
            // named-loss test instead.
            cell(
                4,
                0,
                Value::Error(CellError::new(ErrorKind::Div0, Origin::Authored)),
            ),
            cell(
                4,
                1,
                Value::Error(CellError::new(ErrorKind::Value, Origin::Authored)),
            ),
            cell(
                4,
                2,
                Value::Error(CellError::new(ErrorKind::Ref, Origin::Authored)),
            ),
            cell(
                4,
                3,
                Value::Error(CellError::new(ErrorKind::Name, Origin::Authored)),
            ),
            cell(
                4,
                4,
                Value::Error(CellError::new(ErrorKind::Num, Origin::Authored)),
            ),
            cell(
                4,
                5,
                Value::Error(CellError::new(ErrorKind::Na, Origin::Authored)),
            ),
        ],
    };
    let formulas = Sheet {
        name: String::from("Formulas"),
        part: String::new(),
        cells: vec![
            Cell {
                row: 0,
                col: 0,
                value: Value::Number(5.0),
                formula: Some(String::from("A2+B2")),
                number_format: None,
            },
            Cell {
                row: 0,
                col: 1,
                value: Value::Text(String::from("cached text")),
                formula: Some(String::from("CONCAT(\"cached\",\" text\")")),
                number_format: None,
            },
            Cell {
                row: 0,
                col: 2,
                value: Value::Bool(true),
                formula: Some(String::from("1<2")),
                number_format: None,
            },
            Cell {
                row: 0,
                col: 3,
                value: Value::Error(CellError::new(ErrorKind::Div0, Origin::Authored)),
                formula: Some(String::from("1/0")),
                number_format: None,
            },
            Cell {
                row: 0,
                col: 4,
                value: Value::Blank,
                formula: Some(String::from("IF(FALSE,1,\"\")")),
                number_format: None,
            },
            // A formatted formula cell: both attributes on one cell.
            Cell {
                row: 1,
                col: 0,
                value: Value::Number(0.5),
                formula: Some(String::from("A1/10")),
                number_format: Some(String::from("0.00%")),
            },
        ],
    };
    let sparse = Sheet {
        name: String::from("Sparse"),
        part: String::new(),
        cells: vec![
            Cell {
                row: 0,
                col: 0,
                value: Value::Number(1.0),
                formula: None,
                number_format: Some(String::from("0.00")),
            },
            Cell {
                row: 0,
                col: 1,
                value: Value::Number(2.0),
                formula: None,
                number_format: Some(String::from("\"$\"#,##0.00")), // custom code
            },
            Cell {
                row: 99, // AA100 / ZZ100 — the addresses the reader corpus uses
                col: 26,
                value: Value::Number(100.0),
                formula: None,
                number_format: Some(String::from("mm-dd-yy")), // built-in id
            },
            Cell {
                row: 99,
                col: 701,
                value: Value::Number(702.0),
                formula: None,
                number_format: None,
            },
            // A cell that is nothing but a format — XLSX's style-holding cell.
            Cell {
                row: 500_000,
                col: 3,
                value: Value::Blank,
                formula: None,
                number_format: Some(String::from("0.00")),
            },
        ],
    };
    Workbook {
        sheets: vec![dense, formulas, sparse],
        fidelity: Fidelity::default(),
    }
}

// ------------------------------------------------------------- synthetic

/// Our own workbook, written and re-read: **100% or it is a bug.** Values
/// (including the doubles that stress shortest-rendering), formulas with every
/// cached-value type, both levels of number format, sparse addresses, a
/// format-only cell, three sheets.
#[test]
fn the_synthetic_workbook_round_trips_at_full_fidelity() {
    let book = synthetic();
    let written = write(&book).expect("the synthetic workbook must write");
    let back = read(&written.bytes).expect("our own output must read");
    assert_same_model("synthetic", &book, &back);
    assert!(
        written.report.is_lossless(),
        "the synthetic workbook contains nothing the format cannot carry, \
         so any named loss is a writer defect: {:?}",
        written.report.losses
    );
    assert_eq!(written.report.cells_written, 34);
    assert_eq!(written.report.formulas_written, 6);
    assert_eq!(written.report.number_formats_written, 5);
}

/// DP-A2 for the writer: the same workbook is the same bytes, every time.
#[test]
fn the_writer_is_deterministic() {
    let book = synthetic();
    let first = write(&book).expect("write");
    let second = write(&book).expect("write");
    assert_eq!(first.bytes, second.bytes);
}

/// What the format cannot carry is written as the nearest honest thing and
/// **named** — never silently dropped, never invented (docs/24's fidelity
/// philosophy, applied to the writer).
#[test]
fn non_representable_state_is_a_named_loss_never_a_silent_one() {
    let book = Workbook {
        sheets: vec![Sheet {
            name: String::from("Losses"),
            part: String::new(),
            cells: vec![
                Cell {
                    row: 0,
                    col: 0,
                    value: Value::Number(f64::NAN),
                    formula: None,
                    number_format: None,
                },
                Cell {
                    row: 0,
                    col: 1,
                    value: Value::Number(f64::INFINITY),
                    formula: None,
                    number_format: None,
                },
                Cell {
                    row: 0,
                    col: 2,
                    value: Value::Decimal(Decimal::new(12345, -2)),
                    formula: None,
                    number_format: None,
                },
                Cell {
                    row: 0,
                    col: 3,
                    value: Value::Error(CellError::new(ErrorKind::Circ, Origin::Authored)),
                    formula: None,
                    number_format: None,
                },
                // Found by the Excel COM oracle, not by a test: a bare
                // `#SPILL!` error cell makes Excel refuse the whole file.
                Cell {
                    row: 0,
                    col: 4,
                    value: Value::Error(CellError::new(ErrorKind::Spill, Origin::Authored)),
                    formula: None,
                    number_format: None,
                },
            ],
        }],
        fidelity: Fidelity::default(),
    };
    let written = write(&book).expect("write");
    let reasons: Vec<WriteLossReason> = written.report.losses.iter().map(|l| l.reason).collect();
    assert_eq!(
        reasons,
        vec![
            WriteLossReason::NonFiniteNumber,
            WriteLossReason::NonFiniteNumber,
            WriteLossReason::DecimalWrittenAsNumber,
            WriteLossReason::ErrorOutsideXlsxVocabulary,
            WriteLossReason::ErrorOutsideXlsxVocabulary,
        ]
    );
    assert_eq!(written.report.losses[0].reference, "A1");
    assert_eq!(written.report.losses[3].reference, "D1");
    assert_eq!(written.report.losses[4].reference, "E1");

    // And the degraded spellings are the documented ones.
    let back = read(&written.bytes).expect("read back");
    let sheet = &back.sheets[0];
    for col in [0, 1] {
        match &sheet.cell(0, col).expect("non-finite cell").value {
            Value::Error(e) => assert_eq!(e.kind, ErrorKind::Num),
            other => panic!("expected #NUM!, got {other:?}"),
        }
    }
    assert_eq!(
        sheet.cell(0, 2).expect("decimal cell").value,
        Value::Number(123.45),
        "the decimal's digits are all in the file; the type is what was lost"
    );
    for col in [3, 4] {
        match &sheet.cell(0, col).expect("degraded error cell").value {
            Value::Error(e) => assert_eq!(e.kind, ErrorKind::Na),
            other => panic!("expected #N/A, got {other:?}"),
        }
    }
}

/// XLSX requires a sheet; inventing one would put data in the file that is not
/// in the model, so the writer refuses instead.
#[test]
fn an_empty_workbook_is_refused_rather_than_padded() {
    let book = Workbook {
        sheets: Vec::new(),
        fidelity: Fidelity::default(),
    };
    assert_eq!(write(&book).unwrap_err(), WriteError::NoSheets);
}

// ---------------------------------------------------------------- corpus

fn round_trip(name: &str) -> (Workbook, Written, Workbook) {
    let original = read(&file(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let written = write(&original).unwrap_or_else(|e| panic!("{name}: write: {e:?}"));
    let back = read(&written.bytes)
        .unwrap_or_else(|e| panic!("{name}: our own output failed to re-read: {e:?}"));
    (original, written, back)
}

/// Every corpus file: read → write → re-read through the same sandboxed
/// reader, and the model must survive intact — sheet names, addresses,
/// values, formula texts, number formats. This is also the fuzz-adjacent
/// property: the writer's output must never panic or fail our own reader.
#[test]
fn every_corpus_file_survives_a_model_round_trip() {
    for name in corpus_files() {
        let (original, written, back) = round_trip(&name);
        assert_same_model(&name, &original, &back);
        // The write side must re-emit no active content, ever (docs/24).
        for part in &written.report.parts_written {
            assert!(
                !usk_xlsx::is_active_content(part),
                "{name}: active content {part} was re-emitted"
            );
        }
    }
}

/// The round-trip must also hold *twice*: writing what we re-read yields the
/// same model again (idempotence at the model level), and — because the writer
/// is canonical — byte-identical containers.
#[test]
fn a_second_round_trip_is_byte_identical() {
    for name in corpus_files() {
        let (_, first, back) = round_trip(&name);
        let second = write(&back).unwrap_or_else(|e| panic!("{name}: rewrite: {e:?}"));
        assert_eq!(
            first.bytes, second.bytes,
            "{name}: the writer is not canonical over its own output"
        );
    }
}

/// The per-file write-fidelity report docs/24 asks to be *published*
/// (MEASUREMENTS.md, W-XLSX-WRITE). Printed as a table so the number is
/// inspectable rather than only asserted. Run with `--nocapture` (and
/// `--release` for the timings) to regenerate the published table.
#[test]
fn the_write_fidelity_report() {
    struct Row {
        cells: usize,
        identical: usize,
        formulas: usize,
        formats: usize,
        dropped: usize,
        losses: usize,
        in_bytes: usize,
        out_bytes: usize,
        write_us: u128,
        reread_us: u128,
    }
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();

    for name in corpus_files() {
        let bytes = file(&name);
        let original = read(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));

        let start = std::time::Instant::now();
        let written = write(&original).unwrap_or_else(|e| panic!("{name}: write: {e:?}"));
        let write_us = start.elapsed().as_micros();

        let start = std::time::Instant::now();
        let back = read(&written.bytes).unwrap_or_else(|e| panic!("{name}: re-read: {e:?}"));
        let reread_us = start.elapsed().as_micros();

        let mut cells = 0usize;
        let mut identical = 0usize;
        for (b, a) in original.sheets.iter().zip(&back.sheets) {
            let (bm, am) = (cell_map(b), cell_map(a));
            for (address, cell) in &bm {
                cells += 1;
                if am.get(address) == Some(cell) {
                    identical += 1;
                }
            }
        }

        rows.insert(
            name,
            Row {
                cells,
                identical,
                formulas: written.report.formulas_written,
                formats: written.report.number_formats_written,
                dropped: written.report.parts_dropped.len()
                    + written.report.quarantined_dropped.len()
                    + written.report.parts_unaccounted,
                losses: written.report.losses.len(),
                in_bytes: bytes.len(),
                out_bytes: written.bytes.len(),
                write_us,
                reread_us,
            },
        );
    }

    println!(
        "\n| File | Cells | Identical | Formulas | Formats | Parts dropped | Cell losses | In B | Out B | Write µs | Re-read µs |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let (mut cells, mut identical, mut in_bytes, mut out_bytes) = (0usize, 0usize, 0usize, 0usize);
    let (mut write_us, mut reread_us) = (0u128, 0u128);
    for (name, row) in &rows {
        println!(
            "| `{name}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            row.cells,
            row.identical,
            row.formulas,
            row.formats,
            row.dropped,
            row.losses,
            row.in_bytes,
            row.out_bytes,
            row.write_us,
            row.reread_us
        );
        cells += row.cells;
        identical += row.identical;
        in_bytes += row.in_bytes;
        out_bytes += row.out_bytes;
        write_us += row.write_us;
        reread_us += row.reread_us;
    }
    let percentage = if cells == 0 {
        100.0
    } else {
        100.0 * identical as f64 / cells as f64
    };
    println!(
        "\n{identical}/{cells} cells identical after read → write → re-read = **{percentage:.1}%**; \
         {out_bytes} B written from {in_bytes} B read ({:.2}x, stored entries); \
         {write_us} µs writing, {reread_us} µs re-reading, {} files.",
        out_bytes as f64 / in_bytes as f64,
        rows.len()
    );

    // The published claim. 100.0 is asserted — the *model* must survive its
    // own writer exactly; what the writer cannot carry (source parts it never
    // saw the bytes of) is named in `parts_dropped`, which the corpus test
    // verifies is nonzero for the files that have such parts.
    assert_eq!(
        identical, cells,
        "write fidelity fell below 100% of the modelled surface"
    );
}

/// Writes the round-tripped corpus (plus the synthetic workbook) to
/// `.tmp/xlsx-write/` for validation by external readers — Excel via COM among
/// them. `#[ignore]` because it writes outside the target dir; run explicitly:
/// `cargo test -p usk-xlsx --test roundtrip -- --ignored dump_written`.
#[test]
#[ignore]
fn dump_written_corpus_for_external_validation() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".tmp")
        .join("xlsx-write");
    std::fs::create_dir_all(&out_dir).expect("create .tmp/xlsx-write");
    for name in corpus_files() {
        let (_, written, _) = round_trip(&name);
        let target = out_dir.join(name.replace(".xlsm", ".xlsx"));
        std::fs::write(&target, &written.bytes).expect("write output");
    }
    let written = write(&synthetic()).expect("synthetic");
    std::fs::write(out_dir.join("synthetic.xlsx"), &written.bytes).expect("write synthetic");
    println!("written to {}", out_dir.display());
}
