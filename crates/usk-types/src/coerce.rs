//! Coercion profiles and value arithmetic (ADR-010, docs/12, docs/32).
//!
//! Excel coerces silently and aggressively, and that is the source of its most
//! famous data-integrity failures — the gene-symbol mangling that made the
//! HUGO committee rename genes, and `1E2` quietly becoming `100`. The design
//! keeps the compatible behaviour reachable without making it the only
//! behaviour:
//!
//! * [`Profile::Compat`] — Excel's rules, quirks included. The default for
//!   imported workbooks, where fidelity beats correctness because real models
//!   depend on the quirks.
//! * [`Profile::Strict`] — no silent conversion. Text that looks like a number
//!   stays text; a formula that would have guessed returns `#VALUE!` carrying
//!   an [`Origin::Coercion`] that names both types. The default for natively
//!   created workbooks.
//!
//! The profile is a per-workbook property, never a global: docs/32 forbids
//! conflating the two silently inside one document.

use crate::decimal::parse_decimal;
use crate::{ArithOp, CellError, Decimal, ErrorKind, Origin, TypeTag, Value};
use alloc::format;
use alloc::string::String;

/// Which coercion rule set a workbook runs under (docs/32).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
    /// Excel's rules exactly, including its data-mangling quirks.
    Compat,
    /// Never convert silently; refuse and explain instead.
    Strict,
}

impl Profile {
    /// Interprets **text a user typed or an importer read** as a stored value.
    ///
    /// This is where the famous mangling happens, and where `strict` earns its
    /// keep. Under `Compat`, digit-like text becomes a `Number`, which silently
    /// destroys information:
    ///
    /// | input | `Compat` | `Strict` |
    /// |---|---|---|
    /// | `"1E2"` | `Number(100.0)` — a gene symbol becomes a number | `Text("1E2")` |
    /// | `"0000123"` | `Number(123.0)` — leading zeros gone (part codes, ZIPs) | `Text("0000123")` |
    /// | `"12.50"` | `Number(12.5)` | `Text("12.50")` |
    ///
    /// Note what `Compat` does *not* do here: it never produces a `Decimal`.
    /// Excel has no exact decimal type, so inferring one would be a divergence
    /// dressed as compatibility. Currency cells reach `Decimal` through an
    /// explicit format or an explicit conversion — see [`Profile::to_decimal`].
    pub fn coerce_input(&self, text: &str) -> Value {
        match self {
            Profile::Strict => Value::Text(String::from(text)),
            Profile::Compat => match parse_number_excel(text) {
                Some(n) => Value::Number(n),
                None => Value::Text(String::from(text)),
            },
        }
    }

    /// Coerces a value into the `f64` arithmetic domain.
    ///
    /// Both profiles agree on everything except text: blank is zero, booleans
    /// are 1/0 (Excel's arithmetic rule, and not controversial), and errors
    /// propagate untouched. Only `Compat` will read a number out of text.
    pub fn to_number(&self, value: &Value) -> Result<f64, CellError> {
        match value {
            Value::Blank => Ok(0.0),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Number(n) => Ok(*n),
            Value::Decimal(d) => Ok(d.to_f64()),
            Value::Error(e) => Err(*e),
            Value::Text(t) => match self {
                Profile::Compat => parse_number_excel(t)
                    .ok_or_else(|| CellError::refused_coercion(TypeTag::Text, TypeTag::Number)),
                Profile::Strict => Err(CellError::refused_coercion(TypeTag::Text, TypeTag::Number)),
            },
        }
    }

    /// Coerces a value into the exact decimal domain, or explains why it can't.
    ///
    /// A `Number` converts only when it is *exactly* representable: silently
    /// turning `0.1_f64` into the decimal `0.1` would claim an exactness the
    /// input never had, which is the opposite of this type's purpose.
    pub fn to_decimal(&self, value: &Value) -> Result<Decimal, CellError> {
        match value {
            Value::Blank => Ok(Decimal::ZERO),
            Value::Bool(b) => Ok(if *b { Decimal::ONE } else { Decimal::ZERO }),
            Value::Decimal(d) => Ok(*d),
            Value::Error(e) => Err(*e),
            Value::Number(n) => Decimal::try_from_f64_exact(*n)
                .ok_or_else(|| CellError::refused_coercion(TypeTag::Number, TypeTag::Decimal)),
            Value::Text(t) => match self {
                Profile::Compat => parse_decimal(t)
                    .ok_or_else(|| CellError::refused_coercion(TypeTag::Text, TypeTag::Decimal)),
                Profile::Strict => {
                    Err(CellError::refused_coercion(TypeTag::Text, TypeTag::Decimal))
                }
            },
        }
    }
}

/// Excel's text→number rule, used only by [`Profile::Compat`].
///
/// Accepts optional sign, digits with at most one decimal point, and
/// scientific notation — the last of these being precisely the rule that turns
/// the gene symbol `1E2` into `100`. Surrounding whitespace is tolerated, as
/// Excel does. Anything else stays text.
fn parse_number_excel(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let (mantissa, exponent) = match t.find(['e', 'E']) {
        Some(i) => {
            let exp: i32 = t.get(i + 1..)?.parse().ok()?;
            (t.get(..i)?, exp)
        }
        None => (t, 0),
    };
    let base = parse_decimal(mantissa)?.to_f64();
    let mut v = base;
    let mut e = exponent;
    while e > 0 {
        v *= 10.0;
        e -= 1;
    }
    while e < 0 {
        v /= 10.0;
        e += 1;
    }
    if v.is_finite() {
        Some(v)
    } else {
        None
    }
}

/// Whether a binary arithmetic result should stay exact.
///
/// Mixed `Number`/`Decimal` arithmetic promotes to `Decimal` **only when both
/// operands are exactly representable there**, otherwise it falls back to
/// `f64` (archive/DOC-GRID-DESIGN §V.1). Promoting an inexact float would
/// manufacture precision that was never in the data.
fn both_exact(profile: &Profile, a: &Value, b: &Value) -> Option<(Decimal, Decimal)> {
    let wants_decimal = matches!(a, Value::Decimal(_)) || matches!(b, Value::Decimal(_));
    if !wants_decimal {
        return None;
    }
    Some((profile.to_decimal(a).ok()?, profile.to_decimal(b).ok()?))
}

/// Applies a binary arithmetic operation under `profile`, choosing the exact
/// decimal path when it is available and the `f64` path otherwise.
///
/// Errors are values: an operand error propagates unchanged (carrying its
/// original origin, so the trace survives), and a failure here produces a fresh
/// error naming the operation.
pub fn arith(profile: &Profile, op: ArithOp, a: &Value, b: &Value) -> Value {
    // Propagation first — an error operand outranks everything (docs/04 §5).
    if let Some(e) = a.as_error().or_else(|| b.as_error()) {
        return Value::Error(e);
    }

    if let Some((x, y)) = both_exact(profile, a, b) {
        let exact = match op {
            ArithOp::Add => x.add(&y),
            ArithOp::Sub => x.sub(&y),
            ArithOp::Mul => x.mul(&y),
            ArithOp::Div => x.div(&y),
        };
        if let Some(d) = exact {
            return Value::Decimal(d);
        }
        // Division by zero is a user-facing error, not a fallback to floats.
        if matches!(op, ArithOp::Div) && y.is_zero() {
            return Value::Error(CellError::new(ErrorKind::Div0, Origin::Arithmetic { op }));
        }
        // Otherwise the exact result overflowed 38 digits.
        return Value::Error(CellError::new(ErrorKind::Num, Origin::Arithmetic { op }));
    }

    let (x, y) = match (profile.to_number(a), profile.to_number(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return Value::Error(e),
    };
    if matches!(op, ArithOp::Div) && y == 0.0 {
        return Value::Error(CellError::new(ErrorKind::Div0, Origin::Arithmetic { op }));
    }
    let r = match op {
        ArithOp::Add => x + y,
        ArithOp::Sub => x - y,
        ArithOp::Mul => x * y,
        ArithOp::Div => x / y,
    };
    if r.is_finite() {
        Value::Number(r)
    } else {
        Value::Error(CellError::new(ErrorKind::Num, Origin::Arithmetic { op }))
    }
}

/// Excel's cosmetic 15-significant-digit display rounding (docs/12, docs/32).
///
/// Excel shows 15 significant digits, so `0.1 + 0.2` displays as `0.3` even
/// though the stored `f64` is `0.30000000000000004`.
///
/// This is a **display** rule. It is deliberately not applied inside [`arith`]:
/// letting display rounding feed back into stored values is precisely the bug
/// class docs/04 bans by construction. It does not, on its own, explain
/// `=0.1+0.2-0.3` showing zero — that is a separate rule, see
/// [`compat_final_adjust`].
/// Implemented as a format-and-reparse round trip rather than by scaling.
/// Scaling by `10^k` is the obvious approach and it is wrong at the extremes:
/// `f64` represents powers of ten exactly only up to `10^22`, so building the
/// factor by repeated multiplication or division accumulates error and rounds
/// `1e300` to `9.999999999999978e299`. Rust's float formatter and parser are
/// pure-Rust, locale-free and in `core`, which makes this both exact and
/// identical on every platform — the property DP-A2 demands.
pub fn compat_round_15(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    // 14 digits after the point = 15 significant digits.
    let rendered = format!("{v:.14e}");
    rendered.parse::<f64>().unwrap_or(v)
}

/// Excel's *final-operation* adjustment: the rule that actually makes
/// `=0.1+0.2-0.3` evaluate to exactly zero (docs/12, docs/32).
///
/// Excel forces a result to zero when it is vanishingly small relative to the
/// operands that produced it — catastrophic cancellation between two nearly
/// equal numbers. Real models depend on this, which is why `compat` reproduces
/// it, and why `strict` does not: silently discarding a real (if tiny)
/// difference is a correctness hazard when the difference is the answer.
///
/// `operand_magnitude` is the largest absolute operand of the final operation;
/// the caller supplies it because the rule is about the operation, not the
/// value. This is why it cannot live inside [`compat_round_15`], which sees
/// only a result.
///
/// **Evidence status:** the threshold below is Excel's *documented* behaviour,
/// not oracle-captured. docs/32 is explicit that the documentation lies and the
/// binary does not, and the COM capture harness (ADR-024, assumption A-007) is
/// not built yet. Treat the exact boundary as unvalidated until the oracle
/// corpus confirms it; the *shape* of the rule is not in doubt.
pub fn compat_final_adjust(profile: &Profile, result: f64, operand_magnitude: f64) -> f64 {
    if !matches!(profile, Profile::Compat) {
        return result;
    }
    if result == 0.0 || operand_magnitude == 0.0 || !result.is_finite() {
        return result;
    }
    if (result.abs() / operand_magnitude.abs()) < 1e-15 {
        0.0
    } else {
        result
    }
}
