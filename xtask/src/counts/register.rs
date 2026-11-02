//! Each counted row's limit, read from `TESTING.md`'s Quality register, in the
//! Enforced section's table of counted work or the Measured table (both start
//! `| Row |`): its "Today" figure once measured, else its "Contract",
//! when that holds a plain number (`1.10 × the figure after #1577` holds none).
//! A row measured per kind holds a figure for each, as `click 6, fill 2`, and
//! each kind's limit goes by `<row>.<kind>`, as `C3.commands.click`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// The file holding the register, from the workspace's folder.
const TESTING: &str = "TESTING.md";

/// The section and header the table is found by.
const SECTION: &str = "## Quality register";
const HEADER_START: &str = "| Row |";

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
        Self(self.0 * 1_005 / 1_000)
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

/// Each row's limit, by its name.
pub(super) fn limits(root: &Path) -> Result<BTreeMap<String, Limit>> {
    let path = root.join(TESTING);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text)
}

/// Every table of rows in the register: the Enforced section's counted work
/// and the Measured table.
fn parse(text: &str) -> Result<BTreeMap<String, Limit>> {
    let Some((_, section)) = text.split_once(SECTION) else {
        bail!("{TESTING} has no \"{SECTION}\" section");
    };
    let section = section.split("\n## ").next().unwrap_or(section);
    let mut limits = BTreeMap::new();
    let mut lines = section.lines().peekable();
    while lines.peek().is_some() {
        let mut table = lines
            .by_ref()
            .skip_while(|line| !line.starts_with(HEADER_START))
            .take_while(|line| line.starts_with('|'))
            .peekable();
        if table.peek().is_none() {
            break;
        }
        limits.extend(table_limits(table)?);
    }
    if limits.is_empty() {
        bail!("{TESTING}'s Quality register has no table of rows");
    }
    Ok(limits)
}

/// Each row's limit in one table of rows, its header first.
fn table_limits<'a>(mut lines: impl Iterator<Item = &'a str>) -> Result<BTreeMap<String, Limit>> {
    let Some(header) = lines.next() else {
        bail!("{TESTING}'s Quality register has no table of rows");
    };
    let header = cells(header);
    let column = |name: &str| {
        header
            .iter()
            .position(|cell| *cell == name)
            .with_context(|| format!("the Quality register's table has no column \"{name}\""))
    };
    let (contract, today) = (column("Contract")?, column("Today")?);
    let mut limits = BTreeMap::new();
    for line in lines.skip(1) {
        let row = cells(line);
        let (Some(name), Some(contract), Some(today)) =
            (row.first(), row.get(contract), row.get(today))
        else {
            bail!("the Quality register's row \"{line}\" is short of columns");
        };
        let name = name.trim_matches('`');
        if let Some(kinds) = per_kind(today) {
            for (kind, limit) in kinds {
                limits.insert(format!("{name}.{kind}"), limit);
            }
            continue;
        }
        let limit = measured(today).or_else(|| {
            (!contract.contains('×'))
                .then(|| Limit::first_number(contract))
                .flatten()
        });
        if let Some(limit) = limit {
            limits.insert(name.to_owned(), limit);
        }
    }
    Ok(limits)
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

/// The figure a "Today" cell holds; `after #1583` and the like hold none.
fn measured(today: &str) -> Option<Limit> {
    today
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| Limit::first_number(today))
        .flatten()
}

fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}
