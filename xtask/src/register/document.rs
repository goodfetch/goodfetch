//! TESTING.md's Quality register as every reader finds it (`xtask register`,
//! `xtask counts` and the floor guard): its section, found by its heading
//! line, the sections and tables under it by theirs, and the tables of any
//! Markdown section read by their columns' names.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::markdown::{self, Table};

/// The file holding the register, from the workspace's folder.
pub(crate) const FILE: &str = "TESTING.md";

/// The register's heading line in it.
pub(crate) const SECTION: &str = "## Quality register";

/// The heading of the table of enforced limits.
pub(crate) const ENFORCED: &str = "### Enforced";

/// The heading of the table of Exceptions.
pub(crate) const EXCEPTIONS: &str = "### Exceptions";

/// The register's own section of `testing`, a `TESTING.md`.
pub(crate) struct Document<'a> {
    lines: Vec<&'a str>,
}

impl<'a> Document<'a> {
    /// The register in `testing`, found by its heading line, so prose quoting
    /// the heading is no register; `None` when it has none.
    pub(crate) fn find(testing: &'a str) -> Option<Self> {
        markdown::section(testing.lines(), SECTION).map(|lines| Self { lines })
    }

    /// The lines under the register's `heading` (`### Floor`), up to the next
    /// heading of its level or above; `None` when it has no such heading.
    pub(crate) fn section(&self, heading: &str) -> Option<Vec<&'a str>> {
        markdown::section(self.lines.iter().copied(), heading)
    }

    /// The first table under the register's `heading`; `None` when it has
    /// no such heading or the heading no table.
    pub(crate) fn table(&self, heading: &str) -> Option<Table<'a>> {
        markdown::first_table(&self.section(heading)?)
    }
}

/// One row of a table: its first cell, and each cell by its column's name.
pub(crate) struct Row {
    first: String,
    cells: BTreeMap<String, String>,
}

impl Row {
    /// The first cell, with no code marks.
    pub(crate) fn name(&self) -> String {
        self.first.replace('`', "")
    }

    /// The row's name when it is a counter's, in a code span: `F1.parses`.
    pub(crate) fn counter(&self) -> Option<&str> {
        code_spans(&self.first).first().copied()
    }

    /// The cell of `column`; empty when the table lacks it.
    pub(crate) fn cell(&self, column: &str) -> &str {
        self.cells.get(column).map_or("", String::as_str)
    }

    /// What the row counts: its Metric, or an enforced row's Rule.
    pub(crate) fn metric(&self) -> &str {
        self.cells
            .get("Metric, per unit")
            .map_or_else(|| self.cell("Rule"), String::as_str)
    }

    /// The most the row allows: its Contract, or an enforced row's Rule.
    pub(crate) fn limit(&self) -> &str {
        self.cells
            .get("Contract")
            .map_or_else(|| self.cell("Rule"), String::as_str)
    }
}

/// The rows of the table under `heading` in `text`, which holds every one of
/// `columns`. `None` when the section, the table or a column is missing, or
/// it has no row.
pub(crate) fn table(text: &str, heading: &str, columns: &[&str]) -> Option<Vec<Row>> {
    rows(
        &markdown::first_table(&markdown::section(text.lines(), heading)?)?,
        columns,
    )
}

/// The rows of `table`, which holds every one of `columns`; `None` when a
/// column is missing or it has no row.
fn rows(table: &Table<'_>, columns: &[&str]) -> Option<Vec<Row>> {
    if !columns.iter().all(|column| table.header.contains(column)) {
        return None;
    }
    let rows: Vec<Row> = table
        .rows
        .iter()
        .map(|row| Row {
            first: row.cells.first().map_or("", |first| *first).to_owned(),
            cells: table
                .header
                .iter()
                .zip(&row.cells)
                .map(|(column, value)| ((*column).to_owned(), (*value).to_owned()))
                .collect(),
        })
        .collect();
    (!rows.is_empty()).then_some(rows)
}

/// `text` with the `column` cell of the row of `counter`, in the table under
/// `heading`, set to `value`; `None` when the table, the row or the column is
/// missing.
pub(crate) fn with_cell(
    text: &str,
    heading: &str,
    counter: &str,
    column: &str,
    value: &str,
) -> Option<String> {
    let table = markdown::first_table(&markdown::section(text.lines(), heading)?)?;
    let position = table.column(column)?;
    let row = table.rows.iter().find(|row| {
        row.cells
            .first()
            .is_some_and(|first| code_spans(first).first() == Some(&counter))
    })?;
    let mut pieces: Vec<String> = row.line.split('|').map(str::to_owned).collect();
    // The piece before the row's first `|` is empty.
    *pieces.get_mut(position + 1)? = format!(" {value} ");
    Some(text.replacen(row.line, &pieces.join("|"), 1))
}

/// The heading of the table of measured rows.
pub(crate) const MEASURED: &str = "### Measured, not yet enforced";

/// The heading, inside the Enforced section, of the table of counted rows
/// promoted there: the Measured table's columns, its limits gating.
pub(crate) const COUNTED_WORK: &str = "#### Counted work";

/// The columns a table of counted rows holds, measured or enforced.
const ROW_COLUMNS: [&str; 5] = ["Row", "Metric, per unit", "Contract", "Today", "Benchmark"];

/// The register's three tables.
pub(crate) struct Register {
    /// The Enforced table's rows, then its counted work's.
    pub(crate) enforced: Vec<Row>,
    pub(crate) measured: Vec<Row>,
    pub(crate) exceptions: Vec<Row>,
}

impl Register {
    /// The register in TESTING.md's `text`; `None` when a table is missing.
    pub(crate) fn read(text: &str) -> Option<Self> {
        let document = Document::find(text)?;
        let mut enforced = rows(
            &document.table(ENFORCED)?,
            &["Dimension", "Rule", "Checked by"],
        )?;
        // No row promoted yet leaves the counted work without a heading; one
        // under its heading that cannot be read is a missing table.
        if document.section(COUNTED_WORK).is_some() {
            enforced.extend(rows(&document.table(COUNTED_WORK)?, &ROW_COLUMNS)?);
        }
        Some(Self {
            enforced,
            measured: rows(&document.table(MEASURED)?, &ROW_COLUMNS)?,
            exceptions: rows(&document.table(EXCEPTIONS)?, &["ID", "Rule", "Expires"])?,
        })
    }

    /// Every row that gates or is measured.
    pub(crate) fn rows(&self) -> Vec<&Row> {
        self.enforced.iter().chain(&self.measured).collect()
    }
}

/// The text of each code span of `cell`.
pub(crate) fn code_spans(cell: &str) -> Vec<&str> {
    cell.split('`').skip(1).step_by(2).collect()
}

/// The cap a Contract states at its start (`0`, `≤ 1 (0 streaming)`, `≤
/// 1,000,000`); `None` for one with no plain number (`no cap`, `1.10 × …`).
pub(crate) fn cap(contract: &str) -> Option<u32> {
    static CAP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:≤ *)?(\d[\d,]*?),?(?:$|[ (])").expect("invariant: a valid pattern")
    });
    CAP.captures(contract)?[1].replace(',', "").parse().ok()
}

/// The figure a Today cell starts with; `None` for one not measured yet.
pub(crate) fn figure(today: &str) -> Option<f64> {
    static FIGURE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\d+(?:\.\d+)?(?:$| )").expect("invariant: a valid pattern"));
    FIGURE.find(today)?.as_str().trim().parse().ok()
}
