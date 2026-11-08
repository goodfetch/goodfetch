//! The `row:` Features: every row of ADR 0037's matrix that is not
//! `excluded`, read from the matrix's tables (never copied), with its first
//! spec area and the milestone `features/rows.toml` gives it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};

use super::entry::{Carriers, Entry, unreadable};
use super::files::{self, Kind, OnlyProduct, RowsFile, Status};

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let rows: RowsFile = files::read_optional_toml(root, files::ROWS)
        .map_err(|error| unreadable(error, files::ROWS))?;
    let milestones: BTreeMap<&str, &str> = rows
        .row
        .iter()
        .map(|row| (row.id.as_str(), row.milestone.as_str()))
        .collect();
    let foxhound_only: BTreeSet<&str> = rows
        .row
        .iter()
        .filter(|row| row.product == Some(OnlyProduct::Foxhound))
        .map(|row| row.id.as_str())
        .collect();
    let path = root.join(files::MATRIX);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(matrix_rows(&text)
        .into_iter()
        .filter(|row| row.status != "excluded")
        .map(|row| Entry {
            kind: Kind::Row,
            milestone: milestones
                .get(row.id.as_str())
                .map(|&milestone| milestone.to_owned()),
            carriers: if foxhound_only.contains(row.id.as_str()) {
                Carriers::Foxhound
            } else {
                Carriers::Both
            },
            name: row.id,
            area: row.area,
            status: Status::Active,
        })
        .collect())
}

struct MatrixRow {
    id: String,
    status: String,
    area: Option<u32>,
}

/// The rows of every table whose header names an `ID` or `Pain` column, a
/// `Status` column and a spec area column.
fn matrix_rows(text: &str) -> Vec<MatrixRow> {
    let mut rows = Vec::new();
    let mut columns: Option<Columns> = None;
    for line in text.lines() {
        let Some(cells) = crate::markdown::cells(line) else {
            columns = None;
            continue;
        };
        let Some(header) = &columns else {
            columns = Columns::of(&cells);
            continue;
        };
        if cells
            .iter()
            .all(|cell| cell.chars().all(|c| matches!(c, '-' | ':')))
        {
            continue;
        }
        let cell = |index: usize| cells.get(index).copied().unwrap_or_default();
        let id = cell(header.id).trim_matches('`').to_owned();
        if id.is_empty() {
            continue;
        }
        rows.push(MatrixRow {
            id,
            status: cell(header.status).trim_matches('`').to_owned(),
            area: first_number(cell(header.area)),
        });
    }
    rows
}

struct Columns {
    id: usize,
    status: usize,
    area: usize,
}

impl Columns {
    fn of(cells: &[&str]) -> Option<Self> {
        let position = |names: &[&str]| cells.iter().position(|cell| names.contains(cell));
        Some(Self {
            id: position(&["ID", "Pain"])?,
            status: position(&["Status"])?,
            area: position(&["Spec area", "Spec areas"])?,
        })
    }
}

/// The number that opens a spec area cell, such as `3` in
/// `3 Engine runtime; 11 Client surfaces`.
fn first_number(cell: &str) -> Option<u32> {
    let digits: String = cell.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
