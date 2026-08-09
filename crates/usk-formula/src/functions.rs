//! The function catalogue (docs/12 §Function architecture, BOOTSTRAP row 6).
//!
//! Dispatch is a single `match` on the canonical uppercase name. docs/12
//! specifies declarative registration with per-argument coercion classes and
//! volatility tiers; that table earns its keep at Core-200 scale, and building
//! it for 60 functions would be scaffolding with no load on it yet. What is
//! preserved from that design is what matters now: **storage is canonical
//! English** (names are uppercased at parse time, so display localisation is a
//! pure view concern), and volatiles are injected rather than read from a clock.
//!
//! Conformance is oracle-captured from real Excel (ADR-024) — the binary is the
//! spec, not the documentation. **That harness now exists** (A-007 closed):
//! 1,366 vectors in `tools/oracle-capture/`, scored by `tools/conformance` as
//! workload W-ORACLE. The vectors in `tests/formulas.rs` still encode
//! *documented* behaviour and are still marked as such; where the two
//! disagree, the oracle wins and the divergence is a debt entry, not an
//! opinion. Where this engine deliberately diverges, `Profile::Strict` is the
//! divergence and `Profile::Compat` reproduces Excel.

use crate::eval::{eval, eval_operand, to_text, Context, Grid, Operand};
use crate::parse::Ast;
use alloc::string::String;
use alloc::vec::Vec;
use usk_types::coerce::{arith, compat_final_adjust, Profile};
use usk_types::{ArithOp, CellError, Decimal, ErrorKind, Origin, Value};

/// Every function this build knows. Used by the catalogue-size test and by
/// future name-completion; keeping it next to the dispatch keeps them honest.
pub const CATALOGUE: &[&str] = &[
    // Aggregation and maths
    "SUM",
    "AVERAGE",
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "MIN",
    "MAX",
    "PRODUCT",
    "ABS",
    "SIGN",
    "SQRT",
    "MOD",
    "POWER",
    "INT", // Rounding family
    "ROUND",
    "ROUNDUP",
    "ROUNDDOWN",
    "CEILING",
    "FLOOR", // Logical
    "IF",
    "IFS",
    "AND",
    "OR",
    "NOT",
    "XOR", // Errors and type predicates
    "IFERROR",
    "IFNA",
    "ISERROR",
    "ISNA",
    "ISBLANK",
    "ISNUMBER",
    "ISTEXT",
    "ISLOGICAL",
    "NA",
    // Text
    "CONCAT",
    "CONCATENATE",
    "TEXTJOIN",
    "LEFT",
    "RIGHT",
    "MID",
    "LEN",
    "TRIM",
    "UPPER",
    "LOWER",
    "PROPER",
    "SUBSTITUTE",
    "REPLACE",
    "FIND",
    "SEARCH",
    "REPT",
    "EXACT",
    "VALUE",
    // Lookup
    "VLOOKUP",
    "HLOOKUP",
    "XLOOKUP",
    "INDEX",
    "MATCH", // Conditional aggregation
    "SUMIF",
    "SUMIFS",
    "COUNTIF",
    "COUNTIFS",
    "AVERAGEIF", // Date core
    "TODAY",
    "NOW",
    "DATE",
    "YEAR",
    "MONTH",
    "DAY",
    "WEEKDAY",
    // Added at the first conformance run (W-ORACLE), each because the oracle
    // corpus measured it as a `#NAME?` divergence.
    "EXP",
    "LN",
    "UNICHAR",
    "CHAR",
    "UNICODE",
    "CODE",
];

fn err(kind: ErrorKind) -> Operand {
    Operand::Value(Value::Error(CellError::new(kind, Origin::Authored)))
}

fn val(v: Value) -> Operand {
    Operand::Value(v)
}

fn num(n: f64) -> Operand {
    Operand::Value(Value::Number(n))
}

fn boolean(b: bool) -> Operand {
    Operand::Value(Value::Bool(b))
}

/// Dispatches a call. Unknown names are `#NAME?`, never a panic.
pub fn call<G: Grid>(name: &str, args: &[Ast], ctx: &Context<G>) -> Operand {
    // Lazy functions must see unevaluated arguments: IF must not evaluate the
    // branch it does not take, and IFERROR must not propagate an error it is
    // there to catch.
    match name {
        "IF" => return f_if(args, ctx),
        "IFS" => return f_ifs(args, ctx),
        "IFERROR" => return f_iferror(args, ctx, false),
        "IFNA" => return f_iferror(args, ctx, true),
        _ => {}
    }

    let ops: Vec<Operand> = args.iter().map(|a| eval_operand(a, ctx)).collect();

    // Errors propagate through every strict function, carrying their origin.
    if !matches!(name, "ISERROR" | "ISNA" | "NA" | "COUNTBLANK") {
        if let Some(e) = ops.iter().find_map(|o| o.as_error()) {
            return Operand::Value(Value::Error(e));
        }
    }

    match name {
        "SUM" => f_sum(&ops, ctx),
        "PRODUCT" => f_product(&ops, ctx),
        "AVERAGE" => f_average(&ops, ctx),
        "COUNT" => num(numeric_cells(&ops).len() as f64),
        "COUNTA" => num(ops
            .iter()
            .flat_map(|o| o.cells())
            .filter(|c| !matches!(c, Value::Blank))
            .count() as f64),
        "COUNTBLANK" => num(ops
            .iter()
            .flat_map(|o| o.cells())
            .filter(|c| matches!(c, Value::Blank))
            .count() as f64),
        "MIN" => f_extreme(&ops, true),
        "MAX" => f_extreme(&ops, false),
        "ABS" => unary_num(&ops, ctx, |x| Some(if x < 0.0 { -x } else { x })),
        "SIGN" => unary_num(&ops, ctx, |x| {
            Some(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            })
        }),
        "SQRT" => unary_num(&ops, ctx, |x| if x < 0.0 { None } else { Some(sqrt(x)) }),
        "INT" => unary_num(&ops, ctx, |x| Some(floor(x))),
        "MOD" => f_mod(&ops, ctx),
        "POWER" => f_power(&ops, ctx),
        "ROUND" => f_round(&ops, ctx, Rounding::Half),
        "ROUNDUP" => f_round(&ops, ctx, Rounding::Up),
        "ROUNDDOWN" => f_round(&ops, ctx, Rounding::Down),
        "CEILING" => f_step(&ops, ctx, true),
        "FLOOR" => f_step(&ops, ctx, false),

        "AND" => f_logic(&ops, ctx, true),
        "OR" => f_logic(&ops, ctx, false),
        "NOT" => match ops.first().map(|o| truthy(ctx, &o.scalar())) {
            Some(Ok(b)) => boolean(!b),
            Some(Err(e)) => val(Value::Error(e)),
            None => err(ErrorKind::Value),
        },
        "XOR" => {
            let mut trues = 0usize;
            for o in &ops {
                for c in o.cells() {
                    match truthy(ctx, &c) {
                        Ok(true) => trues += 1,
                        Ok(false) => {}
                        Err(e) => return val(Value::Error(e)),
                    }
                }
            }
            boolean(trues % 2 == 1)
        }

        "ISERROR" => boolean(ops.first().is_some_and(|o| o.as_error().is_some())),
        "ISNA" => boolean(
            ops.first()
                .and_then(|o| o.as_error())
                .is_some_and(|e| e.kind == ErrorKind::Na),
        ),
        "ISBLANK" => boolean(matches!(scalar(&ops, 0), Some(Value::Blank))),
        "ISNUMBER" => boolean(matches!(
            scalar(&ops, 0),
            Some(Value::Number(_)) | Some(Value::Decimal(_))
        )),
        "ISTEXT" => boolean(matches!(scalar(&ops, 0), Some(Value::Text(_)))),
        "ISLOGICAL" => boolean(matches!(scalar(&ops, 0), Some(Value::Bool(_)))),
        "NA" => err(ErrorKind::Na),

        "CONCAT" | "CONCATENATE" => f_concat(&ops, ctx),
        "TEXTJOIN" => f_textjoin(&ops, ctx),
        "LEN" => match text_arg(&ops, 0, ctx) {
            Ok(s) => num(s.chars().count() as f64),
            Err(e) => val(Value::Error(e)),
        },
        "LEFT" | "RIGHT" => f_side(&ops, ctx, name == "LEFT"),
        "MID" => f_mid(&ops, ctx),
        "TRIM" => map_text(&ops, ctx, |s| collapse_spaces(&s)),
        "UPPER" => map_text(&ops, ctx, |s| s.to_uppercase()),
        "LOWER" => map_text(&ops, ctx, |s| s.to_lowercase()),
        "PROPER" => map_text(&ops, ctx, |s| proper_case(&s)),
        "SUBSTITUTE" => f_substitute(&ops, ctx),
        "REPLACE" => f_replace(&ops, ctx),
        "FIND" => f_find(&ops, ctx, true),
        "SEARCH" => f_find(&ops, ctx, false),
        "REPT" => f_rept(&ops, ctx),
        "EXACT" => match (text_arg(&ops, 0, ctx), text_arg(&ops, 1, ctx)) {
            (Ok(a), Ok(b)) => boolean(a == b),
            (Err(e), _) | (_, Err(e)) => val(Value::Error(e)),
        },
        "VALUE" => f_value(&ops, ctx),

        "VLOOKUP" => f_lookup_table(&ops, ctx, true),
        "HLOOKUP" => f_lookup_table(&ops, ctx, false),
        "XLOOKUP" => f_xlookup(&ops, ctx),
        "INDEX" => f_index(&ops, ctx),
        "MATCH" => f_match(&ops, ctx),

        "SUMIF" => f_sumif(&ops, ctx),
        "AVERAGEIF" => f_averageif(&ops, ctx),
        "COUNTIF" => f_countif(&ops, ctx),
        "SUMIFS" => f_ifs_agg(&ops, ctx, true),
        "COUNTIFS" => f_ifs_agg(&ops, ctx, false),

        "TODAY" => num(ctx.today as f64),
        "NOW" => num(ctx.now),
        "DATE" => f_date(&ops, ctx),
        "YEAR" => date_part(&ops, ctx, |y, _, _| y as f64),
        "MONTH" => date_part(&ops, ctx, |_, m, _| m as f64),
        "DAY" => date_part(&ops, ctx, |_, _, d| d as f64),
        "WEEKDAY" => f_weekday(&ops, ctx),

        // Added at the first conformance run: each was a measured `#NAME?`
        // against the oracle corpus, not a guess about what users might want.
        // `UNICHAR`/`UNICODE` alone accounted for 41 divergences, mostly
        // because the capture grids build every non-ASCII input with them
        // (docs/50 §Grids).
        "EXP" => unary_num(&ops, ctx, |x| Some(crate::eval::exp(x))),
        "LN" => unary_num(&ops, ctx, |x| {
            if x > 0.0 {
                Some(crate::eval::ln(x))
            } else {
                None
            }
        }),
        "UNICHAR" | "CHAR" => match num_arg(&ops, 0, ctx) {
            // Excel refuses 0 and anything outside the scalar range. A
            // surrogate is not a scalar, so `char::from_u32` refusing it is
            // the right answer rather than an accident (docs/50 finding 5).
            Ok(n) if n >= 1.0 => match char::from_u32(n as u32) {
                Some(c) => val(Value::Text(String::from(c))),
                None => err(ErrorKind::Value),
            },
            Ok(_) => err(ErrorKind::Value),
            Err(e) => val(Value::Error(e)),
        },
        "UNICODE" | "CODE" => match text_arg(&ops, 0, ctx) {
            Ok(s) => match s.chars().next() {
                Some(c) => num(c as u32 as f64),
                None => err(ErrorKind::Value),
            },
            Err(e) => val(Value::Error(e)),
        },

        _ => err(ErrorKind::Name),
    }
}

// ------------------------------------------------------------- helpers

fn scalar(ops: &[Operand], i: usize) -> Option<Value> {
    ops.get(i).map(|o| o.scalar())
}

fn text_arg<G: Grid>(ops: &[Operand], i: usize, ctx: &Context<G>) -> Result<String, CellError> {
    match ops.get(i) {
        Some(o) => to_text(ctx, &o.scalar()),
        None => Ok(String::new()),
    }
}

fn num_arg<G: Grid>(ops: &[Operand], i: usize, ctx: &Context<G>) -> Result<f64, CellError> {
    match ops.get(i) {
        Some(o) => ctx.profile.to_number(&o.scalar()),
        None => Ok(0.0),
    }
}

/// Like `num_arg`, but for an optional argument whose default is not zero.
/// `WEEKDAY`'s return type defaults to 1, and a *present* zero is `#NUM!`, so
/// the two cases cannot share `num_arg`'s "missing means 0".
fn num_arg_or<G: Grid>(
    ops: &[Operand],
    i: usize,
    ctx: &Context<G>,
    default: f64,
) -> Result<f64, CellError> {
    match ops.get(i) {
        Some(o) if o.scalar() != Value::Blank => ctx.profile.to_number(&o.scalar()),
        _ => Ok(default),
    }
}

/// Numeric cells only. Text and blanks are *skipped*, not coerced — that is
/// Excel's aggregation rule and it is why `COUNT` and `COUNTA` differ.
fn numeric_cells(ops: &[Operand]) -> Vec<Value> {
    ops.iter()
        .flat_map(|o| o.cells())
        .filter(|c| matches!(c, Value::Number(_) | Value::Decimal(_)))
        .collect()
}

fn truthy<G: Grid>(ctx: &Context<G>, v: &Value) -> Result<bool, CellError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Error(e) => Err(*e),
        other => ctx.profile.to_number(other).map(|n| n != 0.0),
    }
}

fn unary_num<G: Grid, F: Fn(f64) -> Option<f64>>(
    ops: &[Operand],
    ctx: &Context<G>,
    f: F,
) -> Operand {
    match num_arg(ops, 0, ctx) {
        Ok(x) => match f(x) {
            Some(r) => num(r),
            None => err(ErrorKind::Num),
        },
        Err(e) => val(Value::Error(e)),
    }
}

fn map_text<G: Grid, F: Fn(String) -> String>(ops: &[Operand], ctx: &Context<G>, f: F) -> Operand {
    match text_arg(ops, 0, ctx) {
        Ok(s) => val(Value::Text(f(s))),
        Err(e) => val(Value::Error(e)),
    }
}

/// Integer square root refinement by Newton's method — no libm (DP-A3).
fn sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = x;
    let mut i = 0;
    while i < 60 {
        let next = 0.5 * (g + x / g);
        if next == g {
            break;
        }
        g = next;
        i += 1;
    }
    g
}

/// Truncation toward zero. `f64::trunc` is `std`; DP-A3 keeps it out.
fn trunc(x: f64) -> f64 {
    if x < 0.0 {
        ceil(x)
    } else {
        floor(x)
    }
}

fn floor(x: f64) -> f64 {
    let t = (x as i64) as f64;
    if x < 0.0 && t != x {
        t - 1.0
    } else {
        t
    }
}

fn ceil(x: f64) -> f64 {
    -floor(-x)
}

// `pow10f` was `ROUND`'s scaling factor and is gone with it: scaling a float
// by a power of ten is exactly what made `ROUND(2.675,2)` return 2.67 where
// Excel says 2.68 (docs/50 §7). Rounding now happens in the decimal domain —
// see `round_decimal`.

// --------------------------------------------------------- aggregation

/// `SUM` stays exact when every addend is exact.
///
/// Mixing a `Decimal` column into an `f64` sum would throw away the whole point
/// of the exact domain, so the accumulator starts exact and degrades to `f64`
/// only when it meets a value that cannot be represented exactly — the same
/// lossless-only promotion rule `coerce::arith` uses for binary operators.
fn f_sum<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let cells = numeric_cells(ops);
    let mut exact = Some(Decimal::ZERO);
    let mut float = 0.0f64;

    for c in &cells {
        if let Some(acc) = exact {
            match ctx.profile.to_decimal(c) {
                Ok(d) => match acc.add(&d) {
                    Some(next) => {
                        exact = Some(next);
                        continue;
                    }
                    None => return err(ErrorKind::Num),
                },
                Err(_) => exact = None,
            }
        }
        match ctx.profile.to_number(c) {
            Ok(n) => float += n,
            Err(e) => return val(Value::Error(e)),
        }
    }

    match exact {
        Some(d) => val(Value::Decimal(d)),
        None => {
            // The exact path was abandoned partway, so re-sum in the float
            // domain rather than combining two accumulators.
            let _ = float;
            match float_sum(&cells, ctx) {
                Ok(total) => num(total),
                Err(e) => val(e),
            }
        }
    }
}

/// Accumulates in the float domain with Excel's **unconditional** cancellation
/// adjustment applied at every step.
///
/// docs/50 finding 2b, and the finding most likely to be missed: the `+`/`-`
/// operators adjust *positionally* (only at a formula's top level) but the
/// accumulating aggregates adjust *unconditionally*, so their zero survives
/// nesting. `=1/SUM(A1,7*2^-52,-A1)` is `#DIV/0!` while
/// `=1/(A1+7*2^-52-A1)` keeps the full residue. Reproducing only the
/// positional rule leaves `SUM` wrong in every nested position.
///
/// Applying it **per step** rather than once at the end is what reproduces the
/// order sensitivity Excel shows: `SUM(A1,7*2^-52,-A1)` cancels on the last
/// addition against an operand of magnitude 1 and is zeroed, while
/// `SUM(A1,-A1,8*2^-52)` cancels first and its last addition has nothing left
/// to cancel against, so the residue is kept.
fn float_sum<G: Grid>(cells: &[Value], ctx: &Context<G>) -> Result<f64, Value> {
    let mut total = 0.0f64;
    for c in cells {
        match ctx.profile.to_number(c) {
            Ok(n) => {
                let magnitude = if total.abs() > n.abs() { total } else { n };
                total = compat_final_adjust(&ctx.profile, total + n, magnitude);
                // Excel reports overflow as `#NUM!`, not as an infinity —
                // `=SUM(1E308,1E308)` (docs/50). An infinity leaking into a
                // cell would then poison every formula reading it.
                if !total.is_finite() {
                    return Err(Value::Error(CellError::new(
                        ErrorKind::Num,
                        Origin::Arithmetic { op: ArithOp::Add },
                    )));
                }
            }
            Err(e) => return Err(Value::Error(e)),
        }
    }
    Ok(total)
}

fn f_product<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let cells = numeric_cells(ops);
    if cells.is_empty() {
        return num(0.0);
    }
    let mut acc = Value::Number(1.0);
    for c in &cells {
        acc = arith(&ctx.profile, ArithOp::Mul, &acc, c);
        if let Some(e) = acc.as_error() {
            return val(Value::Error(e));
        }
    }
    val(acc)
}

fn f_average<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let cells = numeric_cells(ops);
    if cells.is_empty() {
        return err(ErrorKind::Div0);
    }
    let total = match f_sum(ops, ctx) {
        Operand::Value(v) => v,
        other => other.scalar(),
    };
    if let Some(e) = total.as_error() {
        return val(Value::Error(e));
    }
    val(arith(
        &ctx.profile,
        ArithOp::Div,
        &total,
        &Value::Number(cells.len() as f64),
    ))
}

fn f_extreme(ops: &[Operand], want_min: bool) -> Operand {
    let cells = numeric_cells(ops);
    if cells.is_empty() {
        // Excel returns 0, not an error, for MIN/MAX over no numbers.
        return num(0.0);
    }
    let mut best: Option<Value> = None;
    for c in cells {
        best = Some(match best {
            None => c,
            Some(b) => {
                let take = match (&b, &c) {
                    (Value::Decimal(x), Value::Decimal(y)) => {
                        if want_min {
                            y < x
                        } else {
                            y > x
                        }
                    }
                    _ => {
                        let (x, y) = (approx(&b), approx(&c));
                        if want_min {
                            y < x
                        } else {
                            y > x
                        }
                    }
                };
                if take {
                    c
                } else {
                    b
                }
            }
        });
    }
    val(best.unwrap_or(Value::Number(0.0)))
}

fn approx(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        Value::Decimal(d) => d.to_f64(),
        Value::Bool(true) => 1.0,
        _ => 0.0,
    }
}

fn f_mod<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    match (num_arg(ops, 0, ctx), num_arg(ops, 1, ctx)) {
        // Excel's MOD takes the sign of the divisor, unlike Rust's `%`.
        (Ok(n), Ok(d)) => {
            if d == 0.0 {
                err(ErrorKind::Div0)
            } else {
                num(n - d * floor(n / d))
            }
        }
        (Err(e), _) | (_, Err(e)) => val(Value::Error(e)),
    }
}

fn f_power<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    match (num_arg(ops, 0, ctx), num_arg(ops, 1, ctx)) {
        (Ok(x), Ok(y)) => {
            // Two measured divergences from `powf`'s IEEE conventions
            // (docs/50 §7), both `Profile::Compat` behaviour:
            //   * `POWER(0,0)` is `#NUM!` in Excel, where IEEE says 1.
            //   * `POWER(-8,1/3)` is `-1.9999999999999998` — Excel computes
            //     odd roots of negative bases, where IEEE says NaN.
            if x == 0.0 && y == 0.0 {
                return err(ErrorKind::Num);
            }
            let r = if x < 0.0 && y != trunc(y) {
                match odd_root(y) {
                    Some(_) => -crate::eval::powf(-x, y),
                    None => f64::NAN,
                }
            } else {
                crate::eval::powf(x, y)
            };
            if r.is_finite() {
                num(r)
            } else {
                err(ErrorKind::Num)
            }
        }
        (Err(e), _) | (_, Err(e)) => val(Value::Error(e)),
    }
}

/// `Some(n)` when `y` is (very nearly) `1/n` for an odd integer `n` — the
/// exponents for which Excel returns a real odd root of a negative base.
///
/// The tolerance is unavoidable: `1/3` is not representable, so the exponent
/// Excel is handed is `0.333...33` and an exact test would never fire. It is
/// tight enough that no ordinary fractional exponent is captured by accident.
fn odd_root(y: f64) -> Option<i64> {
    if y == 0.0 {
        return None;
    }
    let n = 1.0 / y;
    let rounded = n + if n < 0.0 { -0.5 } else { 0.5 };
    let rounded = rounded as i64;
    if rounded % 2 == 0 || rounded == 0 {
        return None;
    }
    let back = 1.0 / rounded as f64;
    if (back - y).abs() <= 1e-12 * y.abs() {
        Some(rounded)
    } else {
        None
    }
}

enum Rounding {
    Half,
    Up,
    Down,
}

fn f_round<G: Grid>(ops: &[Operand], ctx: &Context<G>, mode: Rounding) -> Operand {
    let (x, digits) = match (num_arg(ops, 0, ctx), num_arg(ops, 1, ctx)) {
        (Ok(x), Ok(d)) => (x, d as i32),
        (Err(e), _) | (_, Err(e)) => return val(Value::Error(e)),
    };
    match round_decimal(x, digits, mode) {
        Some(r) => num(r),
        None => err(ErrorKind::Num),
    }
}

/// Rounds in the **decimal** domain, on the 15-significant-digit rendering.
///
/// The obvious implementation — `floor(x * 10^d + 0.5) / 10^d` — is what this
/// replaced, and the oracle caught it: `2.675 * 100` is `267.49999999999997`,
/// so `ROUND(2.675,2)` came out `2.67` where Excel says `2.68` (docs/50 §7).
/// The multiply loses the very digit being asked about.
///
/// Excel rounds the number *as displayed to 15 significant digits*, so this
/// renders through `core`'s float formatter — pure Rust, locale-free, and
/// identical on every target, the same property `compat_round_15` relies on
/// (DP-A2) — and then rounds exact base-10 integers. No float arithmetic
/// touches the decision.
fn round_decimal(x: f64, digits: i32, mode: Rounding) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    if x == 0.0 {
        return Some(0.0);
    }
    // "d.dddddddddddddde±dd" — one digit before the point, fourteen after,
    // i.e. exactly the 15 significant digits Excel works from.
    let rendered = alloc::format!("{x:.14e}");
    let (mantissa, exponent) = rendered.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    let negative = mantissa.starts_with('-');
    let mut coefficient: i128 = 0;
    for c in mantissa.chars().filter(char::is_ascii_digit) {
        coefficient = coefficient.checked_mul(10)? + (c as u8 - b'0') as i128;
    }
    // value = coefficient × 10^scale, exactly.
    let scale = exponent - 14;
    let target = -digits;
    let drop = target.checked_sub(scale)?;
    if drop <= 0 {
        // Already coarser than the requested precision: nothing to round.
        return Some(x);
    }
    if drop > 40 {
        // Everything rounds away; only the direction is in question.
        return Some(match mode {
            Rounding::Up if coefficient != 0 => {
                let unit = decimal_to_f64(1, target)?;
                if negative {
                    -unit
                } else {
                    unit
                }
            }
            _ => 0.0,
        });
    }
    let divisor = pow10i(drop)?;
    let quotient = coefficient / divisor;
    let remainder = coefficient % divisor;
    let rounded = match mode {
        // Excel rounds half *away from zero*, not half-even. The magnitude is
        // rounded and the sign reapplied, which is what "away from zero"
        // means and what a signed division would get wrong.
        Rounding::Half if remainder * 2 >= divisor => quotient + 1,
        Rounding::Half => quotient,
        Rounding::Up if remainder != 0 => quotient + 1,
        Rounding::Up | Rounding::Down => quotient,
    };
    let magnitude = decimal_to_f64(rounded, target)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// `coefficient × 10^exponent` as an `f64`, through `core`'s correctly-rounded
/// parser rather than through a multiply that would round twice.
fn decimal_to_f64(coefficient: i128, exponent: i32) -> Option<f64> {
    alloc::format!("{coefficient}e{exponent}")
        .parse::<f64>()
        .ok()
}

fn pow10i(n: i32) -> Option<i128> {
    let mut out: i128 = 1;
    for _ in 0..n {
        out = out.checked_mul(10)?;
    }
    Some(out)
}

fn f_step<G: Grid>(ops: &[Operand], ctx: &Context<G>, up: bool) -> Operand {
    let (x, step) = match (num_arg(ops, 0, ctx), ops.get(1)) {
        (Ok(x), Some(_)) => match num_arg(ops, 1, ctx) {
            Ok(s) => (x, s),
            Err(e) => return val(Value::Error(e)),
        },
        (Ok(x), None) => (x, 1.0),
        (Err(e), _) => return val(Value::Error(e)),
    };
    if step == 0.0 {
        // docs/50 §7: Excel is asymmetric here. `CEILING(2.5,0)` is 0 and
        // `FLOOR(2.5,0)` is `#DIV/0!`. Not a rule anyone would derive — it is
        // measured, and the asymmetry is the point.
        return if up { num(0.0) } else { err(ErrorKind::Div0) };
    }
    let q = x / step;
    num(step * if up { ceil(q) } else { floor(q) })
}

// -------------------------------------------------------------- logic

fn f_if<G: Grid>(args: &[Ast], ctx: &Context<G>) -> Operand {
    let Some(cond) = args.first() else {
        return err(ErrorKind::Value);
    };
    let c = eval(cond, ctx);
    match truthy(ctx, &c) {
        Err(e) => val(Value::Error(e)),
        Ok(true) => args
            .get(1)
            .map(|a| eval_operand(a, ctx))
            .unwrap_or_else(|| boolean(true)),
        Ok(false) => args
            .get(2)
            .map(|a| eval_operand(a, ctx))
            .unwrap_or_else(|| boolean(false)),
    }
}

fn f_ifs<G: Grid>(args: &[Ast], ctx: &Context<G>) -> Operand {
    let mut i = 0;
    while i + 1 < args.len() {
        let c = eval(&args[i], ctx);
        match truthy(ctx, &c) {
            Err(e) => return val(Value::Error(e)),
            Ok(true) => return eval_operand(&args[i + 1], ctx),
            Ok(false) => {}
        }
        i += 2;
    }
    err(ErrorKind::Na)
}

/// `IFERROR`/`IFNA` must evaluate lazily: catching an error means never letting
/// it propagate out of the first argument.
fn f_iferror<G: Grid>(args: &[Ast], ctx: &Context<G>, only_na: bool) -> Operand {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    let v = eval_operand(first, ctx);
    let caught = match v.as_error() {
        Some(e) => !only_na || e.kind == ErrorKind::Na,
        None => false,
    };
    if caught {
        args.get(1)
            .map(|a| eval_operand(a, ctx))
            .unwrap_or_else(|| val(Value::Blank))
    } else {
        v
    }
}

fn f_logic<G: Grid>(ops: &[Operand], ctx: &Context<G>, all: bool) -> Operand {
    let mut seen = false;
    let mut acc = all;
    for o in ops {
        for c in o.cells() {
            // Excel's AND/OR skip blanks and text rather than coercing them.
            if matches!(c, Value::Blank | Value::Text(_)) {
                continue;
            }
            match truthy(ctx, &c) {
                Err(e) => return val(Value::Error(e)),
                Ok(b) => {
                    seen = true;
                    acc = if all { acc && b } else { acc || b };
                }
            }
        }
    }
    if seen {
        boolean(acc)
    } else {
        err(ErrorKind::Value)
    }
}

// --------------------------------------------------------------- text

fn f_concat<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let mut out = String::new();
    for o in ops {
        for c in o.cells() {
            match to_text(ctx, &c) {
                Ok(s) => out.push_str(&s),
                Err(e) => return val(Value::Error(e)),
            }
        }
    }
    val(Value::Text(out))
}

fn f_textjoin<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let sep = match text_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    let skip_empty = match ops.get(1).map(|o| truthy(ctx, &o.scalar())) {
        Some(Ok(b)) => b,
        Some(Err(e)) => return val(Value::Error(e)),
        None => true,
    };
    let mut parts: Vec<String> = Vec::new();
    for o in ops.iter().skip(2) {
        for c in o.cells() {
            match to_text(ctx, &c) {
                Ok(s) => {
                    if !(skip_empty && s.is_empty()) {
                        parts.push(s)
                    }
                }
                Err(e) => return val(Value::Error(e)),
            }
        }
    }
    val(Value::Text(parts.join(&sep)))
}

fn f_side<G: Grid>(ops: &[Operand], ctx: &Context<G>, left: bool) -> Operand {
    let s = match text_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    let n = match ops.get(1) {
        Some(_) => match num_arg(ops, 1, ctx) {
            Ok(n) if n < 0.0 => return err(ErrorKind::Value),
            Ok(n) => n as usize,
            Err(e) => return val(Value::Error(e)),
        },
        None => 1,
    };
    let chars: Vec<char> = s.chars().collect();
    let n = n.min(chars.len());
    let slice: String = if left {
        chars[..n].iter().collect()
    } else {
        chars[chars.len() - n..].iter().collect()
    };
    val(Value::Text(slice))
}

fn f_mid<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let s = match text_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    let (start, len) = match (num_arg(ops, 1, ctx), num_arg(ops, 2, ctx)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return val(Value::Error(e)),
    };
    if start < 1.0 || len < 0.0 {
        return err(ErrorKind::Value);
    }
    let chars: Vec<char> = s.chars().collect();
    let from = (start as usize - 1).min(chars.len());
    let to = (from + len as usize).min(chars.len());
    val(Value::Text(chars[from..to].iter().collect()))
}

fn collapse_spaces(s: &str) -> String {
    // Excel's TRIM removes leading/trailing spaces and collapses internal runs.
    // Only the ASCII space, deliberately. Rust's `str::trim` strips every
    // Unicode whitespace scalar including U+00A0, and docs/50 §7 measured
    // Excel *keeping* a non-breaking space: `LEN(TRIM(NBSP&"abc"&NBSP))` is 5,
    // not 3. Web-pasted data is full of NBSP, so this is the common case, not
    // an exotic one.
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.trim_matches(' ').chars() {
        if c == ' ' {
            if !last_space {
                out.push(c);
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out
}

fn proper_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start_of_word = true;
    for c in s.chars() {
        if start_of_word {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start_of_word = !c.is_alphanumeric();
    }
    out
}

fn f_substitute<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let (text, old, new) = match (
        text_arg(ops, 0, ctx),
        text_arg(ops, 1, ctx),
        text_arg(ops, 2, ctx),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return val(Value::Error(e)),
    };
    if old.is_empty() {
        return val(Value::Text(text));
    }
    match ops.get(3) {
        None => val(Value::Text(text.replace(&old, &new))),
        Some(_) => {
            let nth = match num_arg(ops, 3, ctx) {
                Ok(n) if n >= 1.0 => n as usize,
                Ok(_) => return err(ErrorKind::Value),
                Err(e) => return val(Value::Error(e)),
            };
            let mut out = String::new();
            let mut rest = text.as_str();
            let mut seen = 0usize;
            while let Some(i) = rest.find(&old) {
                seen += 1;
                out.push_str(&rest[..i]);
                if seen == nth {
                    out.push_str(&new);
                } else {
                    out.push_str(&old);
                }
                rest = &rest[i + old.len()..];
            }
            out.push_str(rest);
            val(Value::Text(out))
        }
    }
}

fn f_replace<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let text = match text_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    let (start, len) = match (num_arg(ops, 1, ctx), num_arg(ops, 2, ctx)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return val(Value::Error(e)),
    };
    let new = match text_arg(ops, 3, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    if start < 1.0 || len < 0.0 {
        return err(ErrorKind::Value);
    }
    let chars: Vec<char> = text.chars().collect();
    let from = (start as usize - 1).min(chars.len());
    let to = (from + len as usize).min(chars.len());
    let mut out: String = chars[..from].iter().collect();
    out.push_str(&new);
    out.extend(&chars[to..]);
    val(Value::Text(out))
}

/// `FIND` is case-sensitive, `SEARCH` is not — the only difference in v0.1,
/// since wildcard support belongs with the criteria engine.
fn f_find<G: Grid>(ops: &[Operand], ctx: &Context<G>, case_sensitive: bool) -> Operand {
    let (needle, haystack) = match (text_arg(ops, 0, ctx), text_arg(ops, 1, ctx)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return val(Value::Error(e)),
    };
    let start = match ops.get(2) {
        Some(_) => match num_arg(ops, 2, ctx) {
            Ok(n) if n >= 1.0 => n as usize - 1,
            Ok(_) => return err(ErrorKind::Value),
            Err(e) => return val(Value::Error(e)),
        },
        None => 0,
    };
    let (n, h) = if case_sensitive {
        (needle.clone(), haystack.clone())
    } else {
        (needle.to_lowercase(), haystack.to_lowercase())
    };
    let hay: Vec<char> = h.chars().collect();
    if start > hay.len() {
        return err(ErrorKind::Value);
    }
    // SEARCH takes wildcards; FIND does not (TD-35). That split is exactly the
    // case-sensitivity split, so the one flag decides both — which is not a
    // coincidence: FIND is the literal, byte-faithful one in both respects.
    if !case_sensitive && has_wildcard(&n) {
        // The pattern anchors at some start position and may end anywhere, so
        // it is a prefix match tried at each position, left to right.
        for at in start..=hay.len() {
            let tail: String = hay[at..].iter().collect();
            if wildcard_prefix_match(&n, &tail) {
                return num((at + 1) as f64);
            }
        }
        return err(ErrorKind::Value);
    }
    // Even with no active wildcard the escapes are still escapes, so `~*`
    // searches for a literal asterisk. FIND has no sub-language at all and
    // keeps the tilde.
    let n = if case_sensitive {
        n
    } else {
        unescape_wildcards(&n)
    };
    let tail: String = hay[start..].iter().collect();
    match tail.find(&n) {
        // Excel positions are 1-based and in characters, not bytes.
        Some(byte_idx) => {
            let char_idx = tail[..byte_idx].chars().count();
            num((start + char_idx + 1) as f64)
        }
        None => err(ErrorKind::Value),
    }
}

/// `wildcard_match` where the text may continue past the pattern — SEARCH asks
/// "does the pattern occur here", not "is the whole cell this pattern".
fn wildcard_prefix_match(pattern: &str, text: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    (0..=t.len()).any(|end| {
        let head: String = t[..end].iter().collect();
        wildcard_match(pattern, &head)
    })
}

fn f_rept<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let s = match text_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    match num_arg(ops, 1, ctx) {
        Ok(n) if n < 0.0 => err(ErrorKind::Value),
        // Bounded so a formula cannot exhaust memory: a hostile or mistyped
        // REPT is a denial-of-service vector otherwise (DP-E2 resource caps).
        Ok(n) if s.len().saturating_mul(n as usize) > 1 << 20 => err(ErrorKind::Value),
        Ok(n) => val(Value::Text(s.repeat(n as usize))),
        Err(e) => val(Value::Error(e)),
    }
}

/// `VALUE` is an *explicit* conversion, so it applies compat's text→number rule
/// even under `strict`: the user asked for it by name. That is the difference
/// between a silent coercion and a stated one.
fn f_value<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let s = match text_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    match Profile::Compat.to_number(&Value::Text(s)) {
        Ok(n) => num(n),
        Err(_) => err(ErrorKind::Value),
    }
}

// ------------------------------------------------------------- lookup

fn range_dims(op: &Operand) -> (u32, u32, Vec<Value>) {
    match op {
        Operand::Range { rows, cols, cells } => (*rows, *cols, cells.clone()),
        Operand::Value(v) => (1, 1, alloc::vec![v.clone()]),
    }
}

fn cell_at(cells: &[Value], cols: u32, r: u32, c: u32) -> Value {
    cells
        .get((r * cols + c) as usize)
        .cloned()
        .unwrap_or(Value::Blank)
}

/// Excel's wildcard sub-language (TD-35): `*` any run, `?` any single
/// character, `~` escaping the next wildcard so `a~*c` means a literal `a*c`.
/// Case-insensitive, like every other text comparison in a lookup.
///
/// Iterative with a backtrack point rather than recursive: a `no_std` kernel
/// evaluating a hostile formula must not be able to blow the stack, and
/// `"*a*a*a*..."` against a long string is the classic way to do it (DP-E2).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(|c| c.to_lowercase()).collect();
    let t: Vec<char> = text.chars().flat_map(|c| c.to_lowercase()).collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Where to resume if the current `*` turns out to have matched too little.
    let (mut star, mut resume) = (None, 0usize);
    while ti < t.len() {
        let lit = match p.get(pi) {
            Some('*') => {
                star = Some(pi);
                pi += 1;
                resume = ti;
                continue;
            }
            Some('?') => {
                pi += 1;
                ti += 1;
                continue;
            }
            // `~` escapes only a wildcard; before anything else it is literal.
            Some('~') if matches!(p.get(pi + 1), Some('*' | '?' | '~')) => p[pi + 1],
            Some(c) => *c,
            None => {
                // Pattern exhausted with text left over: only a `*` saves it.
                match star {
                    Some(s) => {
                        pi = s + 1;
                        resume += 1;
                        ti = resume;
                        continue;
                    }
                    None => return false,
                }
            }
        };
        let width = if p.get(pi) == Some(&'~') { 2 } else { 1 };
        if t[ti] == lit {
            pi += width;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    // Trailing `*`s may match nothing.
    while p.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi >= p.len()
}

/// Whether a lookup pattern contains an *active* wildcard. `a~*c` does not:
/// its `*` is escaped, so the whole string is a literal and must compare by
/// equality — which is what makes `VLOOKUP("a~*c", …)` find the cell holding
/// the three characters `a*c`.
fn has_wildcard(pattern: &str) -> bool {
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '?' => return true,
            '~' => {
                chars.next();
            }
            _ => {}
        }
    }
    false
}

/// Drops the `~` from each escaped wildcard, turning a pattern with no *active*
/// wildcard back into the literal text it denotes: `a~*c` is the three
/// characters `a*c`. Without this the literal path searches for the tilde
/// itself and finds nothing, which is a silent `#N/A` rather than a visible
/// bug.
fn unescape_wildcards(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '~' && matches!(chars.peek(), Some('*' | '?' | '~')) {
            out.push(chars.next().unwrap_or(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// Equality *or* wildcard match — Excel's exact-match lookups accept patterns.
/// A non-text needle can never be a pattern, and a non-text candidate can
/// never be matched by one.
fn matches_needle(candidate: &Value, needle: &Value) -> bool {
    if let (Value::Text(pattern), Value::Text(text)) = (needle, candidate) {
        if has_wildcard(pattern) {
            return wildcard_match(pattern, text);
        }
        return unescape_wildcards(pattern).eq_ignore_ascii_case(text);
    }
    // A blank needle is not a value that matches other blanks: Excel reads the
    // empty cell as 0, so `MATCH(A1, …, 0)` over a column containing blanks is
    // #N/A rather than the position of the first hole.
    if *needle == Value::Blank {
        return matches!(candidate, Value::Number(n) if *n == 0.0);
    }
    values_equal(candidate, needle)
}

/// Excel's cross-type ordering for a sorted lookup key: numbers before text
/// before logicals, each ordered naturally, text case-insensitively. `None`
/// means the candidate takes no part in the ordering — a blank cell in a key
/// column is a hole, not a value that sorts somewhere.
fn lookup_order(a: &Value, b: &Value) -> Option<core::cmp::Ordering> {
    fn rank(v: &Value) -> Option<u8> {
        match v {
            Value::Number(_) | Value::Decimal(_) => Some(0),
            Value::Text(_) => Some(1),
            Value::Bool(_) => Some(2),
            Value::Blank | Value::Error(_) => None,
        }
    }
    let (ra, rb) = (rank(a)?, rank(b)?);
    if ra != rb {
        return Some(ra.cmp(&rb));
    }
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => {
            let (x, y) = (x.to_lowercase(), y.to_lowercase());
            Some(x.cmp(&y))
        }
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => numeric(a)?.partial_cmp(&numeric(b)?),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => x.eq_ignore_ascii_case(y),
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Blank, Value::Blank) => true,
        _ => match (numeric(a), numeric(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
    }
}

fn numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Decimal(d) => Some(d.to_f64()),
        _ => None,
    }
}

fn f_lookup_table<G: Grid>(ops: &[Operand], ctx: &Context<G>, vertical: bool) -> Operand {
    let needle = match scalar(ops, 0) {
        Some(v) => v,
        None => return err(ErrorKind::Value),
    };
    let Some(table) = ops.get(1) else {
        return err(ErrorKind::Ref);
    };
    let (rows, cols, cells) = range_dims(table);
    let index = match num_arg(ops, 2, ctx) {
        Ok(n) if n >= 1.0 => n as u32 - 1,
        Ok(_) => return err(ErrorKind::Value),
        Err(e) => return val(Value::Error(e)),
    };
    // Excel's *default* is the approximate match, so omitting the 4th argument
    // means TRUE (TD-14). v0.1 refused it rather than fake it; it is faked no
    // longer — see `approximate_index`.
    let approximate = match ops.get(3) {
        Some(o) => match truthy(ctx, &o.scalar()) {
            Ok(b) => b,
            Err(e) => return val(Value::Error(e)),
        },
        None => true,
    };

    let (scan, other) = if vertical { (rows, cols) } else { (cols, rows) };
    if index >= other {
        return err(ErrorKind::Ref);
    }
    let key_at = |i: u32| {
        if vertical {
            cell_at(&cells, cols, i, 0)
        } else {
            cell_at(&cells, cols, 0, i)
        }
    };
    let hit = if approximate {
        approximate_index(scan, &key_at, &needle)
    } else {
        (0..scan).find(|i| matches_needle(&key_at(*i), &needle))
    };
    match hit {
        Some(i) => val(if vertical {
            cell_at(&cells, cols, i, index)
        } else {
            cell_at(&cells, cols, index, i)
        }),
        None => err(ErrorKind::Na),
    }
}

/// Excel's approximate match: the **binary search** it really performs, not a
/// linear scan for the largest key below the needle (TD-14).
///
/// The distinction is load-bearing on *unsorted* data, which Excel does not
/// detect and does not refuse. Measured: over the key column `30, 10, 50, 10,
/// (blank)`, `VLOOKUP(35, …, TRUE)` returns the row holding **10**, because the
/// search probes the middle, finds 50 above the needle, halves downward and
/// lands on the second row. A linear "largest key ≤ 35" would answer 30 — a
/// defensible number that real Excel does not produce.
///
/// This is why the old refusal was the right call at the time and why the fix
/// had to wait for the oracle: the correct behaviour here is not derivable from
/// the documented contract, only from the implementation.
fn approximate_index<F: Fn(u32) -> Value>(len: u32, key_at: &F, needle: &Value) -> Option<u32> {
    if len == 0 {
        return None;
    }
    let below = |i: u32| {
        lookup_order(&key_at(i), needle)
            .map(|o| o != core::cmp::Ordering::Greater)
            // A blank or error in the key column takes no part in the order;
            // treating it as "not below" keeps the search descending past it
            // instead of anchoring on a hole.
            .unwrap_or(false)
    };
    let (mut lo, mut hi) = (0u32, len - 1);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if below(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    below(lo).then_some(lo)
}

/// `XLOOKUP(needle, keys, values, [if_missing], [match_mode], [search_mode])`.
///
/// Match modes: `0` exact (the default, and the reason XLOOKUP exists), `-1`
/// exact or next smaller, `1` exact or next larger, `2` wildcard. Search modes:
/// `1` first-to-last (default) and `-1` last-to-first; `2`/`-2` ask for a binary
/// search, which over sorted data finds the same cell, so they are accepted and
/// answered linearly rather than refused.
///
/// Unlike `VLOOKUP`'s approximate mode, `-1` and `1` do **not** binary-search:
/// they take the best candidate anywhere in the vector, which is what makes
/// XLOOKUP safe on unsorted data and VLOOKUP not.
fn f_xlookup<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let needle = match scalar(ops, 0) {
        Some(v) => v,
        None => return err(ErrorKind::Value),
    };
    let (Some(lookup), Some(result)) = (ops.get(1), ops.get(2)) else {
        return err(ErrorKind::Ref);
    };
    let keys = lookup.cells();
    let values = result.cells();
    // The two vectors must line up. Excel refuses a mismatch rather than
    // padding it with blanks, and so must we: silently returning the fallback
    // would hide a broken formula.
    if keys.len() != values.len() {
        return err(ErrorKind::Value);
    }
    let match_mode = match ops.get(4) {
        Some(_) => match num_arg(ops, 4, ctx) {
            Ok(m) => floor(m) as i64,
            Err(e) => return val(Value::Error(e)),
        },
        None => 0,
    };
    let reverse = match ops.get(5) {
        Some(_) => match num_arg(ops, 5, ctx) {
            Ok(m) => floor(m) < 0.0,
            Err(e) => return val(Value::Error(e)),
        },
        None => false,
    };

    let order: Vec<usize> = if reverse {
        (0..keys.len()).rev().collect()
    } else {
        (0..keys.len()).collect()
    };
    let hit = match match_mode {
        0 => order
            .iter()
            .copied()
            .find(|i| values_equal(&keys[*i], &needle)),
        2 => order
            .iter()
            .copied()
            .find(|i| matches_needle(&keys[*i], &needle)),
        // Exact first, then the nearest candidate on the requested side.
        m => order
            .iter()
            .copied()
            .find(|i| values_equal(&keys[*i], &needle))
            .or_else(|| nearest(&keys, &order, &needle, m < 0)),
    };
    match hit {
        Some(i) => val(values[i].clone()),
        // XLOOKUP's 4th argument is the not-found fallback.
        None => match ops.get(3) {
            Some(o) => val(o.scalar()),
            None => err(ErrorKind::Na),
        },
    }
}

/// The closest key strictly below (`smaller`) or above the needle. Ties keep
/// the first candidate in `order`, so the search mode decides them.
fn nearest(keys: &[Value], order: &[usize], needle: &Value, smaller: bool) -> Option<usize> {
    let mut best: Option<(usize, &Value)> = None;
    for &i in order {
        let Some(side) = lookup_order(&keys[i], needle) else {
            continue;
        };
        let wanted = if smaller {
            side == core::cmp::Ordering::Less
        } else {
            side == core::cmp::Ordering::Greater
        };
        if !wanted {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, b)) => match lookup_order(&keys[i], b) {
                // Closer to the needle means larger among the smaller ones,
                // and smaller among the larger ones.
                Some(core::cmp::Ordering::Greater) => smaller,
                Some(core::cmp::Ordering::Less) => !smaller,
                _ => false,
            },
        };
        if better {
            best = Some((i, &keys[i]));
        }
    }
    best.map(|(i, _)| i)
}

fn f_index<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let Some(source) = ops.first() else {
        return err(ErrorKind::Ref);
    };
    let (rows, cols, cells) = range_dims(source);
    let r = match num_arg(ops, 1, ctx) {
        Ok(n) => n as i64,
        Err(e) => return val(Value::Error(e)),
    };
    let c = match ops.get(2) {
        Some(_) => match num_arg(ops, 2, ctx) {
            Ok(n) => n as i64,
            Err(e) => return val(Value::Error(e)),
        },
        None => 0,
    };
    // A negative index is a different failure from an out-of-range one: Excel
    // says #VALUE! for "that is not an index" and #REF! for "that index is off
    // the end". Collapsing them loses which mistake was made.
    if r < 0 || c < 0 {
        return err(ErrorKind::Value);
    }
    // A single-row or single-column range indexes with one coordinate.
    let (ri, ci) = if rows == 1 && c == 0 {
        (1i64, r)
    } else if cols == 1 && c == 0 {
        (r, 1i64)
    } else {
        (r, c)
    };
    if ri as u32 > rows || ci as u32 > cols {
        return err(ErrorKind::Ref);
    }
    // Index 0 means "the whole row" or "the whole column" — an array, which is
    // why `SUM(INDEX(A1:B5,0,1))` totals a column. In a scalar context it still
    // collapses to the top-left (TD-16: implicit intersection needs the calling
    // cell's position, which arrives with the dependency graph).
    match (ri, ci) {
        (0, 0) => val(Value::Error(CellError::new(
            ErrorKind::Value,
            Origin::Authored,
        ))),
        (0, c) => {
            let column: Vec<Value> = (0..rows)
                .map(|r| cell_at(&cells, cols, r, c as u32 - 1))
                .collect();
            Operand::Range {
                rows,
                cols: 1,
                cells: column,
            }
        }
        (r, 0) => {
            let row: Vec<Value> = (0..cols)
                .map(|c| cell_at(&cells, cols, r as u32 - 1, c))
                .collect();
            Operand::Range {
                rows: 1,
                cols,
                cells: row,
            }
        }
        (r, c) => val(cell_at(&cells, cols, r as u32 - 1, c as u32 - 1)),
    }
}

fn f_match<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let needle = match scalar(ops, 0) {
        Some(v) => v,
        None => return err(ErrorKind::Value),
    };
    let Some(range) = ops.get(1) else {
        return err(ErrorKind::Ref);
    };
    // MATCH wants a vector. A range that is neither a single row nor a single
    // column is #N/A — measured, and worth stating because scanning it in
    // row-major order returns a confident, wrong ordinal instead.
    let (rows, cols, _) = range_dims(range);
    if rows > 1 && cols > 1 {
        return err(ErrorKind::Na);
    }
    // Type 1 (the default) wants the largest value <= needle in an ascending
    // vector; 0 is exact; -1 wants the smallest value >= needle in a
    // descending one.
    let kind = match ops.get(2) {
        Some(_) => match num_arg(ops, 2, ctx) {
            Ok(t) => floor(t) as i64,
            Err(e) => return val(Value::Error(e)),
        },
        None => 1,
    };
    let cells = range.cells();
    let len = cells.len() as u32;
    let key_at = |i: u32| cells.get(i as usize).cloned().unwrap_or(Value::Blank);
    let hit = match kind {
        0 => (0..len).find(|i| matches_needle(&key_at(*i), &needle)),
        1 => approximate_index(len, &key_at, &needle),
        _ => descending_index(len, &key_at, &needle),
    };
    match hit {
        Some(i) => num((i + 1) as f64),
        None => err(ErrorKind::Na),
    }
}

/// `MATCH(..., -1)`: the mirror of `approximate_index` over a descending
/// vector — the last position whose key is still at or above the needle.
fn descending_index<F: Fn(u32) -> Value>(len: u32, key_at: &F, needle: &Value) -> Option<u32> {
    if len == 0 {
        return None;
    }
    let above = |i: u32| {
        lookup_order(&key_at(i), needle)
            .map(|o| o != core::cmp::Ordering::Less)
            .unwrap_or(false)
    };
    let (mut lo, mut hi) = (0u32, len - 1);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if above(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    above(lo).then_some(lo)
}

// -------------------------------------------------- conditional aggregation

/// A parsed `SUMIF`/`COUNTIF` criterion: an optional comparison prefix plus a
/// value. `">5"`, `"<>x"`, `"apple"`.
struct Criterion {
    op: CritOp,
    value: Value,
    /// The payload as the user wrote it, after the operator was stripped. The
    /// wildcard sub-language lives in that *text*, and by the time `value`
    /// exists `coerce_input` has already turned `"7"` into a number.
    text: Option<String>,
    /// The payload was empty: `""` means "blank" and `"<>"` means "not blank".
    /// A state of its own, because the empty string is not the empty *cell*
    /// and comparing them by equality gets both cases wrong.
    empty: bool,
}

#[derive(PartialEq)]
enum CritOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

fn parse_criterion(v: &Value) -> Criterion {
    let Value::Text(s) = v else {
        return Criterion {
            op: CritOp::Eq,
            value: v.clone(),
            text: None,
            empty: false,
        };
    };
    let (op, rest) = if let Some(r) = s.strip_prefix("<>") {
        (CritOp::Ne, r)
    } else if let Some(r) = s.strip_prefix(">=") {
        (CritOp::Ge, r)
    } else if let Some(r) = s.strip_prefix("<=") {
        (CritOp::Le, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (CritOp::Gt, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (CritOp::Lt, r)
    } else if let Some(r) = s.strip_prefix('=') {
        (CritOp::Eq, r)
    } else {
        (CritOp::Eq, s.as_str())
    };
    // A criterion's payload is text the user wrote, so it goes through the
    // same Excel-shaped text→number rule the compat profile uses.
    let value = Profile::Compat.coerce_input(rest);
    Criterion {
        op,
        value,
        text: Some(String::from(rest)),
        empty: rest.is_empty(),
    }
}

/// The number a criterion comparison sees. Unlike a *lookup*, the criteria
/// sub-language coerces: `COUNTIF(range, 7)` counts a cell holding the **text**
/// `"7"`, and so does `COUNTIF(range, "7")`. Measured — the two families
/// disagree about this, which is why they cannot share `values_equal`.
fn criteria_number(v: &Value) -> Option<f64> {
    match v {
        Value::Text(s) => Profile::Compat.to_number(&Value::Text(s.clone())).ok(),
        other => numeric(other),
    }
}

fn matches(c: &Criterion, cell: &Value) -> bool {
    // `""` is the blank cell and `"<>"` is every non-blank one. Neither is a
    // comparison against the empty string.
    if c.empty {
        let blank = *cell == Value::Blank;
        return match c.op {
            CritOp::Ne => !blank,
            _ => blank,
        };
    }
    match c.op {
        CritOp::Eq => criterion_equal(c, cell),
        CritOp::Ne => !criterion_equal(c, cell),
        _ => match (criteria_number(cell), criteria_number(&c.value)) {
            (Some(x), Some(y)) => match c.op {
                CritOp::Lt => x < y,
                CritOp::Le => x <= y,
                CritOp::Gt => x > y,
                CritOp::Ge => x >= y,
                _ => false,
            },
            _ => false,
        },
    }
}

/// Equality inside the criteria sub-language: wildcards first, then numeric
/// coercion across the text boundary, then a case-insensitive text compare.
fn criterion_equal(c: &Criterion, cell: &Value) -> bool {
    if let Some(pattern) = &c.text {
        if has_wildcard(pattern) {
            // A wildcard can only match text — `COUNTIF(range,"*")` counts the
            // text cells and no others.
            return match cell {
                Value::Text(s) => wildcard_match(pattern, s),
                _ => false,
            };
        }
        let literal = unescape_wildcards(pattern);
        if let Value::Text(s) = cell {
            if literal.eq_ignore_ascii_case(s) {
                return true;
            }
        }
    }
    match (criteria_number(cell), criteria_number(&c.value)) {
        (Some(x), Some(y)) => x == y,
        _ => values_equal(cell, &c.value),
    }
}

fn f_countif<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let _ = ctx;
    let (Some(range), Some(crit)) = (ops.first(), ops.get(1)) else {
        return err(ErrorKind::Value);
    };
    let c = parse_criterion(&crit.scalar());
    num(range.cells().iter().filter(|v| matches(&c, v)).count() as f64)
}

fn selected(ops: &[Operand], sum_range_index: usize) -> Option<(Vec<Value>, Vec<bool>)> {
    let range = ops.first()?;
    let crit = parse_criterion(&ops.get(1)?.scalar());
    let cells = range.cells();
    let mask: Vec<bool> = cells.iter().map(|v| matches(&crit, v)).collect();
    let target = match ops.get(sum_range_index) {
        Some(o) => o.cells(),
        None => cells,
    };
    Some((target, mask))
}

fn f_sumif<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let Some((target, mask)) = selected(ops, 2) else {
        return err(ErrorKind::Value);
    };
    let picked: Vec<Operand> = target
        .iter()
        .zip(mask.iter())
        .filter(|(_, m)| **m)
        .map(|(v, _)| Operand::Value(v.clone()))
        .collect();
    f_sum(&picked, ctx)
}

fn f_averageif<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let Some((target, mask)) = selected(ops, 2) else {
        return err(ErrorKind::Value);
    };
    let picked: Vec<Operand> = target
        .iter()
        .zip(mask.iter())
        .filter(|(_, m)| **m)
        .map(|(v, _)| Operand::Value(v.clone()))
        .collect();
    f_average(&picked, ctx)
}

/// `SUMIFS(sum_range, crit_range1, crit1, ...)` and
/// `COUNTIFS(crit_range1, crit1, ...)` — note the differing argument order,
/// which is Excel's, not a slip.
fn f_ifs_agg<G: Grid>(ops: &[Operand], ctx: &Context<G>, summing: bool) -> Operand {
    let first_pair = if summing { 1 } else { 0 };
    if ops.len() < first_pair + 2 {
        return err(ErrorKind::Value);
    }
    let mut mask: Option<Vec<bool>> = None;
    let mut i = first_pair;
    while i + 1 < ops.len() {
        let cells = ops[i].cells();
        // Every criteria range must be the same shape. Excel refuses a
        // mismatch outright rather than zipping to the shorter one, and it is
        // right to: the pairing would be silently wrong, not merely short.
        if mask.as_ref().is_some_and(|m| m.len() != cells.len()) {
            return err(ErrorKind::Value);
        }
        let crit = parse_criterion(&ops[i + 1].scalar());
        let this: Vec<bool> = cells.iter().map(|v| matches(&crit, v)).collect();
        mask = Some(match mask {
            None => this,
            Some(prev) => prev
                .iter()
                .zip(this.iter())
                .map(|(a, b)| *a && *b)
                .collect(),
        });
        i += 2;
    }
    let mask = mask.unwrap_or_default();
    if summing {
        let target = ops[0].cells();
        let picked: Vec<Operand> = target
            .iter()
            .zip(mask.iter())
            .filter(|(_, m)| **m)
            .map(|(v, _)| Operand::Value(v.clone()))
            .collect();
        f_sum(&picked, ctx)
    } else {
        num(mask.iter().filter(|m| **m).count() as f64)
    }
}

// --------------------------------------------------------------- dates

/// Excel's two date systems, and the fictions each one carries.
///
/// Every rule below is **measured** against real Excel 16.0 over COM (ADR-024,
/// docs/50) rather than taken from documentation, which docs/32 warns lies
/// about exactly this area. TD-33 is the debt this pays; the vectors are
/// `tools/oracle-capture/vectors{,-1904}/{DATE,DAY,MONTH,YEAR,WEEKDAY}.json`.
///
/// # 1900 (the default, and the one with the fictions)
/// * Serial 1 is 1900-01-01.
/// * **Serial 0 is "1900-01-00"** — `YEAR(0)=1900`, `MONTH(0)=1`, `DAY(0)=0`.
///   Not an error and not 1899-12-31: a day-zero that Excel prints as such.
/// * **Serial 60 is 1900-02-29**, a date that never existed, inherited from
///   Lotus 1-2-3. Every serial from 61 on is therefore one *larger* than the
///   true day count. `Compat` reproduces this because otherwise every date in
///   every imported file shifts by a day.
/// * The last serial is 2,958,465 (9999-12-31); 2,958,466 is `#NUM!`.
///
/// # 1904 (the Macintosh system, `workbookPr/@date1904`)
/// * Serial 0 is 1904-01-01, and it is a **real** date — `DAY(0)` is 1, not 0.
/// * No phantom leap day: the system starts after 1900, so the Lotus bug has
///   nothing to be compatible with.
/// * The last serial is 2,957,003, exactly 1,462 less than the 1900 system's —
///   which is also the constant offset between the two for every shared date.
///
/// `Strict` keeps neither fiction: its serials run from a real 1900-01-01 with
/// no phantom day, so they are one *smaller* than Excel's from 1900-03-01 on.
/// That divergence is the point of the profile split (docs/32).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DateSystem {
    /// Serial 1 = 1900-01-01, with the day-zero and phantom-leap-day fictions.
    #[default]
    Excel1900,
    /// Serial 0 = 1904-01-01, no fictions.
    Excel1904,
}

impl DateSystem {
    /// Days from the Unix epoch to this system's serial 0.
    fn day_zero(self) -> i64 {
        match self {
            // 1899-12-31, the day *before* serial 1.
            DateSystem::Excel1900 => DAYS_1899_12_31,
            DateSystem::Excel1904 => DAYS_1904_01_01,
        }
    }

    /// The largest serial the system accepts: 9999-12-31 in each.
    fn max_serial(self) -> i64 {
        match self {
            DateSystem::Excel1900 => 2_958_465,
            DateSystem::Excel1904 => 2_957_003,
        }
    }
}

/// Serial → calendar date, or `None` when the serial is outside the system.
///
/// The two fictions live here and nowhere else, which is what lets `f_date`,
/// `date_part` and `f_weekday` stay ordinary.
fn serial_to_ymd(profile: &Profile, sys: DateSystem, serial: i64) -> Option<(i64, u32, u32)> {
    if serial < 0 || serial > sys.max_serial() {
        return None;
    }
    if sys == DateSystem::Excel1900 && *profile == Profile::Compat {
        // Day zero, which Excel renders as the 0th of January 1900.
        if serial == 0 {
            return Some((1900, 1, 0));
        }
        // The phantom day.
        if serial == 60 {
            return Some((1900, 2, 29));
        }
    }
    let real = phantom_removed(profile, sys, serial);
    Some(civil_from_days(real + sys.day_zero()))
}

/// Undoes the phantom day, mapping a serial onto a true count of days since
/// the system's day zero. A no-op everywhere except 1900/`Compat` past day 60.
fn phantom_removed(profile: &Profile, sys: DateSystem, serial: i64) -> i64 {
    if sys == DateSystem::Excel1900 && *profile == Profile::Compat && serial > 60 {
        serial - 1
    } else {
        serial
    }
}

/// Calendar date → serial, or `None` when it falls outside the system.
///
/// The inverse of `phantom_removed`: February 1900 gains a day because serial
/// 60 falls inside it, which is why `DATE(1900,2,29)` is 60 rather than an
/// error and why `DATE(1900,3,1)` is 61 rather than 60.
fn ymd_to_serial(profile: &Profile, sys: DateSystem, y: i64, m: u32, d: i64) -> Option<i64> {
    let days = days_from_civil(y, m, 1) - sys.day_zero();
    let mut serial = days;
    if sys == DateSystem::Excel1900 && *profile == Profile::Compat && days >= 60 {
        serial += 1;
    }
    // Days are added *after* the mapping, so a day index that walks across the
    // phantom picks it up. Excel's own out-of-range day rollover is exactly
    // this addition, which is why `DATE(2024,1,32)` needs no separate rule.
    serial = serial.checked_add(d - 1)?;
    (0..=sys.max_serial()).contains(&serial).then_some(serial)
}

/// Days from the Unix epoch (1970-01-01) back to 1899-12-31, the day *before*
/// serial 1. Negative because it precedes the epoch.
///
/// 1900-01-01 is 25,567 days before 1970-01-01 (70 years of 365 days plus the
/// 17 leap days from 1904 to 1968), so 1899-12-31 is 25,568. Getting this off
/// by one shifted every date in the engine by a day.
const DAYS_1899_12_31: i64 = -25568;

/// Days from the Unix epoch back to 1904-01-01, the 1904 system's serial 0.
/// 1,460 days after 1900-01-01 — four years none of which is a leap year,
/// because 1900 is not one. That is also why the two systems differ by 1,462
/// rather than 1,461: the 1900 system counts a day that does not exist.
const DAYS_1904_01_01: i64 = DAYS_1899_12_31 + 1 + 1460;

/// Howard Hinnant's `days_from_civil`: exact, integer-only, valid for any
/// proleptic Gregorian date. No calendar library, no `std` (DP-A3).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m as i64) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `DATE(year, month, day)`.
///
/// Three separate normalisations, none of which follows from the others, all
/// measured (docs/50 finding 6):
/// * **Years 0–1899 mean 1900+year.** `DATE(0,1,1)` is 1900-01-01 and
///   `DATE(1899,12,31)` is **3799**-12-31, not 1899-12-31. A year above 9999,
///   or below zero, is `#NUM!`.
/// * **Months roll over**, so `DATE(2024,13,1)` is January 2025 and
///   `DATE(2024,0,1)` is December 2023. The offset above is applied to the
///   *argument*, before this rollover — which is why `DATE(1900,0,1)` lands in
///   December 1899 and is `#NUM!` rather than being rescued by it.
/// * **Days roll over too**, and do it in serial space, so a day index that
///   crosses the phantom picks it up (`ymd_to_serial`).
fn f_date<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let (y, m, d) = match (
        num_arg(ops, 0, ctx),
        num_arg(ops, 1, ctx),
        num_arg(ops, 2, ctx),
    ) {
        // Truncation, not rounding: DATE(2024,1,1.9) is 1 January.
        (Ok(a), Ok(b), Ok(c)) => (floor(a) as i64, floor(b) as i64, floor(c) as i64),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return val(Value::Error(e)),
    };
    if !(0..=9999).contains(&y) {
        return err(ErrorKind::Num);
    }
    let y = if y < 1900 { y + 1900 } else { y };
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    match ymd_to_serial(&ctx.profile, ctx.dates, y, m as u32, d) {
        Some(s) => num(s as f64),
        None => err(ErrorKind::Num),
    }
}

fn date_part<G: Grid, F: Fn(i64, u32, u32) -> f64>(
    ops: &[Operand],
    ctx: &Context<G>,
    f: F,
) -> Operand {
    match date_serial_arg(ops, 0, ctx) {
        Ok(s) => match serial_to_ymd(&ctx.profile, ctx.dates, s) {
            Some((y, m, d)) => num(f(y, m, d)),
            None => err(ErrorKind::Num),
        },
        Err(e) => val(Value::Error(e)),
    }
}

/// `WEEKDAY(serial, [return_type])`.
///
/// The weekday is `serial mod 7` in Excel's own numbering, *not* the true
/// weekday of the date: in the 1900 system serial 1 is reported as a Sunday
/// where 1900-01-01 was really a Monday, because the phantom day has not been
/// inserted yet at that point in the sequence. Reproducing the arithmetic
/// rather than consulting the calendar is what makes serials 1–59 agree with
/// Excel (measured: `WEEKDAY(0)` is 7 in 1900 and 6 in 1904).
fn f_weekday<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let serial = match date_serial_arg(ops, 0, ctx) {
        Ok(s) => s,
        Err(e) => return val(Value::Error(e)),
    };
    let kind = match num_arg_or(ops, 1, ctx, 1.0) {
        Ok(k) => floor(k) as i64,
        Err(e) => return val(Value::Error(e)),
    };
    if serial < 0 || serial > ctx.dates.max_serial() {
        return err(ErrorKind::Num);
    }
    // Sunday = 0. The offsets place each system's serial 0 on its real
    // weekday: 1900-01-00 is a Saturday, 1904-01-01 a Friday.
    let shift = match ctx.dates {
        DateSystem::Excel1900 => 6,
        DateSystem::Excel1904 => 5,
    };
    let sunday_zero = (serial + shift).rem_euclid(7);
    // `start` is the weekday that the return type calls 1, in Sunday-zero
    // terms. Types 11..=17 name Monday..Sunday; 1/2/3 are the legacy spellings
    // (2 == 11 and 1 == 17, which the vectors confirm). Everything else,
    // including 0 and 4..=10, is #NUM!.
    let (start, base) = match kind {
        1 => (0, 1),
        2 => (1, 1),
        3 => (1, 0),
        11..=17 => (((kind - 11) + 1).rem_euclid(7), 1),
        _ => return err(ErrorKind::Num),
    };
    num(((sunday_zero - start).rem_euclid(7) + base) as f64)
}

/// A serial argument for a date function, which also accepts an ISO-8601 date
/// string — `YEAR("2024-03-15")` is 2024 in Excel, under both date systems.
///
/// Deliberately ISO only. Excel additionally parses locale-dependent forms
/// (`15/03/2024` is in the fixtures), and those depend on the capture host's
/// locale, so implementing them from this corpus would encode one machine's
/// regional settings as engine behaviour. Recorded as TD-49 rather than
/// guessed at.
fn date_serial_arg<G: Grid>(ops: &[Operand], i: usize, ctx: &Context<G>) -> Result<i64, CellError> {
    if let Some(op) = ops.get(i) {
        if let Value::Text(s) = op.scalar() {
            if let Some(serial) = parse_iso_date(&s, &ctx.profile, ctx.dates) {
                return Ok(serial);
            }
        }
    }
    num_arg(ops, i, ctx).map(|s| floor(s) as i64)
}

/// `yyyy-mm-dd`, strictly: four digits, two, two, ASCII hyphens, nothing else.
fn parse_iso_date(text: &str, profile: &Profile, sys: DateSystem) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: core::ops::Range<usize>| -> Option<i64> {
        let mut n: i64 = 0;
        for &c in &b[r] {
            if !c.is_ascii_digit() {
                return None;
            }
            n = n * 10 + (c - b'0') as i64;
        }
        Some(n)
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    ymd_to_serial(profile, sys, y, m as u32, d)
}

const _: () = {
    // BOOTSTRAP row 6 asks for 60 functions; fail the build, not a test, if the
    // catalogue ever shrinks below it.
    assert!(CATALOGUE.len() >= 60);
};
