//! Moving a formula: relative references shift, absolute ones do not
//! (docs/12, ADR-040).
//!
//! # Why this is a kernel module and not shell code
//! "What does `=A1+$B$2` become one row down" is a question about the **formula
//! language**, not about a window. Three callers need the same answer and must
//! not each invent one: copy/paste, fill-drag, and — when they arrive — the
//! `fill_range` and `copy_range` verbs docs/20 and docs/21 owe the REST and MCP
//! surfaces (DP-D1: one vocabulary, UI ≡ REST ≡ MCP). Putting it here also
//! means it is `no_std`, dependency-free and testable without a display.
//!
//! # Why it rewrites text rather than re-printing an AST
//! The AST is lossy about everything that is not semantics: whitespace, the
//! case a user typed a function name in, how a literal was spelled. A formula
//! that came back from a paste reformatted would be a small, constant betrayal
//! of what the user wrote. So this lexes the source, finds the spans of the
//! `CellRef` tokens, and **rewrites only those substrings** — every other byte
//! survives verbatim.
//!
//! # The `#REF!` rule
//! A reference shifted off the top or left edge of the grid has no cell to name.
//! Excel writes `#REF!` into the formula text at that point, permanently, and so
//! does this: the alternative is clamping to row 1, which silently changes what
//! the formula means. `=A1` copied one row up becomes `=#REF!`, which is
//! wrong-looking because it *is* wrong, and that is the honest outcome.

use crate::lexer::{lex, TokenKind};
use crate::parse::{parse_a1, A1};
use alloc::string::String;

/// What a reference becomes when it is moved off the grid.
const REF_ERROR: &str = "#REF!";

/// Rewrites a formula as if it had been moved `dr` rows down and `dc` columns
/// right.
///
/// Only *relative* components move: `$A$1` is unchanged by any offset, `$A1`
/// moves vertically only, and `A$1` horizontally only — which is the entire
/// purpose of the `$` and the reason a fill handle is useful at all.
///
/// The source may or may not carry a leading `=`; whatever it had, it keeps.
pub fn translate(source: &str, dr: i64, dc: i64) -> String {
    if dr == 0 && dc == 0 {
        return String::from(source);
    }
    let mut out = String::with_capacity(source.len());
    let mut copied = 0usize;
    let tokens = lex(source);
    for (i, token) in tokens.iter().enumerate() {
        if token.kind != TokenKind::CellRef {
            continue;
        }
        // A `CellRef` immediately followed by `(` is a **function call**, not a
        // reference: `LOG10(100)` lexes as one because `looks_like_cell_ref` is
        // a shape test — letters then digits — and `LOG10` has that shape.
        //
        // No function in the current set is spelled that way, so nothing
        // misbehaves today; `LOG10` and `ATAN2` would the moment either is
        // added, and that is filed as TD-68. What this guard buys is that a
        // *fill* never rewrites `LOG10` to `LOG11`. A misparse is visible and
        // recoverable — the user sees an error and fixes it — whereas silently
        // editing the text a user typed is neither.
        if tokens.get(i + 1).map(|t| t.kind) == Some(TokenKind::LParen) {
            continue;
        }
        let (start, end) = (token.start as usize, token.end as usize);
        let Some(text) = source.get(start..end) else {
            continue;
        };
        let Some(a1) = parse_a1(text) else {
            // The lexer called it a reference and the parser cannot read it.
            // Leaving it verbatim is the only safe move: rewriting something
            // this module does not understand is how a paste corrupts a
            // formula.
            continue;
        };
        out.push_str(&source[copied..start]);
        out.push_str(&shifted(&a1, dr, dc));
        copied = end;
    }
    out.push_str(&source[copied..]);
    out
}

/// One reference, moved. `None` becomes `#REF!` at the call site.
fn shifted(a1: &A1, dr: i64, dc: i64) -> String {
    let row = if a1.row_absolute {
        Some(a1.row)
    } else {
        offset(a1.row, dr)
    };
    let col = if a1.col_absolute {
        Some(a1.col)
    } else {
        offset(a1.col, dc)
    };
    match (row, col) {
        (Some(row), Some(col)) => render(&A1 {
            row,
            col,
            row_absolute: a1.row_absolute,
            col_absolute: a1.col_absolute,
        }),
        _ => String::from(REF_ERROR),
    }
}

fn offset(base: u32, delta: i64) -> Option<u32> {
    let moved = base as i64 + delta;
    if moved < 0 {
        return None;
    }
    u32::try_from(moved).ok()
}

/// Renders an `A1` back to text, `$` markers included.
///
/// The inverse of [`parse_a1`], and `a_reference_survives_a_round_trip` holds
/// it to that: a parse that does not round-trip is a rewrite that silently
/// changes references it was only supposed to move.
pub fn render(a1: &A1) -> String {
    let mut out = String::new();
    if a1.col_absolute {
        out.push('$');
    }
    out.push_str(&column_letters(a1.col));
    if a1.row_absolute {
        out.push('$');
    }
    push_u32(&mut out, a1.row + 1);
    out
}

/// Bijective base-26: column 0 is `A`, 25 is `Z`, 26 is `AA` — not `AZ` or
/// `BA`. The same rule `usk_view::column_label` uses for headers, and the two
/// are checked against each other in `usk-view`'s tests.
fn column_letters(index: u32) -> String {
    let mut buf = [0u8; 8];
    let mut at = buf.len();
    let mut n = index as u64 + 1;
    while n > 0 {
        at -= 1;
        buf[at] = b'A' + ((n - 1) % 26) as u8;
        n = (n - 1) / 26;
    }
    String::from_utf8_lossy(&buf[at..]).into_owned()
}

fn push_u32(out: &mut String, mut n: u32) {
    let mut buf = [0u8; 10];
    let mut at = buf.len();
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.push_str(&String::from_utf8_lossy(&buf[at..]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn a_relative_reference_moves_with_the_formula() {
        assert_eq!(translate("=A1+B1", 1, 0), "=A2+B2");
        assert_eq!(translate("=A1+B1", 0, 1), "=B1+C1");
        assert_eq!(translate("=A1", 3, 2), "=C4");
    }

    #[test]
    fn an_absolute_reference_does_not_move_and_that_is_the_whole_point_of_it() {
        assert_eq!(translate("=$A$1", 5, 5), "=$A$1");
        // Mixed: the `$` pins one axis and only one.
        assert_eq!(translate("=$A1", 1, 1), "=$A2");
        assert_eq!(translate("=A$1", 1, 1), "=B$1");
    }

    #[test]
    fn the_common_shape_is_a_pinned_rate_against_a_moving_row() {
        // The reason anyone learns what `$` does.
        assert_eq!(translate("=B2*$F$1", 1, 0), "=B3*$F$1");
        assert_eq!(translate("=B2*$F$1", 2, 0), "=B4*$F$1");
    }

    #[test]
    fn a_range_moves_at_both_ends() {
        assert_eq!(translate("=SUM(A1:A10)", 1, 0), "=SUM(A2:A11)");
        assert_eq!(translate("=SUM($A$1:A10)", 1, 0), "=SUM($A$1:A11)");
    }

    #[test]
    fn a_reference_pushed_off_the_grid_becomes_ref_and_is_not_clamped() {
        // Clamping to row 1 would silently change what the formula means,
        // which is the one outcome worse than a visible error.
        assert_eq!(translate("=A1", -1, 0), "=#REF!");
        assert_eq!(translate("=A1", 0, -1), "=#REF!");
        assert_eq!(translate("=A2+B2", -1, 0), "=A1+B1");
        assert_eq!(translate("=SUM(A1:A5)", -1, 0), "=SUM(#REF!:A4)");
    }

    #[test]
    fn everything_that_is_not_a_reference_survives_byte_for_byte() {
        // The reason this rewrites text instead of re-printing an AST.
        assert_eq!(
            translate("= IF( A1 > 0 , \"yes\" , \"no\" )", 1, 0),
            "= IF( A2 > 0 , \"yes\" , \"no\" )"
        );
        // A quoted string that looks like a reference is a string.
        assert_eq!(translate("=\"A1\"&A1", 1, 0), "=\"A1\"&A2");
        // A function name that *looks* like a reference is not one. `LOG10`
        // lexes as `CellRef` (letters then digits), and without the call guard
        // a fill would quietly rewrite it to `LOG11`. See TD-68 — the lexer
        // ambiguity itself is latent, because no function in the current set is
        // spelled with a trailing digit.
        assert_eq!(translate("=LOG10(A1)", 1, 0), "=LOG10(A2)");
        assert_eq!(translate("=ATAN2(A1,B1)", 1, 0), "=ATAN2(A2,B2)");
    }

    #[test]
    fn a_formula_with_no_references_is_returned_unchanged() {
        assert_eq!(translate("=1+2", 9, 9), "=1+2");
        assert_eq!(translate("=TODAY()", 9, 9), "=TODAY()");
    }

    #[test]
    fn a_zero_offset_is_the_identity_on_every_input() {
        for src in ["=A1", "=$A$1", "=SUM(A1:B2)", "= IF(A1,\"x\",B1) ", "=1"] {
            assert_eq!(translate(src, 0, 0), src.to_string());
        }
    }

    #[test]
    fn a_reference_survives_a_round_trip() {
        for text in ["A1", "$A$1", "$A1", "A$1", "Z9", "AA1", "XFD1048576"] {
            let a1 = parse_a1(text).expect("a valid reference");
            assert_eq!(render(&a1), text, "{text} did not round-trip");
        }
    }

    #[test]
    fn the_column_rollover_is_bijective_base_26() {
        // 25 -> Z, 26 -> AA. Getting this wrong gives `AZ` or `BA`, and it is
        // only visible past column 26.
        assert_eq!(translate("=A1", 0, 25), "=Z1");
        assert_eq!(translate("=A1", 0, 26), "=AA1");
        // 26..701 is AA..ZZ; AAA does not begin until 702.
        assert_eq!(translate("=A1", 0, 701), "=ZZ1");
        assert_eq!(translate("=A1", 0, 702), "=AAA1");
        // Excel's last column.
        assert_eq!(translate("=A1", 0, 16_383), "=XFD1");
    }
}
