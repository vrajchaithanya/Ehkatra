//! Type inference with a **preview before commit** (docs/24).
//!
//! > *type-inference **preview before commit** — the gene-name bug is a
//! > surfaced decision, never silent*
//!
//! The gene-name bug is the canonical case and it is worth stating precisely,
//! because it is the reason this module has the shape it has. The HUGO Gene
//! Nomenclature Committee renamed human genes — *SEPT2* became *SEPTIN2* — after
//! roughly a fifth of published genomics papers were found to contain
//! spreadsheet-mangled gene symbols. Nobody chose that. A spreadsheet inferred
//! it, silently, and the correction cost a naming standard.
//!
//! So the silent path does not exist here to be taken by accident:
//! [`analyze`] returns a [`Report`] and commits nothing, and [`commit`] refuses
//! to run without an explicit [`Decision`] per column. A caller who wants
//! Excel's behaviour can have it in one line — but they have to write the line.
//!
//! What the report is *for* is the second half. "Column 3 is mixed" is a
//! complaint. "Column 3 would become numbers, and these 4 values lose
//! information when it does — here they are, with their line numbers" is a
//! decision a human can actually make.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use usk_types::coerce::Profile;
use usk_types::Value;

use crate::inject::{self, Finding};
use crate::limits::INFERENCE_SAMPLE_ROWS;
use crate::Record;

/// What a column will become.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Excel's rules: digit-like text becomes a number. Fast, familiar, lossy.
    Number,
    /// Keep every field exactly as written.
    Text,
    /// `TRUE`/`FALSE` become booleans.
    Boolean,
    /// Let each field decide for itself under `Profile::Compat`. The most
    /// Excel-like option and the one most likely to produce a mixed column.
    PerCell,
}

/// The information a `Decision::Number` would destroy in one field.
///
/// Each variant is a *measured* loss — the value is reconstructed from the
/// number and compared with the original text — rather than a pattern guess.
/// That matters: a heuristic that flags `"1E2"` because it matches a regex will
/// also flag `"1E2"` when the column really is scientific notation, and a
/// warning that cries wolf is a warning users click through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loss {
    /// `"0000123"` → `123`. Part codes, ZIP codes, phone numbers.
    LeadingZeros,
    /// `"1E2"` → `100`. **The gene-symbol case.**
    ScientificNotation,
    /// `"12.50"` → `12.5`. Prices that stop looking like prices.
    TrailingZeros,
    /// More than 15 significant digits: the value cannot survive `f64` at all.
    /// Credit-card and account numbers live here.
    PrecisionBeyond15Digits,
    /// The text and the round-tripped number differ for some other reason —
    /// grouping separators, a currency symbol, surrounding space.
    Reformatted,
}

/// One field the caller is being asked about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LossSample {
    pub line: usize,
    pub original: String,
    pub as_number: String,
    pub loss: Loss,
}

/// What one column looks like.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ColumnReport {
    pub index: usize,
    pub name: String,
    pub blank: usize,
    pub numeric: usize,
    pub boolean: usize,
    pub textual: usize,
    /// Up to [`ColumnReport::MAX_SAMPLES`] fields that lose information if this
    /// column is committed as `Number`, each with its line.
    pub losses: Vec<LossSample>,
    /// How many fields would lose information in total — which is not
    /// `losses.len()`, because the samples are capped and the count is not.
    pub loss_count: usize,
    /// What [`analyze`] would choose if nobody chose. **Advisory**: nothing
    /// acts on it without being told to.
    pub suggested: Decision,
}

impl ColumnReport {
    pub const MAX_SAMPLES: usize = 5;

    /// True when the two profiles disagree about this column — i.e. when there
    /// is a decision to make at all. A UI shows these first.
    pub fn is_contested(&self) -> bool {
        self.loss_count > 0
    }
}

/// The preview. Returned by [`analyze`]; consumed by a human.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    pub columns: Vec<ColumnReport>,
    /// Records examined. Compared against `rows_total`, this is how the caller
    /// knows whether the report covers the file or a sample of it.
    pub rows_sampled: usize,
    pub rows_total: usize,
    /// Fields that would be formulas in another spreadsheet (docs/24, OWASP).
    /// Import forces every one of these to text; the finding exists so the user
    /// learns their data source is sending them formulas.
    pub injections: Vec<Finding>,
    /// Rows whose field count differs from the header's — the most common
    /// real-world CSV defect, and one that silently shifts every column after
    /// it if nobody looks.
    pub ragged_rows: Vec<usize>,
}

impl Report {
    /// True when the file can be committed without anyone making a choice.
    pub fn is_unambiguous(&self) -> bool {
        self.injections.is_empty()
            && self.ragged_rows.is_empty()
            && !self.columns.iter().any(ColumnReport::is_contested)
    }

    /// The suggestions, ready to hand back to [`commit`] — *after* a human has
    /// looked at them. Convenience, never a default: the caller still has to
    /// call this, which is the difference between a choice and an accident.
    pub fn suggestions(&self) -> Vec<Decision> {
        self.columns.iter().map(|c| c.suggested).collect()
    }

    pub fn truncated(&self) -> bool {
        self.rows_sampled < self.rows_total
    }
}

/// Profiles the records and returns the preview. **Commits nothing.**
///
/// `header` names the columns when the first record is a header row; pass
/// `false` and columns are named `Column 1`, `Column 2`, …
pub fn analyze(records: &[Record], header: bool) -> Report {
    let (names, body) = split_header(records, header);
    let width = body
        .iter()
        .map(|r| r.fields.len())
        .chain(core::iter::once(names.len()))
        .max()
        .unwrap_or(0);

    let mut columns: Vec<ColumnReport> = (0..width)
        .map(|index| ColumnReport {
            index,
            name: names
                .get(index)
                .cloned()
                .unwrap_or_else(|| default_name(index)),
            blank: 0,
            numeric: 0,
            boolean: 0,
            textual: 0,
            losses: Vec::new(),
            loss_count: 0,
            suggested: Decision::Text,
        })
        .collect();

    let mut injections = Vec::new();
    let mut ragged_rows = Vec::new();
    let expected = names.len();

    let sampled = body.len().min(INFERENCE_SAMPLE_ROWS);
    for record in body.iter().take(sampled) {
        if expected > 0 && record.fields.len() != expected {
            ragged_rows.push(record.line);
        }
        for (index, field) in record.fields.iter().enumerate() {
            let Some(column) = columns.get_mut(index) else {
                continue;
            };
            // `classify_import`, not `classify`: on import nothing is typed
            // yet, and the lexical rule would turn every negative number in the
            // file into text.
            if let Some(risk) = inject::classify_import(field) {
                if injections.len() < 64 {
                    injections.push(Finding::new(record.line, index, risk, field));
                }
            }
            classify_field(column, field, record.line);
        }
    }

    for column in &mut columns {
        column.suggested = suggest(column);
    }

    Report {
        columns,
        rows_sampled: sampled,
        rows_total: body.len(),
        injections,
        ragged_rows,
    }
}

fn split_header(records: &[Record], header: bool) -> (Vec<String>, &[Record]) {
    match (header, records.split_first()) {
        (true, Some((first, rest))) => (first.fields.clone(), rest),
        _ => (Vec::new(), records),
    }
}

fn default_name(index: usize) -> String {
    let mut out = String::from("Column ");
    out.push_str(&(index + 1).to_string());
    out
}

fn classify_field(column: &mut ColumnReport, field: &str, line: usize) {
    if field.is_empty() {
        column.blank += 1;
        return;
    }
    if matches!(field, "TRUE" | "FALSE" | "true" | "false") {
        column.boolean += 1;
        return;
    }
    // `coerce_input` under Compat *is* Excel's rule. Asking the engine rather
    // than re-deriving the rule here means the report can never describe a
    // conversion the importer would not actually perform.
    match Profile::Compat.coerce_input(field) {
        Value::Number(n) => {
            column.numeric += 1;
            if let Some(loss) = loss_of(field, n) {
                column.loss_count += 1;
                if column.losses.len() < ColumnReport::MAX_SAMPLES {
                    column.losses.push(LossSample {
                        line,
                        original: String::from(field),
                        as_number: format_number(n),
                        loss,
                    });
                }
            }
        }
        _ => column.textual += 1,
    }
}

/// What committing `field` as `value` would destroy, if anything.
///
/// The test is a **round trip**: render the number back to text and compare. A
/// pattern match on the input would flag `"1E2"` even in a column that really is
/// scientific notation, and a warning that fires on correct data is a warning
/// users learn to dismiss.
fn loss_of(field: &str, value: f64) -> Option<Loss> {
    let rendered = format_number(value);
    if rendered == field {
        return None;
    }
    let digits = field.chars().filter(char::is_ascii_digit).count();
    let trimmed = field.trim_start_matches(['+', '-']);
    if trimmed.len() > 1 && trimmed.starts_with('0') && !trimmed.starts_with("0.") {
        return Some(Loss::LeadingZeros);
    }
    if field.contains(['e', 'E']) {
        return Some(Loss::ScientificNotation);
    }
    if digits > 15 {
        return Some(Loss::PrecisionBeyond15Digits);
    }
    if field.contains('.') && field.ends_with('0') {
        return Some(Loss::TrailingZeros);
    }
    Some(Loss::Reformatted)
}

/// Rust's shortest round-tripping form, which is also what the CSV writer
/// emits — so the report shows the user the exact text they will get back.
fn format_number(n: f64) -> String {
    if n.abs() < 1e15 && n == (n as i64) as f64 {
        return (n as i64).to_string();
    }
    let mut out = String::new();
    let _ = core::fmt::write(&mut out, format_args!("{n:?}"));
    out
}

/// The advisory suggestion. Conservative on purpose: a column is only suggested
/// as `Number` when nothing in it loses information, so following the
/// suggestion blindly can never mangle data. Excel's own answer is available —
/// it is `Decision::PerCell` — and the user has to ask for it.
fn suggest(column: &ColumnReport) -> Decision {
    let populated = column.numeric + column.boolean + column.textual;
    if populated == 0 {
        return Decision::Text;
    }
    if column.boolean == populated {
        return Decision::Boolean;
    }
    if column.numeric == populated && column.loss_count == 0 {
        return Decision::Number;
    }
    Decision::Text
}

/// Turns records into values under the caller's decisions.
///
/// `decisions` must have one entry per column; a short slice means the caller
/// and the report have gone out of step, and the missing columns fall back to
/// `Text` — the choice that cannot lose data.
///
/// Every field is passed through injection handling first: a field that another
/// spreadsheet would read as a formula becomes `Text` **regardless of the
/// decision**, because "this column is numbers" is not consent to import
/// `=WEBSERVICE(...)`.
pub fn commit(records: &[Record], header: bool, decisions: &[Decision]) -> Vec<Vec<Value>> {
    let (_, body) = split_header(records, header);
    body.iter()
        .map(|record| {
            record
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let decision = decisions.get(index).copied().unwrap_or(Decision::Text);
                    commit_field(field, decision)
                })
                .collect()
        })
        .collect()
}

fn commit_field(field: &str, decision: Decision) -> Value {
    if field.is_empty() {
        return Value::Blank;
    }
    if inject::classify_import(field).is_some() {
        // Import-side neutralization: never a formula, always text, and the
        // bytes are kept exactly so the user can see what arrived.
        return Value::Text(String::from(field));
    }
    // A leading apostrophe in front of something formula-shaped is *our own*
    // export neutralization, and stripping it makes export→import an identity
    // for text values. The ambiguity is real and inherent to CSV — a field
    // authored as `'=x` is indistinguishable from a neutralized `=x` — and
    // Excel resolves it the same way, a leading apostrophe meaning "text".
    let field = inject::strip_neutralization(field);
    match decision {
        Decision::Text => Value::Text(String::from(field)),
        Decision::Boolean => match field {
            "TRUE" | "true" => Value::Bool(true),
            "FALSE" | "false" => Value::Bool(false),
            other => Value::Text(String::from(other)),
        },
        Decision::Number => Profile::Compat.coerce_input(field),
        Decision::PerCell => match field {
            "TRUE" | "true" => Value::Bool(true),
            "FALSE" | "false" => Value::Bool(false),
            other => Profile::Compat.coerce_input(other),
        },
    }
}
