//! Row 6 proofs: parse pipeline, evaluation, and function conformance
//! (BOOTSTRAP row 6, docs/12, ADR-011).
//!
//! **Evidence status.** docs/32 and ADR-024 make real Excel the oracle — the
//! binary is the spec, because the documentation lies. That COM capture harness
//! does not exist yet (assumption A-007), so every vector below encodes
//! *documented* Excel behaviour. They are honest tests of what this engine
//! does; they are not yet proof that Excel agrees. Anything the oracle later
//! contradicts is a bug here, not in the oracle.

use usk_formula::eval::{eval, Context, Grid, NoGrid, Operand};
use usk_formula::functions::{DateSystem, CATALOGUE};
use usk_formula::parse::{parse, Ast, A1};
use usk_formula::{evaluate, parse as parse_mod};
use usk_types::coerce::Profile;
use usk_types::{CellError, ErrorKind, Value};

/// A fixed rectangular grid, so formula behaviour is tested independently of
/// how cells are actually stored.
struct Fixture {
    rows: u32,
    cols: u32,
    cells: Vec<Value>,
}

impl Fixture {
    fn new(rows: u32, cols: u32, cells: Vec<Value>) -> Self {
        Fixture { rows, cols, cells }
    }
}

impl Grid for Fixture {
    fn read(&self, row: u32, col: u32) -> Option<Value> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.cells.get((row * self.cols + col) as usize).cloned()
    }
    fn extent(&self) -> (u32, u32) {
        (self.rows, self.cols)
    }
}

fn n(v: f64) -> Value {
    Value::Number(v)
}
fn t(s: &str) -> Value {
    Value::Text(String::from(s))
}
fn dec(s: &str) -> Value {
    Value::Decimal(usk_types::decimal::parse_decimal(s).expect("valid decimal literal"))
}

/// Evaluate with no workbook, in the given profile.
fn ev(src: &str, profile: Profile) -> Value {
    evaluate(src, &NoGrid, profile)
}

/// Evaluate against a fixture grid.
fn evg(src: &str, grid: &Fixture, profile: Profile) -> Value {
    evaluate(src, grid, profile)
}

fn kind_of(v: &Value) -> Option<ErrorKind> {
    v.as_error().map(|e| e.kind)
}

/// Evaluate under the 1904 date system (TD-33), which is a workbook property
/// rather than a profile, so it cannot go through `ev`.
fn ev_1904(src: &str) -> Value {
    let parsed = parse(src);
    let ctx = Context::new(&NoGrid, Profile::Compat).with_dates(DateSystem::Excel1904);
    eval(&parsed.ast, &ctx)
}

// ----------------------------------------------------------- the CST

/// The load-bearing CST property (ADR-011): whatever the user typed comes back
/// byte for byte, including whitespace, comments-in-spacing, and text the
/// parser could not understand. Format-preserving refactors depend on it.
#[test]
fn cst_round_trips_every_input() {
    let inputs = [
        "=1+2",
        "=  SUM( A1 : B2 , 3 )  ",
        "=IF(A1>0,\"yes\",\"no\")",
        "1 + 2",
        "=A1&\"x\"&B2",
        "=-2^2",
        "=((1))",
        "=\"unterminated",
        "=1 +* 2",
        "=@#$%",
        "",
        "=",
        "=SUM(",
    ];
    for src in inputs {
        let p = parse(src);
        assert_eq!(
            p.cst.text(src),
            src,
            "CST did not round-trip {src:?} — a lossless tree that loses bytes is not lossless"
        );
    }
}

/// The CST keeps spans, so an error can point at a column rather than at the
/// whole formula.
#[test]
fn cst_spans_locate_subexpressions() {
    let src = "=1+22";
    let p = parse(src);
    let (start, end) = p.cst.span().expect("root has a span");
    assert_eq!((start, end), (0, src.len() as u32));
}

// --------------------------------------------------------- precedence

/// Excel's precedence, which is not mathematics'. `-2^2` is `4` in a
/// spreadsheet because unary minus binds tighter than `^`. Reproducing this is
/// compatibility (docs/32); "fixing" it would break every imported model.
#[test]
fn unary_minus_binds_tighter_than_exponent() {
    assert_eq!(ev("=-2^2", Profile::Compat), n(4.0));
    assert_eq!(ev("=0-2^2", Profile::Compat), n(-4.0));
}

/// `^` is right-associative: `2^3^2` is `2^(3^2)` = 512, not `(2^3)^2` = 64.
#[test]
fn exponent_is_right_associative() {
    assert_eq!(ev("=2^3^2", Profile::Compat), n(512.0));
}

#[test]
fn operator_precedence_follows_excel() {
    assert_eq!(ev("=1+2*3", Profile::Compat), n(7.0));
    assert_eq!(ev("=(1+2)*3", Profile::Compat), n(9.0));
    // Comparison is loosest, so this is (1+1) = 2, i.e. TRUE.
    assert_eq!(ev("=1+1=2", Profile::Compat), Value::Bool(true));
    // Concat binds looser than arithmetic but tighter than comparison.
    assert_eq!(ev("=\"a\"&1+1", Profile::Compat), t("a2"));
    // Postfix % divides by 100 and binds tightest.
    assert_eq!(ev("=50%", Profile::Compat), n(0.5));
    assert_eq!(ev("=1+50%", Profile::Compat), n(1.5));
}

// ------------------------------------------------------------ parsing

#[test]
fn literals_and_references_parse() {
    assert!(matches!(parse("=1.5").ast, Ast::Literal(Value::Number(_))));
    assert!(matches!(parse("=\"hi\"").ast, Ast::Literal(Value::Text(_))));
    assert!(matches!(
        parse("=TRUE").ast,
        Ast::Literal(Value::Bool(true))
    ));
    assert!(matches!(parse("=#N/A").ast, Ast::Literal(Value::Error(_))));
    assert!(matches!(parse("=A1").ast, Ast::Reference(_)));
    assert!(matches!(
        parse("=$B$7").ast,
        Ast::Range(..) | Ast::Reference(_)
    ));
    assert!(matches!(parse("=A1:B2").ast, Ast::Range(..)));
}

/// A1 is a *view* over identities (DP-A6), and column letters are bijective
/// base-26, so AA is 26 — not 27, and not 0.
#[test]
fn a1_parses_bijective_base_26_columns() {
    let cases = [
        ("A1", 0u32, 0u32),
        ("Z1", 25, 0),
        ("AA1", 26, 0),
        ("B10", 1, 9),
    ];
    for (text, col, row) in cases {
        let r = parse_mod::parse_a1(text).unwrap_or_else(|| panic!("{text} should parse"));
        assert_eq!((r.col, r.row), (col, row), "{text}");
    }
    assert_eq!(
        parse_mod::parse_a1("$A$1"),
        Some(A1 {
            col: 0,
            row: 0,
            col_absolute: true,
            row_absolute: true
        })
    );
    // Row 0 does not exist in A1 notation.
    assert_eq!(parse_mod::parse_a1("A0"), None);
    assert_eq!(parse_mod::parse_a1("1A"), None);
}

/// Escaped quotes inside string literals.
#[test]
fn string_literals_unescape_doubled_quotes() {
    assert_eq!(ev("=\"say \"\"hi\"\"\"", Profile::Compat), t("say \"hi\""));
}

/// A malformed formula is an error *value*, never a panic (DP-A10).
#[test]
fn malformed_formulas_produce_error_values_not_panics() {
    for src in ["=1 +* 2", "=SUM(", "=", "=@#$", "=((1)", "=1 2"] {
        let v = ev(src, Profile::Compat);
        assert!(
            v.as_error().is_some(),
            "{src:?} should evaluate to an error value, got {v:?}"
        );
    }
}

// -------------------------------------------------------- aggregation

#[test]
fn sum_and_friends_aggregate_ranges() {
    let g = Fixture::new(
        3,
        2,
        alloc_cells(&[n(1.0), n(2.0), n(3.0), t("x"), n(4.0), Value::Blank]),
    );
    assert_eq!(evg("=SUM(A1:B3)", &g, Profile::Compat), dec("10"));
    // COUNT counts numbers; COUNTA counts non-blanks; COUNTBLANK counts blanks.
    assert_eq!(evg("=COUNT(A1:B3)", &g, Profile::Compat), n(4.0));
    assert_eq!(evg("=COUNTA(A1:B3)", &g, Profile::Compat), n(5.0));
    assert_eq!(evg("=COUNTBLANK(A1:B3)", &g, Profile::Compat), n(1.0));
    assert_eq!(evg("=MIN(A1:B3)", &g, Profile::Compat), n(1.0));
    assert_eq!(evg("=MAX(A1:B3)", &g, Profile::Compat), n(4.0));
    assert_eq!(evg("=AVERAGE(A1:B3)", &g, Profile::Compat), dec("2.5"));
}

/// `SUM` over exact values stays exact. A currency column that silently became
/// binary floating point would defeat the point of the `Decimal` domain.
#[test]
fn sum_of_decimals_stays_exact() {
    let cells: Vec<Value> = (0..100).map(|_| dec("0.01")).collect();
    let g = Fixture::new(100, 1, cells);
    assert_eq!(evg("=SUM(A1:A100)", &g, Profile::Strict), dec("1"));

    // Mixed with an inexact float, it honestly falls back to the float domain.
    let g2 = Fixture::new(2, 1, alloc_cells(&[dec("0.01"), n(0.1)]));
    let mixed = evg("=SUM(A1:A2)", &g2, Profile::Strict);
    assert!(
        matches!(mixed, Value::Number(_)),
        "must not claim exactness it does not have, got {mixed:?}"
    );
}

#[test]
fn text_and_blanks_are_skipped_by_numeric_aggregation() {
    let g = Fixture::new(1, 3, alloc_cells(&[n(10.0), t("ignored"), Value::Blank]));
    assert_eq!(evg("=SUM(A1:C1)", &g, Profile::Compat), dec("10"));
    assert_eq!(evg("=COUNT(A1:C1)", &g, Profile::Compat), n(1.0));
}

// ------------------------------------------------------------- logic

/// `IF` must not evaluate the branch it does not take — otherwise
/// `IF(A1=0, 0, 1/A1)` would still divide by zero.
#[test]
fn if_evaluates_lazily() {
    let g = Fixture::new(1, 1, alloc_cells(&[n(0.0)]));
    assert_eq!(evg("=IF(A1=0,0,1/A1)", &g, Profile::Compat), n(0.0));
}

#[test]
fn logical_functions_follow_excel() {
    assert_eq!(ev("=AND(TRUE,TRUE)", Profile::Compat), Value::Bool(true));
    assert_eq!(ev("=AND(TRUE,FALSE)", Profile::Compat), Value::Bool(false));
    assert_eq!(ev("=OR(FALSE,TRUE)", Profile::Compat), Value::Bool(true));
    assert_eq!(ev("=NOT(TRUE)", Profile::Compat), Value::Bool(false));
    assert_eq!(ev("=XOR(TRUE,TRUE)", Profile::Compat), Value::Bool(false));
    assert_eq!(ev("=IFS(FALSE,1,TRUE,2)", Profile::Compat), n(2.0));
    // IFS with no matching condition is #N/A, not blank.
    assert_eq!(
        kind_of(&ev("=IFS(FALSE,1)", Profile::Compat)),
        Some(ErrorKind::Na)
    );
}

// ------------------------------------------------------------ errors

/// Errors propagate through functions carrying their original origin, so a
/// `#VALUE!` five calls deep still knows it began as a refused coercion.
#[test]
fn errors_propagate_with_origin_intact() {
    let refused = Value::Error(CellError::refused_coercion(
        usk_types::TypeTag::Text,
        usk_types::TypeTag::Number,
    ));
    let g = Fixture::new(1, 2, alloc_cells(&[refused.clone(), n(1.0)]));
    let result = evg("=SUM(A1:B1)+1", &g, Profile::Strict);
    assert_eq!(result, refused, "origin must survive the whole chain");
}

#[test]
fn iferror_catches_and_ifna_is_narrower() {
    assert_eq!(ev("=IFERROR(1/0,\"caught\")", Profile::Compat), t("caught"));
    assert_eq!(ev("=IFERROR(2,\"caught\")", Profile::Compat), n(2.0));
    assert_eq!(ev("=IFNA(NA(),\"caught\")", Profile::Compat), t("caught"));
    // IFNA must NOT catch a #DIV/0!.
    assert_eq!(
        kind_of(&ev("=IFNA(1/0,\"caught\")", Profile::Compat)),
        Some(ErrorKind::Div0)
    );
}

#[test]
fn type_predicates_classify_values() {
    assert_eq!(ev("=ISERROR(1/0)", Profile::Compat), Value::Bool(true));
    assert_eq!(ev("=ISERROR(1)", Profile::Compat), Value::Bool(false));
    assert_eq!(ev("=ISNA(NA())", Profile::Compat), Value::Bool(true));
    assert_eq!(ev("=ISNA(1/0)", Profile::Compat), Value::Bool(false));
    assert_eq!(ev("=ISNUMBER(1)", Profile::Compat), Value::Bool(true));
    assert_eq!(ev("=ISTEXT(\"a\")", Profile::Compat), Value::Bool(true));
    assert_eq!(ev("=ISLOGICAL(TRUE)", Profile::Compat), Value::Bool(true));
}

#[test]
fn division_by_zero_is_an_error_value() {
    assert_eq!(kind_of(&ev("=1/0", Profile::Compat)), Some(ErrorKind::Div0));
    assert_eq!(
        kind_of(&ev("=MOD(1,0)", Profile::Compat)),
        Some(ErrorKind::Div0)
    );
    // An unknown function name is #NAME?, not a crash.
    assert_eq!(
        kind_of(&ev("=NOSUCHFUNC(1)", Profile::Compat)),
        Some(ErrorKind::Name)
    );
    // A reference outside the grid is #REF!.
    let g = Fixture::new(1, 1, alloc_cells(&[n(1.0)]));
    assert_eq!(
        kind_of(&evg("=Z99", &g, Profile::Compat)),
        Some(ErrorKind::Ref)
    );
}

// -------------------------------------------------------------- text

#[test]
fn text_functions_follow_excel_semantics() {
    assert_eq!(ev("=LEFT(\"abcdef\",3)", Profile::Compat), t("abc"));
    assert_eq!(ev("=RIGHT(\"abcdef\",2)", Profile::Compat), t("ef"));
    // MID is 1-based.
    assert_eq!(ev("=MID(\"abcdef\",2,3)", Profile::Compat), t("bcd"));
    assert_eq!(ev("=LEN(\"abc\")", Profile::Compat), n(3.0));
    // TRIM collapses internal runs as well as trimming the ends.
    assert_eq!(ev("=TRIM(\"  a   b  \")", Profile::Compat), t("a b"));
    assert_eq!(ev("=UPPER(\"aB\")", Profile::Compat), t("AB"));
    assert_eq!(ev("=LOWER(\"aB\")", Profile::Compat), t("ab"));
    assert_eq!(
        ev("=PROPER(\"hello wORLD\")", Profile::Compat),
        t("Hello World")
    );
    assert_eq!(ev("=CONCAT(\"a\",1,TRUE)", Profile::Compat), t("a1TRUE"));
    assert_eq!(ev("=REPT(\"ab\",3)", Profile::Compat), t("ababab"));
    assert_eq!(
        ev("=SUBSTITUTE(\"a-b-c\",\"-\",\"+\")", Profile::Compat),
        t("a+b+c")
    );
    assert_eq!(
        ev("=SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)", Profile::Compat),
        t("a-b+c")
    );
    assert_eq!(
        ev("=REPLACE(\"abcdef\",2,3,\"X\")", Profile::Compat),
        t("aXef")
    );
    assert_eq!(
        ev("=TEXTJOIN(\",\",TRUE,\"a\",\"b\")", Profile::Compat),
        t("a,b")
    );
}

/// `FIND` is case-sensitive and `SEARCH` is not — the documented difference.
#[test]
fn find_is_case_sensitive_and_search_is_not() {
    assert_eq!(ev("=SEARCH(\"B\",\"abc\")", Profile::Compat), n(2.0));
    assert_eq!(
        kind_of(&ev("=FIND(\"B\",\"abc\")", Profile::Compat)),
        Some(ErrorKind::Value)
    );
    assert_eq!(ev("=FIND(\"b\",\"abc\")", Profile::Compat), n(2.0));
    assert_eq!(
        ev("=EXACT(\"a\",\"A\")", Profile::Compat),
        Value::Bool(false)
    );
}

/// `VALUE` is an explicit conversion, so it applies even under `strict` — the
/// user named it. That is the line between a silent coercion and a stated one.
#[test]
fn value_converts_explicitly_even_under_strict() {
    assert_eq!(ev("=VALUE(\"42\")", Profile::Strict), n(42.0));
    // Bare text arithmetic still refuses under strict.
    assert_eq!(
        kind_of(&ev("=\"42\"+1", Profile::Strict)),
        Some(ErrorKind::Value)
    );
    // ...and still succeeds under compat.
    assert_eq!(ev("=\"42\"+1", Profile::Compat), n(43.0));
}

// ------------------------------------------------------------ lookup

fn lookup_grid() -> Fixture {
    Fixture::new(
        3,
        3,
        alloc_cells(&[
            t("apple"),
            n(10.0),
            t("red"),
            t("banana"),
            n(20.0),
            t("yellow"),
            t("cherry"),
            n(30.0),
            t("dark"),
        ]),
    )
}

#[test]
fn lookup_functions_find_and_report_misses() {
    let g = lookup_grid();
    assert_eq!(
        evg("=VLOOKUP(\"banana\",A1:C3,2)", &g, Profile::Compat),
        n(20.0)
    );
    assert_eq!(
        evg("=VLOOKUP(\"banana\",A1:C3,3)", &g, Profile::Compat),
        t("yellow")
    );
    // A miss is #N/A **under exact match**, and lookups are case-insensitive
    // like Excel's. The 4th argument has to be spelled out: Excel's default is
    // the *approximate* match (TD-14), under which "durian" is past the end of
    // the key column and returns the last row rather than failing.
    assert_eq!(
        kind_of(&evg(
            "=VLOOKUP(\"durian\",A1:C3,2,FALSE)",
            &g,
            Profile::Compat
        )),
        Some(ErrorKind::Na)
    );
    assert_eq!(
        evg("=VLOOKUP(\"durian\",A1:C3,2,TRUE)", &g, Profile::Compat),
        n(30.0)
    );
    assert_eq!(
        evg("=VLOOKUP(\"BANANA\",A1:C3,2)", &g, Profile::Compat),
        n(20.0)
    );

    assert_eq!(
        evg("=MATCH(\"cherry\",A1:A3,0)", &g, Profile::Compat),
        n(3.0)
    );
    assert_eq!(evg("=INDEX(A1:C3,2,3)", &g, Profile::Compat), t("yellow"));
    assert_eq!(evg("=INDEX(A1:A3,2)", &g, Profile::Compat), t("banana"));
    // XLOOKUP's fourth argument is the not-found fallback.
    assert_eq!(
        evg(
            "=XLOOKUP(\"durian\",A1:A3,B1:B3,\"none\")",
            &g,
            Profile::Compat
        ),
        t("none")
    );
    assert_eq!(
        evg("=XLOOKUP(\"apple\",A1:A3,C1:C3)", &g, Profile::Compat),
        t("red")
    );
}

// The lookup tests below are TD-14 and TD-35. Every expected value is what real
// Excel 16.0 returned over COM (ADR-024), from
// `tools/oracle-capture/vectors/{VLOOKUP,MATCH,XLOOKUP,SEARCH}.json`.

/// The key column the oracle used, deliberately *not* sorted, because the
/// unsorted case is the one that distinguishes Excel's algorithm from a
/// plausible one. Keys `30, 10, 50, 10, blank`; values name their row.
fn unsorted_key_grid() -> Fixture {
    Fixture::new(
        5,
        2,
        alloc_cells(&[
            n(30.0),
            t("c-thirty"),
            n(10.0),
            t("c-ten-first"),
            n(50.0),
            t("c-fifty"),
            n(10.0),
            t("c-ten-second"),
            Value::Blank,
            t("c-blank"),
        ]),
    )
}

/// A sorted ascending key column, `10..50` by tens, with names in column B.
fn sorted_key_grid() -> Fixture {
    Fixture::new(
        5,
        2,
        alloc_cells(&[
            n(10.0),
            t("ten"),
            n(20.0),
            t("twenty"),
            n(30.0),
            t("thirty"),
            n(40.0),
            t("forty"),
            n(50.0),
            t("fifty"),
        ]),
    )
}

/// TD-14. Excel's approximate match is a **binary search**, and the difference
/// from "scan for the largest key below the needle" is visible — which is why
/// this could only be implemented once the oracle existed.
#[test]
fn approximate_lookup_is_the_binary_search_excel_actually_runs() {
    let sorted = sorted_key_grid();
    // TRUE is the default, so these two must agree.
    assert_eq!(
        evg("=VLOOKUP(35,A1:B5,2,TRUE)", &sorted, Profile::Compat),
        t("thirty")
    );
    assert_eq!(
        evg("=VLOOKUP(35,A1:B5,2)", &sorted, Profile::Compat),
        t("thirty")
    );
    // Past the end takes the last key; below the first is #N/A, not the first.
    assert_eq!(
        evg("=VLOOKUP(99,A1:B5,2,TRUE)", &sorted, Profile::Compat),
        t("fifty")
    );
    assert_eq!(
        kind_of(&evg("=VLOOKUP(5,A1:B5,2,TRUE)", &sorted, Profile::Compat)),
        Some(ErrorKind::Na)
    );

    // THE case that pins the algorithm. Over the unsorted keys 30,10,50,10,
    // Excel answers the row holding **10**, because the search probes the
    // middle, finds 50 above the needle and halves downward. A linear "largest
    // key <= 35" would answer 30 — defensible, and not what Excel does.
    let unsorted = unsorted_key_grid();
    assert_eq!(
        evg("=VLOOKUP(35,A1:B5,2,TRUE)", &unsorted, Profile::Compat),
        t("c-ten-first")
    );
}

/// `MATCH`'s three match types, and its refusal of a two-dimensional range.
#[test]
fn match_types_and_the_shape_match_wants() {
    let g = sorted_key_grid();
    assert_eq!(evg("=MATCH(35,A1:A5,1)", &g, Profile::Compat), n(3.0));
    // Type 1 is the default.
    assert_eq!(evg("=MATCH(35,A1:A5)", &g, Profile::Compat), n(3.0));
    assert_eq!(evg("=MATCH(99,A1:A5,1)", &g, Profile::Compat), n(5.0));
    assert_eq!(evg("=MATCH(30,A1:A5,0)", &g, Profile::Compat), n(3.0));
    // A range that is neither a row nor a column is #N/A — scanning it in
    // row-major order would return a confident, wrong ordinal instead.
    assert_eq!(
        kind_of(&evg("=MATCH(30,A1:B5,0)", &g, Profile::Compat)),
        Some(ErrorKind::Na)
    );
}

/// `XLOOKUP`'s match and search modes. Unlike `VLOOKUP`, the nearest-neighbour
/// modes take the best candidate anywhere in the vector, which is what makes
/// XLOOKUP safe on unsorted data.
#[test]
fn xlookup_match_and_search_modes() {
    let g = sorted_key_grid();
    assert_eq!(
        evg("=XLOOKUP(35,A1:A5,B1:B5,\"none\",-1)", &g, Profile::Compat),
        t("thirty")
    );
    assert_eq!(
        evg("=XLOOKUP(35,A1:A5,B1:B5,\"none\",1)", &g, Profile::Compat),
        t("forty")
    );
    assert_eq!(
        evg("=XLOOKUP(99,A1:A5,B1:B5,\"none\",-1)", &g, Profile::Compat),
        t("fifty")
    );
    // Search mode -1 walks from the end, so a duplicated key resolves to the
    // *last* one rather than the first.
    let dup = unsorted_key_grid();
    assert_eq!(
        evg(
            "=XLOOKUP(10,A1:A5,B1:B5,\"none\",0,-1)",
            &dup,
            Profile::Compat
        ),
        t("c-ten-second")
    );
    assert_eq!(
        evg("=XLOOKUP(10,A1:A5,B1:B5,\"none\",0)", &dup, Profile::Compat),
        t("c-ten-first")
    );
    // Mismatched vector lengths are refused, not padded with blanks.
    assert_eq!(
        kind_of(&evg("=XLOOKUP(30,A1:A5,B1:B4)", &g, Profile::Compat)),
        Some(ErrorKind::Value)
    );
}

/// TD-35: `*`, `?`, and `~` as the escape. The escape matters twice — it turns
/// a pattern into a literal, and that literal still has to have its tildes
/// removed before it is compared.
#[test]
fn wildcards_match_and_the_tilde_escapes_them() {
    let g = Fixture::new(
        5,
        2,
        alloc_cells(&[
            t("apple"),
            n(1.0),
            t("Banana"),
            n(2.0),
            t("cherry"),
            n(3.0),
            t("7"),
            n(4.0),
            t("a*c"),
            n(5.0),
        ]),
    );
    assert_eq!(
        evg("=VLOOKUP(\"a*\",A1:B5,2,FALSE)", &g, Profile::Compat),
        n(1.0)
    );
    assert_eq!(
        evg("=VLOOKUP(\"a?ple\",A1:B5,2,FALSE)", &g, Profile::Compat),
        n(1.0)
    );
    // `a~*c` is the literal three characters `a*c`, so it finds the cell that
    // holds them — not `apple`, which a live `*` would have matched.
    assert_eq!(
        evg("=VLOOKUP(\"a~*c\",A1:B5,2,FALSE)", &g, Profile::Compat),
        n(5.0)
    );
    assert_eq!(evg("=MATCH(\"a*\",A1:A5,0)", &g, Profile::Compat), n(1.0));
    // Text and numbers never match each other, whichever side the text is on.
    assert_eq!(
        kind_of(&evg("=VLOOKUP(7,A1:B5,2,FALSE)", &g, Profile::Compat)),
        Some(ErrorKind::Na)
    );
    assert_eq!(
        evg("=VLOOKUP(\"7\",A1:B5,2,FALSE)", &g, Profile::Compat),
        n(4.0)
    );

    // SEARCH takes wildcards; FIND does not, and keeps the tilde literal.
    assert_eq!(ev("=SEARCH(\"a*c\",\"xxabcyy\")", Profile::Compat), n(3.0));
    assert_eq!(ev("=SEARCH(\"~*\",\"a*b\")", Profile::Compat), n(2.0));
    assert_eq!(
        kind_of(&ev("=FIND(\"a*c\",\"xxabcyy\")", Profile::Compat)),
        Some(ErrorKind::Value)
    );
}

/// TD-34: the criteria sub-language is **not** the lookup one, and the corpus
/// is what says so. Wildcards work in both; coercion does not.
#[test]
fn criteria_take_wildcards_and_coerce_across_the_text_boundary() {
    let g = Fixture::new(
        5,
        2,
        alloc_cells(&[
            t("apple"),
            n(1.0),
            t("Banana"),
            n(2.0),
            t("cherry"),
            n(3.0),
            t("7"),
            n(4.0),
            t("a*c"),
            n(5.0),
        ]),
    );
    assert_eq!(evg("=COUNTIF(A1:A5,\"a*\")", &g, Profile::Compat), n(2.0));
    assert_eq!(evg("=COUNTIF(A1:A5,\"*a*\")", &g, Profile::Compat), n(3.0));
    assert_eq!(
        evg("=COUNTIF(A1:A5,\"?anana\")", &g, Profile::Compat),
        n(1.0)
    );
    assert_eq!(evg("=COUNTIF(A1:A5,\"a~*c\")", &g, Profile::Compat), n(1.0));
    // A bare `*` counts the text cells and nothing else.
    assert_eq!(evg("=COUNTIF(A1:A5,\"*\")", &g, Profile::Compat), n(5.0));
    // Here the two families part company: a criterion of 7 counts the cell
    // holding the *text* "7", where a lookup for 7 would not find it.
    assert_eq!(evg("=COUNTIF(A1:A5,7)", &g, Profile::Compat), n(1.0));
    assert_eq!(evg("=COUNTIF(A1:A5,\"7\")", &g, Profile::Compat), n(1.0));
    assert_eq!(
        evg("=SUMIF(A1:A5,\"a*\",B1:B5)", &g, Profile::Compat),
        dec("6")
    );
    assert_eq!(
        evg("=SUMIF(A1:A5,\"a~*c\",B1:B5)", &g, Profile::Compat),
        dec("5")
    );
}

/// `""` is the blank cell and `"<>"` is every non-blank one. Neither is a
/// comparison against the empty string, and treating them as one gets both
/// wrong in opposite directions.
#[test]
fn an_empty_criterion_means_blank_and_its_negation_means_non_blank() {
    let g = Fixture::new(
        5,
        1,
        alloc_cells(&[n(30.0), n(10.0), n(50.0), n(10.0), Value::Blank]),
    );
    assert_eq!(evg("=COUNTIF(A1:A5,\"\")", &g, Profile::Compat), n(1.0));
    assert_eq!(evg("=COUNTIF(A1:A5,\"<>\")", &g, Profile::Compat), n(4.0));
    assert_eq!(evg("=COUNTIFS(A1:A5,\"\")", &g, Profile::Compat), n(1.0));
}

/// `COUNTIFS`/`SUMIFS` refuse criteria ranges of differing shape rather than
/// zipping to the shorter one — the pairing would be silently wrong, not
/// merely short.
#[test]
fn ifs_aggregates_refuse_mismatched_criteria_ranges() {
    let g = Fixture::new(
        5,
        1,
        alloc_cells(&[n(10.0), n(20.0), n(30.0), n(40.0), n(50.0)]),
    );
    assert_eq!(
        kind_of(&evg(
            "=COUNTIFS(A1:A5,\">15\",A1:A3,\"<45\")",
            &g,
            Profile::Compat
        )),
        Some(ErrorKind::Value)
    );
}

/// A blank needle is not a value that matches other blanks: Excel reads the
/// empty cell as 0, so a column full of holes still answers #N/A.
#[test]
fn a_blank_lookup_value_matches_nothing() {
    let g = unsorted_key_grid();
    assert_eq!(
        kind_of(&evg("=MATCH(A5,A1:A5,0)", &g, Profile::Compat)),
        Some(ErrorKind::Na)
    );
}

// ------------------------------------------ conditional aggregation

#[test]
fn conditional_aggregation_parses_criteria() {
    let g = Fixture::new(
        4,
        2,
        alloc_cells(&[
            t("a"),
            n(1.0),
            t("b"),
            n(2.0),
            t("a"),
            n(3.0),
            t("c"),
            n(4.0),
        ]),
    );
    assert_eq!(
        evg("=SUMIF(A1:A4,\"a\",B1:B4)", &g, Profile::Compat),
        dec("4")
    );
    assert_eq!(evg("=COUNTIF(A1:A4,\"a\")", &g, Profile::Compat), n(2.0));
    assert_eq!(evg("=COUNTIF(B1:B4,\">2\")", &g, Profile::Compat), n(2.0));
    assert_eq!(evg("=SUMIF(B1:B4,\">=3\")", &g, Profile::Compat), dec("7"));
    assert_eq!(evg("=COUNTIF(A1:A4,\"<>a\")", &g, Profile::Compat), n(2.0));
    assert_eq!(
        evg("=AVERAGEIF(A1:A4,\"a\",B1:B4)", &g, Profile::Compat),
        dec("2")
    );
    // SUMIFS takes the sum range FIRST; COUNTIFS does not. That asymmetry is
    // Excel's, and getting it backwards is a classic bug.
    assert_eq!(
        evg("=SUMIFS(B1:B4,A1:A4,\"a\")", &g, Profile::Compat),
        dec("4")
    );
    assert_eq!(evg("=COUNTIFS(A1:A4,\"a\")", &g, Profile::Compat), n(2.0));
    assert_eq!(
        evg("=COUNTIFS(A1:A4,\"a\",B1:B4,\">1\")", &g, Profile::Compat),
        n(1.0)
    );
}

// ------------------------------------------------------------- dates

/// **The 1900 leap-year fiction.** Excel treats serial 60 as 29 February 1900,
/// a date that never existed, for Lotus 1-2-3 compatibility. `compat` must
/// reproduce it or every date before March 1900 shifts by a day against real
/// files; `strict` is free to be correct (docs/32).
#[test]
fn compat_reproduces_the_1900_leap_year_fiction() {
    assert_eq!(ev("=DAY(60)", Profile::Compat), n(29.0));
    assert_eq!(ev("=MONTH(60)", Profile::Compat), n(2.0));
    assert_eq!(ev("=YEAR(60)", Profile::Compat), n(1900.0));

    // Strict has no phantom day, so serial 60 is the real 1 March 1900.
    assert_eq!(ev("=DAY(60)", Profile::Strict), n(1.0));
    assert_eq!(ev("=MONTH(60)", Profile::Strict), n(3.0));

    // Before the phantom day the two profiles agree.
    for serial in [1, 30, 59] {
        let src = alloc::format!("=DAY({serial})");
        assert_eq!(
            ev(&src, Profile::Compat),
            ev(&src, Profile::Strict),
            "profiles should agree before the phantom day, at serial {serial}"
        );
    }
}

// The date tests below are TD-33. **Every expected value is what real Excel
// 16.0 returned over COM** (ADR-024), taken from
// `tools/oracle-capture/vectors{,-1904}/{DATE,DAY,MONTH,YEAR,WEEKDAY}.json`.
// None of them is derived from documentation, which docs/32 warns lies about
// precisely this area — and docs/50 finding 6 recorded five date rules none of
// which follows from the others.

#[test]
fn the_phantom_day_is_reachable_from_date_and_is_exactly_one_day_wide() {
    // DAY(60)=29 already proves the read direction; this is the write one.
    assert_eq!(ev("=DATE(1900,2,29)", Profile::Compat), n(60.0));
    assert_eq!(ev("=DATE(1900,2,28)", Profile::Compat), n(59.0));
    assert_eq!(ev("=DATE(1900,3,1)", Profile::Compat), n(61.0));
    // Excel's own self-contradiction: February 1900 has 29 days going in and
    // the gap between 28 Feb and 1 Mar is two days coming out.
    assert_eq!(
        ev("=DATE(1900,3,1)-DATE(1900,2,28)", Profile::Compat),
        n(2.0)
    );
}

#[test]
fn serial_zero_is_a_real_position_in_both_date_systems() {
    // 1900 calls it the 0th of January - a day number of zero, not an error
    // and not 1899-12-31.
    assert_eq!(ev("=DAY(0)", Profile::Compat), n(0.0));
    assert_eq!(ev("=MONTH(0)", Profile::Compat), n(1.0));
    assert_eq!(ev("=YEAR(0)", Profile::Compat), n(1900.0));
    // 1904 has no such fiction: serial 0 is an ordinary 1 January 1904.
    assert_eq!(ev_1904("=DAY(0)"), n(1.0));
    assert_eq!(ev_1904("=MONTH(0)"), n(1.0));
    assert_eq!(ev_1904("=YEAR(0)"), n(1904.0));
}

#[test]
fn date_arguments_roll_over_instead_of_erroring() {
    assert_eq!(ev("=DATE(2024,13,1)", Profile::Compat), n(45658.0));
    assert_eq!(ev("=DATE(2024,25,1)", Profile::Compat), n(46023.0));
    assert_eq!(ev("=DATE(2024,0,1)", Profile::Compat), n(45261.0));
    assert_eq!(ev("=DATE(2024,-1,1)", Profile::Compat), n(45231.0));
    assert_eq!(ev("=DATE(2024,1,32)", Profile::Compat), n(45323.0));
    assert_eq!(ev("=DATE(2024,2,30)", Profile::Compat), n(45352.0));
    assert_eq!(ev("=DATE(2024,1,0)", Profile::Compat), n(45291.0));
    assert_eq!(ev("=DATE(2024,1,-5)", Profile::Compat), n(45286.0));
    // 2023 is not a leap year, so 29 February rolls into March.
    assert_eq!(ev("=DATE(2023,2,29)", Profile::Compat), n(44986.0));
    // Fractional arguments truncate rather than round.
    assert_eq!(ev("=DATE(2024,1,1.9)", Profile::Compat), n(45292.0));
}

#[test]
fn a_year_below_1900_means_1900_plus_that_year() {
    // The rule that surprises people: DATE(1899,12,31) is in the *38th*
    // century, because 1899 is read as an offset rather than as a year.
    assert_eq!(ev("=DATE(0,1,1)", Profile::Compat), n(1.0));
    assert_eq!(ev("=DATE(100,1,1)", Profile::Compat), n(36526.0));
    assert_eq!(ev("=DATE(1899,1,1)", Profile::Compat), n(693598.0));
    assert_eq!(ev("=DATE(1899,12,31)", Profile::Compat), n(693962.0));
    // It applies to the argument, before month rollover — so a month of 0 in
    // year 1900 lands in December 1899 and is refused rather than rescued.
    assert_eq!(
        kind_of(&ev("=DATE(1900,0,1)", Profile::Compat)),
        Some(ErrorKind::Num)
    );
}

#[test]
fn the_serial_range_is_closed_at_both_ends() {
    assert_eq!(ev("=DATE(9999,12,31)", Profile::Compat), n(2_958_465.0));
    assert_eq!(ev_1904("=DATE(9999,12,31)"), n(2_957_003.0));
    for (src, why) in [
        ("=DATE(10000,1,1)", "a year past 9999"),
        ("=DATE(-1,1,1)", "a negative year"),
        ("=DATE(1900,1,-1)", "a day before serial 0"),
        ("=YEAR(2958466)", "a serial past 9999-12-31"),
    ] {
        assert_eq!(
            kind_of(&ev(src, Profile::Compat)),
            Some(ErrorKind::Num),
            "{why} should be #NUM!: {src}"
        );
    }
    // The lower bound is inclusive, and reachable by rollover.
    assert_eq!(ev("=DATE(1900,1,0)", Profile::Compat), n(0.0));
}

#[test]
fn weekday_return_types_follow_excels_numbering() {
    // 45292 is Monday 1 January 2024, so every return type below is the same
    // day read through a different convention.
    for (kind, expected) in [
        ("", 2.0),
        (",1", 2.0),
        (",2", 1.0),
        (",3", 0.0),
        (",11", 1.0),
        (",12", 7.0),
        (",13", 6.0),
        (",14", 5.0),
        (",15", 4.0),
        (",16", 3.0),
        (",17", 2.0),
    ] {
        let src = alloc::format!("=WEEKDAY(45292{kind})");
        assert_eq!(ev(&src, Profile::Compat), n(expected), "{src}");
    }
    // The gap in the numbering is real: 0 and 4..=10 are not return types.
    for kind in ["0", "4"] {
        let src = alloc::format!("=WEEKDAY(45292,{kind})");
        assert_eq!(
            kind_of(&ev(&src, Profile::Compat)),
            Some(ErrorKind::Num),
            "{src}"
        );
    }
    // Serial 0 has a weekday, and the two systems disagree about which,
    // because their day zeros are different real days.
    assert_eq!(ev("=WEEKDAY(0)", Profile::Compat), n(7.0));
    assert_eq!(ev_1904("=WEEKDAY(0)"), n(6.0));
}

#[test]
fn the_1904_system_is_1462_days_behind_and_carries_no_phantom() {
    // The offset is 1462, not 1461: the 1900 system counts a day that never
    // existed, so the two calendars differ by one more than the four years
    // between their epochs.
    assert_eq!(ev("=DATE(2024,1,1)", Profile::Compat), n(45292.0));
    assert_eq!(ev_1904("=DATE(2024,1,1)"), n(43830.0));
    assert_eq!(ev_1904("=YEAR(45292)"), n(2028.0));
    // Nothing before 1904-01-01 exists in it — including the phantom day and
    // every date the 1900 system's fictions were invented for.
    for src in [
        "=DATE(1900,1,1)",
        "=DATE(1900,2,29)",
        "=DATE(1900,3,1)",
        "=DATE(0,1,1)",
    ] {
        assert_eq!(kind_of(&ev_1904(src)), Some(ErrorKind::Num), "{src}");
    }
    // The year offset rule is not a 1900-system quirk; it survives the switch.
    assert_eq!(ev_1904("=DATE(1899,12,31)"), n(692_500.0));
}

#[test]
fn an_iso_date_string_is_a_date_argument() {
    // Excel coerces a date-shaped string to its serial, and the *calendar*
    // date is what survives the switch of system, not the serial.
    assert_eq!(ev("=YEAR(\"2024-03-15\")", Profile::Compat), n(2024.0));
    assert_eq!(ev("=MONTH(\"2024-03-15\")", Profile::Compat), n(3.0));
    assert_eq!(ev("=DAY(\"2024-03-15\")", Profile::Compat), n(15.0));
    assert_eq!(ev_1904("=YEAR(\"2024-03-15\")"), n(2024.0));
    assert_eq!(ev_1904("=DAY(\"2024-03-15\")"), n(15.0));
    // Text that is not a date is still a coercion failure, not a silent zero.
    assert_eq!(
        kind_of(&ev("=YEAR(\"not a date\")", Profile::Compat)),
        Some(ErrorKind::Value)
    );
}

#[test]
fn date_core_round_trips() {
    // Excel's canonical anchor: 1 January 2000 is serial 36526.
    assert_eq!(ev("=DATE(2000,1,1)", Profile::Compat), n(36526.0));
    assert_eq!(ev("=YEAR(36526)", Profile::Compat), n(2000.0));
    assert_eq!(ev("=MONTH(36526)", Profile::Compat), n(1.0));
    assert_eq!(ev("=DAY(36526)", Profile::Compat), n(1.0));
    // Serial 1 is 1 January 1900.
    assert_eq!(ev("=DATE(1900,1,1)", Profile::Compat), n(1.0));
    // Out-of-range months roll over, as Excel does.
    assert_eq!(
        ev("=DATE(2024,13,1)", Profile::Compat),
        ev("=DATE(2025,1,1)", Profile::Compat)
    );
    // A leap day that really exists.
    assert_eq!(ev("=DAY(DATE(2024,2,29))", Profile::Compat), n(29.0));
    assert_eq!(ev("=MONTH(DATE(2024,2,29))", Profile::Compat), n(2.0));
}

/// Volatiles are **materialised**, never read from a clock: DP-A2 forbids
/// ambient time in kernel paths, so a replay produces the same answer forever.
#[test]
fn volatiles_come_from_the_context_not_a_clock() {
    let grid = NoGrid;
    let mut ctx = Context::new(&grid, Profile::Compat);
    ctx.today = 45000;
    ctx.now = 45000.5;
    let today = parse("=TODAY()");
    let now = parse("=NOW()");
    assert_eq!(eval(&today.ast, &ctx), n(45000.0));
    assert_eq!(eval(&now.ast, &ctx), n(45000.5));

    // A second evaluation with the same context gives the same answer, which
    // is the property that makes deterministic replay possible.
    assert_eq!(eval(&today.ast, &ctx), n(45000.0));
}

// ------------------------------------------------------------- maths

#[test]
fn rounding_family_rounds_half_away_from_zero_like_excel() {
    assert_eq!(ev("=ROUND(2.5,0)", Profile::Compat), n(3.0));
    assert_eq!(ev("=ROUND(-2.5,0)", Profile::Compat), n(-3.0));
    assert_eq!(ev("=ROUND(1.2345,2)", Profile::Compat), n(1.23));
    assert_eq!(ev("=ROUNDUP(1.21,1)", Profile::Compat), n(1.3));
    assert_eq!(ev("=ROUNDDOWN(1.29,1)", Profile::Compat), n(1.2));
    assert_eq!(ev("=CEILING(2.1,1)", Profile::Compat), n(3.0));
    assert_eq!(ev("=FLOOR(2.9,1)", Profile::Compat), n(2.0));
    assert_eq!(ev("=INT(-1.5)", Profile::Compat), n(-2.0));
}

#[test]
fn arithmetic_functions_compute() {
    assert_eq!(ev("=ABS(-3)", Profile::Compat), n(3.0));
    assert_eq!(ev("=SIGN(-3)", Profile::Compat), n(-1.0));
    assert_eq!(ev("=SQRT(9)", Profile::Compat), n(3.0));
    assert_eq!(
        kind_of(&ev("=SQRT(-1)", Profile::Compat)),
        Some(ErrorKind::Num)
    );
    assert_eq!(ev("=POWER(2,10)", Profile::Compat), n(1024.0));
    assert_eq!(ev("=PRODUCT(2,3,4)", Profile::Compat), n(24.0));
    // Excel's MOD takes the sign of the divisor, unlike Rust's `%`.
    assert_eq!(ev("=MOD(-1,3)", Profile::Compat), n(2.0));
}

/// `^` uses an integer fast path, so whole-number exponents are exact rather
/// than approximated through `exp(y·ln x)`.
#[test]
fn integer_exponents_are_exact() {
    assert_eq!(ev("=2^10", Profile::Compat), n(1024.0));
    assert_eq!(ev("=10^3", Profile::Compat), n(1000.0));
    assert_eq!(ev("=2^-2", Profile::Compat), n(0.25));
    // Fractional exponents go through the series; check to a tolerance.
    let root = ev("=9^0.5", Profile::Compat);
    match root {
        Value::Number(v) => assert!((v - 3.0).abs() < 1e-12, "9^0.5 = {v}"),
        other => panic!("expected a number, got {other:?}"),
    }
}

// ---------------------------------------------------------- catalogue

/// BOOTSTRAP row 6 asks for 60 functions, and every name in the catalogue must
/// actually dispatch — a name that resolves to `#NAME?` would inflate the count
/// without delivering anything.
#[test]
fn catalogue_covers_the_row_6_function_list() {
    assert!(
        CATALOGUE.len() >= 60,
        "row 6 requires 60 functions, catalogue has {}",
        CATALOGUE.len()
    );

    let g = lookup_grid();
    for name in CATALOGUE {
        // Call each with a plausible argument list; we only assert that the
        // name is *known*, not what it returns.
        let src = alloc::format!("={name}(A1:A3,1,1,1)");
        let v = evg(&src, &g, Profile::Compat);
        assert_ne!(
            kind_of(&v),
            Some(ErrorKind::Name),
            "{name} is in the catalogue but does not dispatch"
        );
    }
}

/// Function names are canonical English in storage and case-insensitive on
/// input, so display localisation stays a pure view concern (docs/12).
#[test]
fn function_names_are_case_insensitive() {
    assert_eq!(ev("=sum(1,2)", Profile::Compat), dec("3"));
    assert_eq!(ev("=SuM(1,2)", Profile::Compat), dec("3"));
    assert_eq!(ev("=true", Profile::Compat), Value::Bool(true));
}

/// A range operand collapses to a scalar where a scalar is wanted, and errors
/// inside a range still propagate.
#[test]
fn ranges_and_scalars_interoperate() {
    let g = Fixture::new(1, 2, alloc_cells(&[n(5.0), n(6.0)]));
    assert_eq!(evg("=A1:B1", &g, Profile::Compat), n(5.0));
    match usk_formula::eval::eval_operand(&parse("=A1:B1").ast, &Context::new(&g, Profile::Compat))
    {
        Operand::Range { rows, cols, cells } => {
            assert_eq!((rows, cols), (1, 2));
            assert_eq!(cells, alloc_cells(&[n(5.0), n(6.0)]));
        }
        other => panic!("expected a range operand, got {other:?}"),
    }
}

fn alloc_cells(values: &[Value]) -> Vec<Value> {
    values.to_vec()
}

extern crate alloc;
