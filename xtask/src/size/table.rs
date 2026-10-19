//! The one table every mode writes, one row per Product and file, in
//! Markdown so a pull-request comment and the release notes reuse it.

use std::fmt::Write as _;

use super::budget::{Bytes, Limit};
use super::files::Measured;

/// One file's figures and verdict.
pub(crate) struct Row {
    pub(crate) file: Measured,
    /// `main`'s figure for the same Product, target and file, when known.
    pub(crate) main: Option<Bytes>,
    pub(crate) limit: Limit,
    pub(crate) verdict: Verdict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Within its limit, or held to none.
    Within,
    /// No figure of `main` to compare with: reported, never gated.
    New,
    /// No report of `main`'s tree at all: reported, never gated.
    NoBaseline,
    /// Past its increase threshold over `main`'s figure.
    Tripped,
    /// Over its ceiling, which no acceptance passes.
    OverCeiling,
    /// Past its increase threshold, with the pull request's acceptance.
    Accepted,
}

impl Row {
    /// The growth over `main`'s figure, in bytes, when `main` has one.
    pub(crate) fn change(&self) -> Option<i128> {
        self.main
            .map(|main| i128::from(self.file.bytes.0) - i128::from(main.0))
    }
}

pub(crate) fn render(rows: &[Row]) -> String {
    let mut table = String::from(
        "| Product | Target | File | Size | main | Change | Limit | Verdict |\n\
         |---|---|---|---:|---:|---:|---:|---|\n",
    );
    for row in rows {
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            row.file.product.name(),
            row.file.target.as_deref().unwrap_or("—"),
            row.file.name,
            row.file.bytes.megabytes(),
            row.main.map_or_else(|| "—".to_owned(), Bytes::megabytes),
            row.change().map_or_else(|| "—".to_owned(), change),
            row.limit,
            match row.verdict {
                Verdict::Within => "within",
                Verdict::New => "new",
                Verdict::NoBaseline => "no baseline",
                Verdict::Tripped => "over",
                Verdict::OverCeiling => "over its ceiling",
                Verdict::Accepted => "accepted",
            },
        );
    }
    table
}

/// A change in MB with two decimals, rounded away from zero, so the figure a
/// "Size increase" section copies is never under the measured one.
pub(crate) fn change(bytes: i128) -> String {
    let sign = if bytes < 0 { '-' } else { '+' };
    let hundredths = bytes.unsigned_abs().div_ceil(10_000);
    format!("{sign}{}.{:02} MB", hundredths / 100, hundredths % 100)
}
