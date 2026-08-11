//! XLSX read against the 20-file starter corpus (BOOTSTRAP row 12).
//!
//! The corpus is built by `make_corpus.py` from the ECMA-376 shapes Excel
//! actually emits — including the awkward ones — rather than by a spreadsheet
//! library, for the same reason the ZIP corpus is: a reader tested against files
//! its own writer produced proves only that two bugs agree.

use std::collections::BTreeMap;
use std::path::PathBuf;

use usk_types::{ErrorKind, Value};
use usk_xlsx::{read, Fidelity, LossReason, Workbook};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
}

fn file(name: &str) -> Vec<u8> {
    std::fs::read(corpus_dir().join(name)).unwrap_or_else(|e| panic!("corpus {name}: {e}"))
}

fn open(name: &str) -> Workbook {
    read(&file(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"))
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

fn value_at(book: &Workbook, sheet: usize, row: u32, col: u32) -> Value {
    book.sheets[sheet]
        .cell(row, col)
        .map(|c| c.value.clone())
        .unwrap_or(Value::Blank)
}

// ------------------------------------------------------------------- values

#[test]
fn the_minimal_workbook_reads() {
    let book = open("01-minimal.xlsx");
    assert_eq!(book.sheets.len(), 1);
    assert_eq!(book.sheets[0].name, "Sheet1");
    assert_eq!(value_at(&book, 0, 0, 0), Value::Number(42.0));
    assert_eq!(book.fidelity.cells_read, 1);
}

#[test]
fn numbers_survive_including_the_ones_that_stress_f64() {
    let book = open("02-numbers.xlsx");
    assert_eq!(value_at(&book, 0, 0, 0), Value::Number(0.0));
    assert_eq!(value_at(&book, 0, 0, 1), Value::Number(-1.5));
    assert_eq!(value_at(&book, 0, 0, 2), Value::Number(1e-17));
    assert_eq!(value_at(&book, 0, 0, 3), Value::Number(1234567890123456.0));
    // `0.1` must arrive as the double Excel stored, not as a decimal
    // reinterpretation of the text — the two differ and the difference is the
    // whole of `Profile::Compat`.
    assert_eq!(value_at(&book, 0, 1, 0), Value::Number(0.1));
    assert_eq!(
        value_at(&book, 0, 1, 1),
        Value::Number(core::f64::consts::PI)
    );
}

/// Without the shared-string table a workbook is a workbook of numbers, which
/// is a failure mode that looks like success.
#[test]
fn shared_strings_resolve_and_repeat() {
    let book = open("03-shared-strings.xlsx");
    assert_eq!(value_at(&book, 0, 0, 0), Value::Text(String::from("hello")));
    assert_eq!(value_at(&book, 0, 0, 1), Value::Text(String::from("world")));
    assert_eq!(
        value_at(&book, 0, 1, 0),
        Value::Text(String::from("hello")),
        "an index used twice resolves twice"
    );
}

/// XLSX stores a formula *and* the value Excel last calculated. Both are kept:
/// the formula is the authored intent, the cached value is the evidence, and
/// comparing them is how the conformance story gets its inputs.
#[test]
fn formulas_are_read_alongside_their_cached_values() {
    let book = open("04-formulas.xlsx");
    let c1 = book.sheets[0].cell(0, 2).expect("C1");
    assert_eq!(c1.formula.as_deref(), Some("A1+B1"));
    assert_eq!(c1.value, Value::Number(5.0));

    let a2 = book.sheets[0].cell(1, 0).expect("A2");
    assert_eq!(a2.formula.as_deref(), Some("SUM(A1:B1)"));
    assert_eq!(book.fidelity.formulas_read, 2);
}

#[test]
fn error_cells_map_to_the_engine_s_error_kinds() {
    let book = open("05-errors.xlsx");
    let kinds: Vec<ErrorKind> = (0..5)
        .map(|col| match value_at(&book, 0, 0, col) {
            Value::Error(e) => e.kind,
            other => panic!("expected an error at column {col}, got {other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ErrorKind::Div0,
            ErrorKind::Na,
            ErrorKind::Ref,
            ErrorKind::Name,
            ErrorKind::Value
        ]
    );
}

#[test]
fn booleans_inline_strings_and_formula_strings_read() {
    let booleans = open("06-booleans.xlsx");
    assert_eq!(value_at(&booleans, 0, 0, 0), Value::Bool(true));
    assert_eq!(value_at(&booleans, 0, 0, 1), Value::Bool(false));
    assert_eq!(value_at(&booleans, 0, 0, 2), Value::Bool(true));

    let strings = open("07-inline-strings.xlsx");
    assert_eq!(
        value_at(&strings, 0, 0, 0),
        Value::Text(String::from("inline")),
        "text stored in the cell rather than the shared table"
    );
    assert_eq!(
        value_at(&strings, 0, 0, 1),
        Value::Text(String::from("X")),
        "a formula whose cached result is text"
    );
}

/// Number formats need two levels of indirection — cell → `cellXfs` → `numFmt`
/// — and both built-ins and custom codes have to resolve.
#[test]
fn number_formats_resolve_through_both_levels() {
    let book = open("08-number-formats.xlsx");
    let sheet = &book.sheets[0];
    assert_eq!(
        sheet.cell(0, 0).unwrap().number_format.as_deref(),
        Some("0.00")
    );
    assert_eq!(
        sheet.cell(0, 1).unwrap().number_format.as_deref(),
        Some("mm-dd-yy"),
        "a built-in id"
    );
    assert_eq!(
        sheet.cell(0, 2).unwrap().number_format.as_deref(),
        Some("\"$\"#,##0.00"),
        "a custom code, with its entities expanded"
    );
    assert_eq!(
        sheet.cell(0, 3).unwrap().number_format,
        None,
        "General is the absence of a format, not a format"
    );
    assert_eq!(book.fidelity.number_formats_resolved, 3);
}

#[test]
fn every_sheet_is_read_with_its_name() {
    let book = open("09-multi-sheet.xlsx");
    let names: Vec<&str> = book.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["First", "Second", "Third"]);
    for (index, sheet) in book.sheets.iter().enumerate() {
        assert_eq!(sheet.cells[0].value, Value::Number(index as f64 + 1.0));
    }
}

/// **A reader that assumes `sheetN.xml` is sheet N gets the wrong sheet.** The
/// relationship table is what maps them, and this file crosses the two over.
#[test]
fn sheets_are_located_through_relationships_not_by_filename() {
    let book = open("10-rels-out-of-order.xlsx");
    assert_eq!(book.sheets[0].name, "Alpha");
    assert_eq!(book.sheets[0].part, "xl/worksheets/sheetB.xml");
    assert_eq!(
        book.sheets[0].cells[0].value,
        Value::Number(222.0),
        "Alpha points at sheetB, which holds 222"
    );
    assert_eq!(book.sheets[1].name, "Beta");
    assert_eq!(book.sheets[1].cells[0].value, Value::Number(111.0));
}

#[test]
fn a_sparse_sheet_keeps_its_addresses() {
    let book = open("11-sparse.xlsx");
    let sheet = &book.sheets[0];
    assert_eq!(sheet.cells.len(), 4);
    assert_eq!(sheet.cell(0, 0).unwrap().value, Value::Number(1.0));
    assert_eq!(sheet.cell(4, 2).unwrap().value, Value::Number(5.0));
    // AA100 → row 99, column 26. ZZ100 → column 701.
    assert_eq!(sheet.cell(99, 26).unwrap().value, Value::Number(100.0));
    assert_eq!(sheet.cell(99, 701).unwrap().value, Value::Number(702.0));
}

#[test]
fn entities_and_astral_characters_survive() {
    let book = open("12-entities.xlsx");
    assert_eq!(
        value_at(&book, 0, 0, 0),
        Value::Text(String::from("a & b < c"))
    );
    assert_eq!(value_at(&book, 0, 0, 1), Value::Text(String::from("café")));
    assert_eq!(
        value_at(&book, 0, 0, 2),
        Value::Text(String::from("\u{1F600}")),
        "one scalar, matching the engine's LEN semantics (D-082)"
    );
}

// -------------------------------------------------------- docs/24 active content

/// docs/24: *active (vbaProject, OLE, ActiveX, DDE) → quarantine ... never
/// executed, never re-emitted*. Nothing here executes anything, but "we
/// happened not to run it" is not the same claim as "we found it and set it
/// aside" — so the part is named in the report and its bytes are never
/// decompressed.
#[test]
fn active_content_is_quarantined_and_named() {
    let book = open("13-macro-enabled.xlsm");
    assert_eq!(book.fidelity.quarantined, vec!["xl/vbaProject.bin"]);
    assert!(
        !book.fidelity.parts_read.iter().any(|p| p.contains("vba")),
        "a quarantined part must never be read"
    );
    // The rest of the workbook still opens: quarantine is not refusal.
    assert_eq!(value_at(&book, 0, 0, 0), Value::Number(1.0));
    // Quarantining is the *correct* outcome, so it must not count against the
    // coverage ratio — the denominator excludes it. Asserted against the
    // computed value rather than a constant, so what is being tested is the
    // rule and not one file's arithmetic.
    let f = &book.fidelity;
    let considered = f.parts_total - f.quarantined.len() - f.parts_structural.len();
    let expected = 100.0 * f.parts_read.len() as f64 / considered as f64;
    assert_eq!(f.part_coverage(), expected);
    assert!(
        (f.part_coverage() - 100.0).abs() < 1e-9,
        "a file we read completely scores 100%; plumbing is not lost data \
         and charts are, which is the distinction this ratio exists to make"
    );
}

/// Parts v0.1 does not model are listed individually, so the fidelity number
/// distinguishes "we ignored the chart" from "we did not recognise this".
#[test]
fn unmodelled_parts_are_named_rather_than_lumped_together() {
    let book = open("14-unmodelled-parts.xlsx");
    let ignored = &book.fidelity.parts_ignored;
    assert!(ignored.iter().any(|p| p == "xl/charts/chart1.xml"));
    assert!(ignored.iter().any(|p| p == "xl/drawings/drawing1.xml"));
    assert!(ignored.iter().any(|p| p == "xl/theme/theme1.xml"));
    assert!(!book.fidelity.is_lossless(), "and it says so");
}

// --------------------------------------------------------------- degradation

#[test]
fn a_stored_uncompressed_container_reads() {
    assert_eq!(
        value_at(&open("15-stored.xlsx"), 0, 0, 0),
        Value::Number(15.0)
    );
}

/// Every degradation is a *named* loss with its cell reference, never a silent
/// substitution. The workbook still opens — refusing the whole file over one
/// bad cell would be its own kind of data loss.
#[test]
fn defects_degrade_into_named_losses_and_the_workbook_still_opens() {
    let dangling = open("16-dangling-style.xlsx");
    assert_eq!(dangling.fidelity.losses.len(), 1);
    assert_eq!(
        dangling.fidelity.losses[0].reason,
        LossReason::UnresolvedStyle
    );
    assert_eq!(dangling.fidelity.losses[0].reference, "A1");
    assert_eq!(value_at(&dangling, 0, 0, 0), Value::Number(1.0));

    let bad_index = open("17-bad-shared-index.xlsx");
    assert!(bad_index
        .fidelity
        .losses
        .iter()
        .any(|l| l.reason == LossReason::SharedStringOutOfRange));
    assert_eq!(
        value_at(&bad_index, 0, 0, 0),
        Value::Text(String::from("only one")),
        "the good cell is unaffected"
    );

    let odd = open("18-odd-cells.xlsx");
    let reasons: Vec<LossReason> = odd.fidelity.losses.iter().map(|l| l.reason).collect();
    assert!(reasons.contains(&LossReason::UnsupportedCellType));
    assert!(reasons.contains(&LossReason::UnparseableReference));
    assert_eq!(
        value_at(&odd, 0, 0, 1),
        Value::Text(String::from("ok")),
        "a cell that says it is numeric but is not degrades to its text"
    );
}

#[test]
fn optional_parts_really_are_optional() {
    assert_eq!(
        value_at(&open("19-no-optional-parts.xlsx"), 0, 0, 0),
        Value::Number(19.0),
        "no sharedStrings, no styles"
    );
}

/// A missing relationship part must not lose the sheet: Excel's naming
/// convention is a better answer than dropping data.
#[test]
fn a_missing_relationship_part_falls_back_to_the_convention() {
    let book = open("20-missing-rels.xlsx");
    assert_eq!(book.sheets.len(), 1);
    assert_eq!(book.sheets[0].name, "Recovered");
    assert_eq!(value_at(&book, 0, 0, 0), Value::Number(20.0));
}

// ------------------------------------------------------- the corpus as a whole

#[test]
fn the_corpus_is_the_twenty_files_bootstrap_asks_for() {
    assert_eq!(corpus_files().len(), 20);
}

/// Every file opens, and every file's report is internally consistent. This is
/// the test that fails when a new corpus file is added and forgotten.
#[test]
fn every_corpus_file_opens_with_a_coherent_report() {
    for name in corpus_files() {
        let book = read(&file(&name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let f = &book.fidelity;
        assert!(!f.parts_read.is_empty(), "{name}: read nothing");
        assert!(
            f.parts_read.len() + f.parts_ignored.len() + f.quarantined.len() <= f.parts_total,
            "{name}: the report counts more parts than the container has"
        );
        assert_eq!(
            f.cells_read,
            book.sheets.iter().map(|s| s.cells.len()).sum::<usize>(),
            "{name}: the cell count disagrees with the cells"
        );
        assert!(
            f.part_coverage() > 0.0 && f.part_coverage() <= 100.0,
            "{name}"
        );
    }
}

/// Truncating every corpus file at a spread of points: a partial download must
/// produce a named error, never a panic and never a workbook that looks whole.
#[test]
fn truncated_containers_are_refused_rather_than_half_read() {
    for name in corpus_files() {
        let bytes = file(&name);
        for divisor in [2usize, 3, 4, 8, 16] {
            let cut = bytes.len() / divisor;
            match read(&bytes[..cut]) {
                Err(_) => {}
                Ok(book) => panic!(
                    "{name} truncated to {cut} bytes produced {} sheets",
                    book.sheets.len()
                ),
            }
        }
    }
}

/// The per-file fidelity report docs/24 asks to be *published*. Printed as a
/// table so the number is inspectable rather than only asserted.
#[test]
fn the_per_file_fidelity_report() {
    let mut rows: BTreeMap<String, (f64, usize, usize, usize, usize)> = BTreeMap::new();
    for name in corpus_files() {
        let book = read(&file(&name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let f: &Fidelity = &book.fidelity;
        rows.insert(
            name,
            (
                f.part_coverage(),
                f.cells_read,
                f.formulas_read,
                f.losses.len(),
                f.quarantined.len(),
            ),
        );
    }

    println!("\n| File | Part coverage | Cells | Formulas | Losses | Quarantined |");
    println!("|---|---:|---:|---:|---:|---:|");
    let mut total_cells = 0usize;
    let mut lossless = 0usize;
    for (name, (coverage, cells, formulas, losses, quarantined)) in &rows {
        println!("| `{name}` | {coverage:.1}% | {cells} | {formulas} | {losses} | {quarantined} |");
        total_cells += cells;
        if *losses == 0 {
            lossless += 1;
        }
    }
    println!(
        "\n{lossless}/{} files read with no loss; {total_cells} cells total.",
        rows.len()
    );

    // The corpus deliberately contains files that *must* report a loss, so a
    // clean sweep would mean the degradation paths are not being exercised.
    assert!(lossless >= 14, "too many files losing data: {lossless}/20");
    assert!(
        lossless < 20,
        "the defective files must still report losses"
    );
}
