//! The Quality register of `TESTING.md` as the floor guard reads it, and what
//! weakens it from the merge base to the head, by the register's Rules:
//! tightening is silent, loosening is loud, and a change whose direction cannot
//! be read is loud.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use anyhow::{Context as _, Result, anyhow, bail};
use regex::Regex;

use crate::check::Finding;
use crate::markdown::Table;
use crate::register::document::{COUNTED_WORK, Document, ENFORCED, EXCEPTIONS, MEASURED};

/// The sections whose every line is a rule of the Floor's, compared as text.
const TEXT_SECTIONS: [&str; 3] = ["Floor", "Recorded changes", "Rules"];
const GATE_FILES_BULLET: &str = "- **No edit to the gate's own files**";

/// A table's rows by their first cell, each row's cells by its column's name.
type Rows = BTreeMap<String, BTreeMap<String, String>>;

pub(crate) struct Register {
    /// Each text section's lines, with how often each stands there.
    text: BTreeMap<&'static str, BTreeMap<String, usize>>,
    enforced: Rows,
    /// The Enforced section's counted work: rows of the Measured table's
    /// shape, promoted.
    counted: Rows,
    measured: Rows,
    exceptions: Rows,
    gate_files: Vec<String>,
}

/// The first table of the section under `heading`, which must hold `columns`.
fn table(document: &Document<'_>, heading: &str, columns: &[&str]) -> Result<Rows> {
    if document.section(heading).is_none() {
        bail!("the register has no `{heading}` section");
    }
    let table = document
        .table(heading)
        .ok_or_else(|| anyhow!("`{heading}` holds no table"))?;
    rows(&table, heading, columns)
}

/// The table under the `heading` line of the register, which must hold
/// `columns`; none when the register has no such heading. `section` is the
/// section its errors name it by, the one the register holds it under.
fn subtable(
    document: &Document<'_>,
    section: &str,
    heading: &str,
    columns: &[&str],
) -> Result<Rows> {
    if document.section(heading).is_none() {
        return Ok(Rows::new());
    }
    let table = document
        .table(heading)
        .ok_or_else(|| anyhow!("`{section}` holds no table"))?;
    rows(&table, section, columns)
}

/// The rows of `table`, of the section under `section`, which must hold
/// `columns`. A row's name loses the backticks a Quality-change trailer
/// would not write.
fn rows(table: &Table<'_>, section: &str, columns: &[&str]) -> Result<Rows> {
    for column in columns {
        if table.column(column).is_none() {
            bail!("`{section}`'s table has no `{column}` column");
        }
    }
    let mut rows = Rows::new();
    for row in &table.rows {
        if row.cells.len() != table.header.len() {
            bail!(
                "a row of `{section}` has {} cells where its table has {}: {}",
                row.cells.len(),
                table.header.len(),
                row.line
            );
        }
        let name = row.cells[0].trim_matches('`').to_owned();
        rows.insert(
            name,
            table
                .header
                .iter()
                .zip(&row.cells)
                .map(|(column, cell)| ((*column).to_owned(), (*cell).to_owned()))
                .collect(),
        );
    }
    Ok(rows)
}

/// The paths the Floor's bullet on the gate's own files names, each a file or,
/// ending in `/`, a folder.
fn gate_files(floor: &[&str]) -> Vec<String> {
    floor
        .iter()
        .find(|line| line.starts_with(GATE_FILES_BULLET))
        .map(|line| {
            line.split('`')
                .skip(1)
                .step_by(2)
                .filter(|span| span.contains('/'))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a `TESTING.md`; `None` when it holds no register.
pub(crate) fn read(testing: &str) -> Result<Option<Register>> {
    let Some(document) = Document::find(testing) else {
        return Ok(None);
    };
    let mut text = BTreeMap::new();
    let mut gate_files = Vec::new();
    for section in TEXT_SECTIONS {
        let lines = document
            .section(&format!("### {section}"))
            .ok_or_else(|| anyhow!("the register has no `### {section}` section"))?;
        let mut counted: BTreeMap<String, usize> = BTreeMap::new();
        for line in lines.iter().filter(|line| !line.trim().is_empty()) {
            *counted.entry((*line).to_owned()).or_default() += 1;
        }
        text.insert(section, counted);
        if section == "Floor" {
            gate_files = self::gate_files(&lines);
        }
    }
    Ok(Some(Register {
        text,
        enforced: table(&document, ENFORCED, &["Rule", "Checked by", "Runs at"])
            .context("reading the Enforced table")?,
        counted: subtable(
            &document,
            ENFORCED,
            COUNTED_WORK,
            &["Contract", "Today", "Direction"],
        )
        .context("reading the Enforced section's counted work")?,
        measured: table(&document, MEASURED, &["Contract", "Today", "Direction"])
            .context("reading the Measured table")?,
        exceptions: table(&document, EXCEPTIONS, &["ID", "Rule", "Expires"])
            .context("reading the Exceptions table")?,
        gate_files,
    }))
}

impl Register {
    /// Whether the Floor's bullet names this path as one of the gate's files.
    pub(crate) fn is_gate_file(&self, path: &str) -> bool {
        self.gate_files.iter().any(|file| {
            if file.ends_with('/') {
                path.starts_with(file.as_str())
            } else {
                path == file
            }
        })
    }

    fn cell<'a>(row: &'a BTreeMap<String, String>, column: &str) -> &'a str {
        row.get(column).map_or("", String::as_str)
    }
}

/// A limit's leading number: `≤ 1 (0 streaming)` is 1, `no cap` none.
fn number(cell: &str) -> Option<f64> {
    static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[≤≥<>=~\s]*(\d+(?:\.\d+)?)")
            .expect("invariant: the guard's patterns are valid")
    });
    NUMBER.captures(cell)?[1].parse().ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// A maximum: raising it loosens it.
    MustNotGrow,
    /// A minimum: lowering it loosens it.
    MustNotFall,
}

/// Whether a limit's cell got looser. Text that holds no number on either side
/// says nothing about a limit when `text_is_free`, and is otherwise loud.
fn loosened(base: &str, head: &str, direction: Direction, text_is_free: bool) -> bool {
    if base == head {
        return false;
    }
    match (number(base), number(head)) {
        (Some(was), Some(now)) if (was - now).abs() > f64::EPSILON => match direction {
            Direction::MustNotGrow => now > was,
            Direction::MustNotFall => now < was,
        },
        // The same figure with other words around it.
        (Some(_), _) => true,
        (None, Some(_)) => false,
        (None, None) => !text_is_free,
    }
}

/// The names in an Exception's Rule cell.
fn names(cell: &str) -> BTreeSet<&str> {
    cell.split(',').map(str::trim).collect()
}

fn weakened(row: &str, why: &str) -> Finding {
    Finding::new(
        "row-weakened",
        row,
        format!(
            "{why}; the register's Rules ask the owner, and a `Quality-change` trailer naming it"
        ),
    )
}

/// The moves that weaken `head` against `base`.
pub(super) fn findings(base: &Register, head: &Register) -> Vec<Finding> {
    let mut findings = Vec::new();
    for section in TEXT_SECTIONS {
        let same = base.text.get(section) == head.text.get(section);
        if !same {
            findings.push(weakened(section, &format!("`### {section}` changed")));
        }
    }
    enforced(base, head, &mut findings);
    measured(base, head, &mut findings);
    exceptions(base, head, &mut findings);
    findings
}

fn enforced(base: &Register, head: &Register, findings: &mut Vec<Finding>) {
    for (name, was) in &base.enforced {
        let Some(now) = head.enforced.get(name) else {
            findings.push(weakened(name, "an enforced row was removed or demoted"));
            continue;
        };
        if loosened(
            Register::cell(was, "Rule"),
            Register::cell(now, "Rule"),
            Direction::MustNotGrow,
            false,
        ) {
            findings.push(weakened(name, "its Rule got looser"));
        }
        for column in ["Checked by", "Runs at"] {
            if Register::cell(was, column) != Register::cell(now, column) {
                findings.push(weakened(name, &format!("its {column} changed")));
            }
        }
    }
}

/// The measured rows, and the counted ones the Enforced section holds, each
/// held to its ratchet wherever it stands at the head.
fn measured(base: &Register, head: &Register, findings: &mut Vec<Finding>) {
    for (name, was) in &base.measured {
        // Promotion tightens: an enforced row names it, or the counted work
        // holds it with its figures.
        let Some(now) = head.measured.get(name).or_else(|| head.counted.get(name)) else {
            if !head.enforced.keys().any(|row| row == name) {
                findings.push(weakened(name, "a measured row was removed"));
            }
            continue;
        };
        ratchet(name, was, now, findings);
    }
    for (name, was) in &base.counted {
        let Some(now) = head.counted.get(name) else {
            findings.push(weakened(name, "an enforced row was removed or demoted"));
            continue;
        };
        ratchet(name, was, now, findings);
    }
}

/// What weakens a counted row's limits from `was` to `now`.
fn ratchet(
    name: &str,
    was: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
    findings: &mut Vec<Finding>,
) {
    let direction = if Register::cell(was, "Direction").contains("fall") {
        Direction::MustNotFall
    } else {
        Direction::MustNotGrow
    };
    if loosened(
        Register::cell(was, "Contract"),
        Register::cell(now, "Contract"),
        direction,
        false,
    ) {
        findings.push(weakened(name, "its Contract got looser"));
    }
    // "after #1583" becomes a figure, or waits on something else.
    if loosened(
        Register::cell(was, "Today"),
        Register::cell(now, "Today"),
        direction,
        true,
    ) {
        findings.push(weakened(name, "its Today figure got worse"));
    }
    if Register::cell(was, "Direction") != Register::cell(now, "Direction") {
        findings.push(weakened(name, "its Direction changed"));
    }
}

fn exceptions(base: &Register, head: &Register, findings: &mut Vec<Finding>) {
    for (id, now) in &head.exceptions {
        let Some(was) = base.exceptions.get(id) else {
            findings.push(Finding::new(
                "exception-added",
                id.as_str(),
                "a new Exception needs the owner, or a first measure's adoption, and a `Quality-change` trailer naming it",
            ));
            continue;
        };
        // ISO dates order as text.
        let extended = Register::cell(now, "Expires") > Register::cell(was, "Expires");
        let widened =
            !names(Register::cell(now, "Rule")).is_subset(&names(Register::cell(was, "Rule")));
        if extended || widened {
            findings.push(Finding::new(
                "exception-weakened",
                id.as_str(),
                "an Exception was extended; that needs the owner, and a `Quality-change` trailer naming it",
            ));
        }
    }
}
