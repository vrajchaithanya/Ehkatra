//! Formula lexer — the first stage of `text → lexer → Pratt parser → lossless
//! CST → AST → binder` (docs/12).
//!
//! The lexer is **lossless**: whitespace and unrecognised bytes become tokens
//! rather than being discarded, so the token stream concatenates back to the
//! exact input. That property is what lets the CST above it support
//! refactoring that preserves formatting, precise error carets, and AI
//! explanation anchoring (ADR-011).

use alloc::vec::Vec;

/// A lexical category. Trivia (`Whitespace`) and `Unknown` are first-class so
/// nothing is ever dropped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Number,
    /// A quoted string literal, `"like this"`, with `""` as an escaped quote.
    Text,
    /// An error literal typed directly into a formula, e.g. `#N/A`.
    ErrorLiteral,
    /// A bare name: function name, defined name, boolean, or a cell reference.
    /// The parser decides which; the lexer only recognises the shape.
    Ident,
    /// `$A$1`-style cell reference, including any absolute markers.
    CellRef,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Ampersand,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    LParen,
    RParen,
    Comma,
    Colon,
    Whitespace,
    /// A byte the lexer cannot classify. Kept so the round trip holds and so
    /// the parser can report a caret at exactly the right column.
    Unknown,
}

/// A token as a half-open byte range into the source. Storing spans rather than
/// copies is what keeps the round trip exact and the allocation count low.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub start: u32,
    pub end: u32,
}

impl Token {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        source
            .get(self.start as usize..self.end as usize)
            .unwrap_or("")
    }
}

/// Tokenises a formula body (without any leading `=`).
///
/// Never fails: unclassifiable input becomes [`TokenKind::Unknown`], because a
/// lexer that returns `Err` cannot preserve what the user typed, and the user's
/// text is the thing the CST exists to preserve.
pub fn lex(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                    i += 1;
                }
                TokenKind::Whitespace
            }
            b'0'..=b'9' | b'.' => {
                // Digits with at most one decimal point and an optional
                // exponent. `1E2` is a *number literal* here, unlike text
                // coercion where the same shape is the gene-symbol hazard —
                // inside a formula the user wrote a number on purpose.
                let mut seen_point = false;
                while i < bytes.len() {
                    match bytes[i] {
                        b'0'..=b'9' => i += 1,
                        b'.' if !seen_point => {
                            seen_point = true;
                            i += 1;
                        }
                        b'e' | b'E' if i + 1 < bytes.len() && is_exponent_tail(bytes, i + 1) => {
                            i += 2;
                            while i < bytes.len() && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                        }
                        _ => break,
                    }
                }
                TokenKind::Number
            }
            b'"' => {
                i += 1;
                loop {
                    if i >= bytes.len() {
                        break; // Unterminated: the parser reports it.
                    }
                    if bytes[i] == b'"' {
                        // `""` inside a string is one escaped quote.
                        if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                TokenKind::Text
            }
            b'#' => {
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || matches!(bytes[i], b'/' | b'!' | b'?' | b'_'))
                {
                    i += 1;
                }
                TokenKind::ErrorLiteral
            }
            b'$' | b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'$' | b'_' | b'.'))
                {
                    i += 1;
                }
                let word = source.get(start..i).unwrap_or("");
                if looks_like_cell_ref(word) {
                    TokenKind::CellRef
                } else {
                    TokenKind::Ident
                }
            }
            b'<' => {
                i += 1;
                match bytes.get(i) {
                    Some(b'>') => {
                        i += 1;
                        TokenKind::NotEq
                    }
                    Some(b'=') => {
                        i += 1;
                        TokenKind::LtEq
                    }
                    _ => TokenKind::Lt,
                }
            }
            b'>' => {
                i += 1;
                if bytes.get(i) == Some(&b'=') {
                    i += 1;
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            c => {
                i += 1;
                match c {
                    b'+' => TokenKind::Plus,
                    b'-' => TokenKind::Minus,
                    b'*' => TokenKind::Star,
                    b'/' => TokenKind::Slash,
                    b'^' => TokenKind::Caret,
                    b'%' => TokenKind::Percent,
                    b'&' => TokenKind::Ampersand,
                    b'=' => TokenKind::Eq,
                    b'(' => TokenKind::LParen,
                    b')' => TokenKind::RParen,
                    b',' => TokenKind::Comma,
                    b':' => TokenKind::Colon,
                    _ => TokenKind::Unknown,
                }
            }
        };
        tokens.push(Token {
            kind,
            start: start as u32,
            end: i as u32,
        });
    }
    tokens
}

/// True when the bytes after an `e`/`E` form an exponent rather than the start
/// of a name — so `1E2` lexes as a number but `1EA` does not.
fn is_exponent_tail(bytes: &[u8], at: usize) -> bool {
    match bytes.get(at) {
        Some(b'+') | Some(b'-') => bytes.get(at + 1).is_some_and(u8::is_ascii_digit),
        Some(c) => c.is_ascii_digit(),
        None => false,
    }
}

/// Recognises the A1 *shape*: optional `$`, letters, optional `$`, digits, and
/// nothing else. Whether the reference is in range, or points anywhere real, is
/// the binder's problem — this is purely lexical.
fn looks_like_cell_ref(word: &str) -> bool {
    let bytes = word.as_bytes();
    let mut i = 0;
    if bytes.get(i) == Some(&b'$') {
        i += 1;
    }
    let letters_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == letters_start {
        return false;
    }
    if bytes.get(i) == Some(&b'$') {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i == bytes.len() && i > digits_start
}
