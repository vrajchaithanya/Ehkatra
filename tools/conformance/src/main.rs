//! conformance — the Excel conformance runner (ADR-024, docs/32, docs/50).
//!
//! Reads every vector the oracle capture produced, evaluates each case through
//! `usk-formula` under `Profile::Compat`, compares against what real Excel did,
//! and reports a per-function pass rate. Workload id **W-ORACLE** (docs/38);
//! a number without one is invalid.
//!
//! # Why this is the number that matters
//! docs/32 makes Excel compatibility "a measured profile", and BOOTSTRAP's
//! differentiator 7 promises a `compat` profile that "keeps Excel's quirks for
//! imported files". Until this binary existed, every claim about that profile
//! was an intention. The corpus is 1,366 cases captured from Excel 16.0 build
//! 20228 over COM — not from documentation, which docs/32 says lies.
//!
//! # The comparison rules, stated so the number means something
//! * **Numbers** are compared as `f64` against `number_r17`, the *string*
//!   field, because docs/50 measured a JSON reader moving a value by an ULP
//!   and producing six false failures. Equality is exact. A second, looser
//!   bucket (`near`, relative difference ≤ 1e-12) is counted separately and
//!   never folded into the headline: it is the honest way to distinguish "the
//!   last bits differ" (TD-15) from "the answer is wrong", without letting the
//!   first quietly become a pass.
//! * **Text** is compared exactly, byte for byte. **Logicals** exactly.
//! * **Errors** are compared by canonical name (`#DIV/0!`), so returning the
//!   right kind of error for the wrong reason still passes and returning
//!   `#VALUE!` where Excel says `#NUM!` does not.
//! * **Cases Excel itself rejected** (`observed_status: rejected-by-excel`)
//!   pass only if our parser also refuses. That is conformance data, not a
//!   gap: an engine that accepts `=1E308` where Excel refuses it has diverged
//!   at parse time (D-081, TD-32).
//! * `general_text` (Excel's value→text coercion, where `compat_round_15`
//!   lives) is **not** yet asserted. Saying so is part of the number: the
//!   headline is value conformance, not display conformance.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use usk_formula::eval::{Context, Grid};
use usk_formula::parse::{self, Ast};
use usk_json::Json;
use usk_types::coerce::Profile;
use usk_types::Value;

/// The grid extent presented to the evaluator.
///
/// Excel's real extent would make a stray full-column reference iterate a
/// million cells; the corpus's fixtures reach row 20 and column H, so this is
/// generous by two orders of magnitude while staying cheap. Unset cells inside
/// it read as blank, which is Excel's semantics — `#REF!` comes from a deleted
/// reference, not from an empty cell.
const GRID_ROWS: u32 = 4096;
const GRID_COLS: u32 = 256;

/// Materialised volatiles (ADR-009). `TODAY`/`NOW` cannot have fixed vectors,
/// so docs/50 captures them as structural invariants instead
/// (`=TODAY()=INT(NOW())`); these values make those invariants meaningful
/// rather than trivially true at zero.
const TODAY_SERIAL: i32 = 46_000;
const NOW_SERIAL: f64 = 46_000.25;

/// Relative difference below which a numeric miss is reported as `near`.
const NEAR_TOLERANCE: f64 = 1e-12;

fn main() {
    let root = repo_root();
    let mut corpora = Vec::new();
    for (label, dir) in [
        ("1900", root.join("tools/oracle-capture/vectors")),
        ("1904", root.join("tools/oracle-capture/vectors-1904")),
    ] {
        match run_corpus(&dir) {
            Ok(report) => corpora.push((label, dir, report)),
            Err(err) => {
                eprintln!("conformance: cannot read {}: {err}", dir.display());
                std::process::exit(1);
            }
        }
    }

    let markdown = render(&corpora);
    print!("{markdown}");

    let out_dir = root.join(".tmp");
    let _ = std::fs::create_dir_all(&out_dir);
    if let Err(err) = std::fs::write(out_dir.join("oracle-report.md"), &markdown) {
        eprintln!("conformance: could not write the report: {err}");
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ------------------------------------------------------------------ results

#[derive(Default, Clone)]
struct Tally {
    total: usize,
    pass: usize,
    near: usize,
    fail: usize,
    /// A case the runner could not judge — an unreadable vector or a kind it
    /// does not understand. Counted and reported, never silently dropped: a
    /// conformance percentage that quietly skips what it cannot handle is the
    /// easiest number in the world to make look good.
    unjudged: usize,
}

impl Tally {
    fn add(&mut self, outcome: Outcome) {
        self.total += 1;
        match outcome {
            Outcome::Pass => self.pass += 1,
            Outcome::Near => {
                self.near += 1;
                self.fail += 1;
            }
            Outcome::Fail => self.fail += 1,
            Outcome::Unjudged => self.unjudged += 1,
        }
    }

    fn merge(&mut self, other: &Tally) {
        self.total += other.total;
        self.pass += other.pass;
        self.near += other.near;
        self.fail += other.fail;
        self.unjudged += other.unjudged;
    }

    fn rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        100.0 * self.pass as f64 / self.total as f64
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pass,
    /// Numerically close but not equal — a fail that is worth naming.
    Near,
    Fail,
    Unjudged,
}

struct Failure {
    id: String,
    formula: String,
    expected: String,
    actual: String,
    near: bool,
}

#[derive(Default)]
struct CorpusReport {
    overall: Tally,
    by_function: BTreeMap<String, Tally>,
    failures: Vec<Failure>,
}

// ------------------------------------------------------------------- driver

fn run_corpus(dir: &Path) -> std::io::Result<CorpusReport> {
    let mut report = CorpusReport::default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter(|p| p.file_name().is_some_and(|n| n != "_index.json"))
        .collect();
    files.sort();

    for path in files {
        let bytes = std::fs::read(&path)?;
        let doc = match usk_json::parse(&bytes) {
            Ok(doc) => doc,
            Err(err) => {
                eprintln!("conformance: {} is not JSON: {err:?}", path.display());
                let tally = report
                    .by_function
                    .entry(file_stem(&path))
                    .or_insert_with(Tally::default);
                tally.add(Outcome::Unjudged);
                continue;
            }
        };
        let function = doc
            .get("function")
            .and_then(Json::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| file_stem(&path));
        let cases = doc.get("cases").and_then(Json::as_array).unwrap_or(&[]);
        let tally = report
            .by_function
            .entry(function)
            .or_insert_with(Tally::default);
        for case in cases {
            let (outcome, failure) = judge(case);
            tally.add(outcome);
            if let Some(failure) = failure {
                report.failures.push(failure);
            }
        }
    }

    let merged: Vec<Tally> = report.by_function.values().cloned().collect();
    for tally in &merged {
        report.overall.merge(tally);
    }
    Ok(report)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

/// Evaluates one case and decides whether it matches Excel.
fn judge(case: &Json) -> (Outcome, Option<Failure>) {
    let id = case.get("id").and_then(Json::as_str).unwrap_or("?");
    let Some(formula) = case.get("formula").and_then(Json::as_str) else {
        return (Outcome::Unjudged, None);
    };
    let grid = build_grid(case.get("fixture"));
    let parsed = parse::parse(formula);

    // Excel refused to store this formula at all (D-081). We pass only by
    // refusing it too.
    if case.get("observed_status").and_then(Json::as_str) == Some("rejected-by-excel") {
        let refused = matches!(parsed.ast, Ast::Invalid(_));
        return if refused {
            (Outcome::Pass, None)
        } else {
            (
                Outcome::Fail,
                Some(Failure {
                    id: id.to_string(),
                    formula: formula.to_string(),
                    expected: String::from("rejected by Excel's parser"),
                    actual: describe(&evaluate(&parsed.ast, &grid)),
                    near: false,
                }),
            )
        };
    }

    let Some(observed) = case.get("observed").filter(|o| !o.is_null()) else {
        return (Outcome::Unjudged, None);
    };
    let actual = evaluate(&parsed.ast, &grid);
    let (outcome, expected) = compare(observed, &actual);
    let failure = match outcome {
        Outcome::Pass | Outcome::Unjudged => None,
        Outcome::Near | Outcome::Fail => Some(Failure {
            id: id.to_string(),
            formula: formula.to_string(),
            expected,
            actual: describe(&actual),
            near: outcome == Outcome::Near,
        }),
    };
    (outcome, failure)
}

fn evaluate(ast: &Ast, grid: &FixtureGrid) -> Value {
    let mut ctx = Context::new(grid, Profile::Compat);
    ctx.today = TODAY_SERIAL;
    ctx.now = NOW_SERIAL;
    // `eval_top`, because a corpus case *is* a whole formula — the position
    // where the cancellation adjustment fires (docs/50 finding 2).
    usk_formula::eval::eval_top(ast, &ctx)
}

fn compare(observed: &Json, actual: &Value) -> (Outcome, String) {
    match observed.get("kind").and_then(Json::as_str) {
        Some("number") => {
            // `number_r17` is authoritative; `number` is a convenience the
            // corpus itself warns against (docs/50 §Corpus format).
            let Some(expected) = observed
                .get("number_r17")
                .and_then(Json::as_str)
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| observed.get("number").and_then(Json::as_f64))
            else {
                return (Outcome::Unjudged, String::new());
            };
            let label = format!("{expected:?}");
            match numeric(actual) {
                Some(got) if got == expected => (Outcome::Pass, label),
                Some(got) if is_near(got, expected) => (Outcome::Near, label),
                _ => (Outcome::Fail, label),
            }
        }
        Some("text") => {
            let expected = observed.get("text").and_then(Json::as_str).unwrap_or("");
            let ok = matches!(actual, Value::Text(s) if s == expected);
            (pass_if(ok), format!("{expected:?}"))
        }
        Some("logical") => {
            let Some(expected) = observed.get("logical").and_then(Json::as_bool) else {
                return (Outcome::Unjudged, String::new());
            };
            let ok = matches!(actual, Value::Bool(b) if *b == expected);
            (pass_if(ok), expected.to_string())
        }
        Some("error") => {
            let expected = observed.get("error").and_then(Json::as_str).unwrap_or("");
            let ok = actual
                .as_error()
                .is_some_and(|e| e.kind.as_str() == expected);
            (pass_if(ok), expected.to_string())
        }
        Some("blank") => (
            pass_if(matches!(actual, Value::Blank)),
            String::from("blank"),
        ),
        _ => (Outcome::Unjudged, String::new()),
    }
}

fn pass_if(ok: bool) -> Outcome {
    if ok {
        Outcome::Pass
    } else {
        Outcome::Fail
    }
}

/// A `Decimal` result is compared in the float domain: Excel has no exact
/// decimal type, so an exact answer that agrees with Excel's float answer is a
/// pass, and one that does not is a genuine divergence either way.
fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => Some(*n),
        Value::Decimal(d) => Some(d.to_f64()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn is_near(got: f64, expected: f64) -> bool {
    if !got.is_finite() || !expected.is_finite() {
        return false;
    }
    let scale = got.abs().max(expected.abs());
    if scale == 0.0 {
        return true;
    }
    (got - expected).abs() / scale <= NEAR_TOLERANCE
}

fn describe(value: &Value) -> String {
    match value {
        Value::Blank => String::from("blank"),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format!("{n:?}"),
        Value::Decimal(d) => format!("{:?}", d.to_f64()),
        Value::Text(s) => format!("{s:?}"),
        Value::Error(e) => e.kind.as_str().to_string(),
    }
}

// -------------------------------------------------------------------- grid

struct FixtureGrid {
    cells: BTreeMap<(u32, u32), Value>,
}

impl Grid for FixtureGrid {
    fn read(&self, row: u32, col: u32) -> Option<Value> {
        if row >= GRID_ROWS || col >= GRID_COLS {
            return None;
        }
        Some(self.cells.get(&(row, col)).cloned().unwrap_or(Value::Blank))
    }

    fn extent(&self) -> (u32, u32) {
        (GRID_ROWS, GRID_COLS)
    }
}

/// Builds the case's fixture grid.
///
/// Literal cells land first, then formula cells are evaluated against what is
/// already there. Two passes, because a fixture cell may name another one and
/// the corpus does not promise a topological order — this is a fixture loader,
/// not the dependency graph, and two passes cover every shape the corpus has.
fn build_grid(fixture: Option<&Json>) -> FixtureGrid {
    let mut grid = FixtureGrid {
        cells: BTreeMap::new(),
    };
    let Some(entries) = fixture.and_then(Json::as_array) else {
        return grid;
    };

    let mut formulas: Vec<((u32, u32), String)> = Vec::new();
    for entry in entries {
        let Some(at) = entry.get("ref").and_then(Json::as_str).and_then(cell_ref) else {
            continue;
        };
        if let Some(formula) = entry.get("formula").and_then(Json::as_str) {
            formulas.push((at, formula.to_string()));
            continue;
        }
        if let Some(value) = fixture_value(entry) {
            grid.cells.insert(at, value);
        }
    }

    for _ in 0..2 {
        for (at, source) in &formulas {
            let parsed = parse::parse(source);
            let value = evaluate(&parsed.ast, &grid);
            grid.cells.insert(*at, value);
        }
    }
    grid
}

fn fixture_value(entry: &Json) -> Option<Value> {
    // `text` forces a text cell even when the content looks numeric — the
    // gene-name case (D-041's sibling) depends on the distinction.
    if let Some(text) = entry.get("text").and_then(Json::as_str) {
        return Some(Value::Text(text.to_string()));
    }
    // `codepoints` carries non-ASCII input, because the grid files are pure
    // ASCII so no encoding guess can corrupt the input half of a vector.
    if let Some(points) = entry.get("codepoints").and_then(Json::as_array) {
        let mut out = String::new();
        for point in points {
            if let Some(ch) = point.as_f64().and_then(|n| char::from_u32(n as u32)) {
                out.push(ch);
            }
        }
        return Some(Value::Text(out));
    }
    if entry.get("blank").and_then(Json::as_bool) == Some(true) {
        return Some(Value::Blank);
    }
    match entry.get("value") {
        Some(Json::Number(_)) => entry.get("value").and_then(Json::as_f64).map(Value::Number),
        Some(Json::Bool(b)) => Some(Value::Bool(*b)),
        Some(Json::String(s)) => Some(Value::Text(s.clone())),
        Some(Json::Null) | None => None,
        Some(_) => None,
    }
}

/// `"A1"` / `"$B$3"` → 0-based `(row, col)`, through the engine's own parser so
/// the runner cannot disagree with it about what a reference means.
fn cell_ref(text: &str) -> Option<(u32, u32)> {
    parse::parse_a1(text).map(|a| (a.row, a.col))
}

// ------------------------------------------------------------------ report

fn render(corpora: &[(&str, PathBuf, CorpusReport)]) -> String {
    let mut out = String::new();
    let mut grand = Tally::default();
    for (_, _, report) in corpora {
        grand.merge(&report.overall);
    }

    let _ = writeln!(out, "# W-ORACLE — Excel conformance (docs/38, docs/50)\n");
    let _ = writeln!(
        out,
        "**{:.1}%** of {} oracle cases match real Excel exactly under `Profile::Compat`.",
        grand.rate(),
        grand.total
    );
    let _ = writeln!(
        out,
        "{} pass · {} fail (of which {} are numerically near, relative difference <= {NEAR_TOLERANCE:e}) · {} unjudged.\n",
        grand.pass, grand.fail, grand.near, grand.unjudged
    );

    for (label, dir, report) in corpora {
        let _ = writeln!(
            out,
            "## {} date system — {:.1}% ({}/{})\n",
            label,
            report.overall.rate(),
            report.overall.pass,
            report.overall.total
        );
        let _ = writeln!(out, "Corpus: `{}`\n", relative(dir));
        let _ = writeln!(out, "| Function | Cases | Pass | Fail | Near | Rate |");
        let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|");
        let mut rows: Vec<(&String, &Tally)> = report.by_function.iter().collect();
        rows.sort_by(|a, b| {
            a.1.rate()
                .partial_cmp(&b.1.rate())
                .unwrap_or(core::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(b.0))
        });
        for (function, tally) in rows {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} | {:.1}% |",
                function,
                tally.total,
                tally.pass,
                tally.fail,
                tally.near,
                tally.rate()
            );
        }
        let _ = writeln!(out);

        let _ = writeln!(
            out,
            "<details><summary>{} divergences</summary>\n",
            report.failures.len()
        );
        let _ = writeln!(out, "| Case | Formula | Excel | Ehkatra | |");
        let _ = writeln!(out, "|---|---|---|---|---|");
        for failure in &report.failures {
            let _ = writeln!(
                out,
                "| `{}` | `{}` | `{}` | `{}` | {} |",
                failure.id,
                failure.formula.replace('|', "\\|"),
                failure.expected.replace('|', "\\|"),
                failure.actual.replace('|', "\\|"),
                if failure.near { "near" } else { "" }
            );
        }
        let _ = writeln!(out, "\n</details>\n");
    }
    out
}

fn relative(dir: &Path) -> String {
    dir.components()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
