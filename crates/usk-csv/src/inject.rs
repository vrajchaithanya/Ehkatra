//! Formula-injection neutralization, **both directions** (docs/24, OWASP).
//!
//! > *formula-injection neutralization on both import and export per OWASP*
//!
//! The attack: a CSV field beginning `=`, `+`, `-`, `@`, TAB or CR is
//! interpreted as a *formula* by spreadsheet software that opens the file, so a
//! value that merely passed through a database ends up executing
//! `=cmd|'/c calc'!A0` or exfiltrating a range through a `WEBSERVICE` call on
//! somebody else's machine. It is a data-provenance bug, not a parser bug, which
//! is why it has to be handled at both boundaries.
//!
//! # Import
//! Ehkatra never turns an imported field into a formula — [`crate::infer`]
//! produces values, and a risky field is forced to `Text` and **reported**.
//! Structurally we are safe here; the report exists because the user needs to
//! know their supplier is sending them formulas.
//!
//! # Export — the half people forget
//! Neutralization is applied **to text values only**, and that is the whole
//! design. The naive OWASP rule ("prefix any field starting with `-`") mangles
//! every negative number in the file, which is a data-corruption bug introduced
//! in the name of security. We export from *typed* cells, so `Number(-1)`
//! renders as `-1` and is never touched, while `Text("-1+1")` is neutralized.
//! Knowing the type is the advantage a spreadsheet has over a generic CSV
//! writer, and this is where it pays.

use alloc::string::String;

/// Why a field is dangerous to hand to another spreadsheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Risk {
    /// A leading character that makes a spreadsheet parse the field as a
    /// formula.
    FormulaLead(char),
    /// A leading tab or carriage return. These are stripped by some importers
    /// *before* the formula check, so `\t=1+1` becomes `=1+1` and evades a
    /// naive filter — which is exactly why they are listed separately.
    ControlLead(char),
}

/// The characters that begin a formula in Excel, LibreOffice and Sheets.
const FORMULA_LEADS: [char; 4] = ['=', '+', '-', '@'];

/// Classifies a field's *text*. Returns `None` when the field is safe as-is.
pub fn classify(text: &str) -> Option<Risk> {
    let first = text.chars().next()?;
    if FORMULA_LEADS.contains(&first) {
        return Some(Risk::FormulaLead(first));
    }
    if first == '\t' || first == '\r' {
        return Some(Risk::ControlLead(first));
    }
    None
}

/// Classifies a field on **import**, where nothing is typed yet.
///
/// The lexical test above is right for export, where the caller already knows
/// the value is text. On import every field is a string, and applying the
/// lexical rule directly turns **every negative number in the file into text** —
/// which is the same data-corruption bug as the naive export rule, arriving
/// from the other side. The tests caught it immediately, which is the argument
/// for having written them.
///
/// The refinement is exact rather than heuristic: *a field that parses as a
/// plain number is not a risk, because the formula it would become evaluates to
/// that same number.* `-3` is safe; `-3+cmd|'/c calc'!A0` is not, because it is
/// not a number. `parse` here is the engine's own `coerce_input`, so the test
/// can never disagree with what the importer would actually store.
pub fn classify_import(text: &str) -> Option<Risk> {
    let risk = classify(text)?;
    match usk_types::coerce::Profile::Compat.coerce_input(text) {
        usk_types::Value::Number(_) => None,
        _ => Some(risk),
    }
}

/// Neutralizes a **text** field for export.
///
/// The neutralization is a leading apostrophe: the spelling every spreadsheet
/// reads as "the rest of this is literal text", and the one that survives a
/// round trip back into Ehkatra because [`strip_neutralization`] is its exact inverse.
///
/// It does change the bytes, and pretending otherwise would be the dishonest
/// option — so the export report counts every field this touched. A caller that
/// truly wants raw bytes is writing a data file, not a spreadsheet, and should
/// say so.
pub fn neutralize(text: &str) -> String {
    match classify(text) {
        Some(_) => {
            let mut out = String::with_capacity(text.len() + 1);
            out.push('\'');
            out.push_str(text);
            out
        }
        None => String::from(text),
    }
}

/// The inverse of [`neutralize`]: strips one leading apostrophe when what
/// follows would have needed neutralizing.
///
/// Conditioned on the remainder being risky rather than stripping any leading
/// apostrophe, so an honest field of `'quoted'` is left alone. Round-trip
/// fidelity is a property this crate has to keep, not a nicety.
pub fn strip_neutralization(text: &str) -> &str {
    match text.strip_prefix('\'') {
        Some(rest) if classify(rest).is_some() => rest,
        _ => text,
    }
}

/// Where a risky field was found. Carried into both the import report and the
/// export report so "we changed your data" is always attributable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finding {
    pub line: usize,
    pub column: usize,
    pub risk: Risk,
    /// The field, truncated — a report is read by a human and the payload is
    /// often long by design.
    pub sample: String,
}

impl Finding {
    pub const SAMPLE_CHARS: usize = 48;

    pub fn new(line: usize, column: usize, risk: Risk, text: &str) -> Finding {
        Finding {
            line,
            column,
            risk,
            sample: text.chars().take(Finding::SAMPLE_CHARS).collect(),
        }
    }
}
