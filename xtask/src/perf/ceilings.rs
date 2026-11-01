//! The absolute ceilings (ADR 0134), checked beside the comparison with the
//! base: each ceiling of `budget` against the head's figures, its own or one
//! derived from several benchmarks. No "Performance change" section passes
//! a ceiling. A ceiling starts when the base or the head first measures one
//! of the benchmarks it reads, and from then on the head must have every
//! figure it reads, so renaming or removing a benchmark never drops one.

#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::num::NonZeroU64;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;

use super::budget::{self, Ceiling, Gating};
use super::compare::Row;
use super::figure::{self, Figure, Sign};
use super::table;

/// One ceiling's verdict on the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Within,
    Over,
    /// The base or the head measured a benchmark it reads, but the head has
    /// no figure for one.
    NotMeasured,
    /// Neither the base nor the head measured a benchmark it reads: the
    /// ceiling starts when one first runs.
    NotRun,
}

/// One ceiling, as the head meets it.
pub(crate) struct Check {
    /// The benchmark or entry it is named after.
    subject: &'static str,
    /// Its figure's name: a measured figure's, or a derived one.
    figure: &'static str,
    /// The head's figure, as the table shows it.
    head: Option<String>,
    /// The ceiling, as the table shows it.
    ceiling: String,
    verdict: Verdict,
}

impl Check {
    fn id(&self) -> String {
        figure::id(self.subject, self.figure)
    }
}

/// The head's figures, by benchmark or entry and figure.
type Figures<'a> = BTreeMap<(&'a str, Figure), u64>;

/// Each ceiling of `budget` against the head's figures in `rows`; `base`
/// holds the benchmarks and entries the base measured.
pub(crate) fn check(root: &Path, rows: &[Row], base: &BTreeSet<String>) -> Result<Vec<Check>> {
    let head: Figures<'_> = rows
        .iter()
        .map(|row| {
            (
                (row.measured.subject.as_str(), row.measured.figure),
                row.measured.head,
            )
        })
        .collect();
    let measured: BTreeSet<&str> = head
        .keys()
        .map(|(subject, _)| *subject)
        .chain(base.iter().map(String::as_str))
        .collect();
    budget::ceilings()
        .map(|ceiling| one(root, ceiling, &head, &measured))
        .collect()
}

fn one(
    root: &Path,
    ceiling: &Ceiling,
    head: &Figures<'_>,
    measured: &BTreeSet<&str>,
) -> Result<Check> {
    let started = subjects(ceiling)
        .iter()
        .any(|subject| measured.contains(subject));
    let (subject, figure, limit) = describe(ceiling);
    let (shown, verdict) = if started {
        match evaluate(root, ceiling, head)? {
            None => (None, Verdict::NotMeasured),
            Some((shown, over)) => (
                Some(shown),
                if over { Verdict::Over } else { Verdict::Within },
            ),
        }
    } else {
        (None, Verdict::NotRun)
    };
    Ok(Check {
        subject,
        figure,
        head: shown,
        ceiling: limit,
        verdict,
    })
}

/// The benchmarks or entries a ceiling reads.
fn subjects(ceiling: &Ceiling) -> Vec<&'static str> {
    match *ceiling {
        Ceiling::Figure { subject, .. } | Ceiling::PerBodyByte { subject, .. } => vec![subject],
        Ceiling::PerUrl { subject, empty, .. } => vec![subject, empty],
    }
}

/// A ceiling's subject, figure name and limit, as the table shows them.
fn describe(ceiling: &Ceiling) -> (&'static str, &'static str, String) {
    match *ceiling {
        Ceiling::Figure {
            subject,
            figure,
            max,
        } => (
            subject,
            figure.name(),
            format!("≤ {}", table::value(figure, max)),
        ),
        Ceiling::PerUrl {
            subject, max_bytes, ..
        } => (
            subject,
            "heap-bytes-per-url",
            format!("≤ {max_bytes} bytes"),
        ),
        Ceiling::PerBodyByte {
            subject,
            max_multiple,
            ..
        } => (subject, "bytes-per-body-byte", format!("≤ {max_multiple}×")),
    }
}

/// The head's figure, as the table shows it, and whether it is over the
/// ceiling, or `None` when the head lacks a figure the ceiling reads. The
/// comparison is exact; only the shown figure is rounded.
fn evaluate(root: &Path, ceiling: &Ceiling, head: &Figures<'_>) -> Result<Option<(String, bool)>> {
    Ok(match *ceiling {
        Ceiling::Figure {
            subject,
            figure,
            max,
        } => head
            .get(&(subject, figure))
            .map(|value| (table::value(figure, *value), *value > max)),
        Ceiling::PerUrl {
            subject,
            empty,
            urls,
            max_bytes,
        } => head
            .get(&(subject, Figure::GlobalMaximumBytes))
            .zip(head.get(&(empty, Figure::GlobalMaximumBytes)))
            .map(|(full, empty)| {
                let bytes = i128::from(*full) - i128::from(*empty);
                (
                    format!("{} bytes", hundredths(bytes, i128::from(urls))),
                    bytes > i128::from(max_bytes) * i128::from(urls),
                )
            }),
        Ceiling::PerBodyByte {
            subject,
            fixture,
            max_multiple,
        } => match head.get(&(subject, Figure::TotalBytes)) {
            None => None,
            Some(total) => {
                let body = body_bytes(&root.join(fixture))?;
                Some((
                    format!("{}×", hundredths(i128::from(*total), i128::from(body))),
                    u128::from(*total) > u128::from(max_multiple) * u128::from(body),
                ))
            }
        },
    })
}

/// What a crawl fixture's TOML file records of its pages.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Pages {
    body_bytes: NonZeroU64,
}

/// The total body bytes the fixture's file at `path` records.
fn body_bytes(path: &Path) -> Result<u64> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let pages: Pages = toml::from_str(&text).with_context(|| {
        format!(
            "{} records no `body-bytes` above zero for its ceiling",
            path.display()
        )
    })?;
    Ok(pages.body_bytes.get())
}

/// `numerator / denominator` with two decimals, rounded up, so a figure
/// over its ceiling never shows as at it.
fn hundredths(numerator: i128, denominator: i128) -> String {
    let scaled = numerator * 100;
    let quotient = scaled.div_euclid(denominator) + i128::from(scaled.rem_euclid(denominator) != 0);
    figure::hundredths(quotient, Sign::WhenNegative)
}

pub(crate) fn render(checks: &[Check]) -> String {
    let mut table = String::from(
        "| Benchmark or entry | Figure | Head | Ceiling | Verdict |\n\
         |---|---|---:|---:|---|\n",
    );
    for check in checks {
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} | {} |",
            check.subject,
            check.figure,
            check.head.as_deref().unwrap_or("—"),
            check.ceiling,
            match check.verdict {
                Verdict::Within => "within",
                Verdict::Over => "over its ceiling",
                Verdict::NotMeasured => "no head figure",
                Verdict::NotRun => "not run yet",
            },
        );
    }
    table.push_str(
        "\nA \"Performance change\" section never passes a ceiling; only the owner moves one (ADR 0134).\n",
    );
    table
}

/// Each figure over its ceiling, a finding of the budget that `gating`
/// decides.
pub(crate) fn over(checks: &[Check], gating: Gating) -> Vec<Finding> {
    checks
        .iter()
        .filter(|check| check.verdict == Verdict::Over)
        .map(|check| {
            Finding::new(
                "over-ceiling",
                check.id(),
                format!(
                    "is {} against its ceiling of {}; {}",
                    check.head.as_deref().unwrap_or_default(),
                    check.ceiling,
                    gating.remedy(
                        "no \"Performance change\" section passes a ceiling, and only the owner moves one"
                    )
                ),
            )
        })
        .collect()
}

/// Each ceiling whose figure the head lost: a broken measurement, which
/// fails whatever the gating.
pub(crate) fn not_measured(checks: &[Check]) -> Vec<Finding> {
    checks
        .iter()
        .filter(|check| check.verdict == Verdict::NotMeasured)
        .map(|check| Finding::new(
            "ceiling-not-measured",
            check.id(),
            "has a ceiling and was measured before, but the head has no figure for it; renaming or removing a benchmark never drops its ceiling, which only the owner moves",
        ))
        .collect()
}
