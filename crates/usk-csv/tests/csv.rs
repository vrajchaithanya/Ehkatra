//! Row 12's CSV half: the grammar, the preview-before-commit rule, and
//! injection neutralization in both directions (docs/24).
//!
//! The tests worth reading first are `the_gene_symbol_is_a_surfaced_decision`
//! — the reason docs/24 demands a preview at all — and
//! `a_negative_number_is_not_neutralized_but_a_formula_is_text`, which is the
//! one place the naive OWASP rule corrupts data and this one does not.

use usk_csv::infer::{self, Decision, Loss};
use usk_csv::inject::{self, Risk};
use usk_csv::reader::{parse_all, CsvParser};
use usk_csv::writer::{render, write_csv};
use usk_csv::{limits, CsvError, Dialect, Record};
use usk_types::{CellError, ErrorKind, Origin, Value};

fn records(text: &str) -> Vec<Record> {
    parse_all(text.as_bytes(), Dialect::default()).expect("parses")
}

fn fields(text: &str) -> Vec<Vec<String>> {
    records(text).into_iter().map(|r| r.fields).collect()
}

// ------------------------------------------------------------ the grammar

#[test]
fn rfc4180_quoting_including_the_awkward_parts() {
    assert_eq!(fields("a,b\n1,2\n"), vec![vec!["a", "b"], vec!["1", "2"]]);
    // A quoted field may contain the delimiter, a newline, and doubled quotes.
    assert_eq!(
        fields("\"Smith, J.\",\"line1\nline2\",\"say \"\"hi\"\"\"\n"),
        vec![vec!["Smith, J.", "line1\nline2", "say \"hi\""]]
    );
    // Empty fields, and a final record with no trailing newline.
    assert_eq!(
        fields("a,,c\n,,\nx"),
        vec![vec!["a", "", "c"], vec!["", "", ""], vec!["x"]]
    );
    // CRLF is one line ending; a bare CR inside a field is data.
    assert_eq!(
        fields("a,b\r\nc,d\r\n"),
        vec![vec!["a", "b"], vec!["c", "d"]]
    );
    assert_eq!(fields("\"a\rb\"\n"), vec![vec!["a\rb"]]);
    // Excel keeps text after a closing quote, so refusing it would reject files
    // that open fine in the program we are compatible with.
    assert_eq!(fields("\"a\"b,c\n"), vec![vec!["ab", "c"]]);
}

/// A BOM is metadata. Left in the stream it becomes part of the first header
/// name, and every later lookup of that column fails for a reason invisible in
/// a diff.
#[test]
fn a_utf8_bom_does_not_become_part_of_the_first_header() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"id,name\n1,x\n");
    let out = parse_all(&bytes, Dialect::default()).expect("parses");
    assert_eq!(out[0].fields[0], "id");
}

/// **Regression, found by the fuzz corpus on its first run.** The obvious BOM
/// implementation strips it from the front of the first chunk, and fails
/// silently the moment a chunk boundary lands inside the three-byte sequence —
/// `\u{FEFF}` then becomes part of the first header name. Every split of the
/// BOM is checked, including the two that used to be broken.
#[test]
fn a_bom_split_across_chunks_is_still_stripped() {
    let mut document = vec![0xEF, 0xBB, 0xBF];
    document.extend_from_slice(b"id,name\n1,x\n");
    for split in 0..=document.len() {
        let mut parser = CsvParser::new(Dialect::default());
        let mut out = Vec::new();
        parser.push(&document[..split], &mut out).expect("first");
        parser.push(&document[split..], &mut out).expect("second");
        parser.finish(&mut out).expect("finish");
        assert_eq!(
            out[0].fields[0], "id",
            "the BOM leaked when split at byte {split}"
        );
    }
    // A document that is nothing but a BOM has no records, however it arrives.
    for split in 0..=3 {
        let mut parser = CsvParser::new(Dialect::default());
        let mut out = Vec::new();
        parser
            .push(&document[..split.min(3)], &mut out)
            .expect("first");
        parser
            .push(&document[split.min(3)..3], &mut out)
            .expect("second");
        parser.finish(&mut out).expect("finish");
        assert!(out.is_empty(), "a bare BOM is not a record (split {split})");
    }
}

/// **Regression, found by `a_clean_table_survives_a_full_round_trip`.** Applying
/// the lexical OWASP rule on *import* turns every negative number in the file
/// into text — the same data-corruption bug as the naive export rule, arriving
/// from the other side.
#[test]
fn a_negative_number_survives_import_but_a_formula_does_not() {
    assert_eq!(inject::classify_import("-3"), None);
    assert_eq!(inject::classify_import("-1.5e3"), None);
    assert_eq!(inject::classify_import("+7"), None);
    assert_eq!(
        inject::classify_import("-3+cmd|'/c calc'!A0"),
        Some(Risk::FormulaLead('-'))
    );
    assert_eq!(
        inject::classify_import("=1+1"),
        Some(Risk::FormulaLead('='))
    );

    let values = infer::commit(&records("v\n-3\n"), true, &[Decision::Number]);
    assert_eq!(values[0][0], Value::Number(-3.0));
}

/// **The streaming property.** The same bytes split at every possible point
/// must produce the same records — including splits inside a quoted field, a
/// CRLF pair, and an escaped quote.
#[test]
fn chunking_never_changes_the_records() {
    let text = "id,note\r\n1,\"a,b\"\"c\nd\"\r\n2,plain\r\n3,\n";
    let expected = records(text);
    for split in 0..text.len() {
        let mut parser = CsvParser::new(Dialect::default());
        let mut out = Vec::new();
        parser
            .push(&text.as_bytes()[..split], &mut out)
            .expect("first chunk");
        parser
            .push(&text.as_bytes()[split..], &mut out)
            .expect("second chunk");
        parser.finish(&mut out).expect("finish");
        assert_eq!(out, expected, "split at {split} changed the parse");
    }
}

/// Line numbers survive newlines inside quoted fields — a report that points at
/// the wrong line is worse than no report.
#[test]
fn line_numbers_count_embedded_newlines() {
    let out = records("h\n\"a\nb\nc\"\nlast\n");
    assert_eq!(out[0].line, 1);
    assert_eq!(out[1].line, 2);
    assert_eq!(out[2].line, 5, "the quoted field spanned three lines");
}

#[test]
fn hostile_input_is_refused_by_name_not_by_allocation() {
    let long = format!("{},b\n", "x".repeat(limits::MAX_FIELD_BYTES + 10));
    assert!(matches!(
        parse_all(long.as_bytes(), Dialect::default()),
        Err(CsvError::FieldTooLong { line: 1, .. })
    ));

    let wide = "a,".repeat(limits::MAX_FIELDS + 10);
    assert!(matches!(
        parse_all(wide.as_bytes(), Dialect::default()),
        Err(CsvError::TooManyFields { line: 1 })
    ));

    // An unterminated quote means every delimiter after it was misread, so the
    // records already emitted are suspect and the caller must be told.
    assert!(matches!(
        parse_all(b"a,\"never closed\n1,2\n", Dialect::default()),
        Err(CsvError::UnterminatedQuote { .. })
    ));

    assert!(matches!(
        parse_all(&[b'a', b',', 0xFF, b'\n'], Dialect::default()),
        Err(CsvError::NotUtf8 { line: 1 })
    ));
}

/// Delimiter sniffing goes by *consistency*, not frequency: counting
/// occurrences picks the comma out of a semicolon-delimited European file the
/// moment one field contains prose, which is the common case.
#[test]
fn the_delimiter_is_sniffed_by_consistency_not_by_frequency() {
    let european = "name;note\nAda;a long, comma-laden, prose-filled note\nGrace;another, one\n";
    assert_eq!(Dialect::sniff(european.as_bytes()).delimiter, b';');
    assert_eq!(Dialect::sniff(b"a,b,c\n1,2,3\n").delimiter, b',');
    assert_eq!(Dialect::sniff(b"a\tb\n1\t2\n").delimiter, b'\t');
    // A comma inside quotes must not vote.
    assert_eq!(
        Dialect::sniff(b"a|b\n\"x,y,z,w\"|2\n\"p,q,r,s\"|4\n").delimiter,
        b'|'
    );
}

// -------------------------------------------- preview before commit (docs/24)

/// **The reason this module exists.** `"1E2"` is a gene symbol; Excel makes it
/// `100`. Here it is a decision with the evidence attached, and the silent path
/// does not exist to be taken by accident.
#[test]
fn the_gene_symbol_is_a_surfaced_decision() {
    let report = infer::analyze(&records("gene,count\n1E2,5\nSEPT2,7\n2E3,9\n"), true);

    let genes = &report.columns[0];
    assert!(genes.is_contested(), "the column has a decision in it");
    assert_eq!(genes.loss_count, 2, "1E2 and 2E3 would be mangled");
    assert_eq!(genes.losses[0].loss, Loss::ScientificNotation);
    assert_eq!(genes.losses[0].original, "1E2");
    assert_eq!(genes.losses[0].as_number, "100");
    assert_eq!(genes.losses[0].line, 2, "and the report says where");
    assert_eq!(
        genes.suggested,
        Decision::Text,
        "the suggestion never loses data; Excel's answer must be asked for"
    );

    let counts = &report.columns[1];
    assert!(
        !counts.is_contested(),
        "an honest numeric column is not flagged"
    );
    assert_eq!(counts.suggested, Decision::Number);
    assert!(!report.is_unambiguous(), "so the caller must choose");

    // Committing the suggestion keeps the symbols; asking for Excel's rule
    // gives Excel's answer. Both are reachable, neither is automatic.
    let kept = infer::commit(&records("gene\n1E2\n"), true, &[Decision::Text]);
    assert_eq!(kept[0][0], Value::Text(String::from("1E2")));
    let mangled = infer::commit(&records("gene\n1E2\n"), true, &[Decision::Number]);
    assert_eq!(mangled[0][0], Value::Number(100.0));
}

#[test]
fn every_kind_of_information_loss_is_named() {
    let csv = "a\n0000123\n12.50\n1234567890123456789\n1E2\n";
    let report = infer::analyze(&records(csv), true);
    let kinds: Vec<Loss> = report.columns[0].losses.iter().map(|l| l.loss).collect();
    assert!(kinds.contains(&Loss::LeadingZeros));
    assert!(kinds.contains(&Loss::TrailingZeros));
    assert!(kinds.contains(&Loss::PrecisionBeyond15Digits));
    assert!(kinds.contains(&Loss::ScientificNotation));
}

/// The loss test is a round trip, not a pattern match: a column that really is
/// scientific notation, written the way the engine writes it, is not flagged.
/// A warning that fires on correct data is a warning users learn to dismiss.
#[test]
fn a_faithful_number_is_not_flagged_as_a_loss() {
    let report = infer::analyze(&records("v\n1\n2.5\n-3\n1000000\n"), true);
    assert_eq!(report.columns[0].loss_count, 0);
    assert_eq!(report.columns[0].suggested, Decision::Number);
    assert!(report.is_unambiguous());
}

/// Ragged rows silently shift every column after them. The report names the
/// lines rather than letting the import look successful.
#[test]
fn ragged_rows_are_reported_with_their_line_numbers() {
    let report = infer::analyze(&records("a,b,c\n1,2,3\n4,5\n6,7,8,9\n"), true);
    assert_eq!(report.ragged_rows, vec![3, 4]);
    assert!(!report.is_unambiguous());
}

#[test]
fn a_sampled_report_says_that_it_is_sampled() {
    let mut csv = String::from("v\n");
    for i in 0..(limits::INFERENCE_SAMPLE_ROWS + 50) {
        csv.push_str(&format!("{i}\n"));
    }
    let report = infer::analyze(&records(&csv), true);
    assert_eq!(report.rows_sampled, limits::INFERENCE_SAMPLE_ROWS);
    assert_eq!(report.rows_total, limits::INFERENCE_SAMPLE_ROWS + 50);
    assert!(
        report.truncated(),
        "a partial sample must never look like a whole-file guarantee"
    );
}

// ------------------------------------------------ injection (docs/24, OWASP)

#[test]
fn every_owasp_lead_character_is_classified() {
    for (text, expected) in [
        ("=1+1", Risk::FormulaLead('=')),
        ("+1", Risk::FormulaLead('+')),
        ("-1+1", Risk::FormulaLead('-')),
        ("@SUM(A1)", Risk::FormulaLead('@')),
        ("\t=1+1", Risk::ControlLead('\t')),
        ("\r=1+1", Risk::ControlLead('\r')),
    ] {
        assert_eq!(inject::classify(text), Some(expected), "{text:?}");
    }
    assert_eq!(inject::classify("ordinary"), None);
    assert_eq!(inject::classify(""), None);
}

/// **Import-side neutralization.** A field another spreadsheet would execute
/// becomes text — *whatever the column decision says*. "This column is numbers"
/// is not consent to import `=WEBSERVICE(...)`.
#[test]
fn an_imported_formula_becomes_text_whatever_the_decision_says() {
    let csv = "v\n=cmd|'/c calc'!A0\n1\n";
    let report = infer::analyze(&records(csv), true);
    assert_eq!(report.injections.len(), 1);
    assert_eq!(report.injections[0].line, 2);
    assert!(!report.is_unambiguous());

    for decision in [Decision::Number, Decision::PerCell, Decision::Text] {
        let values = infer::commit(&records(csv), true, &[decision]);
        assert_eq!(
            values[0][0],
            Value::Text(String::from("=cmd|'/c calc'!A0")),
            "{decision:?} must not produce a formula"
        );
    }
}

/// **Export-side, and the design decision that matters.** The naive OWASP rule
/// prefixes anything starting `-` and corrupts every negative number in the
/// file. Exporting from *typed* values means the writer never has to guess.
#[test]
fn a_negative_number_is_not_neutralized_but_a_formula_is_text() {
    let rows = vec![vec![
        Value::Number(-1.0),
        Value::Text(String::from("-1+1")),
        Value::Text(String::from("=HYPERLINK(\"http://evil\")")),
        Value::Text(String::from("ordinary")),
    ]];
    let (csv, report) = write_csv(&rows, Dialect::default());

    assert!(csv.starts_with("-1,"), "the number survived: {csv}");
    assert!(csv.contains("'-1+1"), "the text was neutralized: {csv}");
    assert_eq!(report.neutralized.len(), 2, "and both changes are reported");
    assert!(!report.is_clean());
    assert_eq!(report.neutralized[0].column, 1);
}

/// Neutralization has an exact inverse, so a file exported and re-imported is
/// the file that went out. A security fix that quietly rewrites data on every
/// round trip is a data-loss bug wearing a badge.
#[test]
fn neutralization_round_trips_exactly() {
    for text in ["=1+1", "-1+1", "@x", "\t=1", "ordinary", "'quoted'", ""] {
        assert_eq!(
            inject::strip_neutralization(&inject::neutralize(text)),
            text,
            "{text:?} did not survive the round trip"
        );
    }
}

// ----------------------------------------------------------------- writing

#[test]
fn quoting_is_applied_only_where_the_grammar_needs_it() {
    let rows = vec![vec![
        Value::Text(String::from("plain")),
        Value::Text(String::from("has,comma")),
        Value::Text(String::from("has\"quote")),
        Value::Text(String::from("has\nnewline")),
        Value::Text(String::from(" padded ")),
    ]];
    let (csv, _) = write_csv(&rows, Dialect::default());
    assert_eq!(
        csv,
        "plain,\"has,comma\",\"has\"\"quote\",\"has\nnewline\",\" padded \"\n"
    );
}

#[test]
fn values_render_the_way_a_reader_will_recognise_them() {
    assert_eq!(render(&Value::Blank), "");
    assert_eq!(render(&Value::Bool(true)), "TRUE");
    assert_eq!(render(&Value::Number(1.0)), "1");
    assert_eq!(render(&Value::Number(2.5)), "2.5");
    assert_eq!(render(&Value::Number(f64::INFINITY)), "#NUM!");
    assert_eq!(
        render(&Value::Error(CellError::new(
            ErrorKind::Div0,
            Origin::Authored
        ))),
        "#DIV/0!"
    );
}

/// Values → CSV → records → values, with the decisions the report suggested.
/// This is the property an import/export pair has to have and the one a corpus
/// test would otherwise only sample.
#[test]
fn a_clean_table_survives_a_full_round_trip() {
    let rows = vec![
        vec![
            Value::Text(String::from("id")),
            Value::Text(String::from("label")),
            Value::Text(String::from("qty")),
        ],
        vec![
            Value::Number(1.0),
            Value::Text(String::from("a,b")),
            Value::Number(2.5),
        ],
        vec![
            Value::Number(2.0),
            Value::Text(String::from("say \"hi\"")),
            Value::Number(-3.0),
        ],
    ];
    let (csv, export) = write_csv(&rows, Dialect::default());
    assert!(export.is_clean());

    let back = records(&csv);
    let report = infer::analyze(&back, true);
    assert!(report.is_unambiguous(), "{report:?}");
    let values = infer::commit(&back, true, &report.suggestions());
    assert_eq!(values, rows[1..].to_vec());
}
