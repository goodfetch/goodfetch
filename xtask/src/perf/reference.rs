//! `drift.reference` (ADR 0140): each counted figure against a pinned
//! reference build, which `ci/perf-reference.toml` names: #1577's tuned build,
//! then each release tag. ADR 0071's comparison with the base sees one change
//! at a time, so steps under +5 % add up unseen; this one sees them. A figure
//! past `budget::DRIFT_PERCENT` over its reference fails whatever `GATING`
//! says and whatever a pull request accepts: only an ADR amendment the owner
//! approves passes it. With no revision pinned, it reports that it has not
//! run.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

use super::budget;
use super::compare::Row;
use super::figure::{Figure, Measured, Sign, away_from_zero, hundredths, id};
use super::table;

/// The file that pins the reference build, relative to the repository.
pub(crate) const PIN_FILE: &str = "ci/perf-reference.toml";

/// The register's row, which every finding and the report name.
const ROW: &str = "drift.reference";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    /// The commit or tag of the reference build.
    revision: Option<String>,
}

/// The revision `root`'s pin file names, when it names one. A missing file
/// is an error: it is the gate's switch, and losing it must not stop the gate.
pub(crate) fn pinned(root: &Path) -> Result<Option<String>> {
    let text = std::fs::read_to_string(root.join(PIN_FILE))
        .with_context(|| format!("reading {PIN_FILE}"))?;
    let pin: Pin = toml::from_str(&text).with_context(|| format!("parsing {PIN_FILE}"))?;
    Ok(pin.revision)
}

/// One head figure against the reference.
pub(crate) struct Held<'a> {
    subject: &'a str,
    figure: Figure,
    /// `None` when the figure is new since the reference.
    reference: Option<u64>,
    head: u64,
}

impl Held<'_> {
    fn over(&self) -> bool {
        self.reference
            .is_some_and(|reference| budget::drifted(reference, self.head))
    }
}

/// The check's result, once the reports are read.
pub(crate) enum Check<'a> {
    /// No reference build was measured: none is pinned.
    NotRunYet,
    Compared(Vec<Held<'a>>),
}

/// Holds `rows`' figures to `reference`'s; `None` when none was measured.
pub(crate) fn check(rows: &[Row], reference: Option<Vec<Measured>>) -> Check<'_> {
    let Some(reference) = reference else {
        return Check::NotRunYet;
    };
    let reference: BTreeMap<String, u64> = reference
        .iter()
        .map(|measured| (measured.id(), measured.head))
        .collect();
    Check::Compared(
        rows.iter()
            .map(|row| Held {
                subject: &row.measured.subject,
                figure: row.measured.figure,
                reference: reference.get(&row.measured.id()).copied(),
                head: row.measured.head,
            })
            .collect(),
    )
}

/// The limit as a multiple, `1.10`.
fn limit() -> String {
    hundredths(i128::from(100 + budget::DRIFT_PERCENT), Sign::WhenNegative)
}

/// The figure as a multiple of its reference, rounded up so a figure over the
/// limit never shows as at it; `None` over a reference of 0.
fn multiple(reference: u64, head: u64) -> Option<String> {
    (reference != 0).then(|| {
        hundredths(
            away_from_zero(i128::from(head) * 100, i128::from(reference)),
            Sign::WhenNegative,
        )
    })
}

/// The check's section of the report.
pub(crate) fn render(check: &Check<'_>) -> String {
    let Check::Compared(held) = check else {
        return format!(
            "`{ROW}`: not run yet: `{PIN_FILE}` pins no reference build (#1577 pins its tuned build, ADR 0140).\n"
        );
    };
    let mut table = format!(
        "`{ROW}`: each figure at most {} times its pinned reference build's.\n\n\
         | Benchmark or entry | Figure | Reference | Head | Times | Verdict |\n\
         |---|---|---:|---:|---:|---|\n",
        limit()
    );
    for held in held {
        let (reference, times, verdict) = match held.reference {
            None => ("—".to_owned(), "—".to_owned(), "new since it, not compared"),
            Some(reference) => (
                table::value(held.figure, reference),
                multiple(reference, held.head).unwrap_or_else(|| "—".to_owned()),
                if held.over() { "over" } else { "within" },
            ),
        };
        let _ = writeln!(
            table,
            "| {} | {} | {reference} | {} | {times} | {verdict} |",
            held.subject,
            held.figure,
            table::value(held.figure, held.head),
        );
    }
    table
}

/// What fails the change: each figure over the limit, and a reference that
/// held none of the head's figures, which compared nothing.
pub(crate) fn findings(check: &Check<'_>) -> Vec<Finding> {
    let Check::Compared(held) = check else {
        return Vec::new();
    };
    if !held.is_empty() && held.iter().all(|held| held.reference.is_none()) {
        return vec![Finding::new(
            "drift-reference-not-measured",
            ROW,
            format!(
                "the pinned reference build measured none of the head's {} figures, so nothing was compared",
                held.len()
            ),
        )];
    }
    held.iter()
        .filter(|held| held.over())
        .filter_map(|held| {
            let reference = held.reference?;
            let value = |value| table::value(held.figure, value);
            let head = value(held.head);
            let measured = match multiple(reference, held.head) {
                Some(times) => format!("{head} is {times} times its reference's {}", value(reference)),
                None => format!("{head} where its reference measured 0"),
            };
            Some(Finding::new(
                "drift-reference",
                id(held.subject, held.figure),
                format!(
                    "{measured}, over {} ({ROW}); no pull request accepts this: only an ADR amendment the owner approves does (ADR 0140)",
                    limit()
                ),
            ))
        })
        .collect()
}
