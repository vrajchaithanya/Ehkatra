//! The clipboard: a block of cells, two flavours, and one honesty rule
//! (TD-64, ADR-040, docs/25 §the grid).
//!
//! # Two flavours, because a spreadsheet has two audiences
//! Copying cells has to satisfy two readers that want different things:
//!
//! * **Another application** wants text. Excel, Sheets, Notepad and every
//!   terminal read `text/plain`, and what a spreadsheet writes there is
//!   **tab-separated displayed values** — not formulas. Paste `=A1+B1` into
//!   Notepad and Excel gives you `42`, because the formula would mean nothing
//!   there.
//! * **This application** wants everything: the formula behind the 42, so a
//!   paste can move its references and stay live.
//!
//! So a copy writes TSV to the OS clipboard *and* keeps the full [`Block`]
//! in-process.
//!
//! # The ownership rule, which is the honesty part
//! On paste, the in-process block is used **only if the OS clipboard still
//! holds exactly the text we wrote**. If it does not, the user copied something
//! else in between — from a browser, from Excel — and we parse the text like
//! anyone else would. There is no portable "do I own the clipboard" query, and
//! comparing what is there against what we put there is both cheap and exactly
//! the question. Guessing the other way would paste a stale formula over
//! whatever the user actually copied.

use usk_types::Value;

use crate::text;

/// One cell, as a clipboard carries it.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Blank,
    Value(Value),
    /// A formula: the source as authored, and the value it was showing.
    ///
    /// Both, because the two flavours need different halves — `text/plain`
    /// carries `display`, an internal paste carries `source`, and recomputing
    /// one from the other at paste time is not possible in either direction.
    Formula {
        source: String,
        display: String,
    },
}

impl Cell {
    /// What `text/plain` shows for this cell.
    pub fn display(&self) -> String {
        match self {
            Cell::Blank => String::new(),
            Cell::Value(v) => text::render_value(v).unwrap_or_default(),
            Cell::Formula { display, .. } => display.clone(),
        }
    }
}

/// A rectangular block of cells, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Cell>,
    /// The `(row, column)` ordinals the block's top-left came from.
    ///
    /// A formula's references move by *where the block landed minus where it
    /// came from*, which is one delta for the whole block and cannot be
    /// recovered from the destination alone. Zero for a block parsed from
    /// foreign text, which carries no formulas for the delta to apply to.
    pub origin: (usize, usize),
    /// Whether a paste should move the formulas' relative references.
    ///
    /// True for a copy and **false for a cut**, which is Excel's rule and not
    /// an arbitrary one: a copy makes a second formula that should say the same
    /// thing about its own row, while a cut *moves* the one formula, which
    /// should go on meaning what it meant.
    pub translate: bool,
}

impl Block {
    pub fn get(&self, row: usize, col: usize) -> Option<&Cell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.cells.get(row * self.cols + col)
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0 || self.cols == 0
    }
}

/// Renders a block as tab-separated text.
///
/// Fields containing a tab, a newline or a quote are quoted and their quotes
/// doubled — the same rule CSV uses, and the one Excel writes and reads. Rows
/// end `\r\n`, because that is what Windows applications expect and every other
/// platform tolerates.
pub fn to_tsv(block: &Block) -> String {
    let mut out = String::new();
    for r in 0..block.rows {
        if r > 0 {
            out.push_str("\r\n");
        }
        for c in 0..block.cols {
            if c > 0 {
                out.push('\t');
            }
            let text = block.get(r, c).map(Cell::display).unwrap_or_default();
            if text.contains(['\t', '\n', '\r', '"']) {
                out.push('"');
                for ch in text.chars() {
                    if ch == '"' {
                        out.push('"');
                    }
                    out.push(ch);
                }
                out.push('"');
            } else {
                out.push_str(&text);
            }
        }
    }
    out
}

/// Parses tab-separated text into a block of **values**.
///
/// Each field goes through the same rule as a value typed into a cell, so text
/// pasted from another application and text typed by hand cannot disagree about
/// what `1E2` or `TRUE` means. Ragged input is padded to a rectangle with
/// blanks: a block is a rectangle by definition, and refusing a ragged paste
/// would reject most of what arrives from the web.
pub fn from_tsv(text: &str) -> Block {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => quoted = true,
            '\t' => row.push(cell_of(&core::mem::take(&mut field))),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(cell_of(&core::mem::take(&mut field)));
                rows.push(core::mem::take(&mut row));
            }
            '\n' => {
                row.push(cell_of(&core::mem::take(&mut field)));
                rows.push(core::mem::take(&mut row));
            }
            other => field.push(other),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(cell_of(&field));
        rows.push(row);
    }

    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let height = rows.len();
    let mut cells = Vec::with_capacity(height * cols);
    for r in rows {
        let len = r.len();
        cells.extend(r);
        cells.extend(core::iter::repeat_n(Cell::Blank, cols - len));
    }
    Block {
        rows: height,
        cols,
        cells,
        origin: (0, 0),
        translate: true,
    }
}

fn cell_of(field: &str) -> Cell {
    if field.is_empty() {
        return Cell::Blank;
    }
    Cell::Value(crate::app::literal(field))
}

/// The system clipboard, with the in-process block that gives a same-app paste
/// its fidelity.
pub struct Clipboard {
    os: Option<arboard::Clipboard>,
    /// The text last written, and the block it was rendered from.
    mine: Option<(String, Block)>,
    /// Why the OS clipboard is unavailable, if it is.
    unavailable: Option<String>,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard {
    /// Opens the system clipboard, degrading to in-process only when there is
    /// none.
    ///
    /// A headless Linux CI runner has no clipboard, and neither does a machine
    /// whose compositor has gone away. That is a fact about the machine and not
    /// a reason to refuse to start (DP-A10), so copy and paste keep working
    /// *within* the application and the reason is available to be shown.
    pub fn new() -> Clipboard {
        match arboard::Clipboard::new() {
            Ok(os) => Clipboard {
                os: Some(os),
                mine: None,
                unavailable: None,
            },
            Err(err) => Clipboard {
                os: None,
                mine: None,
                unavailable: Some(format!("no system clipboard: {err}")),
            },
        }
    }

    /// An in-process clipboard that never touches the OS — what tests use, so
    /// the suite needs no display and one test cannot see another's copy.
    #[cfg(test)]
    pub fn detached() -> Clipboard {
        Clipboard {
            os: None,
            mine: None,
            unavailable: Some(String::from("detached")),
        }
    }

    pub fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    /// Puts a block on the clipboard: TSV to the OS, the block itself here.
    pub fn write(&mut self, block: Block) {
        let tsv = to_tsv(&block);
        if let Some(os) = self.os.as_mut() {
            // A clipboard the OS refused is worth knowing about, but not worth
            // losing the copy over: the in-process block still works.
            if let Err(err) = os.set_text(tsv.clone()) {
                self.unavailable = Some(format!("the system clipboard refused the copy: {err}"));
            }
        }
        self.mine = Some((tsv, block));
    }

    /// Takes whatever is on the clipboard as a block.
    ///
    /// The in-process block wins **only** when the OS clipboard still holds the
    /// text we wrote. Anything else is parsed as TSV, which is what it is.
    pub fn read(&mut self) -> Option<Block> {
        let os_text = self.os.as_mut().and_then(|os| os.get_text().ok());
        match (&self.mine, os_text) {
            (Some((mine, block)), Some(text)) if *mine == text => Some(block.clone()),
            // No OS clipboard at all: the in-process block is all there is, and
            // it is ours by construction.
            (Some((_, block)), None) if self.os.is_none() => Some(block.clone()),
            (_, Some(text)) if !text.is_empty() => Some(from_tsv(&text)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(rows: usize, cols: usize, cells: Vec<Cell>) -> Block {
        Block {
            rows,
            cols,
            cells,
            origin: (0, 0),
            translate: true,
        }
    }

    fn num(n: f64) -> Cell {
        Cell::Value(Value::Number(n))
    }

    fn txt(s: &str) -> Cell {
        Cell::Value(Value::Text(String::from(s)))
    }

    #[test]
    fn a_block_renders_as_tab_separated_rows() {
        let b = block(2, 2, vec![num(1.0), num(2.0), txt("a"), Cell::Blank]);
        assert_eq!(to_tsv(&b), "1\t2\r\na\t");
    }

    #[test]
    fn text_plain_carries_the_value_of_a_formula_and_not_the_formula() {
        // Excel's rule, and the reason `Cell::Formula` keeps both halves.
        let b = block(
            1,
            1,
            vec![Cell::Formula {
                source: String::from("=A1+B1"),
                display: String::from("42"),
            }],
        );
        assert_eq!(to_tsv(&b), "42");
    }

    #[test]
    fn a_field_with_a_tab_or_a_newline_is_quoted() {
        let b = block(1, 2, vec![txt("a\tb"), txt("c\nd")]);
        assert_eq!(to_tsv(&b), "\"a\tb\"\t\"c\nd\"");
    }

    #[test]
    fn a_quote_inside_a_field_is_doubled_and_survives_a_round_trip() {
        let b = block(1, 1, vec![txt("say \"hi\"")]);
        let tsv = to_tsv(&b);
        assert_eq!(tsv, "\"say \"\"hi\"\"\"");
        assert_eq!(from_tsv(&tsv).cells, vec![txt("say \"hi\"")]);
    }

    #[test]
    fn tsv_parses_back_into_a_rectangle() {
        let b = from_tsv("1\t2\r\n3\t4");
        assert_eq!((b.rows, b.cols), (2, 2));
        assert_eq!(b.cells, vec![num(1.0), num(2.0), num(3.0), num(4.0)]);
    }

    #[test]
    fn a_ragged_paste_is_padded_rather_than_refused() {
        // Most of what arrives from a web page is ragged, and refusing it would
        // be technically defensible and practically useless.
        let b = from_tsv("1\t2\t3\n4");
        assert_eq!((b.rows, b.cols), (2, 3));
        assert_eq!(b.get(1, 1), Some(&Cell::Blank));
        assert_eq!(b.get(1, 2), Some(&Cell::Blank));
    }

    #[test]
    fn a_pasted_field_reads_exactly_as_a_typed_one_would() {
        // The gene-symbol case arrives through the clipboard as often as
        // through the keyboard, and the two must not disagree.
        let b = from_tsv("1E2\tTRUE\t'1E2\thello");
        assert_eq!(b.cells[0], num(100.0));
        assert_eq!(b.cells[1], Cell::Value(Value::Bool(true)));
        assert_eq!(b.cells[2], txt("1E2"));
        assert_eq!(b.cells[3], txt("hello"));
    }

    #[test]
    fn both_line_endings_parse_and_a_trailing_one_adds_no_row() {
        assert_eq!(from_tsv("1\n2").rows, 2);
        assert_eq!(from_tsv("1\r\n2").rows, 2);
        assert_eq!(from_tsv("1\r\n2\r\n").rows, 2);
    }

    #[test]
    fn a_detached_clipboard_still_round_trips_in_process() {
        // What a headless CI runner gets, and it must not be a broken feature.
        let mut cb = Clipboard::detached();
        assert!(cb.read().is_none());
        let b = block(1, 2, vec![num(7.0), txt("x")]);
        cb.write(b.clone());
        assert_eq!(cb.read(), Some(b));
    }

    #[test]
    fn a_formula_survives_a_same_process_round_trip_at_full_fidelity() {
        let mut cb = Clipboard::detached();
        let b = block(
            1,
            1,
            vec![Cell::Formula {
                source: String::from("=SUM(A1:A5)"),
                display: String::from("15"),
            }],
        );
        cb.write(b.clone());
        // Not "15" — the whole point of keeping the block in process.
        assert_eq!(cb.read(), Some(b));
    }
}
