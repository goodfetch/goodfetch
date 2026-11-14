//! Each counted row's limit, read from `TESTING.md`'s Quality register, in the
//! Enforced section's table of counted work or the Measured table (both start
//! `| Row |`): its "Today" figure once measured, else its "Contract",
//! when that holds a plain number (`1.10 × the figure after #1577` holds none).
//! A row measured per kind holds a figure for each, as `click 6, fill 2`, and
//! each kind's limit goes by `<row>.<kind>`, as `C3.commands.click`. A row two
//! families count, each to a figure of its own, holds a figure for each, as
//! `B1 4, B4 3`, and [`limit`] reads each family's.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::markdown::Table;
use crate::perf;
use crate::register::document::{COUNTED_WORK, Document, FILE as TESTING, MEASURED, SECTION};

/// The first column of a table of rows.
const ROW_COLUMN: &str = "Row";

/// The thousandths a limit and a figure are compared in: every figure in the
/// register has at most three decimals.
const SCALE: u128 = 1_000;

/// A row's limit, in thousandths per unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Limit(u128);

impl Limit {
    /// The first number of `text`, as `≤ 1 (0 streaming)` or `5.077` hold it.
    fn first_number(text: &str) -> Option<Self> {
        let start = text.find(|c: char| c.is_ascii_digit())?;
        let number: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let (whole, fraction) = number.split_once('.').unwrap_or((&number, ""));
        if fraction.len() > 3 {
            return None;
        }
        let whole: u128 = whole.parse().ok()?;
        let fraction: u128 = format!("{fraction:0<3}").parse().ok()?;
        Some(Self(whole * SCALE + fraction))
    }

    /// Whether `count` over `units` passes this limit: exact, with no
    /// tolerance, as counted work has none.
    pub(super) fn exceeded_by(self, count: u64, units: u64) -> bool {
        u128::from(count) * SCALE > self.0 * u128::from(units)
    }

    /// This limit with the 0.5 % a figure read under Valgrind may vary by
    /// (the register's ratchet).
    pub(super) const fn with_valgrind_tolerance(self) -> Self {
        Self(perf::with_valgrind_tolerance(self.0))
    }

    /// `count` over `units`, as the limit is written: rounded up to
    /// thousandths, so a figure over its limit never prints as equal to it.
    pub(super) fn figure(count: u64, units: u64) -> Self {
        Self((u128::from(count) * SCALE).div_ceil(u128::from(units)))
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, fraction) = (self.0 / SCALE, self.0 % SCALE);
        if fraction == 0 {
            return write!(formatter, "{whole}");
        }
        let fraction = format!("{fraction:03}");
        write!(formatter, "{whole}.{}", fraction.trim_end_matches('0'))
    }
}

/// The register's counted rows, and each one's limit by its name.
pub(super) struct Register {
    pub(super) limits: BTreeMap<String, Limit>,
    pub(super) rows: Vec<Held>,
}

/// One row of a table of rows, as `xtask counts` holds it to its counters.
pub(super) struct Held {
    /// Its name in the register, `<path>.<work>`.
    pub(super) name: String,
    /// The families its Benchmark names: `B1, B2` names two.
    pub(super) families: Vec<String>,
    /// Whether the Enforced section's counted work holds it, not the
    /// Measured table.
    pub(super) enforced: bool,
    /// The names its benchmark prints it under: its own, or `<name>.<kind>`
    /// for each kind its Today holds a figure of.
    pub(super) counted_as: Vec<String>,
}

/// The register `TESTING.md` holds.
pub(super) fn read(root: &Path) -> Result<Register> {
    let path = root.join(TESTING);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text)
}

/// Every table of rows in the register: the Enforced section's counted work
/// and the Measured table.
fn parse(text: &str) -> Result<Register> {
    // Found by its heading line, the part `xtask affected` reads it as, so
    // prose quoting the heading is no register.
    let document =
        Document::find(text).with_context(|| format!("{TESTING} has no \"{SECTION}\" section"))?;
    let mut register = Register {
        limits: BTreeMap::new(),
        rows: Vec::new(),
    };
    for (heading, enforced) in [(COUNTED_WORK, true), (MEASURED, false)] {
        if let Some(table) = document.table(heading)
            && table.header.first() == Some(&ROW_COLUMN)
        {
            read_table(&table, enforced, &mut register)?;
        }
    }
    if register.limits.is_empty() {
        bail!("{TESTING}'s Quality register has no table of rows");
    }
    Ok(register)
}

/// Each row of one table of rows, and its limit, into `register`;
/// `enforced` when the table is the Enforced section's.
fn read_table(table: &Table<'_>, enforced: bool, register: &mut Register) -> Result<()> {
    let column = |name: &str| {
        table
            .column(name)
            .with_context(|| format!("the Quality register's table has no column \"{name}\""))
    };
    let (contract, today) = (column("Contract")?, column("Today")?);
    let benchmark = table.column("Benchmark");
    for line in &table.rows {
        let row = &line.cells;
        let (Some(name), Some(contract), Some(today)) =
            (row.first(), row.get(contract), row.get(today))
        else {
            bail!(
                "the Quality register's row \"{}\" is short of columns",
                line.line
            );
        };
        let name = name.trim_matches('`');
        let families = benchmark
            .and_then(|column| row.get(column))
            .map(|cell| {
                cell.split(',')
                    .map(|family| family.trim().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let mut held = Held {
            name: name.to_owned(),
            families,
            enforced,
            counted_as: vec![name.to_owned()],
        };
        if let Some(families) = per_family(today) {
            for (family, limit) in families {
                register.limits.insert(on_family(name, family), limit);
            }
            register.rows.push(held);
            continue;
        }
        if let Some(kinds) = per_kind(today) {
            held.counted_as.clear();
            for (kind, limit) in kinds {
                let counted = format!("{name}.{kind}");
                register.limits.insert(counted.clone(), limit);
                held.counted_as.push(counted);
            }
            register.rows.push(held);
            continue;
        }
        register.rows.push(held);
        let limit = measured(today).or_else(|| {
            (!contract.contains('×'))
                .then(|| Limit::first_number(contract))
                .flatten()
        });
        if let Some(limit) = limit {
            register.limits.insert(name.to_owned(), limit);
        }
    }
    Ok(())
}

/// Each kind's figure a "Today" cell holds before any `;`, as `click 6, fill
/// 2, press 4; human-like input not run yet`: none unless every part is a
/// lowercase kind and its figure.
fn per_kind(today: &str) -> Option<Vec<(&str, Limit)>> {
    let figures = today.split(';').next()?;
    figures
        .split(',')
        .map(|part| {
            let (kind, figure) = part.trim().split_once(' ')?;
            let named = !kind.is_empty()
                && kind.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                && figure.starts_with(|c: char| c.is_ascii_digit());
            named
                .then(|| Limit::first_number(figure))
                .flatten()
                .map(|limit| (kind, limit))
        })
        .collect()
}

/// Each family's figure a "Today" cell holds before any `;`, as `B1 4, B4
/// 3; until #2189`: none unless every part is a family, as ADR 0140 names
/// one, and its figure.
fn per_family(today: &str) -> Option<Vec<(&str, Limit)>> {
    let figures = today.split(';').next()?;
    figures
        .split(',')
        .map(|part| {
            let (family, figure) = part.trim().split_once(' ')?;
            let named = family.strip_prefix('B').is_some_and(|number| {
                !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
            }) && figure.starts_with(|c: char| c.is_ascii_digit());
            named
                .then(|| Limit::first_number(figure))
                .flatten()
                .map(|limit| (family, limit))
        })
        .collect()
}

/// The name `family`'s own figure of the row `name` goes by in
/// [`Register::limits`]: no row's name holds an `@`.
fn on_family(name: &str, family: &str) -> String {
    format!("{name}@{family}")
}

/// The limit `family`'s count of the row `name` is held to: the family's
/// own figure where the row's Today holds one for each family, else the
/// row's.
pub(super) fn limit<'a>(
    limits: &'a BTreeMap<String, Limit>,
    name: &str,
    family: &str,
) -> Option<&'a Limit> {
    limits
        .get(&on_family(name, family))
        .or_else(|| limits.get(name))
}

/// The figure a "Today" cell holds; `after #1583` and the like hold none.
fn measured(today: &str) -> Option<Limit> {
    today
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| Limit::first_number(today))
        .flatten()
}
