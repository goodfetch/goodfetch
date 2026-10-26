//! CSV as RFC 4180 writes it: a header row, then one row per Record, each
//! line ended by CR LF; a field holding the delimiter, a double quote, a
//! carriage return or a line feed is quoted, its double quotes doubled. A
//! UTF-8 byte order mark starts the file only when `export.csv.bom` asks
//! for it (ADR 0119). A table of no column writes no line.

use super::columns::Cell;

/// The writer of one CSV Export's rows.
#[derive(Clone, Copy, Debug)]
pub(super) struct Csv {
    pub(super) bom: bool,
    pub(super) delimiter: char,
}

impl Csv {
    pub(super) fn begin(self, names: &[String], out: &mut Vec<u8>) {
        if self.bom {
            out.extend_from_slice("\u{feff}".as_bytes());
        }
        let header: Vec<Cell> = names.iter().cloned().map(Cell::Text).collect();
        self.row(&header, out);
    }

    pub(super) fn row(self, cells: &[Cell], out: &mut Vec<u8>) {
        if cells.is_empty() {
            return;
        }
        let mut line = String::new();
        for (column, cell) in cells.iter().enumerate() {
            if column > 0 {
                line.push(self.delimiter);
            }
            self.field(cell.text(), cells.len(), &mut line);
        }
        line.push_str("\r\n");
        out.extend_from_slice(line.as_bytes());
    }

    fn field(self, text: &str, columns: usize, line: &mut String) {
        // A lone empty field would make an empty line, which readers skip.
        let quoted = (text.is_empty() && columns == 1)
            || text.chars().any(|character| {
                matches!(character, '"' | '\r' | '\n') || character == self.delimiter
            });
        if quoted {
            line.push('"');
            line.push_str(&text.replace('"', "\"\""));
            line.push('"');
        } else {
            line.push_str(text);
        }
    }
}
