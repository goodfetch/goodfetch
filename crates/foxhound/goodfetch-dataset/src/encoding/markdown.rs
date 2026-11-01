//! A GitHub Flavored Markdown table: a header row, its delimiter row, then
//! one row per Record. Each cell's text has a backslash before every
//! character that would start inline markup or end its cell (`\`, `` ` ``,
//! `*`, `_`, `[`, `]`, `<`, `>`, `&`, `~` and `|`), and each line break
//! written as `<br>`, since a table row is one line. A table of no column
//! writes nothing.

use super::columns::Cell;

/// The writer of one Markdown Export's rows.
#[derive(Clone, Copy, Debug)]
pub(super) struct Markdown;

impl Markdown {
    pub(super) fn begin(names: &[String], out: &mut Vec<u8>) {
        if names.is_empty() {
            return;
        }
        let header: Vec<Cell> = names.iter().cloned().map(Cell::Text).collect();
        Self::row(&header, out);
        let mut line = String::from("|");
        for _ in names {
            line.push_str(" --- |");
        }
        line.push('\n');
        out.extend_from_slice(line.as_bytes());
    }

    pub(super) fn row(cells: &[Cell], out: &mut Vec<u8>) {
        if cells.is_empty() {
            return;
        }
        let mut line = String::from("|");
        for cell in cells {
            line.push(' ');
            escape(cell.text(), &mut line);
            line.push_str(" |");
        }
        line.push('\n');
        out.extend_from_slice(line.as_bytes());
    }
}

fn escape(text: &str, out: &mut String) {
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '~' | '|' => {
                out.push('\\');
                out.push(character);
            }
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                out.push_str("<br>");
            }
            '\n' => out.push_str("<br>"),
            other => out.push(other),
        }
    }
}
