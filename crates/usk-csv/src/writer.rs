//! CSV serialization, with export-side injection neutralization (docs/24).
//!
//! The writer takes **typed values**, not strings, and that is what lets
//! [`crate::inject`] neutralize text without mangling numbers. A generic CSV
//! writer handed `-1` cannot tell a negative number from the start of a
//! formula; this one never has to guess.

use alloc::string::String;
use alloc::vec::Vec;
use usk_types::Value;

use crate::inject::{self, Finding};
use crate::Dialect;

/// What an export changed, so the user can be told (docs/16's honesty rule
/// applied to a different boundary: silent modification is forbidden).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ExportReport {
    /// Fields that were neutralized, with their positions.
    pub neutralized: Vec<Finding>,
    pub rows: usize,
    pub columns: usize,
}

impl ExportReport {
    pub fn is_clean(&self) -> bool {
        self.neutralized.is_empty()
    }
}

/// Serializes rows of values.
///
/// Quoting is applied only where the grammar requires it — a delimiter, a
/// quote, a newline, or leading/trailing space that would otherwise be eaten by
/// a reader that trims. Quoting everything would be simpler and would make
/// every diff of an exported file useless.
pub fn write_csv(rows: &[Vec<Value>], dialect: Dialect) -> (String, ExportReport) {
    let mut out = String::new();
    let mut report = ExportReport {
        rows: rows.len(),
        columns: rows.iter().map(Vec::len).max().unwrap_or(0),
        ..ExportReport::default()
    };

    for (row_index, row) in rows.iter().enumerate() {
        for (column, value) in row.iter().enumerate() {
            if column > 0 {
                out.push(dialect.delimiter as char);
            }
            let rendered = render(value);
            let text = match value {
                // Only text is neutralized. `Number(-1)` renders as `-1` and is
                // left alone — the naive OWASP rule would prefix it and corrupt
                // every negative number in the file.
                Value::Text(_) => match inject::classify(&rendered) {
                    Some(risk) => {
                        report.neutralized.push(Finding::new(
                            row_index + 1,
                            column,
                            risk,
                            &rendered,
                        ));
                        inject::neutralize(&rendered)
                    }
                    None => rendered,
                },
                _ => rendered,
            };
            write_field(&text, dialect, &mut out);
        }
        // RFC 4180 says CRLF. Every reader accepts LF, and LF keeps the file
        // usable in the Unix tooling people actually pipe CSV through.
        out.push('\n');
    }
    (out, report)
}

fn write_field(text: &str, dialect: Dialect, out: &mut String) {
    let quote = dialect.quote as char;
    let needs_quoting = text
        .chars()
        .any(|c| c == dialect.delimiter as char || c == quote || c == '\n' || c == '\r')
        || text.starts_with(' ')
        || text.ends_with(' ');

    if !needs_quoting {
        out.push_str(text);
        return;
    }
    out.push(quote);
    for c in text.chars() {
        if c == quote {
            out.push(quote);
        }
        out.push(c);
    }
    out.push(quote);
}

/// A value's CSV text.
///
/// Errors export as their canonical spelling (`#DIV/0!`), which is what Excel
/// writes and what a reader will therefore recognise. Blank is the empty field
/// — distinct from `""`, which is a quoted empty *string*, a distinction CSV can
/// carry and most writers throw away.
pub fn render(value: &Value) -> String {
    match value {
        Value::Blank => String::new(),
        Value::Bool(true) => String::from("TRUE"),
        Value::Bool(false) => String::from("FALSE"),
        Value::Number(n) => render_number(*n),
        Value::Decimal(d) => {
            let mut out = String::new();
            let _ = core::fmt::write(&mut out, format_args!("{d}"));
            out
        }
        Value::Text(s) => s.clone(),
        Value::Error(e) => String::from(e.kind.as_str()),
    }
}

fn render_number(n: f64) -> String {
    let mut out = String::new();
    if !n.is_finite() {
        // No CSV spelling exists for these, and Excel has none either — it
        // stores `#NUM!`. Matching that beats emitting `inf` for a reader to
        // silently turn into text.
        return String::from("#NUM!");
    }
    if n.abs() < 1e15 && n == (n as i64) as f64 {
        let _ = core::fmt::write(&mut out, format_args!("{}", n as i64));
    } else {
        let _ = core::fmt::write(&mut out, format_args!("{n:?}"));
    }
    out
}
