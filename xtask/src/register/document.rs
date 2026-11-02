//! The tables of TESTING.md's Quality register, and of any Markdown section
//! found by its heading, read by their columns' names.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

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

/// The rows of the table under `heading`, which holds every one of `columns`.
/// `None` when the section, the table or a column is missing, or it has no row.
pub(crate) fn table(text: &str, heading: &str, columns: &[&str]) -> Option<Vec<Row>> {
    let mut lines = text
        .lines()
        .skip_while(|line| line.trim() != heading)
        .skip(1)
        .take_while(|line| !line.starts_with('#'))
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'));
    let header = cells(lines.next()?);
    if !columns.iter().all(|column| header.contains(column)) {
        return None;
    }
    let rows: Vec<Row> = lines
        // The header's separator.
        .skip(1)
        .map(|line| {
            let values = cells(line);
            Row {
                first: values.first().map_or("", |first| *first).to_owned(),
                cells: header
                    .iter()
                    .zip(&values)
                    .map(|(column, value)| ((*column).to_owned(), (*value).to_owned()))
                    .collect(),
            }
        })
        .collect();
    (!rows.is_empty()).then_some(rows)
}

fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
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
    let mut lines = text
        .lines()
        .skip_while(|line| line.trim() != heading)
        .skip(1)
        .take_while(|line| !line.starts_with('#'))
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'));
    let position = cells(lines.next()?)
        .iter()
        .position(|cell| *cell == column)?;
    let row = lines.find(|line| {
        cells(line)
            .first()
            .is_some_and(|first| code_spans(first).first() == Some(&counter))
    })?;
    let mut pieces: Vec<String> = row.split('|').map(str::to_owned).collect();
    // The piece before the row's first `|` is empty.
    *pieces.get_mut(position + 1)? = format!(" {value} ");
    Some(text.replacen(row, &pieces.join("|"), 1))
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
        let section: String = text
            .lines()
            .skip_while(|line| line.trim() != "## Quality register")
            .skip(1)
            .take_while(|line| !line.starts_with("## "))
            .collect::<Vec<_>>()
            .join("\n");
        let mut enforced = table(
            &section,
            "### Enforced",
            &["Dimension", "Rule", "Checked by"],
        )?;
        // No row promoted yet leaves the counted work without a heading; one
        // under its heading that cannot be read is a missing table.
        if section.lines().any(|line| line.trim() == COUNTED_WORK) {
            enforced.extend(table(&section, COUNTED_WORK, &ROW_COLUMNS)?);
        }
        Some(Self {
            enforced,
            measured: table(&section, MEASURED, &ROW_COLUMNS)?,
            exceptions: table(&section, "### Exceptions", &["ID", "Rule", "Expires"])?,
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
