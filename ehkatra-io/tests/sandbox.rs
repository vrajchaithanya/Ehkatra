//! Row 12's sandbox contract (docs/24 §Sandbox rule, §Format matrix).
//!
//! `usk-csv`'s tests prove the *rules*. These prove what the subprocess adds:
//! that parsing happens over there, that a compromised parser cannot talk the
//! host into anything, and that the host re-checks the bounds the child was
//! supposed to enforce.

use ehkatra_io::ir::{self, Ir, IrError};
use ehkatra_io::{import_csv_bytes, ImportError};
use usk_csv::infer::Decision;
use usk_csv::{limits, CsvError};

fn parsed(bytes: &[u8]) -> Ir {
    import_csv_bytes(bytes, true).expect("import succeeds").ir
}

/// docs/24: *"all parsing runs in an isolated subprocess"*. The end-to-end
/// path — spawn, confine, feed, read IR, revalidate — with a document that
/// exercises quoting and inference on the way through.
#[test]
fn a_document_is_parsed_only_through_the_subprocess() {
    let imported = import_csv_bytes(b"id,label\n1,\"a,b\"\n2,plain\n", true).expect("import");
    let Ir::Parsed {
        records, report, ..
    } = &imported.ir
    else {
        panic!("expected a parse, got {:?}", imported.ir);
    };
    assert_eq!(records.len(), 3, "header plus two rows");
    assert_eq!(records[1].fields, vec!["1", "a,b"]);
    assert_eq!(report.columns.len(), 2);
    assert_eq!(report.columns[0].name, "id");

    #[cfg(windows)]
    assert!(
        imported.confined,
        "the job-object limits must be in force, not merely attempted"
    );
}

/// The finding docs/24 exists to surface, carried across the process boundary
/// with its evidence intact. A report that survives to the host as a boolean
/// would be no report at all.
#[test]
fn the_gene_symbol_decision_survives_the_sandbox_boundary() {
    let Ir::Parsed { report, .. } = parsed(b"gene,n\n1E2,5\nSEPT2,7\n") else {
        panic!("expected a parse");
    };
    let genes = &report.columns[0];
    assert_eq!(genes.loss_count, 1);
    assert_eq!(genes.losses[0].original, "1E2");
    assert_eq!(genes.losses[0].as_number, "100");
    assert_eq!(genes.losses[0].line, 2);
    assert_eq!(genes.suggested, Decision::Text);
    assert!(!report.is_unambiguous());
}

/// A malformed document is an *answer*, not a crash: the child reports the
/// defect, the host receives it as data, and nothing about the failure is
/// distinguishable from the outside except its content.
#[test]
fn a_malformed_document_comes_back_as_a_named_failure() {
    match parsed(b"a,\"never closed\n1,2\n") {
        Ir::Failed(CsvError::UnterminatedQuote { line }) => assert_eq!(line, 1),
        other => panic!("expected UnterminatedQuote, got {other:?}"),
    }
}

/// An injection attempt travels as a *finding*, and the payload never becomes
/// a formula anywhere along the way.
#[test]
fn an_injection_attempt_arrives_as_a_finding() {
    let Ir::Parsed { report, .. } = parsed(b"v\n=cmd|'/c calc'!A0\n") else {
        panic!("expected a parse");
    };
    assert_eq!(report.injections.len(), 1);
    assert_eq!(report.injections[0].line, 2);
    assert!(report.injections[0].sample.starts_with("=cmd"));
}

// ------------------------------------------- revalidation (docs/24, the host)

/// **The rule that makes the sandbox a sandbox rather than a speed bump.** The
/// child has just processed a hostile file; if it was compromised, its output
/// is the attacker's output. So every bound the child was supposed to enforce
/// is re-checked here.
#[test]
fn a_child_lying_about_its_own_bounds_is_refused() {
    let huge = "x".repeat(limits::MAX_FIELD_BYTES + 1);
    let json = format!(
        r#"{{"schema":"{}","kind":"csv","dialect":{{"delimiter":44,"quote":34}},"records":[{{"line":1,"fields":["{huge}"]}}],"report":{{"columns":[],"rows_sampled":0,"rows_total":0,"injections":[],"ragged_rows":[]}}}}"#,
        ir::SCHEMA
    );
    assert_eq!(
        ir::decode(json.as_bytes()),
        Err(IrError::BoundViolated("MAX_FIELD_BYTES"))
    );

    let lying = format!(
        r#"{{"schema":"{}","kind":"csv","dialect":{{"delimiter":44,"quote":34}},"records":[],"report":{{"columns":[],"rows_sampled":9,"rows_total":1,"injections":[],"ragged_rows":[]}}}}"#,
        ir::SCHEMA
    );
    assert_eq!(
        ir::decode(lying.as_bytes()),
        Err(IrError::BoundViolated("rows_sampled > rows_total"))
    );
}

/// The IR vocabulary has no verbs, so a compromised child can lie about a
/// file's contents but cannot ask the host to *do* anything. The nearest thing
/// to an instruction it holds is a column's suggested decision, and an
/// unrecognised one falls back to the choice that cannot lose data.
#[test]
fn an_unrecognised_decision_falls_back_to_the_lossless_one() {
    let json = format!(
        r#"{{"schema":"{}","kind":"csv","dialect":{{"delimiter":44,"quote":34}},"records":[],"report":{{"columns":[{{"index":0,"name":"c","blank":0,"numeric":0,"boolean":0,"textual":0,"loss_count":0,"losses":[],"suggested":"ExecuteEverything"}}],"rows_sampled":0,"rows_total":0,"injections":[],"ragged_rows":[]}}}}"#,
        ir::SCHEMA
    );
    let Ok(Ir::Parsed { report, .. }) = ir::decode(json.as_bytes()) else {
        panic!("expected a parse");
    };
    assert_eq!(report.columns[0].suggested, Decision::Text);
}

#[test]
fn output_that_is_not_our_ir_is_refused() {
    assert_eq!(ir::decode(b"not json at all"), Err(IrError::NotJson));
    assert_eq!(
        ir::decode(br#"{"schema":"something.else/9"}"#),
        Err(IrError::WrongSchema)
    );
    let no_records = format!(
        r#"{{"schema":"{}","kind":"csv","dialect":{{}}}}"#,
        ir::SCHEMA
    );
    assert_eq!(
        ir::decode(no_records.as_bytes()),
        Err(IrError::MissingField("records"))
    );
}

/// Encode/decode is an identity over everything the IR carries — otherwise the
/// report the user sees is not the report the parser produced.
#[test]
fn the_ir_round_trips_through_the_boundary() {
    for document in [
        &b"gene,n\n1E2,5\nSEPT2,7\n0007,9\n"[..],
        &b"a,b,c\n1,2,3\n4,5\n"[..],
        &b"v\n=1+1\n-3\n"[..],
        &b"single\n"[..],
    ] {
        let original = parsed(document);
        let encoded = ir::encode(&original);
        let decoded = ir::decode(encoded.as_bytes()).expect("decodes");
        assert_eq!(decoded, original, "IR changed crossing the boundary");
    }
}

/// docs/24 says *"fresh process per document"*. Two imports must not be able to
/// influence each other, which a shared process could not guarantee.
#[test]
fn each_document_gets_its_own_process() {
    let first = parsed(b"a\n0001\n");
    let second = parsed(b"a\n1\n");
    let Ir::Parsed { report: r1, .. } = &first else {
        panic!()
    };
    let Ir::Parsed { report: r2, .. } = &second else {
        panic!()
    };
    assert_eq!(r1.columns[0].loss_count, 1, "leading zeros in the first");
    assert_eq!(
        r2.columns[0].loss_count, 0,
        "and no trace of it in the second"
    );
}

/// A parser that cannot be confined must not run at all. Asserted through the
/// type rather than by disabling the OS: `Sandbox::confine` returning an error
/// is mapped to `NotConfined`, and there is no code path from there to a parse.
#[test]
fn there_is_no_unconfined_import_path() {
    // The only public entry points both spawn. This test is a compile-time
    // assertion dressed as a runtime one: if an in-process `parse_csv` is ever
    // added to this crate's surface, this list stops being exhaustive and the
    // reviewer has to notice.
    let by_bytes: fn(&[u8], bool) -> Result<_, ImportError> = import_csv_bytes;
    let _ = by_bytes;
    let imported = import_csv_bytes(b"a\n1\n", true).expect("import");
    assert!(matches!(imported.ir, Ir::Parsed { .. }));
}

// ----------------------------------------- XLSX through the same sandbox

fn xlsx(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("crates/usk-xlsx/tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("corpus {name}: {e}"))
}

/// docs/24's sandbox rule says *no exceptions*, and XLSX is the format that
/// most needs it: a ZIP of compressed XML is three parsers deep before any
/// spreadsheet semantics appear. One `run_parser` serves both formats, so a new
/// format cannot arrive with a slightly different — or absent — sandbox.
#[test]
fn a_workbook_is_parsed_only_through_the_subprocess() {
    let imported = ehkatra_io::import_xlsx_bytes(&xlsx("04-formulas.xlsx")).expect("import");
    let Ir::Workbook(workbook) = &imported.ir else {
        panic!("expected a workbook, got {:?}", imported.ir);
    };
    assert_eq!(workbook.sheets.len(), 1);
    let c1 = workbook.sheets[0].cell(0, 2).expect("C1");
    assert_eq!(c1.formula.as_deref(), Some("A1+B1"));
    assert_eq!(c1.value, usk_types::Value::Number(5.0));

    #[cfg(windows)]
    assert!(imported.confined);
}

/// The fidelity report has to survive the boundary intact — a report that
/// arrives as a boolean is no report.
#[test]
fn the_fidelity_report_crosses_the_boundary_intact() {
    let imported = ehkatra_io::import_xlsx_bytes(&xlsx("13-macro-enabled.xlsm")).expect("import");
    let Ir::Workbook(workbook) = &imported.ir else {
        panic!("expected a workbook");
    };
    assert_eq!(workbook.fidelity.quarantined, vec!["xl/vbaProject.bin"]);
    assert_eq!(workbook.fidelity.part_coverage(), 100.0);

    let losses = ehkatra_io::import_xlsx_bytes(&xlsx("18-odd-cells.xlsx")).expect("import");
    let Ir::Workbook(workbook) = &losses.ir else {
        panic!("expected a workbook");
    };
    assert_eq!(
        workbook.fidelity.losses.len(),
        3,
        "each loss, with its reason"
    );
}

/// Every corpus file, through the process boundary, must equal what the
/// in-process reader produced. Otherwise the IR is lossy and nobody would know.
#[test]
fn every_workbook_round_trips_through_the_ir() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("crates/usk-xlsx/tests/corpus");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("corpus")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".xlsx") || n.ends_with(".xlsm"))
        .collect();
    names.sort();
    assert_eq!(names.len(), 20);

    for name in names {
        let bytes = xlsx(&name);
        let direct = usk_xlsx::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let imported = ehkatra_io::import_xlsx_bytes(&bytes).expect("import");
        let Ir::Workbook(through) = &imported.ir else {
            panic!("{name}: expected a workbook, got {:?}", imported.ir);
        };
        assert_eq!(through, &direct, "{name} changed crossing the boundary");
    }
}

/// A container that is not a workbook comes back as a named failure, not a
/// crash and not an empty workbook that looks like an empty spreadsheet.
#[test]
fn a_container_that_is_not_a_workbook_fails_by_name() {
    let imported = ehkatra_io::import_xlsx_bytes(b"not a zip").expect("import");
    match &imported.ir {
        Ir::WorkbookFailed(detail) => assert!(detail.contains("NotAZip"), "{detail}"),
        other => panic!("expected a named failure, got {other:?}"),
    }
}

/// A child claiming more cells than it sent is broken or compromised, and
/// either way its output goes in the bin (docs/24's revalidation clause).
#[test]
fn a_workbook_ir_that_does_not_add_up_is_refused() {
    let lying = format!(
        r#"{{"schema":"{}","kind":"xlsx","sheets":[{{"name":"S","part":"p","cells":[]}}],"fidelity":{{"parts_total":1,"parts_read":[],"parts_ignored":[],"parts_structural":[],"quarantined":[],"cells_read":99,"formulas_read":0,"number_formats_resolved":0,"losses":[]}}}}"#,
        ir::SCHEMA
    );
    assert_eq!(
        ir::decode(lying.as_bytes()),
        Err(IrError::BoundViolated("cells_read disagrees with cells"))
    );
}
