//! `drift.reference` (ADR 0140): each counted figure against a pinned
//! reference build, which `ci/perf-reference.toml` names: #1577's tuned build,
//! then each release tag. ADR 0071's comparison with the base sees one change
//! at a time, so steps under +5 % add up unseen; this one sees them. A figure
//! past `budget::DRIFT_PERCENT` over its reference fails whatever `GATING`
//! says and whatever a pull request accepts: only an ADR amendment the owner
//! approves passes it: the pin file's `accepted` list names each figure such
//! an amendment accepts, with the value it is held to instead. With no
//! revision pinned, it reports that it has not run.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
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
    /// The figures an amendment of ADR 0140 the owner approved accepts over
    /// the limit. A release's pin starts the list again.
    #[serde(default)]
    accepted: Vec<Accepted>,
}

/// A figure the owner accepted over `budget::DRIFT_PERCENT`, by a note of
/// ADR 0140: it is held to `up_to` instead, with the 0.5 % the register
/// gives a figure read under Valgrind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Accepted {
    /// The figure's identifier, `<subject> <figure>`, as a finding names it.
    figure: String,
    /// The value the figure is held to.
    up_to: u64,
    /// Why the figure grew.
    cause: String,
    /// The note of ADR 0140 that records the owner's decision.
    decision: String,
}

impl Accepted {
    /// Whether `head` is past `up_to`, with the register's 0.5 % for a
    /// figure read under Valgrind (Callgrind's and DHAT's) and none for a
    /// size-limit entry's or a resident-memory peak.
    fn passed_by(&self, figure: Figure, head: u64) -> bool {
        // In thousandths, as `xtask counts` holds its rows.
        let thousandths = |value: u64| u128::from(value) * 1_000;
        let limit = if figure.is_benchmark() {
            budget::with_valgrind_tolerance(thousandths(self.up_to))
        } else {
            thousandths(self.up_to)
        };
        thousandths(head) > limit
    }
}

fn read(root: &Path) -> Result<Pin> {
    let text = std::fs::read_to_string(root.join(PIN_FILE))
        .with_context(|| format!("reading {PIN_FILE}"))?;
    toml::from_str(&text).with_context(|| format!("parsing {PIN_FILE}"))
}

/// The revision `root`'s pin file names, when it names one. A missing file
/// is an error: it is the gate's switch, and losing it must not stop the gate.
pub(crate) fn pinned(root: &Path) -> Result<Option<String>> {
    Ok(read(root)?.revision)
}

/// The figures `root`'s pin file accepts over the limit. A missing file
/// accepts none, which can only hold the head tighter; `pinned` fails on it.
/// A figure accepted twice is an error: which value held it would depend on
/// the entries' order.
pub(crate) fn accepted(root: &Path) -> Result<Vec<Accepted>> {
    if !root.join(PIN_FILE).exists() {
        return Ok(Vec::new());
    }
    let accepted = read(root)?.accepted;
    let mut figures = BTreeSet::new();
    if let Some(twice) = accepted
        .iter()
        .find(|accepted| !figures.insert(accepted.figure.as_str()))
    {
        anyhow::bail!("{PIN_FILE} accepts {} twice", twice.figure);
    }
    Ok(accepted)
}

/// One head figure against the reference.
pub(crate) struct Held<'a> {
    subject: &'a str,
    figure: Figure,
    /// `None` when the figure is new since the reference.
    reference: Option<u64>,
    head: u64,
    /// The pin file's acceptance of this figure, when it has one.
    accepted: Option<&'a Accepted>,
}

impl Held<'_> {
    /// Over its limit: past `budget::DRIFT_PERCENT` over its reference and,
    /// when accepted, past its accepted value too. Within the limit, an
    /// accepted figure is held to it alone, as its acceptance is no longer
    /// needed.
    fn over(&self) -> bool {
        self.reference.is_some_and(|reference| {
            budget::drifted(reference, self.head)
                && self
                    .accepted
                    .is_none_or(|accepted| accepted.passed_by(self.figure, self.head))
        })
    }

    /// Accepted over the limit, yet back within it: its acceptance is no
    /// longer needed.
    fn acceptance_unneeded(&self) -> bool {
        self.accepted.is_some()
            && self
                .reference
                .is_some_and(|reference| !budget::drifted(reference, self.head))
    }
}

/// The check's result, once the reports are read.
pub(crate) enum Check<'a> {
    /// No reference build was measured: none is pinned.
    NotRunYet,
    Compared(Vec<Held<'a>>),
}

/// Holds `rows`' figures to `reference`'s, each figure `accepted` names to
/// its accepted value; `None` when none was measured.
pub(crate) fn check<'a>(
    rows: &'a [Row],
    reference: Option<Vec<Measured>>,
    accepted: &'a [Accepted],
) -> Check<'a> {
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
                accepted: accepted
                    .iter()
                    .find(|accepted| accepted.figure == row.measured.id()),
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
            None => (
                "—".to_owned(),
                "—".to_owned(),
                "new since it, not compared".to_owned(),
            ),
            Some(reference) => (
                table::value(held.figure, reference),
                multiple(reference, held.head).unwrap_or_else(|| "—".to_owned()),
                verdict(held),
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

/// A compared figure's verdict: over or within its limit, and for an
/// accepted figure, the value it is held to and why.
fn verdict(held: &Held<'_>) -> String {
    let over = if held.over() { "over" } else { "within" };
    match held.accepted {
        None => over.to_owned(),
        Some(_) if held.acceptance_unneeded() => {
            format!("{over}; its acceptance is no longer needed")
        }
        Some(accepted) => format!(
            "{over}: accepted up to {} ({}): {}",
            table::value(held.figure, accepted.up_to),
            accepted.decision,
            accepted.cause
        ),
    }
}

/// What fails the change: each figure over its limit, each acceptance of a
/// figure back within the limit, which outlived its cause, and a reference
/// that held none of the head's figures, which compared nothing.
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
    let mut findings: Vec<Finding> = held
        .iter()
        .filter(|held| held.over())
        .filter_map(|held| {
            let measured = measured(held)?;
            let over = match held.accepted {
                Some(accepted) => format!(
                    "over its accepted {} ({}; {ROW})",
                    table::value(held.figure, accepted.up_to),
                    accepted.decision
                ),
                None => format!("over {} ({ROW})", limit()),
            };
            Some(Finding::new(
                "drift-reference",
                id(held.subject, held.figure),
                format!(
                    "{measured}, {over}; no pull request accepts this: only an ADR amendment the owner approves does (ADR 0140)"
                ),
            ))
        })
        .collect();
    findings.extend(
        held.iter()
            .filter(|held| held.acceptance_unneeded())
            .filter_map(|held| {
                Some(Finding::new(
                    "drift-reference-acceptance-unneeded",
                    id(held.subject, held.figure),
                    format!(
                        "{}, within {} ({ROW}): remove its acceptance from {PIN_FILE}",
                        measured(held)?,
                        limit()
                    ),
                ))
            }),
    );
    findings
}

/// The figure against its reference, `1250 is 1.25 times its reference's
/// 1000`; `None` for a figure new since the reference.
fn measured(held: &Held<'_>) -> Option<String> {
    let reference = held.reference?;
    let value = |value| table::value(held.figure, value);
    let figure = value(held.head);
    Some(match multiple(reference, held.head) {
        Some(times) => format!(
            "{figure} is {times} times its reference's {}",
            value(reference)
        ),
        None => format!("{figure} where its reference measured 0"),
    })
}
