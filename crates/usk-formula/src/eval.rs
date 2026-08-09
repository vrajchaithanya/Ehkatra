//! Formula evaluation (docs/12).
//!
//! Evaluation is total: every path returns a `Value`, and an error is a value
//! carrying its origin, never a panic and never a thrown exception across a
//! boundary (DP-A10, docs/04 invariant 5).
//!
//! Arithmetic is delegated to `usk_types::coerce::arith` rather than
//! reimplemented, so decimal promotion, profile-driven coercion and error
//! propagation behave identically whether a value was produced by a formula or
//! written directly.

use crate::functions::DateSystem;
use crate::parse::{Ast, BinOp, UnOp, A1};
use alloc::string::String;
use alloc::vec::Vec;
use usk_types::coerce::{arith, Profile};
use usk_types::{ArithOp, CellError, ErrorKind, Origin, TypeTag, Value};

/// What a formula can read. Implemented over the workbook by callers; kept as a
/// trait so the evaluator is testable against a fixture grid and so nothing
/// here depends on how cells are stored.
pub trait Grid {
    /// Reads a cell by view ordinals. `None` means "outside the grid", which is
    /// `#REF!`; a blank but in-range cell is `Some(Value::Blank)`.
    fn read(&self, row: u32, col: u32) -> Option<Value>;

    /// Current extent, used to clamp ranges. `(rows, cols)`.
    fn extent(&self) -> (u32, u32);
}

/// An empty grid, so operator-only formulas can be evaluated with no workbook.
pub struct NoGrid;

impl Grid for NoGrid {
    fn read(&self, _row: u32, _col: u32) -> Option<Value> {
        None
    }
    fn extent(&self) -> (u32, u32) {
        (0, 0)
    }
}

/// Everything an evaluation needs that is not the formula itself.
///
/// `today` and `now` are **injected**, never read from a clock: DP-A2 forbids
/// ambient time in kernel paths, and ADR-009 materialises volatiles at the
/// Calculation Authority so a replay produces the same result forever.
pub struct Context<'a, G: Grid> {
    pub grid: &'a G,
    pub profile: Profile,
    /// Materialised `TODAY()` as a date serial.
    pub today: i32,
    /// Materialised `NOW()` as a date serial plus fraction of day.
    pub now: f64,
    /// Which date system the workbook uses (TD-33). This is a workbook-level
    /// property in the file format (`workbookPr/@date1904`), so it belongs on
    /// the evaluation context rather than on any one function: it changes what
    /// every serial in the workbook *means*, not how a function behaves.
    pub dates: DateSystem,
}

impl<'a, G: Grid> Context<'a, G> {
    pub fn new(grid: &'a G, profile: Profile) -> Self {
        Context {
            grid,
            profile,
            today: 0,
            now: 0.0,
            dates: DateSystem::default(),
        }
    }

    /// Selects the workbook's date system. Chainable, so a caller needing the
    /// non-default one says so where the context is built.
    pub fn with_dates(mut self, dates: DateSystem) -> Self {
        self.dates = dates;
        self
    }
}

/// A single value or a rectangular block of them — what an argument can be.
///
/// Ranges stay unflattened until a function asks, because `SUM` wants every
/// cell while `IF` wants one, and `COUNT` needs to know which cells were blank.
#[derive(Clone, PartialEq, Debug)]
pub enum Operand {
    Value(Value),
    Range {
        rows: u32,
        cols: u32,
        cells: Vec<Value>,
    },
}

impl Operand {
    /// Collapses a range to a single value for scalar contexts.
    ///
    /// v0.1 takes the top-left cell. Excel would apply implicit intersection
    /// (`@`) against the calling cell's row/column, which needs the caller's
    /// position; that arrives with the dependency graph in Row 7, and is
    /// tracked rather than faked.
    pub fn scalar(&self) -> Value {
        match self {
            Operand::Value(v) => v.clone(),
            Operand::Range { cells, .. } => cells.first().cloned().unwrap_or(Value::Blank),
        }
    }

    /// Every cell, row-major.
    pub fn cells(&self) -> Vec<Value> {
        match self {
            Operand::Value(v) => alloc::vec![v.clone()],
            Operand::Range { cells, .. } => cells.clone(),
        }
    }

    pub fn as_error(&self) -> Option<CellError> {
        match self {
            Operand::Value(v) => v.as_error(),
            Operand::Range { cells, .. } => cells.iter().find_map(|c| c.as_error()),
        }
    }
}

/// Evaluates an AST to a single value.
pub fn eval<G: Grid>(ast: &Ast, ctx: &Context<G>) -> Value {
    eval_operand(ast, ctx).scalar()
}

/// Evaluates a formula **as a formula** — i.e. at the position where Excel's
/// cancellation adjustment is allowed to fire.
///
/// docs/50 finding 2: the `+`/`-` adjustment is *positional*. It applies to a
/// formula's top-level result and to nothing else, which is why
/// `=0.1+0.2-0.3` is `0` while `=(0.1+0.2-0.3)` is `5.55e-17` and
/// `=1/(0.1+0.2-0.3)` is `2^54`. Every caller that is storing a cell's value
/// should use this; every caller evaluating a sub-expression should use
/// [`eval`], and the difference between them is exactly the rule.
///
/// The operands are evaluated **once**: the add/subtract arm reproduces
/// `eval_operand`'s `Binary` case rather than calling it and then re-evaluating
/// the operands to recover their magnitudes. The first version did re-evaluate,
/// which doubled the work of every top-level `+`/`-` formula and moved
/// W-CHAIN-100K from 92.6 ms to 145.2 ms — the bench caught it during the v0.1
/// audit, and it is the reason the duplication below is deliberate.
pub fn eval_top<G: Grid>(ast: &Ast, ctx: &Context<G>) -> Value {
    let Ast::Binary(op @ (BinOp::Add | BinOp::Sub), lhs, rhs) = ast else {
        return eval(ast, ctx);
    };
    let a = eval(lhs, ctx);
    let b = eval(rhs, ctx);
    let value = binary(ctx, *op, &a, &b);
    let Value::Number(result) = value else {
        return value;
    };
    let (Some(x), Some(y)) = (as_number(&a), as_number(&b)) else {
        return value;
    };
    Value::Number(usk_types::coerce::compat_final_adjust(
        &ctx.profile,
        result,
        if x.abs() > y.abs() { x } else { y },
    ))
}

/// The float view of a value, for the cancellation rule's operand magnitude.
/// Deliberately does not coerce text: a text operand means the arithmetic
/// already errored or promoted, and neither is this rule's business.
fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => Some(*n),
        Value::Decimal(d) => Some(d.to_f64()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Blank => Some(0.0),
        _ => None,
    }
}

/// Evaluates an AST, preserving range-ness so callers like `SUM` see all cells.
pub fn eval_operand<G: Grid>(ast: &Ast, ctx: &Context<G>) -> Operand {
    match ast {
        Ast::Literal(v) => Operand::Value(v.clone()),
        Ast::Invalid(kind) => Operand::Value(Value::Error(CellError::new(*kind, Origin::Authored))),
        // An unresolved name is `#NAME?`. docs/12 wants these to live-rebind
        // when the name later exists; that requires the name table and the
        // dependency graph, so v0.1 reports rather than rebinds.
        Ast::Name(_) => Operand::Value(Value::Error(CellError::new(
            ErrorKind::Name,
            Origin::Authored,
        ))),
        Ast::Reference(r) => Operand::Value(read_cell(ctx, r)),
        Ast::Range(a, b) => read_range(ctx, a, b),
        // Transparent to evaluation. The node exists only so `eval_top` can
        // see that a formula's outermost operator is not an add/subtract.
        Ast::Paren(inner) => eval_operand(inner, ctx),
        Ast::Unary(op, inner) => {
            let v = eval(inner, ctx);
            Operand::Value(match op {
                UnOp::Plus => v,
                UnOp::Neg => arith(&ctx.profile, ArithOp::Sub, &Value::Number(0.0), &v),
            })
        }
        Ast::Percent(inner) => {
            let v = eval(inner, ctx);
            Operand::Value(arith(&ctx.profile, ArithOp::Div, &v, &Value::Number(100.0)))
        }
        Ast::Binary(op, lhs, rhs) => {
            let a = eval(lhs, ctx);
            let b = eval(rhs, ctx);
            Operand::Value(binary(ctx, *op, &a, &b))
        }
        Ast::Call { name, args } => crate::functions::call(name, args, ctx),
    }
}

fn read_cell<G: Grid>(ctx: &Context<G>, r: &A1) -> Value {
    match ctx.grid.read(r.row, r.col) {
        Some(v) => v,
        None => Value::Error(CellError::new(ErrorKind::Ref, Origin::Authored)),
    }
}

fn read_range<G: Grid>(ctx: &Context<G>, a: &A1, b: &A1) -> Operand {
    let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
    let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
    let (max_rows, max_cols) = ctx.grid.extent();
    if r0 >= max_rows || c0 >= max_cols {
        return Operand::Value(Value::Error(CellError::new(
            ErrorKind::Ref,
            Origin::Authored,
        )));
    }
    // Clamp to the live grid rather than erroring: a range that overhangs the
    // used area is ordinary in a spreadsheet.
    let r1 = r1.min(max_rows.saturating_sub(1));
    let c1 = c1.min(max_cols.saturating_sub(1));

    let mut cells = Vec::new();
    for row in r0..=r1 {
        for col in c0..=c1 {
            cells.push(ctx.grid.read(row, col).unwrap_or(Value::Blank));
        }
    }
    Operand::Range {
        rows: r1 - r0 + 1,
        cols: c1 - c0 + 1,
        cells,
    }
}

fn binary<G: Grid>(ctx: &Context<G>, op: BinOp, a: &Value, b: &Value) -> Value {
    // Errors win over everything, carrying their origin (docs/04 §5).
    if let Some(e) = a.as_error().or_else(|| b.as_error()) {
        return Value::Error(e);
    }
    match op {
        BinOp::Add => arith(&ctx.profile, ArithOp::Add, a, b),
        BinOp::Sub => arith(&ctx.profile, ArithOp::Sub, a, b),
        BinOp::Mul => arith(&ctx.profile, ArithOp::Mul, a, b),
        BinOp::Div => arith(&ctx.profile, ArithOp::Div, a, b),
        BinOp::Pow => power(ctx, a, b),
        BinOp::Concat => concat(ctx, a, b),
        BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
            compare(ctx, op, a, b)
        }
    }
}

fn power<G: Grid>(ctx: &Context<G>, a: &Value, b: &Value) -> Value {
    let (x, y) = match (ctx.profile.to_number(a), ctx.profile.to_number(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return Value::Error(e),
    };
    let r = powf(x, y);
    if r.is_finite() {
        Value::Number(r)
    } else {
        Value::Error(CellError::new(
            ErrorKind::Num,
            Origin::Arithmetic { op: ArithOp::Mul },
        ))
    }
}

/// `x^y` without `std`/libm.
///
/// Integer exponents — overwhelmingly the common case in spreadsheets — are
/// computed by exact repeated multiplication. Fractional exponents go through
/// `exp(y · ln x)` implemented here in series form, because the kernel has no
/// libm and DP-A2 forbids depending on a platform math library whose last-bit
/// behaviour varies between systems.
pub fn powf(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    if y == (y as i64) as f64 && y.abs() < 1024.0 {
        let n = y as i64;
        let mut acc = 1.0f64;
        let mut base = x;
        let mut e = n.unsigned_abs();
        while e > 0 {
            if e & 1 == 1 {
                acc *= base;
            }
            base *= base;
            e >>= 1;
        }
        return if n < 0 { 1.0 / acc } else { acc };
    }
    if x < 0.0 {
        // A negative base with a fractional exponent has no real result.
        return f64::NAN;
    }
    exp(y * ln(x))
}

/// Natural log by argument reduction to [1, 2) plus an atanh series.
pub(crate) fn ln(mut x: f64) -> f64 {
    if x <= 0.0 {
        return f64::NAN;
    }
    let mut exponent = 0i32;
    while x >= 2.0 {
        x /= 2.0;
        exponent += 1;
    }
    while x < 1.0 {
        x *= 2.0;
        exponent -= 1;
    }
    // ln x = 2·atanh((x-1)/(x+1)), which converges fast on [1, 2).
    let t = (x - 1.0) / (x + 1.0);
    let t2 = t * t;
    let mut term = t;
    let mut sum = 0.0;
    let mut k = 0;
    while k < 40 {
        sum += term / (2 * k + 1) as f64;
        term *= t2;
        k += 1;
    }
    2.0 * sum + exponent as f64 * core::f64::consts::LN_2
}

/// `e^x` by range reduction plus a Taylor series.
pub(crate) fn exp(x: f64) -> f64 {
    if x > 709.0 {
        return f64::INFINITY;
    }
    if x < -745.0 {
        return 0.0;
    }
    // e^x = 2^k · e^r, with r small.
    let k = (x / core::f64::consts::LN_2 + if x >= 0.0 { 0.5 } else { -0.5 }) as i32;
    let r = x - k as f64 * core::f64::consts::LN_2;
    let mut term = 1.0f64;
    let mut sum = 1.0f64;
    let mut n = 1;
    while n < 30 {
        term *= r / n as f64;
        sum += term;
        n += 1;
    }
    let mut result = sum;
    let mut e = k;
    while e > 0 {
        result *= 2.0;
        e -= 1;
    }
    while e < 0 {
        result /= 2.0;
        e += 1;
    }
    result
}

fn concat<G: Grid>(ctx: &Context<G>, a: &Value, b: &Value) -> Value {
    match (to_text(ctx, a), to_text(ctx, b)) {
        (Ok(mut x), Ok(y)) => {
            x.push_str(&y);
            Value::Text(x)
        }
        (Err(e), _) | (_, Err(e)) => Value::Error(e),
    }
}

/// Renders a value as text for `&` and the text functions.
pub fn to_text<G: Grid>(ctx: &Context<G>, v: &Value) -> Result<String, CellError> {
    use alloc::format;
    Ok(match v {
        Value::Blank => String::new(),
        Value::Bool(b) => String::from(if *b { "TRUE" } else { "FALSE" }),
        Value::Number(n) => format!("{n}"),
        Value::Decimal(d) => format!("{d}"),
        Value::Text(t) => t.clone(),
        Value::Error(e) => {
            let _ = ctx;
            return Err(*e);
        }
    })
}

/// Excel's comparison ordering: numbers < text < FALSE < TRUE, with blanks
/// coercing to the other operand's zero value, and text compared
/// case-insensitively.
fn compare<G: Grid>(ctx: &Context<G>, op: BinOp, a: &Value, b: &Value) -> Value {
    use core::cmp::Ordering;
    let ordering = match (a, b) {
        (Value::Text(x), Value::Text(y)) => x.to_ascii_uppercase().cmp(&y.to_ascii_uppercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        // A type rank keeps the order total, which Excel requires for sorting
        // and for `<`/`>` between mixed types.
        _ if rank(a) != rank(b) => rank(a).cmp(&rank(b)),
        _ => match (ctx.profile.to_number(a), ctx.profile.to_number(b)) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (Err(e), _) | (_, Err(e)) => return Value::Error(e),
        },
    };
    let result = match op {
        BinOp::Eq => ordering == Ordering::Equal,
        BinOp::NotEq => ordering != Ordering::Equal,
        BinOp::Lt => ordering == Ordering::Less,
        BinOp::Gt => ordering == Ordering::Greater,
        BinOp::LtEq => ordering != Ordering::Greater,
        BinOp::GtEq => ordering != Ordering::Less,
        _ => return Value::Error(CellError::refused_coercion(TypeTag::Blank, TypeTag::Bool)),
    };
    Value::Bool(result)
}

/// Excel's cross-type ordering rank: numbers sort before text, text before
/// booleans. Blank takes the numeric rank because it coerces to zero.
fn rank(v: &Value) -> u8 {
    match v {
        Value::Blank | Value::Number(_) | Value::Decimal(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
    }
}
