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
pub fn parse(source: &str) -> Parsed {
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
            ast = Ast::Binary(op, Box::new(ast), Box::new(rhs_ast));
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
                    Ast::Unary(op, Box::new(rhs_ast)),
                )
            }
            TokenKind::LParen => {
                let (inner_cst, inner_ast) = self.expression(0);
                children.push(inner_cst);
                let ast = match self.peek() {
                    Some((TokenKind::RParen, close_idx)) => {
                        let close = self.take_through(close_idx, &mut children);
                        children.push(Cst::Token(close));
                        inner_ast
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
                let value = tok
                    .text(self.source)
                    .parse::<f64>()
                    .map(Value::Number)
                    // A number the lexer accepted but `f64` will not parse is
                    // out of range, which is `#NUM!`, not a crash.
                    .unwrap_or(Value::Error(usk_types::CellError::new(
                        ErrorKind::Num,
                        usk_types::Origin::Authored,
                    )));
                (
                    Cst::Node {
                        kind: NodeKind::Literal,
                        children,
                    },
                    Ast::Literal(value),
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
                let (arg_cst, arg_ast) = self.expression(0);
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
