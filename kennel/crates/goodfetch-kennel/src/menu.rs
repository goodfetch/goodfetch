//! The menu's edits: the changes that turn the rows a menu shows into the
//! rows of the next view in place, for a Tray whose menu cannot be replaced
//! once it is set (Linux's `AppIndicator`, #2077).
//!
//! A row keeps its toolkit item while it stays the same kind, so a change
//! of text, of Lifecycle or of counts redraws that item alone, and an open
//! submenu stays open while its Run changes. Only a change of kind, or of
//! the number of rows, inserts or removes items.

#[cfg(test)]
mod tests;

use crate::state::Row;

/// One change to the menu, applied to the menu as the edits before it left
/// it. `at` is the row's place: its index in the menu, then its index in
/// each submenu down to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit<'a> {
    /// Redraw the item at `at` as `row`, which is of the same kind as the
    /// row it shows: a line (a text, a detail or a choice) whose text or
    /// choice changed, or a submenu whose label changed. A submenu's rows
    /// change through edits of their own.
    Redraw {
        /// Where the item is.
        at: Vec<usize>,
        /// What it shows now.
        row: &'a Row,
    },
    /// Insert an item showing `row`, a submenu's rows included, at `at`.
    Insert {
        /// Where it goes.
        at: Vec<usize>,
        /// What it shows.
        row: &'a Row,
    },
    /// Remove the item at `at`, a submenu's items included.
    Remove {
        /// Where the item is.
        at: Vec<usize>,
    },
}

/// The edits that turn a menu showing `shown` into one showing `next`.
pub fn edits<'a>(shown: &[Row], next: &'a [Row]) -> Vec<Edit<'a>> {
    let mut edits = Vec::new();
    edit_rows(&mut Vec::new(), shown, next, &mut edits);
    edits
}

/// The kind of toolkit item that shows a row: a row keeps its item while
/// its kind stays the same.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A text, a detail or a choice: one menu item, enabled or not.
    Line,
    Separator,
    Submenu,
}

fn kind(row: &Row) -> Kind {
    match row {
        Row::Text(_) | Row::Detail(_) | Row::Choice { .. } => Kind::Line,
        Row::Separator => Kind::Separator,
        Row::Submenu { .. } => Kind::Submenu,
    }
}

/// Adds to `edits` those that turn the rows `shown` under `above` into
/// `next`: row by row in place, then the rows past the shorter list
/// inserted in order or removed from the last.
fn edit_rows<'a>(
    above: &mut Vec<usize>,
    shown: &[Row],
    next: &'a [Row],
    edits: &mut Vec<Edit<'a>>,
) {
    let at = |above: &Vec<usize>, index: usize| {
        let mut at = above.clone();
        at.push(index);
        at
    };
    for (index, (was, row)) in shown.iter().zip(next).enumerate() {
        if kind(was) != kind(row) {
            edits.push(Edit::Remove {
                at: at(above, index),
            });
            edits.push(Edit::Insert {
                at: at(above, index),
                row,
            });
            continue;
        }
        match (was, row) {
            (
                Row::Submenu {
                    label: was_label,
                    rows: was_rows,
                },
                Row::Submenu { label, rows },
            ) => {
                if was_label != label {
                    edits.push(Edit::Redraw {
                        at: at(above, index),
                        row,
                    });
                }
                above.push(index);
                edit_rows(above, was_rows, rows, edits);
                above.pop();
            }
            _ if was != row => edits.push(Edit::Redraw {
                at: at(above, index),
                row,
            }),
            _ => {}
        }
    }
    for (index, row) in next.iter().enumerate().skip(shown.len()) {
        edits.push(Edit::Insert {
            at: at(above, index),
            row,
        });
    }
    for index in (next.len()..shown.len()).rev() {
        edits.push(Edit::Remove {
            at: at(above, index),
        });
    }
}
