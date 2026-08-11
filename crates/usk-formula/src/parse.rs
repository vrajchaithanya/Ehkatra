//! Pratt parser → lossless CST → AST (docs/12, ADR-011).
//!
//! Two trees, on purpose:
//!
//! * The **CST** keeps every byte the user typed, whitespace included, so
//!   `Cst::text()` reproduces the input exactly. That is what makes
//!   format-preserving refactors, precise error carets, and AI explanation
//!   anchoring possible (ADR-011).
//! * The **AST** is the CST with trivia dropped and shapes normalised — what
//!   the binder and evaluator actually walk.
//!
//! Precedence follows Excel, which is *not* ordinary mathematical precedence.
//! The notable case is unary minus binding tighter than `^`, so `-2^2` is `4`
//! in a spreadsheet and `-4` in a maths textbook. Reproducing that is
//! compatibility, not a bug (docs/32).

use crate::lexer::{lex, Token, TokenKind};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use usk_types::coerce::Profile;
use usk_types::{ErrorKind, Value};

/// A CST node's role. Deliberately coarse: the CST records *shape*, and the AST
/// records meaning.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    Root,
    Literal,
    Reference,
    Range,
    Name,
    Call,
    ArgList,
    Unary,
    Postfix,
    Binary,
    Paren,
    /// A span the parser could not make sense of. Kept in the tree so the text
    /// still round-trips and the caret still points somewhere true.
    Error,
}

/// The lossless concrete syntax tree.
#[derive(Clone, Debug)]
pub enum Cst {
    Node { kind: NodeKind, children: Vec<Cst> },
    Token(Token),
}

impl Cst {
    /// Reconstructs the exact source text this tree came from. The round trip
    /// is a test, not an aspiration — see `cst_round_trips_every_input`.
    pub fn text(&self, source: &str) -> String {
        let mut out = String::new();
        self.write_text(source, &mut out);
        out
    }

    fn write_text(&self, source: &str, out: &mut String) {
        match self {
            Cst::Token(t) => out.push_str(t.text(source)),
            Cst::Node { children, .. } => {
                for c in children {
                    c.write_text(source, out);
                }
            }
        }
    }

    /// Byte span covered by this subtree, for error carets.
    pub fn span(&self) -> Option<(u32, u32)> {
        match self {
            Cst::Token(t) => Some((t.start, t.end)),
            Cst::Node { children, .. } => {
                let first = children.iter().find_map(|c| c.span())?;
                let last = children.iter().rev().find_map(|c| c.span())?;
                Some((first.0, last.1))
            }
        }
    }
}

/// Binary operators, in Excel's vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Plus,
}

/// An A1-style reference as *written*. The binder turns this into identities;
/// until then it is just text the user typed (DP-A6: A1 is a view).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct A1 {
    /// 0-based column ordinal in the current view.
    pub col: u32,
    /// 0-based row ordinal in the current view.
    pub row: u32,
    pub col_absolute: bool,
    pub row_absolute: bool,
}

/// The abstract syntax tree the binder and evaluator walk.
#[derive(Clone, PartialEq, Debug)]
pub enum Ast {
    Literal(Value),
    Reference(A1),
    Range(A1, A1),
    /// A parenthesised sub-expression. Evaluation is transparent, but the node
    /// is retained because Excel's cancellation rule is positional and
    /// parentheses suppress it (docs/50 finding 2, D-041 as amended).
    Paren(Box<Ast>),
    /// A defined name, or anything name-shaped the binder must resolve.
    Name(String),
    Call {
        name: String,
        args: Vec<Ast>,
    },
    Unary(UnOp, Box<Ast>),
    /// Postfix `%`, which Excel treats as "divide by 100".
    Percent(Box<Ast>),
    Binary(BinOp, Box<Ast>, Box<Ast>),
    /// A parse failure, carried as a value so evaluation never panics and the
    /// user still sees a cell result (DP-A10).
    Invalid(ErrorKind),
}

/// Result of parsing: both trees plus the source they describe.
pub struct Parsed {
    pub source: String,
    pub cst: Cst,
    pub ast: Ast,
}

/// Parses a formula. `source` may include a leading `=`, which is retained in
/// the CST and skipped by the AST.
///
/// Defaults to `Profile::Compat`, because parsing is where Excel's destructive
/// literal rules live (TD-32, D-081) and the overwhelmingly common caller is an
/// imported workbook. Use `parse_with` to say otherwise.
pub fn parse(source: &str) -> Parsed {
    parse_with(source, Profile::Compat)
}

/// Parses a formula under a stated profile.
///
/// The profile reaches the *parser* and not only the evaluator because
/// `compat_parse_15` is destructive: it truncates a literal to 15 significant
/// digits and refuses one outside Excel's range, both before evaluation ever
/// runs. No amount of correct arithmetic downstream recovers a literal that
/// changed meaning here — which is exactly why D-081 put the rule in the lexer
/// rather than treating it as a display concern.
pub fn parse_with(source: &str, profile: Profile) -> Parsed {
    let body_offset = if source.starts_with('=') { 1 } else { 0 };
    let tokens = lex(source.get(body_offset..).unwrap_or(""));
    // Shift spans so they index the original string, `=` included.
    let tokens: Vec<Token> = tokens
        .into_iter()
        .map(|t| Token {
            kind: t.kind,
            start: t.start + body_offset as u32,
            end: t.end + body_offset as u32,
        })
        .collect();

    let mut p = Parser {
        source,
        tokens,
        pos: 0,
        profile,
    };
    let mut children = Vec::new();
    if body_offset == 1 {
        children.push(Cst::Token(Token {
            kind: TokenKind::Unknown,
            start: 0,
            end: 1,
        }));
    }
    let (cst, ast) = p.expression(0);
    children.push(cst);
    // Anything left over is a trailing-garbage error, but its bytes stay.
    let mut trailing = false;
    while p.pos < p.tokens.len() {
        let tok = p.tokens[p.pos];
        trailing = trailing || tok.kind != TokenKind::Whitespace;
        children.push(Cst::Token(tok));
        p.pos += 1;
    }

    Parsed {
        source: source.to_string(),
        cst: Cst::Node {
            kind: NodeKind::Root,
            children,
        },
        ast: if trailing {
            Ast::Invalid(ErrorKind::Name)
        } else {
            ast
        },
    }
}

struct Parser<'a> {
    source: &'a str,
    profile: Profile,
    tokens: Vec<Token>,
    pos: usize,
}

/// Binding powers. Higher binds tighter. Excel's order, low to high:
/// comparison < `&` < `+ -` < `* /` < `^` < unary `-` < postfix `%` < `:`.
const BP_COMPARE: u8 = 10;
const BP_CONCAT: u8 = 20;
const BP_ADD: u8 = 30;
const BP_MUL: u8 = 40;
const BP_POW: u8 = 50;
const BP_UNARY: u8 = 60;
const BP_RANGE: u8 = 80;

impl<'a> Parser<'a> {
    /// Looks ahead past whitespace **without consuming it**, returning the next
    /// significant token's kind and index.
    ///
    /// Non-consuming is the whole trick. An earlier version buffered trivia in
    /// the parser and attached it to whichever node was built next; when a
    /// lookahead did not lead to a match, that buffer was flushed in the wrong
    /// place and the round trip silently moved the user's whitespace. Leaving
    /// `pos` alone means trivia is emitted exactly once, by whoever actually
    /// consumes the token after it.
    fn peek(&self) -> Option<(TokenKind, usize)> {
        let mut i = self.pos;
        while let Some(t) = self.tokens.get(i) {
            if t.kind == TokenKind::Whitespace {
                i += 1;
            } else {
                return Some((t.kind, i));
            }
        }
        None
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|(k, _)| k)
    }

    /// Consumes through token `idx`, pushing any skipped trivia into `out` so
    /// it lands in source order.
    fn take_through(&mut self, idx: usize, out: &mut Vec<Cst>) -> Token {
        while self.pos < idx {
            out.push(Cst::Token(self.tokens[self.pos]));
            self.pos += 1;
        }
        let t = self.tokens[idx];
        self.pos = idx + 1;
        t
    }

    /// Pratt loop: parse a prefix, then absorb infix operators while their
    /// binding power exceeds `min_bp`.
    fn expression(&mut self, min_bp: u8) -> (Cst, Ast) {
        let (mut cst, mut ast) = self.prefix();

        while let Some((kind, idx)) = self.peek() {
            // Postfix `%` binds tightest and takes no right operand.
            if kind == TokenKind::Percent {
                let mut children = alloc::vec![cst];
                let tok = self.take_through(idx, &mut children);
                children.push(Cst::Token(tok));
                cst = Cst::Node {
                    kind: NodeKind::Postfix,
                    children,
                };
                ast = Ast::Percent(Box::new(ast));
                continue;
            }
            if kind == TokenKind::Colon {
                if BP_RANGE < min_bp {
                    break;
                }
                let mut children = alloc::vec![cst];
                let tok = self.take_through(idx, &mut children);
                children.push(Cst::Token(tok));
                let (rhs_cst, rhs_ast) = self.expression(BP_RANGE + 1);
                let combined = match (&ast, &rhs_ast) {
                    (Ast::Reference(a), Ast::Reference(b)) => Ast::Range(*a, *b),
                    // `1:2` and friends are not ranges over cells.
                    _ => Ast::Invalid(ErrorKind::Ref),
                };
                children.push(rhs_cst);
                cst = Cst::Node {
                    kind: NodeKind::Range,
                    children,
                };
                ast = combined;
                continue;
            }

            let Some((op, bp, right_assoc)) = infix_op(kind) else {
                break;
            };
            if bp < min_bp {
                break;
            }
            let mut children = alloc::vec![cst];
            let tok = self.take_through(idx, &mut children);
            children.push(Cst::Token(tok));
            let next_min = if right_assoc { bp } else { bp + 1 };
            let (rhs_cst, rhs_ast) = self.expression(next_min);
            children.push(rhs_cst);
            cst = Cst::Node {
                kind: NodeKind::Binary,
                children,
            };
            // A literal Excel's parser refused invalidates the *formula*, not
            // just the subexpression, so it propagates instead of being wrapped.
            ast = match (&ast, &rhs_ast) {
                (Ast::Invalid(k), _) | (_, Ast::Invalid(k)) => Ast::Invalid(*k),
                _ => Ast::Binary(op, Box::new(ast), Box::new(rhs_ast)),
            };
        }
        (cst, ast)
    }

    fn prefix(&mut self) -> (Cst, Ast) {
        let Some((kind, idx)) = self.peek() else {
            // Nothing but trivia left: emit it so the round trip holds.
            let mut children = Vec::new();
            while self.pos < self.tokens.len() {
                children.push(Cst::Token(self.tokens[self.pos]));
                self.pos += 1;
            }
            return (
                Cst::Node {
                    kind: NodeKind::Error,
                    children,
                },
                Ast::Invalid(ErrorKind::Name),
            );
        };

        let mut children = Vec::new();
        let tok = self.take_through(idx, &mut children);
        children.push(Cst::Token(tok));

        match kind {
            TokenKind::Minus | TokenKind::Plus => {
                // Unary binds tighter than `^`: `-2^2` is 4 in Excel.
                let (rhs_cst, rhs_ast) = self.expression(BP_UNARY);
                children.push(rhs_cst);
                let op = if kind == TokenKind::Minus {
                    UnOp::Neg
                } else {
                    UnOp::Plus
                };
                (
                    Cst::Node {
                        kind: NodeKind::Unary,
                        children,
                    },
                    match rhs_ast {
                        // `-1E308` is refused because the *literal* is out of
                        // range; the minus never gets to run.
                        Ast::Invalid(k) => Ast::Invalid(k),
                        rhs => Ast::Unary(op, Box::new(rhs)),
                    },
                )
            }
            TokenKind::LParen => {
                let (inner_cst, inner_ast) = self.expression(0);
                children.push(inner_cst);
                let ast = match self.peek() {
                    Some((TokenKind::RParen, close_idx)) => {
                        let close = self.take_through(close_idx, &mut children);
                        children.push(Cst::Token(close));
                        // The parentheses stay in the AST rather than being
                        // folded away. Excel's cancellation adjustment is
                        // *positional* — it fires on a formula whose top-level
                        // node is `+`/`-`, and `=(0.1+0.2-0.3)` is not such a
                        // formula: it returns 5.55e-17 where the bare form
                        // returns 0 (docs/50 finding 2). An AST that discards
                        // the parens cannot tell those two formulas apart.
                        Ast::Paren(Box::new(inner_ast))
                    }
                    _ => Ast::Invalid(ErrorKind::Name),
                };
                (
                    Cst::Node {
                        kind: NodeKind::Paren,
                        children,
                    },
                    ast,
                )
            }
            TokenKind::Number => {
                let ast = match compat_parse_15(tok.text(self.source), self.profile) {
                    LiteralParse::Value(v) => Ast::Literal(Value::Number(v)),
                    // Excel's parser refuses the formula outright rather than
                    // storing a value it cannot represent (D-081, TD-32).
                    LiteralParse::Refused => Ast::Invalid(ErrorKind::Num),
                    // A number the lexer accepted but `f64` will not parse is
                    // out of range, which is `#NUM!`, not a crash.
                    LiteralParse::Unparseable => Ast::Literal(Value::Error(
                        usk_types::CellError::new(ErrorKind::Num, usk_types::Origin::Authored),
                    )),
                };
                (
                    Cst::Node {
                        kind: NodeKind::Literal,
                        children,
                    },
                    ast,
                )
            }
            TokenKind::Text => {
                let raw = tok.text(self.source);
                (
                    Cst::Node {
                        kind: NodeKind::Literal,
                        children,
                    },
                    Ast::Literal(Value::Text(unquote(raw))),
                )
            }
            TokenKind::ErrorLiteral => {
                let ast = match error_literal(tok.text(self.source)) {
                    Some(k) => Ast::Literal(Value::Error(usk_types::CellError::new(
                        k,
                        usk_types::Origin::Authored,
                    ))),
                    None => Ast::Invalid(ErrorKind::Name),
                };
                (
                    Cst::Node {
                        kind: NodeKind::Literal,
                        children,
                    },
                    ast,
                )
            }
            TokenKind::CellRef => {
                // A reference followed by `(` is a **function call** (TD-68).
                //
                // The lexer's `looks_like_cell_ref` is a *shape* test — letters,
                // an optional `$`, then digits — so `LOG10`, `ATAN2` and
                // `SUMXMY2` all lex as `CellRef`. Without this, `=LOG10(100)`
                // parsed as a reference to a cell in column `LOG` and the
                // `(100)` after it became trailing garbage.
                //
                // Latent until it would not have been: no function in the v0.1
                // set is spelled with a trailing digit, so nothing misbehaved —
                // and the first one added would have looked like a bug in
                // *that* function rather than in the lexer. The `Ident` arm
                // below has always made this check; it belongs here as well,
                // and in the parser rather than in each caller, because it is a
                // question about the grammar and not about any one consumer.
                if self.peek_kind() == Some(TokenKind::LParen) {
                    return self.call(children, tok.text(self.source));
                }
                let ast = match parse_a1(tok.text(self.source)) {
                    Some(r) => Ast::Reference(r),
                    None => Ast::Invalid(ErrorKind::Ref),
                };
                (
                    Cst::Node {
                        kind: NodeKind::Reference,
                        children,
                    },
                    ast,
                )
            }
            TokenKind::Ident => {
                let word = tok.text(self.source);
                if self.peek_kind() == Some(TokenKind::LParen) {
                    return self.call(children, word);
                }
                let ast = match word.to_ascii_uppercase().as_str() {
                    "TRUE" => Ast::Literal(Value::Bool(true)),
                    "FALSE" => Ast::Literal(Value::Bool(false)),
                    other => Ast::Name(other.to_string()),
                };
                (
                    Cst::Node {
                        kind: NodeKind::Name,
                        children,
                    },
                    ast,
                )
            }
            _ => (
                Cst::Node {
                    kind: NodeKind::Error,
                    children,
                },
                Ast::Invalid(ErrorKind::Name),
            ),
        }
    }

    /// `NAME( arg, arg, ... )`. `children` already holds the name token and any
    /// trivia before it.
    fn call(&mut self, mut children: Vec<Cst>, name_text: &str) -> (Cst, Ast) {
        let name = name_text.to_ascii_uppercase();
        if let Some((TokenKind::LParen, idx)) = self.peek() {
            let open = self.take_through(idx, &mut children);
            children.push(Cst::Token(open));
        }

        let mut args = Vec::new();
        let mut arg_children = Vec::new();
        let mut bad = false;

        if self.peek_kind() != Some(TokenKind::RParen) {
            loop {
                // An **omitted argument** — `IF(TRUE,,2)`, `IFERROR(1/0,)` —
                // is Excel's shorthand for "this slot is empty", and it reads
                // as blank (TD-52). It is not a parse error, which is what an
                // unconditional `expression(0)` made of it. The empty slot
                // contributes no tokens, so the CST node has no children and
                // the round trip is unaffected.
                let (arg_cst, arg_ast) = match self.peek_kind() {
                    Some(TokenKind::Comma) | Some(TokenKind::RParen) | None => (
                        Cst::Node {
                            kind: NodeKind::Literal,
                            children: Vec::new(),
                        },
                        Ast::Literal(Value::Blank),
                    ),
                    _ => self.expression(0),
                };
                arg_children.push(arg_cst);
                args.push(arg_ast);
                match self.peek() {
                    Some((TokenKind::Comma, idx)) => {
                        let comma = self.take_through(idx, &mut arg_children);
                        arg_children.push(Cst::Token(comma));
                    }
                    _ => break,
                }
            }
        }
        children.push(Cst::Node {
            kind: NodeKind::ArgList,
            children: arg_children,
        });

        match self.peek() {
            Some((TokenKind::RParen, idx)) => {
                let close = self.take_through(idx, &mut children);
                children.push(Cst::Token(close));
            }
            _ => bad = true,
        }

        let node = Cst::Node {
            kind: NodeKind::Call,
            children,
        };
        let ast = if bad {
            Ast::Invalid(ErrorKind::Name)
        } else {
            Ast::Call { name, args }
        };
        (node, ast)
    }
}

fn infix_op(kind: TokenKind) -> Option<(BinOp, u8, bool)> {
    Some(match kind {
        TokenKind::Eq => (BinOp::Eq, BP_COMPARE, false),
        TokenKind::NotEq => (BinOp::NotEq, BP_COMPARE, false),
        TokenKind::Lt => (BinOp::Lt, BP_COMPARE, false),
        TokenKind::Gt => (BinOp::Gt, BP_COMPARE, false),
        TokenKind::LtEq => (BinOp::LtEq, BP_COMPARE, false),
        TokenKind::GtEq => (BinOp::GtEq, BP_COMPARE, false),
        TokenKind::Ampersand => (BinOp::Concat, BP_CONCAT, false),
        TokenKind::Plus => (BinOp::Add, BP_ADD, false),
        TokenKind::Minus => (BinOp::Sub, BP_ADD, false),
        TokenKind::Star => (BinOp::Mul, BP_MUL, false),
        TokenKind::Slash => (BinOp::Div, BP_MUL, false),
        // `^` is right-associative: 2^3^2 is 2^(3^2).
        TokenKind::Caret => (BinOp::Pow, BP_POW, true),
        _ => return None,
    })
}

/// Strips the surrounding quotes and unescapes `""`.
fn unquote(raw: &str) -> String {
    let inner = raw
        .strip_prefix('"')
        .map(|s| s.strip_suffix('"').unwrap_or(s))
        .unwrap_or(raw);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' && chars.peek() == Some(&'"') {
            chars.next();
        }
        out.push(c);
    }
    out
}

fn error_literal(text: &str) -> Option<ErrorKind> {
    Some(match text.to_ascii_uppercase().as_str() {
        "#DIV/0!" => ErrorKind::Div0,
        "#VALUE!" => ErrorKind::Value,
        "#REF!" => ErrorKind::Ref,
        "#NAME?" => ErrorKind::Name,
        "#NUM!" => ErrorKind::Num,
        "#N/A" => ErrorKind::Na,
        "#SPILL!" => ErrorKind::Spill,
        _ => return None,
    })
}

/// Parses `$A$1` into 0-based ordinals. Column letters are bijective base-26,
/// so `A`=0, `Z`=25, `AA`=26.
pub fn parse_a1(text: &str) -> Option<A1> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let col_absolute = bytes.first() == Some(&b'$');
    if col_absolute {
        i += 1;
    }
    let mut col: u32 = 0;
    let letters_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        let d = (bytes[i].to_ascii_uppercase() - b'A') as u32 + 1;
        col = col.checked_mul(26)?.checked_add(d)?;
        i += 1;
    }
    if i == letters_start {
        return None;
    }
    let row_absolute = bytes.get(i) == Some(&b'$');
    if row_absolute {
        i += 1;
    }
    let mut row: u32 = 0;
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        row = row.checked_mul(10)?.checked_add((bytes[i] - b'0') as u32)?;
        i += 1;
    }
    if i != bytes.len() || i == digits_start || row == 0 {
        return None;
    }
    Some(A1 {
        col: col - 1,
        row: row - 1,
        col_absolute,
        row_absolute,
    })
}

// ---------------------------------------------- Excel's literal parser (TD-32)

/// What `compat_parse_15` decided about a numeric literal.
enum LiteralParse {
    /// Acceptable, after any compat truncation.
    Value(f64),
    /// Excel's parser refuses the whole formula rather than storing this.
    Refused,
    /// Not a number at all -- the lexer accepted something `f64` will not.
    Unparseable,
}

/// Excel's largest magnitude. `1E308` is itself refused, so this bound is only
/// ever reached from below.
const COMPAT_MAX: f64 = 1e308;

/// Excel's smallest *normal* magnitude. Below it a literal flushes to zero
/// rather than becoming a subnormal.
const COMPAT_MIN_NORMAL: f64 = 2.2250738585072014e-308;

/// Excel's literal rules, applied at parse time (D-081, TD-32). All four are
/// measured from the COM capture (`grids/91-literal-parser.psd1`); none of them
/// follows from the documented "15 significant digits of precision".
///
/// 1. **Truncate to 15 significant digits -- truncate, not round.**
///    `=9999999999999999` is `9999999999999990`, where rounding would give
///    `1e16`. Destructive and irreversible, which is why it belongs here and
///    not in a display layer.
/// 2. **A magnitude at or above `1E308` is refused by the parser**, not stored
///    as `#NUM!` -- Excel rejects the formula. `-1E308` is refused too, because
///    the out-of-range thing is the *literal* and the minus never runs.
/// 3. **Below the smallest normal, a literal flushes to zero**: `=1E-308` and
///    `=1E-309` are both `0`.
/// 4. **But an exponent below -309 is refused outright**, so `=1E-310` is a
///    parse error while `=1E-309` is zero. An odd boundary, and a measured one:
///    it sits at the *written* exponent, not at anything representable.
///
/// `Strict` keeps the literal exactly as written; every rule above is a
/// compatibility fiction.
fn compat_parse_15(raw: &str, profile: Profile) -> LiteralParse {
    let Ok(v) = raw.parse::<f64>() else {
        return LiteralParse::Unparseable;
    };
    if profile != Profile::Compat {
        return LiteralParse::Value(v);
    }
    if !v.is_finite() || v.abs() >= COMPAT_MAX {
        return LiteralParse::Refused;
    }
    if v != 0.0 && v.abs() < COMPAT_MIN_NORMAL {
        return if written_exponent(raw) < -309 {
            LiteralParse::Refused
        } else {
            LiteralParse::Value(0.0)
        };
    }
    LiteralParse::Value(truncate_to_15(raw).unwrap_or(v))
}

/// The power of ten the literal's leading significant digit sits on, derived
/// from the **text** rather than from the parsed double.
///
/// Deliberately textual: near the subnormal boundary the parsed value has
/// already lost precision, so `1E-310` and `1E-309` stop being reliably
/// distinguishable as doubles -- and rule 4 draws its line exactly there.
fn written_exponent(raw: &str) -> i32 {
    let (mantissa, exp) = match raw.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().unwrap_or(0)),
        None => (raw, 0),
    };
    let digits = mantissa.trim_start_matches(['+', '-']);
    let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));
    let int_significant = int_part.trim_start_matches('0');
    if !int_significant.is_empty() {
        // 123.4e5 -> the leading digit sits at 10^(2+5).
        return exp + int_significant.len() as i32 - 1;
    }
    // 0.00123e5 -> the leading digit sits at 10^(-3+5).
    let leading_zeros = frac_part.len() - frac_part.trim_start_matches('0').len();
    exp - (leading_zeros as i32) - 1
}

/// Keeps the first 15 significant decimal digits of the literal **as written**
/// and zeroes the rest. `None` when there is nothing to truncate.
///
/// On the text, not on the parsed double, and that is the whole point:
/// `9999999999999999` has no exact `f64`, so it parses to `1e16` and by then
/// the digits Excel truncates are already gone. Excel truncates first and
/// converts second, giving `9999999999999990`. Rebuilding through a formatted
/// round trip also keeps this identical on every target -- `core`'s float code
/// is pure Rust and locale-free (DP-A2) -- where scaling by a built-up power of
/// ten is inexact past `10^22` (D-041).
fn truncate_to_15(raw: &str) -> Option<f64> {
    let (mantissa, _) = raw.split_once(['e', 'E']).unwrap_or((raw, ""));
    let sign = if mantissa.starts_with('-') { "-" } else { "" };
    let unsigned = mantissa.trim_start_matches(['+', '-']);
    let (int_part, frac_part) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    // Count the significant digits *without* building the concatenation. Every
    // numeric literal in every formula reaches this function and almost none
    // needs truncating, so the common path should not allocate.
    //
    // Honest about its own evidence: this was written to chase an apparent 25%
    // graph-build regression, and it did **not** move the number — W-CHAIN-100K's
    // graph build measures 801-918 ms across five runs either way, so the
    // single sample that prompted it was noise (D-112). The change is kept
    // because doing less work on the hot path is right regardless, not because
    // it was shown to be faster.
    let int_significant = int_part.trim_start_matches('0');
    let significant_len = if int_significant.is_empty() {
        frac_part.trim_start_matches('0').len()
    } else {
        int_significant.len() + frac_part.len()
    };
    if significant_len <= 15 {
        return None;
    }
    let mut digits = String::from(int_part);
    digits.push_str(frac_part);
    let significant = digits.trim_start_matches('0');
    // `written_exponent` already located the leading digit, so the truncated
    // value is `0.<15 digits>` scaled one place past it.
    let exp = written_exponent(raw) + 1;
    alloc::format!("{sign}0.{}e{exp}", &significant[..15])
        .parse::<f64>()
        .ok()
}
