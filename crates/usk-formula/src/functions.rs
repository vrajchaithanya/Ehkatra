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
//! spec, not the documentation. That harness does not exist yet (A-007), so the
//! vectors in `tests/formulas.rs` encode documented behaviour and are marked as
//! such. Where this engine deliberately diverges, `Profile::Strict` is the
//! divergence and `Profile::Compat` reproduces Excel.

use crate::eval::{eval, eval_operand, to_text, Context, Grid, Operand};
use crate::parse::Ast;
use alloc::string::String;
use alloc::vec::Vec;
use usk_types::coerce::{arith, Profile};
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

fn pow10f(n: i32) -> f64 {
    let mut acc = 1.0f64;
    let mut k = n;
    while k > 0 {
        acc *= 10.0;
        k -= 1;
    }
    while k < 0 {
        acc /= 10.0;
        k += 1;
    }
    acc
}

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
            let mut total = 0.0;
            for c in &cells {
                match ctx.profile.to_number(c) {
                    Ok(n) => total += n,
                    Err(e) => return val(Value::Error(e)),
                }
            }
            let _ = float;
            num(total)
        }
    }
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
            let r = crate::eval::powf(x, y);
            if r.is_finite() {
                num(r)
            } else {
                err(ErrorKind::Num)
            }
        }
        (Err(e), _) | (_, Err(e)) => val(Value::Error(e)),
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
    let scale = pow10f(digits);
    let scaled = x * scale;
    let rounded = match mode {
        // Excel rounds half *away from zero*, not half-even. Reproducing that
        // is compatibility; the exact-decimal path is where half-even lives.
        Rounding::Half => {
            if scaled < 0.0 {
                -floor(-scaled + 0.5)
            } else {
                floor(scaled + 0.5)
            }
        }
        Rounding::Up => {
            if scaled < 0.0 {
                -ceil(-scaled)
            } else {
                ceil(scaled)
            }
        }
        Rounding::Down => {
            if scaled < 0.0 {
                -floor(-scaled)
            } else {
                floor(scaled)
            }
        }
    };
    num(rounded / scale)
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
        return num(0.0);
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
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.trim().chars() {
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
    // v0.1 implements exact match only. Excel's default is the *approximate*
    // match, which requires a sorted key and a binary search; supporting it
    // without the sortedness contract would return confidently wrong answers,
    // so an explicit TRUE 4th argument is rejected rather than faked.
    if let Some(o) = ops.get(3) {
        match truthy(ctx, &o.scalar()) {
            Ok(true) => return err(ErrorKind::Na),
            Ok(false) => {}
            Err(e) => return val(Value::Error(e)),
        }
    }

    let (scan, other) = if vertical { (rows, cols) } else { (cols, rows) };
    if index >= other {
        return err(ErrorKind::Ref);
    }
    for i in 0..scan {
        let key = if vertical {
            cell_at(&cells, cols, i, 0)
        } else {
            cell_at(&cells, cols, 0, i)
        };
        if values_equal(&key, &needle) {
            let found = if vertical {
                cell_at(&cells, cols, i, index)
            } else {
                cell_at(&cells, cols, index, i)
            };
            return val(found);
        }
    }
    err(ErrorKind::Na)
}

fn f_xlookup<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let _ = ctx;
    let needle = match scalar(ops, 0) {
        Some(v) => v,
        None => return err(ErrorKind::Value),
    };
    let (Some(lookup), Some(result)) = (ops.get(1), ops.get(2)) else {
        return err(ErrorKind::Ref);
    };
    let keys = lookup.cells();
    let values = result.cells();
    for (i, k) in keys.iter().enumerate() {
        if values_equal(k, &needle) {
            return val(values.get(i).cloned().unwrap_or(Value::Blank));
        }
    }
    // XLOOKUP's 4th argument is the not-found fallback — the reason it exists.
    match ops.get(3) {
        Some(o) => val(o.scalar()),
        None => err(ErrorKind::Na),
    }
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
    // A single-row or single-column range indexes with one coordinate.
    let (ri, ci) = if rows == 1 && c == 0 {
        (0i64, r)
    } else if cols == 1 && c == 0 {
        (r, 1i64)
    } else {
        (r, c)
    };
    if ri < 1 || ci < 1 || ri as u32 > rows || ci as u32 > cols {
        return err(ErrorKind::Ref);
    }
    val(cell_at(&cells, cols, ri as u32 - 1, ci as u32 - 1))
}

fn f_match<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let needle = match scalar(ops, 0) {
        Some(v) => v,
        None => return err(ErrorKind::Value),
    };
    let Some(range) = ops.get(1) else {
        return err(ErrorKind::Ref);
    };
    // Match type 0 (exact) only, for the same reason VLOOKUP is exact-only.
    if let Some(_o) = ops.get(2) {
        match num_arg(ops, 2, ctx) {
            Ok(t) if t != 0.0 => return err(ErrorKind::Na),
            Ok(_) => {}
            Err(e) => return val(Value::Error(e)),
        }
    }
    for (i, c) in range.cells().iter().enumerate() {
        if values_equal(c, &needle) {
            return num((i + 1) as f64);
        }
    }
    err(ErrorKind::Na)
}

// -------------------------------------------------- conditional aggregation

/// A parsed `SUMIF`/`COUNTIF` criterion: an optional comparison prefix plus a
/// value. `">5"`, `"<>x"`, `"apple"`.
struct Criterion {
    op: CritOp,
    value: Value,
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
    Criterion { op, value }
}

fn matches(c: &Criterion, cell: &Value) -> bool {
    match c.op {
        CritOp::Eq => values_equal(cell, &c.value),
        CritOp::Ne => !values_equal(cell, &c.value),
        _ => match (numeric(cell), numeric(&c.value)) {
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

/// Excel's 1900 date system: serial 1 is 1900-01-01.
///
/// Serial 60 is 1900-02-29 — a date that never existed, inherited from Lotus
/// 1-2-3. `Compat` reproduces it, because every serial after it would otherwise
/// shift by a day against real Excel files. `Strict` does not, so its serials
/// are one *larger* than Excel's for any date from 1900-03-01 onward. That
/// divergence is the point of the profile split (docs/32).
fn serial_to_ymd(profile: &Profile, serial: i64) -> Option<(i64, u32, u32)> {
    if serial < 1 {
        return None;
    }
    let adjusted = match profile {
        Profile::Compat => {
            if serial == 60 {
                // The phantom day. Excel reports 29 February 1900.
                return Some((1900, 2, 29));
            } else if serial > 60 {
                serial - 1
            } else {
                serial
            }
        }
        Profile::Strict => serial,
    };
    // Days since 1899-12-31, converted via the civil-from-days algorithm.
    Some(civil_from_days(adjusted + DAYS_1899_12_31))
}

fn ymd_to_serial(profile: &Profile, y: i64, m: u32, d: u32) -> Option<i64> {
    let days = days_from_civil(y, m, d) - DAYS_1899_12_31;
    if days < 1 {
        return None;
    }
    Some(match profile {
        // Re-insert the phantom day for anything at or after 1900-03-01.
        Profile::Compat if days >= 60 => days + 1,
        _ => days,
    })
}

/// Days from the Unix epoch (1970-01-01) back to 1899-12-31, the day *before*
/// serial 1. Negative because it precedes the epoch.
///
/// 1900-01-01 is 25,567 days before 1970-01-01 (70 years of 365 days plus the
/// 17 leap days from 1904 to 1968), so 1899-12-31 is 25,568. Getting this off
/// by one shifted every date in the engine by a day.
const DAYS_1899_12_31: i64 = -25568;

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

fn f_date<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    let (y, m, d) = match (
        num_arg(ops, 0, ctx),
        num_arg(ops, 1, ctx),
        num_arg(ops, 2, ctx),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a as i64, b as i64, c as i64),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return val(Value::Error(e)),
    };
    // Excel normalises out-of-range months and days by rolling over, so
    // DATE(2024, 13, 1) is January 2025.
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    let base = days_from_civil(y, m as u32, 1);
    let (ny, nm, nd) = civil_from_days(base + d - 1);
    match ymd_to_serial(&ctx.profile, ny, nm, nd) {
        Some(s) => num(s as f64),
        None => err(ErrorKind::Num),
    }
}

fn date_part<G: Grid, F: Fn(i64, u32, u32) -> f64>(
    ops: &[Operand],
    ctx: &Context<G>,
    f: F,
) -> Operand {
    match num_arg(ops, 0, ctx) {
        Ok(s) => match serial_to_ymd(&ctx.profile, floor(s) as i64) {
            Some((y, m, d)) => num(f(y, m, d)),
            None => err(ErrorKind::Num),
        },
        Err(e) => val(Value::Error(e)),
    }
}

fn f_weekday<G: Grid>(ops: &[Operand], ctx: &Context<G>) -> Operand {
    match num_arg(ops, 0, ctx) {
        Ok(s) => {
            let serial = floor(s) as i64;
            if serial < 1 {
                return err(ErrorKind::Num);
            }
            // Serial 1 (1900-01-01) is a Sunday in Excel's world, which is
            // weekday 1 under the default return type.
            num(((serial - 1).rem_euclid(7) + 1) as f64)
        }
        Err(e) => val(Value::Error(e)),
    }
}

const _: () = {
    // BOOTSTRAP row 6 asks for 60 functions; fail the build, not a test, if the
    // catalogue ever shrinks below it.
    assert!(CATALOGUE.len() >= 60);
};
