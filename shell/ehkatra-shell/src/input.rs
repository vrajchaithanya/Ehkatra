//! The keymap: platform keys in, **intents** out (docs/25 §Interaction rules).
//!
//! # Why this is a module and not a `match` inside the event loop
//! docs/25 asks for an *"Excel-parity default keymap (remappable)"* and for
//! *"every action reachable without a mouse"*. Both are claims about a table,
//! and a table can be tested. Nothing here touches winit, a window, or a GPU,
//! so the entire keymap is proven by ordinary unit tests on a machine with no
//! display — which is the same reason `usk-view` is a kernel crate.
//!
//! # The one rule that shapes everything
//! A key means different things depending on whether the in-cell editor is
//! open. `Enter` moves down in the grid and commits in the editor; a printable
//! character *starts* an edit in the grid and *inserts* in the editor. So
//! [`translate`] takes the mode, and the mode is the only state it has.

/// A key, named the way a user would name it rather than the way a scancode
/// does. `Character` carries the *text* the platform produced, so a layout that
/// puts `/` where a US keyboard puts `-` needs nothing here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Tab,
    Escape,
    F2,
    Delete,
    Backspace,
    Character(char),
}

/// Modifier state. `alt` is carried but unbound: on Windows it opens the menu
/// bar, which is a platform adapter's business and not the grid's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };

    pub fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::NONE
        }
    }

    pub fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::NONE
        }
    }
}

/// How far a cursor move goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
    /// Excel's `Ctrl+Arrow`: to the edge of the current data region.
    EdgeLeft,
    EdgeRight,
    EdgeUp,
    EdgeDown,
    /// `Home` — the first column of the current row.
    RowStart,
    PageUp,
    PageDown,
    /// `Ctrl+Home` / `Ctrl+End`.
    SheetStart,
    SheetEnd,
}

/// What the editor should start out holding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seed {
    /// `F2` — the cell's current source, caret at the end.
    Existing,
    /// `Backspace` — cleared, ready to retype.
    Empty,
    /// A printable character typed over the cell, which replaces it.
    Typed(char),
}

/// One thing the application should do. The vocabulary the window and the
/// tests both speak, so a scripted session and a real one are the same session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intent {
    /// Move the active cell. `extend` grows the selection from its anchor
    /// instead of collapsing it, which is what `Shift` means everywhere.
    Move {
        step: Step,
        extend: bool,
    },
    BeginEdit(Seed),
    Insert(char),
    Backspace,
    DeleteForward,
    CaretLeft,
    CaretRight,
    CaretHome,
    CaretEnd,
    /// Write the editor's contents and move on.
    Commit {
        then: Step,
    },
    /// Abandon the edit, or collapse the selection when there is no edit.
    Cancel,
    /// Clear the selected cells.
    Clear,
    Undo,
    Redo,
    Copy,
    /// Copy, and mark the source to be cleared when the block is next pasted.
    Cut,
    Paste,
    InsertRow,
    DeleteRow,
    InsertCol,
    DeleteCol,
}

/// Whether the in-cell editor is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Grid,
    Editing,
}

/// The keymap. `None` for a key this mode does not bind — which is a real
/// answer and not a failure, and is why an unbound key does not consume a
/// redraw.
pub fn translate(key: Key, mods: Mods, mode: Mode) -> Option<Intent> {
    match mode {
        Mode::Editing => editing(key, mods),
        Mode::Grid => grid(key, mods),
    }
}

fn editing(key: Key, mods: Mods) -> Option<Intent> {
    Some(match key {
        // A commit that also moves is one intent, not two: Excel's `Enter`
        // writes *and* steps down, and splitting them would let a caller do
        // half of it.
        Key::Enter => Intent::Commit {
            then: if mods.shift { Step::Up } else { Step::Down },
        },
        Key::Tab => Intent::Commit {
            then: if mods.shift { Step::Left } else { Step::Right },
        },
        Key::Up => Intent::Commit { then: Step::Up },
        Key::Down => Intent::Commit { then: Step::Down },
        Key::Escape => Intent::Cancel,
        Key::Left => Intent::CaretLeft,
        Key::Right => Intent::CaretRight,
        Key::Home => Intent::CaretHome,
        Key::End => Intent::CaretEnd,
        Key::Backspace => Intent::Backspace,
        Key::Delete => Intent::DeleteForward,
        Key::Character(c) if !mods.ctrl && !mods.alt && !c.is_control() => Intent::Insert(c),
        _ => return None,
    })
}

fn grid(key: Key, mods: Mods) -> Option<Intent> {
    let move_to = |step: Step| {
        Some(Intent::Move {
            step,
            extend: mods.shift,
        })
    };
    match key {
        Key::Left if mods.ctrl => move_to(Step::EdgeLeft),
        Key::Right if mods.ctrl => move_to(Step::EdgeRight),
        Key::Up if mods.ctrl => move_to(Step::EdgeUp),
        Key::Down if mods.ctrl => move_to(Step::EdgeDown),
        Key::Left => move_to(Step::Left),
        Key::Right => move_to(Step::Right),
        Key::Up => move_to(Step::Up),
        Key::Down => move_to(Step::Down),
        Key::Home if mods.ctrl => move_to(Step::SheetStart),
        Key::Home => move_to(Step::RowStart),
        Key::End if mods.ctrl => move_to(Step::SheetEnd),
        // Bare `End` in Excel arms a mode; unarmed it does nothing, and doing
        // nothing is more honest than inventing a third meaning.
        Key::End => None,
        Key::PageUp => move_to(Step::PageUp),
        Key::PageDown => move_to(Step::PageDown),
        Key::Enter => move_to(if mods.shift { Step::Up } else { Step::Down }),
        Key::Tab => move_to(if mods.shift { Step::Left } else { Step::Right }),
        Key::F2 => Some(Intent::BeginEdit(Seed::Existing)),
        Key::Backspace => Some(Intent::BeginEdit(Seed::Empty)),
        Key::Delete => Some(Intent::Clear),
        Key::Escape => Some(Intent::Cancel),
        // Structural edits. Excel puts these behind a dialog that asks *which
        // way to shift*; there is no dialog layer yet (docs/33's platform
        // adapters), so rows and columns are split by `Shift` instead and the
        // divergence is recorded rather than hidden.
        Key::Character('+' | '=') if mods.ctrl => Some(if mods.shift {
            Intent::InsertCol
        } else {
            Intent::InsertRow
        }),
        Key::Character('-') if mods.ctrl => Some(if mods.shift {
            Intent::DeleteCol
        } else {
            Intent::DeleteRow
        }),
        Key::Character('z' | 'Z') if mods.ctrl => Some(if mods.shift {
            Intent::Redo
        } else {
            Intent::Undo
        }),
        Key::Character('y' | 'Y') if mods.ctrl => Some(Intent::Redo),
        Key::Character('c' | 'C') if mods.ctrl => Some(Intent::Copy),
        Key::Character('x' | 'X') if mods.ctrl => Some(Intent::Cut),
        Key::Character('v' | 'V') if mods.ctrl => Some(Intent::Paste),
        // Typing over a cell replaces it — the single most-used editing gesture
        // in a spreadsheet, and the reason `Seed` exists.
        Key::Character(c) if !mods.ctrl && !mods.alt && !c.is_control() => {
            Some(Intent::BeginEdit(Seed::Typed(c)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keys_move_and_shift_extends() {
        assert_eq!(
            translate(Key::Down, Mods::NONE, Mode::Grid),
            Some(Intent::Move {
                step: Step::Down,
                extend: false
            })
        );
        assert_eq!(
            translate(Key::Down, Mods::shift(), Mode::Grid),
            Some(Intent::Move {
                step: Step::Down,
                extend: true
            })
        );
    }

    #[test]
    fn ctrl_arrow_jumps_to_the_data_edge() {
        assert_eq!(
            translate(Key::Right, Mods::ctrl(), Mode::Grid),
            Some(Intent::Move {
                step: Step::EdgeRight,
                extend: false
            })
        );
    }

    #[test]
    fn typing_a_character_in_the_grid_starts_an_edit_that_replaces() {
        assert_eq!(
            translate(Key::Character('7'), Mods::NONE, Mode::Grid),
            Some(Intent::BeginEdit(Seed::Typed('7')))
        );
        assert_eq!(
            translate(Key::Character('='), Mods::NONE, Mode::Grid),
            Some(Intent::BeginEdit(Seed::Typed('=')))
        );
    }

    #[test]
    fn f2_edits_in_place_and_backspace_edits_empty() {
        assert_eq!(
            translate(Key::F2, Mods::NONE, Mode::Grid),
            Some(Intent::BeginEdit(Seed::Existing))
        );
        assert_eq!(
            translate(Key::Backspace, Mods::NONE, Mode::Grid),
            Some(Intent::BeginEdit(Seed::Empty))
        );
    }

    #[test]
    fn the_same_key_means_different_things_in_the_two_modes() {
        assert_eq!(
            translate(Key::Enter, Mods::NONE, Mode::Grid),
            Some(Intent::Move {
                step: Step::Down,
                extend: false
            })
        );
        assert_eq!(
            translate(Key::Enter, Mods::NONE, Mode::Editing),
            Some(Intent::Commit { then: Step::Down })
        );
        assert_eq!(
            translate(Key::Character('a'), Mods::NONE, Mode::Editing),
            Some(Intent::Insert('a'))
        );
        // An arrow in the editor moves the caret, not the cell.
        assert_eq!(
            translate(Key::Left, Mods::NONE, Mode::Editing),
            Some(Intent::CaretLeft)
        );
    }

    #[test]
    fn an_arrow_up_or_down_while_editing_commits_the_way_excel_does() {
        assert_eq!(
            translate(Key::Down, Mods::NONE, Mode::Editing),
            Some(Intent::Commit { then: Step::Down })
        );
    }

    #[test]
    fn shift_reverses_enter_and_tab_in_both_modes() {
        assert_eq!(
            translate(Key::Tab, Mods::shift(), Mode::Grid),
            Some(Intent::Move {
                step: Step::Left,
                extend: true
            })
        );
        assert_eq!(
            translate(Key::Tab, Mods::shift(), Mode::Editing),
            Some(Intent::Commit { then: Step::Left })
        );
    }

    #[test]
    fn undo_and_redo_are_bound_both_ways() {
        assert_eq!(
            translate(Key::Character('z'), Mods::ctrl(), Mode::Grid),
            Some(Intent::Undo)
        );
        assert_eq!(
            translate(
                Key::Character('z'),
                Mods {
                    ctrl: true,
                    shift: true,
                    alt: false
                },
                Mode::Grid
            ),
            Some(Intent::Redo)
        );
        assert_eq!(
            translate(Key::Character('y'), Mods::ctrl(), Mode::Grid),
            Some(Intent::Redo)
        );
    }

    #[test]
    fn a_ctrl_chord_never_types_a_character() {
        // The bug this prevents: `Ctrl+S` inserting an `s` into a cell.
        assert_eq!(
            translate(Key::Character('s'), Mods::ctrl(), Mode::Grid),
            None
        );
        assert_eq!(
            translate(Key::Character('s'), Mods::ctrl(), Mode::Editing),
            None
        );
    }

    #[test]
    fn the_clipboard_chords_are_the_ones_every_platform_agrees_on() {
        assert_eq!(
            translate(Key::Character('c'), Mods::ctrl(), Mode::Grid),
            Some(Intent::Copy)
        );
        assert_eq!(
            translate(Key::Character('x'), Mods::ctrl(), Mode::Grid),
            Some(Intent::Cut)
        );
        assert_eq!(
            translate(Key::Character('v'), Mods::ctrl(), Mode::Grid),
            Some(Intent::Paste)
        );
        // Not while editing: inside the editor these belong to the text field,
        // and binding them to a *cell* copy would take the user's selection out
        // from under them mid-word.
        assert_eq!(
            translate(Key::Character('c'), Mods::ctrl(), Mode::Editing),
            None
        );
    }

    #[test]
    fn structural_edits_split_rows_and_columns_by_shift() {
        assert_eq!(
            translate(Key::Character('+'), Mods::ctrl(), Mode::Grid),
            Some(Intent::InsertRow)
        );
        assert_eq!(
            translate(
                Key::Character('-'),
                Mods {
                    ctrl: true,
                    shift: true,
                    alt: false
                },
                Mode::Grid
            ),
            Some(Intent::DeleteCol)
        );
    }
}
