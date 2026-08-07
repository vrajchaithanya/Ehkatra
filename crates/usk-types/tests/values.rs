//! Row 5 proofs: the value lattice, exact decimals, and the compat/strict
//! coercion split (BOOTSTRAP row 5, ADR-010, ADR-035, docs/12, docs/32).
//!
//! These are conformance vectors, not smoke tests: each one pins a documented
//! behaviour, and several pin a *deliberate divergence from Excel* that only
//! counts as correct if the compat profile still reproduces Excel exactly.

use usk_types::coerce::{arith, compat_final_adjust, compat_round_15, Profile};
use usk_types::decimal::parse_decimal;
use usk_types::{ArithOp, CellError, Decimal, ErrorKind, Origin, TypeTag, Value};

fn dec(s: &str) -> Decimal {
    match parse_decimal(s) {
        Some(d) => d,
        None => panic!("test vector {s:?} is not a valid decimal literal"),
    }
}

// ---------------------------------------------------------------- exactness

/// The headline claim: `0.1 + 0.2` is exactly `0.3`, which `f64` cannot do.
/// Differentiator #7 lives or dies on this line.
#[test]
fn decimal_addition_is_exact_where_f64_is_not() {
    let sum = dec("0.1").add(&dec("0.2"));
    assert_eq!(sum, Some(dec("0.3")));

    // The same sum in the Number domain is *not* 0.3 — this is what we fix.
    assert_ne!(0.1_f64 + 0.2_f64, 0.3_f64);
}

/// Cent-level reconciliation: a hundred one-cent rows sum to exactly one
/// currency unit. In `f64` this drifts, which is the phantom-penny bug that
/// finance teams chase through Excel models.
#[test]
fn cent_reconciliation_has_no_phantom_pennies() {
    let mut exact = Decimal::ZERO;
    let mut float = 0.0_f64;
    for _ in 0..100 {
        exact = exact.add(&dec("0.01")).expect("no overflow at this scale");
        float += 0.01;
    }
    assert_eq!(exact, Decimal::ONE);
    assert_eq!(exact.to_string(), "1");
    assert_ne!(
        float, 1.0,
        "f64 accumulation drifts — the bug we are fixing"
    );
}

/// One value, one representation (DP-A4): trailing zeros carry no identity, so
/// `1.50` and `1.5` are the same value *and* encode to the same bytes.
#[test]
fn trailing_zeros_do_not_create_a_second_representation() {
    let a = dec("1.50");
    let b = dec("1.5");
    assert_eq!(a, b);
    assert_eq!(a.coefficient(), b.coefficient());
    assert_eq!(a.exponent(), b.exponent());

    let (mut ea, mut eb) = (Vec::new(), Vec::new());
    Value::Decimal(a).encode_into(&mut ea);
    Value::Decimal(b).encode_into(&mut eb);
    assert_eq!(ea, eb, "two encodings of one value would break DP-A4");

    // Zero is canonical regardless of how it was written.
    assert_eq!(dec("0.000"), Decimal::ZERO);
    assert_eq!(dec("-0.0"), Decimal::ZERO);
}

/// Comparison is exact and never routes through `f64`, including across wildly
/// different exponents where alignment would overflow.
#[test]
fn decimal_comparison_is_exact() {
    assert!(dec("0.1") < dec("0.2"));
    assert!(dec("-5") < dec("0.0001"));
    assert_eq!(dec("1.50"), dec("1.5"));
    assert!(dec("10").compare(&dec("9.999999")).is_gt());

    // Magnitudes too far apart to align in 38 digits still order correctly.
    let huge = Decimal::new(i128::MAX / 2, 30);
    let tiny = Decimal::new(1, -30);
    assert!(huge > tiny);
    assert!(tiny.compare(&huge).is_lt());

    // Sign dominates, and negatives order by magnitude the right way round.
    let huge_neg = huge.neg().expect("negatable");
    assert!(huge_neg < tiny);
    assert!(huge_neg < dec("-1"));
}

/// Division rounds half-even, not half-up: repeated half-up rounding biases
/// sums upward, which is the drift this type exists to remove.
#[test]
fn division_rounds_half_even() {
    // Terminating divisions stay exact.
    assert_eq!(dec("1").div(&dec("4")), Some(dec("0.25")));
    assert_eq!(dec("10").div(&dec("2")), Some(dec("5")));

    // A repeating quotient is rounded, not wrong.
    let third = dec("1").div(&dec("3")).expect("defined");
    assert!(third < dec("0.34") && third > dec("0.33"));

    // Exact halves round to even in both directions.
    let half_up_case = dec("0.5").div(&dec("1")).expect("defined");
    assert_eq!(half_up_case, dec("0.5"));
}

/// Errors are values, never traps (DP-A10): division by zero and overflow
/// return `None` for the value layer to turn into an error.
#[test]
fn undefined_decimal_operations_return_none_rather_than_panicking() {
    assert_eq!(dec("1").div(&Decimal::ZERO), None);
    assert_eq!(Decimal::ZERO.div(&Decimal::ZERO), None);

    let near_max = Decimal::new(i128::MAX, 0);
    // Tripling cannot be absorbed by the exponent, so the coefficient overflows.
    assert_eq!(near_max.mul(&Decimal::new(3, 0)), None);
    assert_eq!(near_max.add(&near_max), None);
    assert_eq!(near_max.sub(&Decimal::new(-1, -20)), None);
}

/// Multiplying by a power of ten is *not* overflow: the exponent absorbs it.
/// This is the payoff of storing a scale rather than a fixed point, and it is
/// why the overflow vectors above have to multiply by 3.
#[test]
fn powers_of_ten_are_absorbed_by_the_exponent() {
    let near_max = Decimal::new(i128::MAX, 0);
    let scaled = near_max
        .mul(&Decimal::new(10, 0))
        .expect("exponent absorbs it");
    assert_eq!(scaled.coefficient(), i128::MAX);
    assert_eq!(scaled.exponent(), 1);
    assert!(scaled > near_max);
}

/// `f64` → `Decimal` promotes only when the float's *true* value fits. This is
/// the guard against manufacturing precision the data never had.
#[test]
fn float_to_decimal_conversion_refuses_inexact_values() {
    // Exactly representable binary fractions convert.
    assert_eq!(Decimal::try_from_f64_exact(0.5), Some(dec("0.5")));
    assert_eq!(Decimal::try_from_f64_exact(12.5), Some(dec("12.5")));
    assert_eq!(Decimal::try_from_f64_exact(-3.0), Some(dec("-3")));
    assert_eq!(Decimal::try_from_f64_exact(0.0), Some(Decimal::ZERO));

    // 0.1_f64 is really 0.1000000000000000055511151231257827..., needing 55
    // decimal digits. Claiming it is exactly 0.1 would be a lie.
    assert_eq!(Decimal::try_from_f64_exact(0.1), None);
    assert_eq!(Decimal::try_from_f64_exact(f64::NAN), None);
    assert_eq!(Decimal::try_from_f64_exact(f64::INFINITY), None);
}

/// The literal parser accepts plain decimals only. Scientific notation is
/// excluded on purpose — it is Excel's data-mangling vector, and admitting it
/// here would smuggle that coercion past the `strict` profile.
#[test]
fn decimal_parser_rejects_scientific_notation_and_junk() {
    assert_eq!(dec("-12.75").to_string(), "-12.75");
    assert_eq!(dec("+7").to_string(), "7");
    assert_eq!(dec("0.0001").to_string(), "0.0001");

    assert_eq!(parse_decimal("1E2"), None);
    assert_eq!(parse_decimal("1.2.3"), None);
    assert_eq!(parse_decimal(""), None);
    assert_eq!(parse_decimal("abc"), None);
    assert_eq!(parse_decimal("-"), None);
}

/// Display pads the fraction, so 1.05 never prints as 1.5.
#[test]
fn decimal_display_is_plain_and_padded() {
    assert_eq!(dec("1.05").to_string(), "1.05");
    assert_eq!(dec("0.001").to_string(), "0.001");
    assert_eq!(dec("-0.25").to_string(), "-0.25");
    assert_eq!(Decimal::ZERO.to_string(), "0");
    assert_eq!(Decimal::new(15, 1).to_string(), "150");
}

// ------------------------------------------------------- the Excel quirks

/// **The canonical case.** A gene symbol like `1E2` is silently converted to
/// the number 100 by Excel — the behaviour that forced the HUGO committee to
/// rename genes. `compat` must reproduce it; `strict` must refuse it.
#[test]
fn gene_symbol_survives_strict_and_is_mangled_by_compat() {
    assert_eq!(
        Profile::Compat.coerce_input("1E2"),
        Value::Number(100.0),
        "compat must reproduce Excel's mangling, quirks included"
    );
    assert_eq!(
        Profile::Strict.coerce_input("1E2"),
        Value::Text(String::from("1E2")),
        "strict must preserve the gene symbol"
    );
}

/// Leading zeros carry meaning in part codes, ZIP codes and account numbers,
/// and `compat` destroys them exactly as Excel does.
#[test]
fn leading_zeros_are_lost_in_compat_and_kept_in_strict() {
    assert_eq!(
        Profile::Compat.coerce_input("0000123"),
        Value::Number(123.0)
    );
    assert_eq!(
        Profile::Strict.coerce_input("0000123"),
        Value::Text(String::from("0000123"))
    );
}

/// Text that is not number-shaped stays text under both profiles, so `compat`
/// is not indiscriminate — it is Excel-shaped.
#[test]
fn non_numeric_text_is_untouched_by_both_profiles() {
    for input in ["SEPT1", "MARCH1", "12abc", "", "  "] {
        assert_eq!(
            Profile::Compat.coerce_input(input),
            Value::Text(String::from(input)),
            "compat should not have converted {input:?}"
        );
        assert_eq!(
            Profile::Strict.coerce_input(input),
            Value::Text(String::from(input))
        );
    }
}

/// `compat` never infers an exact `Decimal` from text: Excel has no such type,
/// so producing one would be a divergence dressed up as compatibility.
#[test]
fn compat_input_never_invents_a_decimal() {
    assert_eq!(Profile::Compat.coerce_input("12.50"), Value::Number(12.5));
}

/// Excel's cosmetic 15-significant-digit *display* rounding: `0.1 + 0.2` shows
/// as `0.3` even though the stored float is `0.30000000000000004`.
#[test]
fn compat_15_digit_rounding_is_a_display_rule() {
    let sum = 0.1_f64 + 0.2_f64;
    assert_ne!(sum, 0.3_f64, "the stored float really is not 0.3");
    assert_eq!(compat_round_15(sum), 0.3_f64, "but Excel displays 0.3");

    // It is a rounding, not a truncation, and it leaves ordinary values alone.
    assert_eq!(compat_round_15(1.0), 1.0);
    assert_eq!(compat_round_15(-2.5), -2.5);
    assert_eq!(compat_round_15(0.0), 0.0);
    assert_eq!(compat_round_15(1.0e300), 1.0e300);
}

/// The *final-operation* adjustment is the rule that actually makes
/// `=0.1+0.2-0.3` evaluate to zero: Excel zeroes a result that is vanishingly
/// small next to its operands. `strict` keeps the real difference.
#[test]
fn compat_final_adjustment_zeroes_catastrophic_cancellation() {
    let raw = 0.1_f64 + 0.2_f64 - 0.3_f64;
    assert_ne!(raw, 0.0, "the underlying f64 really is non-zero");

    assert_eq!(
        compat_final_adjust(&Profile::Compat, raw, 0.3),
        0.0,
        "compat must reproduce Excel's zeroing"
    );
    assert_eq!(
        compat_final_adjust(&Profile::Strict, raw, 0.3),
        raw,
        "strict must not discard a real difference"
    );

    // A genuinely small result is not cancellation and must survive.
    assert_eq!(
        compat_final_adjust(&Profile::Compat, 1.0e-17, 1.0e-17),
        1.0e-17
    );
}

/// Neither compat rule feeds back into stored values (docs/04): `arith`
/// returns the raw float, and rounding stays a decision of the display and
/// evaluation layers above it.
#[test]
fn compat_display_rules_never_feed_back_into_arithmetic() {
    let raw = 0.1_f64 + 0.2_f64 - 0.3_f64;
    let computed = arith(
        &Profile::Compat,
        ArithOp::Sub,
        &arith(
            &Profile::Compat,
            ArithOp::Add,
            &Value::Number(0.1),
            &Value::Number(0.2),
        ),
        &Value::Number(0.3),
    );
    assert_eq!(computed, Value::Number(raw));
}

// ------------------------------------------------------------- coercion

/// Arithmetic coercion: both profiles agree except on text.
#[test]
fn arithmetic_coercion_differs_only_on_text() {
    for profile in [Profile::Compat, Profile::Strict] {
        assert_eq!(profile.to_number(&Value::Blank), Ok(0.0));
        assert_eq!(profile.to_number(&Value::Bool(true)), Ok(1.0));
        assert_eq!(profile.to_number(&Value::Bool(false)), Ok(0.0));
        assert_eq!(profile.to_number(&Value::Number(4.5)), Ok(4.5));
        assert_eq!(profile.to_number(&Value::Decimal(dec("2.25"))), Ok(2.25));
    }
    assert_eq!(
        Profile::Compat.to_number(&Value::Text(String::from("42"))),
        Ok(42.0)
    );
    assert_eq!(
        Profile::Strict.to_number(&Value::Text(String::from("42"))),
        Err(CellError::refused_coercion(TypeTag::Text, TypeTag::Number))
    );
}

/// A refused coercion explains itself: the error names both types, which is
/// the whole point of error provenance (DP-A11).
#[test]
fn refused_coercion_names_both_types() {
    let err = Profile::Strict
        .to_number(&Value::Text(String::from("7")))
        .expect_err("strict must refuse");
    assert_eq!(err.kind, ErrorKind::Value);
    assert_eq!(
        err.origin,
        Origin::Coercion {
            from: TypeTag::Text,
            to: TypeTag::Number
        }
    );

    let dec_err = Profile::Strict
        .to_decimal(&Value::Text(String::from("7")))
        .expect_err("strict must refuse");
    assert_eq!(
        dec_err.origin,
        Origin::Coercion {
            from: TypeTag::Text,
            to: TypeTag::Decimal
        }
    );
}

// ----------------------------------------------------------- promotion

/// Mixed `Number`/`Decimal` arithmetic promotes to exact only when the float
/// operand is exactly representable; otherwise it stays in `f64` rather than
/// pretending to a precision the input never had.
#[test]
fn mixed_arithmetic_promotes_only_when_promotion_is_lossless() {
    let money = Value::Decimal(dec("10.05"));

    // 0.5 is exactly representable in binary → exact decimal result.
    let exact = arith(&Profile::Strict, ArithOp::Add, &money, &Value::Number(0.5));
    assert_eq!(exact, Value::Decimal(dec("10.55")));

    // 0.1 is not → the result honestly falls back to the float domain.
    let inexact = arith(&Profile::Strict, ArithOp::Add, &money, &Value::Number(0.1));
    assert!(
        matches!(inexact, Value::Number(_)),
        "must not manufacture exactness, got {inexact:?}"
    );

    // Two decimals always stay exact.
    assert_eq!(
        arith(
            &Profile::Strict,
            ArithOp::Mul,
            &Value::Decimal(dec("1.1")),
            &Value::Decimal(dec("1.1"))
        ),
        Value::Decimal(dec("1.21"))
    );

    // Two numbers stay in the float domain — no surprise promotion.
    assert_eq!(
        arith(
            &Profile::Strict,
            ArithOp::Add,
            &Value::Number(1.5),
            &Value::Number(2.5)
        ),
        Value::Number(4.0)
    );
}

/// Undefined arithmetic yields an error value that names the operation.
#[test]
fn undefined_arithmetic_produces_explained_errors() {
    let div0 = arith(
        &Profile::Strict,
        ArithOp::Div,
        &Value::Decimal(Decimal::ONE),
        &Value::Decimal(Decimal::ZERO),
    );
    assert_eq!(
        div0,
        Value::Error(CellError::new(
            ErrorKind::Div0,
            Origin::Arithmetic { op: ArithOp::Div }
        ))
    );

    // The float path reports the same way.
    let float_div0 = arith(
        &Profile::Strict,
        ArithOp::Div,
        &Value::Number(1.0),
        &Value::Number(0.0),
    );
    assert_eq!(float_div0.as_error().map(|e| e.kind), Some(ErrorKind::Div0));

    // Exceeding 38 digits is #NUM!, not a wrong answer and not a panic.
    let overflow = arith(
        &Profile::Strict,
        ArithOp::Mul,
        &Value::Decimal(Decimal::new(i128::MAX, 0)),
        &Value::Decimal(Decimal::new(3, 0)),
    );
    assert_eq!(
        overflow,
        Value::Error(CellError::new(
            ErrorKind::Num,
            Origin::Arithmetic { op: ArithOp::Mul }
        ))
    );
}

/// An error operand propagates **with its original origin intact** — that is
/// what makes "where did this come from" answerable after a chain of
/// operations (docs/04 invariant 5, DP-A11).
#[test]
fn errors_propagate_carrying_their_original_origin() {
    let refused = Value::Error(CellError::refused_coercion(TypeTag::Text, TypeTag::Number));
    let result = arith(
        &Profile::Strict,
        ArithOp::Add,
        &refused,
        &Value::Number(1.0),
    );
    assert_eq!(result, refused, "origin must survive the operation");

    // Also when the error is the right-hand operand.
    let flipped = arith(
        &Profile::Strict,
        ArithOp::Mul,
        &Value::Number(2.0),
        &refused,
    );
    assert_eq!(flipped, refused);

    // An authored error is distinguishable from a derived one.
    let authored = Value::Error(CellError::new(ErrorKind::Na, Origin::Authored));
    let propagated = arith(
        &Profile::Strict,
        ArithOp::Sub,
        &authored,
        &Value::Number(1.0),
    );
    assert_eq!(
        propagated.as_error().map(|e| e.origin),
        Some(Origin::Authored)
    );
}

// ------------------------------------------------------------- encoding

/// Adding `Decimal` and the error origin must not disturb the bytes of values
/// that already existed — the property that keeps the replay corpus hash
/// stable across this row (DP-A4).
#[test]
fn existing_value_encodings_are_byte_stable() {
    let cases: [(Value, &[u8]); 4] = [
        (Value::Blank, &[0x00]),
        (Value::Bool(false), &[0x01]),
        (Value::Bool(true), &[0x02]),
        (
            Value::Number(1.0),
            &[0x03, 0x3F, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
    ];
    for (value, expected) in cases {
        let mut buf = Vec::new();
        value.encode_into(&mut buf);
        assert_eq!(buf, expected, "encoding of {value:?} changed");
    }

    // Text keeps tag 0x04 with a big-endian u32 length prefix.
    let mut text = Vec::new();
    Value::Text(String::from("hi")).encode_into(&mut text);
    assert_eq!(text, vec![0x04, 0, 0, 0, 2, b'h', b'i']);

    // Decimal is the new tag and does not collide.
    let mut decimal = Vec::new();
    Value::Decimal(dec("1.5")).encode_into(&mut decimal);
    assert_eq!(decimal[0], 0x06);
    assert_eq!(decimal.len(), 1 + 16 + 2);
}

/// Distinct values never share an encoding, including across the new variants.
#[test]
fn distinct_values_encode_distinctly() {
    let values = [
        Value::Blank,
        Value::Bool(false),
        Value::Bool(true),
        Value::Number(1.0),
        Value::Decimal(Decimal::ONE),
        Value::Text(String::from("1")),
        Value::Error(CellError::new(ErrorKind::Na, Origin::Authored)),
        Value::Error(CellError::new(ErrorKind::Na, Origin::Propagated)),
        Value::Error(CellError::refused_coercion(TypeTag::Text, TypeTag::Number)),
    ];
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for v in &values {
        let mut buf = Vec::new();
        v.encode_into(&mut buf);
        assert!(!seen.contains(&buf), "encoding collision for {v:?}");
        seen.push(buf);
    }

    // Two errors differing only in origin really are different values: the
    // origin is part of the value, not a side annotation.
    assert_ne!(
        Value::Error(CellError::new(ErrorKind::Na, Origin::Authored)),
        Value::Error(CellError::new(ErrorKind::Na, Origin::Propagated))
    );
}
