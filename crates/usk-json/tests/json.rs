//! JSON conformance for the in-house reader (D-083).
//!
//! Writing a parser instead of taking `serde_json` buys dependency budget and
//! owes tests in exchange. These are the cases that actually bite: escapes,
//! surrogate pairs, number texts that must survive verbatim, and the malformed
//! inputs an agent can send an MCP server (docs/37).

use usk_json::{number, parse_str, Json, JsonErrorKind, MAX_DEPTH};

fn parse_ok(text: &str) -> Json {
    parse_str(text).unwrap_or_else(|e| panic!("{text:?} should parse: {e:?}"))
}

fn kind(text: &str) -> JsonErrorKind {
    parse_str(text).expect_err("should not parse").kind
}

#[test]
fn scalars_and_containers_round_trip() {
    for text in [
        "null",
        "true",
        "false",
        "0",
        "-1",
        "1.5",
        "1e10",
        "-2.5E-7",
        "\"\"",
        "\"abc\"",
        "[]",
        "{}",
        "[1,2,3]",
        "{\"a\":1,\"b\":[true,null]}",
    ] {
        let parsed = parse_ok(text);
        assert_eq!(parsed.to_json_string(), text, "compact form is stable");
        assert_eq!(
            parse_ok(&parsed.to_json_pretty()),
            parsed,
            "the pretty form parses back to the same value"
        );
    }
}

/// The reason [`Json::Number`] holds text: docs/50 measured a JSON reader
/// moving `0.10000000000000009` by one ULP, which produced six false failures
/// in the capture harness's own validator.
#[test]
fn a_number_keeps_its_source_text_exactly() {
    for text in [
        "0.10000000000000009",
        "0.30000000000000004",
        "1.7976931348623157e+308",
        "4.94065645841247E-324",
        "9007199254740993",
    ] {
        let parsed = parse_ok(text);
        assert_eq!(
            parsed.as_number_text(),
            Some(text),
            "the literal survives the parse"
        );
        assert_eq!(
            parsed.as_f64(),
            text.parse::<f64>().ok(),
            "and parses through core, which is correctly rounded"
        );
    }
}

#[test]
fn escapes_decode_and_re_encode() {
    let parsed = parse_ok(r#""a\"b\\c\/d\be\ff\ng\rh\tiAé""#);
    assert_eq!(
        parsed.as_str(),
        Some("a\"b\\c/d\u{08}e\u{0C}f\ng\rh\ti\u{41}\u{e9}")
    );
    // Re-encoding is canonical, not verbatim: `\/` and `A` have shorter
    // spellings and the writer uses them.
    assert_eq!(
        parse_ok(&parsed.to_json_string()),
        parsed,
        "canonicalised output still means the same string"
    );
}

/// docs/50 finding 5: the corpus contains astral characters, so surrogate
/// pairs are an exercised path, not a theoretical one.
#[test]
fn a_surrogate_pair_decodes_to_one_scalar() {
    let parsed = parse_ok(r#""😀""#);
    let s = parsed.as_str().expect("string");
    assert_eq!(s.chars().count(), 1, "one scalar, not two code units");
    assert_eq!(s, "\u{1F600}");
    assert_eq!(parse_ok(&parsed.to_json_string()).as_str(), Some(s));
}

#[test]
fn an_unpaired_surrogate_is_an_error_not_a_replacement_character() {
    assert_eq!(kind(r#""\uD83D""#), JsonErrorKind::UnpairedSurrogate);
    assert_eq!(kind(r#""\uD83Dx""#), JsonErrorKind::UnpairedSurrogate);
    assert_eq!(kind(r#""\uD83DA""#), JsonErrorKind::UnpairedSurrogate);
    assert_eq!(kind(r#""\uDE00""#), JsonErrorKind::UnpairedSurrogate);
}

#[test]
fn malformed_documents_are_named_errors() {
    assert_eq!(kind(""), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind("{"), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind("[1,"), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind("01"), JsonErrorKind::TrailingContent);
    assert_eq!(kind("1."), JsonErrorKind::BadNumber);
    assert_eq!(kind("-"), JsonErrorKind::BadNumber);
    assert_eq!(kind("1e"), JsonErrorKind::BadNumber);
    assert_eq!(kind("{\"a\"}"), JsonErrorKind::UnexpectedByte(b'}'));
    assert_eq!(kind("[1] [2]"), JsonErrorKind::TrailingContent);
    assert_eq!(kind("\"a\nb\""), JsonErrorKind::ControlCharacterInString);
    assert_eq!(kind(r#""\x""#), JsonErrorKind::BadEscape);
    assert_eq!(kind(r#""\uZZZZ""#), JsonErrorKind::BadUnicodeEscape);
}

/// Recursion over untrusted input is bounded, or an agent can end the process
/// with one line of JSON (docs/37).
#[test]
fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
    let deep = "[".repeat(MAX_DEPTH + 5) + &"]".repeat(MAX_DEPTH + 5);
    assert_eq!(kind(&deep), JsonErrorKind::DepthExceeded);

    let fine = "[".repeat(MAX_DEPTH - 1) + &"]".repeat(MAX_DEPTH - 1);
    assert!(
        parse_str(&fine).is_ok(),
        "just inside the bound still parses"
    );
}

#[test]
fn objects_keep_their_order_and_answer_lookups() {
    let parsed = parse_ok(r#"{"z":1,"a":2,"z":3}"#);
    assert_eq!(parsed.get("a").and_then(Json::as_f64), Some(2.0));
    assert_eq!(
        parsed.get("z").and_then(Json::as_f64),
        Some(1.0),
        "duplicate keys are legal JSON; first wins, as every reader does"
    );
    assert_eq!(parsed.to_json_string(), r#"{"z":1,"a":2,"z":3}"#);
    assert!(parsed.get("missing").is_none());
}

#[test]
fn writing_a_float_produces_a_number_that_reads_back_identically() {
    for v in [0.1, 1.0 / 3.0, -2.5e-17, f64::MAX, f64::MIN_POSITIVE] {
        let written = number(v);
        assert_eq!(written.as_f64(), Some(v), "shortest round-trip form");
        assert_eq!(parse_ok(&written.to_json_string()).as_f64(), Some(v));
    }
    // Non-finite values have no JSON spelling. The choice is made explicitly
    // rather than by whatever the formatter happens to emit.
    assert!(number(f64::NAN).is_null());
    assert!(number(f64::INFINITY).is_null());
}
