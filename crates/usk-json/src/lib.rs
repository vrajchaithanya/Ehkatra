//! usk-json — a total JSON reader/writer with no dependencies (DP-S2).
//!
//! # Why this exists rather than `serde_json`
//! Three v0.1 surfaces need JSON: the conformance runner (docs/50's vector
//! corpus), the import/export reports (docs/24) and the MCP server's
//! JSON-Schema I/O (docs/21). `serde` + `serde_json` is roughly six crates
//! against a workspace ceiling of 40 that stands at 29 after `rusqlite`
//! (D-073) — 15% of the remaining budget for a format whose grammar fits on a
//! postcard. DP-S1/S2 says one of each hard thing, and JSON is not one of the
//! hard things.
//!
//! The same reasoning produced D-046 (in-crate `powf` rather than libm) and
//! D-052 (seeded sweeps rather than `proptest`). Recorded as D-083.
//!
//! # What is guaranteed
//! * **Total.** Every malformed input is a named error with a byte offset,
//!   never a panic (DP-A10). Nesting is bounded, so a hostile document cannot
//!   exhaust the stack — MCP input is agent-controlled (docs/37).
//! * **Numbers keep their source text.** docs/50 is explicit that a JSON
//!   number round-tripped through a non-correctly-rounding parser moves by an
//!   ULP, which produced six false failures in the capture harness's own
//!   validator. [`Json::Number`] therefore holds the literal, and
//!   [`Json::as_f64`] parses it with `core`'s float parser, which *is*
//!   correctly rounded and locale-free (the same property DP-A2 relies on in
//!   `compat_round_15`).
//! * **RFC 8259 strings**, surrogate pairs included: `😀` decodes to
//!   one scalar, and an unpaired surrogate is an error rather than a silent
//!   replacement character.

#![no_std]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// How deep a document may nest. A bound, not a format limit: the input is
/// untrusted and recursion is not.
pub const MAX_DEPTH: usize = 128;

/// A JSON value.
///
/// Objects keep insertion order in a `Vec` rather than a map: JSON objects are
/// ordered in practice, duplicate keys are legal in the grammar, and preserving
/// both makes the writer's output a function of the parse.
#[derive(Clone, PartialEq, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number's literal source text, unparsed. See the crate docs.
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Why a byte string is not JSON. Errors are values (DP-A10).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JsonError {
    pub at: usize,
    pub kind: JsonErrorKind,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JsonErrorKind {
    UnexpectedEnd,
    UnexpectedByte(u8),
    BadNumber,
    BadEscape,
    BadUnicodeEscape,
    UnpairedSurrogate,
    ControlCharacterInString,
    TrailingContent,
    DepthExceeded,
}

impl Json {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The number as an `f64`, parsed by `core` — correctly rounded, so a
    /// value survives the round trip the capture harness warns about.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(raw) => raw.parse::<f64>().ok(),
            _ => None,
        }
    }

    /// The number's literal source text.
    pub fn as_number_text(&self) -> Option<&str> {
        match self {
            Json::Number(raw) => Some(raw),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Field lookup by key. First match wins, which is what every JSON reader
    /// in practice does with the duplicate keys the grammar permits.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// Compact serialization.
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        self.write_into(&mut out, None, 0);
        out
    }

    /// Indented serialization, for a human or a diff.
    pub fn to_json_pretty(&self) -> String {
        let mut out = String::new();
        self.write_into(&mut out, Some(2), 0);
        out
    }

    fn write_into(&self, out: &mut String, indent: Option<usize>, depth: usize) {
        let pad = |out: &mut String, depth: usize| {
            if let Some(n) = indent {
                out.push('\n');
                for _ in 0..n * depth {
                    out.push(' ');
                }
            }
        };
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(raw) => out.push_str(raw),
            Json::String(s) => write_string(s, out),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    pad(out, depth + 1);
                    item.write_into(out, indent, depth + 1);
                }
                pad(out, depth);
                out.push(']');
            }
            Json::Object(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    pad(out, depth + 1);
                    write_string(key, out);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    value.write_into(out, indent, depth + 1);
                }
                pad(out, depth);
                out.push('}');
            }
        }
    }
}

/// Builds a JSON number from an `f64`.
///
/// Rust's float formatter emits the shortest representation that round-trips,
/// which is what JSON wants. Non-finite values have no JSON spelling, so they
/// become `null` — the choice every JSON writer has to make, made explicitly.
pub fn number(value: f64) -> Json {
    if value.is_finite() {
        Json::Number(format!("{value:?}"))
    } else {
        Json::Null
    }
}

pub fn string(value: impl Into<String>) -> Json {
    Json::String(value.into())
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Parses one complete JSON document. Trailing non-whitespace is an error —
/// a reader that stops early is how half a document gets treated as the whole.
pub fn parse(bytes: &[u8]) -> Result<Json, JsonError> {
    let mut p = Parser { b: bytes, at: 0 };
    p.skip_ws();
    let value = p.value(0)?;
    p.skip_ws();
    if p.at != p.b.len() {
        return Err(p.err(JsonErrorKind::TrailingContent));
    }
    Ok(value)
}

/// Parses a document from a `&str`.
pub fn parse_str(text: &str) -> Result<Json, JsonError> {
    parse(text.as_bytes())
}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn err(&self, kind: JsonErrorKind) -> JsonError {
        JsonError { at: self.at, kind }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, want: u8) -> Result<(), JsonError> {
        match self.peek() {
            Some(c) if c == want => {
                self.at += 1;
                Ok(())
            }
            Some(c) => Err(self.err(JsonErrorKind::UnexpectedByte(c))),
            None => Err(self.err(JsonErrorKind::UnexpectedEnd)),
        }
    }

    fn literal(&mut self, word: &[u8], value: Json) -> Result<Json, JsonError> {
        if self.b[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.err(JsonErrorKind::UnexpectedByte(self.b[self.at])))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err(JsonErrorKind::DepthExceeded));
        }
        match self
            .peek()
            .ok_or_else(|| self.err(JsonErrorKind::UnexpectedEnd))?
        {
            b'n' => self.literal(b"null", Json::Null),
            b't' => self.literal(b"true", Json::Bool(true)),
            b'f' => self.literal(b"false", Json::Bool(false)),
            b'"' => Ok(Json::String(self.string()?)),
            b'[' => self.array(depth),
            b'{' => self.object(depth),
            c if c == b'-' || c.is_ascii_digit() => self.number(),
            c => Err(self.err(JsonErrorKind::UnexpectedByte(c))),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                Some(c) => return Err(self.err(JsonErrorKind::UnexpectedByte(c))),
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                Some(c) => return Err(self.err(JsonErrorKind::UnexpectedByte(c))),
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
            }
        }
    }

    /// The RFC 8259 number grammar, validated rather than guessed at, and kept
    /// as source text (see the crate docs on why the value is not parsed here).
    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(c) if c.is_ascii_digit() => {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.at += 1;
                }
            }
            _ => return Err(self.err(JsonErrorKind::BadNumber)),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.err(JsonErrorKind::BadNumber));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.at += 1;
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.err(JsonErrorKind::BadNumber));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.at += 1;
            }
        }
        // The slice is ASCII by construction, so this cannot fail; the
        // fallible form is used anyway because DP-C1 has no exception for
        // "obviously fine".
        match core::str::from_utf8(&self.b[start..self.at]) {
            Ok(text) => Ok(Json::Number(text.to_string())),
            Err(_) => Err(self.err(JsonErrorKind::BadNumber)),
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let c = self
                .peek()
                .ok_or_else(|| self.err(JsonErrorKind::UnexpectedEnd))?;
            match c {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    self.escape(&mut out)?;
                }
                0x00..=0x1F => return Err(self.err(JsonErrorKind::ControlCharacterInString)),
                _ => {
                    // Copy one whole UTF-8 sequence. The input's validity is
                    // checked here rather than assumed: a file on disk is an
                    // untrusted input like any other.
                    let len = utf8_len(c);
                    let end = self.at + len;
                    let slice = self
                        .b
                        .get(self.at..end)
                        .ok_or_else(|| self.err(JsonErrorKind::UnexpectedEnd))?;
                    match core::str::from_utf8(slice) {
                        Ok(text) => out.push_str(text),
                        Err(_) => return Err(self.err(JsonErrorKind::UnexpectedByte(c))),
                    }
                    self.at = end;
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let c = self
            .peek()
            .ok_or_else(|| self.err(JsonErrorKind::UnexpectedEnd))?;
        self.at += 1;
        let simple = match c {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{08}'),
            b'f' => Some('\u{0C}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'u' => None,
            _ => return Err(self.err(JsonErrorKind::BadEscape)),
        };
        if let Some(ch) = simple {
            out.push(ch);
            return Ok(());
        }

        let first = self.hex4()?;
        // A high surrogate must be followed by its low half. Excel's own
        // corpus contains astral characters (docs/50 finding 5), so this path
        // is exercised rather than theoretical.
        if (0xD800..0xDC00).contains(&first) {
            if !self.b[self.at..].starts_with(b"\\u") {
                return Err(self.err(JsonErrorKind::UnpairedSurrogate));
            }
            self.at += 2;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.err(JsonErrorKind::UnpairedSurrogate));
            }
            let scalar = 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            match char::from_u32(scalar) {
                Some(ch) => out.push(ch),
                None => return Err(self.err(JsonErrorKind::BadUnicodeEscape)),
            }
            return Ok(());
        }
        if (0xDC00..0xE000).contains(&first) {
            return Err(self.err(JsonErrorKind::UnpairedSurrogate));
        }
        match char::from_u32(first) {
            Some(ch) => out.push(ch),
            None => return Err(self.err(JsonErrorKind::BadUnicodeEscape)),
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self
                .peek()
                .ok_or_else(|| self.err(JsonErrorKind::UnexpectedEnd))?;
            let digit = match c {
                b'0'..=b'9' => u32::from(c - b'0'),
                b'a'..=b'f' => u32::from(c - b'a') + 10,
                b'A'..=b'F' => u32::from(c - b'A') + 10,
                _ => return Err(self.err(JsonErrorKind::BadUnicodeEscape)),
            };
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}
