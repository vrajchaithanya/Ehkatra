//! Fill-drag: extending a selection by its own pattern (TD-64, ADR-040,
//! docs/25 §the grid — *"selection/fill/drag grammar per Excel"*).
//!
//! # What a fill actually decides
//! Dragging the handle asks one question per target cell: *what should be
//! here?* Excel answers it three ways, and this implements those three:
//!
//! * **A formula** moves. Its relative references shift by how far the cell has
//!   travelled, which is `usk_formula::translate` and is why that lives in the
//!   kernel rather than here.
//! * **Two or more numbers in a straight line** extrapolate. `1, 2` fills
//!   `3, 4, 5`; `10, 20` fills `30, 40`. This is the behaviour people mean when
//!   they say "drag to fill", and getting it wrong by repeating instead is
//!   immediately, obviously wrong.
//! * **Everything else** repeats, cycling the source. One number repeats — Excel
//!   needs `Ctrl` to make a single number count up, and a bare drag copies it.
//!
//! # What it deliberately does not decide
//! Excel also fills dates by month, weekday names, custom lists, and text with
//! a trailing number (`Item 1` → `Item 2`). Each needs a model this build does
//! not have — a date type (TD-40), a locale (TD-49), a user list — and guessing
//! at them here would put a worse copy of each in the shell. Recorded in TD-69
//! rather than half-built.

use usk_types::Value;

use crate::clipboard::Cell;

/// Numbers this far apart in relative terms are the "same" step.
///
/// A tolerance is needed because `0.1, 0.2, 0.3` does not have an exactly equal
/// step in binary floating point, and a user who typed a decimal series and got
/// a repeat instead of an extrapolation would be right to call it a bug.
const STEP_EPSILON: f64 = 1e-9;

/// The arithmetic step of a numeric run, if it has one.
///
/// `None` unless every cell is a number and every gap is the same. A single
/// cell has no step by construction — one point does not make a line, and
/// Excel agrees: a bare drag from one number repeats it.
fn step_of(source: &[Cell]) -> Option<f64> {
    if source.len() < 2 {
        return None;
    }
    let mut numbers = Vec::with_capacity(source.len());
    for cell in source {
        match cell {
            Cell::Value(Value::Number(n)) => numbers.push(*n),
            _ => return None,
        }
    }
    let step = numbers[1] - numbers[0];
    let scale = numbers
        .iter()
        .fold(1.0f64, |acc, n| acc.max(n.abs()))
        .max(1.0);
    for pair in numbers.windows(2) {
        if (pair[1] - pair[0] - step).abs() > STEP_EPSILON * scale {
            return None;
        }
    }
    Some(step)
}

/// What one filled cell should hold.
#[derive(Clone, Debug, PartialEq)]
pub enum Filled {
    Value(Value),
    /// A formula source, already translated for its destination.
    Formula(String),
    Blank,
}

/// Fills `count` cells beyond `source`, in one direction.
///
/// `offset_of(i)` gives the `(rows, cols)` a target cell has travelled from the
/// source cell it takes after — the caller owns that, because only it knows
/// whether the drag went down, up, left or right.
///
/// Filling *backwards* (up or left) is a negative-going series, which falls out
/// of the same arithmetic: `step` is applied per position, and the caller
/// numbers positions in drag order.
pub fn fill<F>(source: &[Cell], count: usize, offset_of: F) -> Vec<Filled>
where
    F: Fn(usize) -> (i64, i64),
{
    let mut out = Vec::with_capacity(count);
    if source.is_empty() {
        return out;
    }
    let step = step_of(source);
    let last_number = match source.last() {
        Some(Cell::Value(Value::Number(n))) => *n,
        _ => 0.0,
    };

    for i in 0..count {
        // Which source cell this target takes after. Cycling is what makes a
        // three-cell pattern repeat as a three-cell pattern.
        let from = &source[i % source.len()];
        out.push(match (step, from) {
            // The series case wins over cycling: a numeric run extrapolates
            // rather than repeating.
            (Some(step), _) => Filled::Value(Value::Number(last_number + step * (i as f64 + 1.0))),
            (None, Cell::Formula { source, .. }) => {
                let (dr, dc) = offset_of(i);
                Filled::Formula(usk_formula::translate::translate(source, dr, dc))
            }
            (None, Cell::Value(v)) => Filled::Value(v.clone()),
            (None, Cell::Blank) => Filled::Blank,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> Cell {
        Cell::Value(Value::Number(n))
    }

    /// A straight downward drag: cell `i` is `i + 1` rows below the last source
    /// cell.
    fn down(i: usize) -> (i64, i64) {
        (i as i64 + 1, 0)
    }

    fn values(filled: &[Filled]) -> Vec<f64> {
        filled
            .iter()
            .map(|f| match f {
                Filled::Value(Value::Number(n)) => *n,
                other => panic!("expected a number, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn two_numbers_extrapolate_the_series_they_describe() {
        assert_eq!(
            values(&fill(&[num(1.0), num(2.0)], 3, down)),
            [3.0, 4.0, 5.0]
        );
        assert_eq!(
            values(&fill(&[num(10.0), num(20.0)], 3, down)),
            [30.0, 40.0, 50.0]
        );
    }

    #[test]
    fn a_descending_series_keeps_descending() {
        assert_eq!(
            values(&fill(&[num(10.0), num(7.0)], 3, down)),
            [4.0, 1.0, -2.0]
        );
    }

    #[test]
    fn a_decimal_series_extrapolates_despite_binary_floating_point() {
        // `0.1, 0.2, 0.3` has no exactly equal step in f64. Without the
        // tolerance this repeats instead of counting, which a user would
        // rightly call a bug.
        let out = values(&fill(&[num(0.1), num(0.2), num(0.3)], 2, down));
        assert!((out[0] - 0.4).abs() < 1e-9, "{out:?}");
        assert!((out[1] - 0.5).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn one_number_repeats_because_a_point_is_not_a_line() {
        // Excel needs Ctrl to make a single number count up; a bare drag copies.
        assert_eq!(values(&fill(&[num(7.0)], 3, down)), [7.0, 7.0, 7.0]);
    }

    #[test]
    fn numbers_that_are_not_in_a_straight_line_repeat_rather_than_guess() {
        assert_eq!(
            values(&fill(&[num(1.0), num(2.0), num(4.0)], 3, down)),
            [1.0, 2.0, 4.0]
        );
    }

    #[test]
    fn a_non_numeric_pattern_cycles() {
        let src = vec![
            Cell::Value(Value::Text(String::from("a"))),
            Cell::Value(Value::Text(String::from("b"))),
        ];
        let out = fill(&src, 5, down);
        let text: Vec<String> = out
            .iter()
            .map(|f| match f {
                Filled::Value(Value::Text(s)) => s.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(text, ["a", "b", "a", "b", "a"]);
    }

    #[test]
    fn a_formula_travels_and_its_references_travel_with_it() {
        let src = vec![Cell::Formula {
            source: String::from("=SUM(A1:A3)"),
            display: String::from("6"),
        }];
        let out = fill(&src, 2, down);
        assert_eq!(out[0], Filled::Formula(String::from("=SUM(A2:A4)")));
        assert_eq!(out[1], Filled::Formula(String::from("=SUM(A3:A5)")));
    }

    #[test]
    fn a_pinned_reference_stays_pinned_all_the_way_down() {
        // The `$` is the reason anyone drags a formula rather than retyping it.
        let src = vec![Cell::Formula {
            source: String::from("=B1*$F$1"),
            display: String::from("0"),
        }];
        let out = fill(&src, 3, down);
        assert_eq!(out[2], Filled::Formula(String::from("=B4*$F$1")));
    }

    #[test]
    fn filling_sideways_moves_the_columns_and_not_the_rows() {
        let src = vec![Cell::Formula {
            source: String::from("=A1"),
            display: String::from("0"),
        }];
        let right = |i: usize| (0i64, i as i64 + 1);
        let out = fill(&src, 2, right);
        assert_eq!(out[0], Filled::Formula(String::from("=B1")));
        assert_eq!(out[1], Filled::Formula(String::from("=C1")));
    }

    #[test]
    fn a_blank_source_fills_blanks_and_not_zeroes() {
        assert_eq!(
            fill(&[Cell::Blank], 2, down),
            [Filled::Blank, Filled::Blank]
        );
    }

    #[test]
    fn filling_nothing_is_not_an_error() {
        assert!(fill(&[], 5, down).is_empty());
        assert!(fill(&[num(1.0)], 0, down).is_empty());
    }
}
