//! The one table a run writes to its job's summary page (ADR 0071): figure,
//! base, head, change and verdict, in Markdown.

use std::fmt::Write as _;

use super::compare::{Increase, Row, Verdict};
use super::figure::{Figure, Sign, hundredths};

pub(crate) fn render(rows: &[Row]) -> String {
    let mut table = String::from(
        "| Benchmark or entry | Figure | Base | Head | Change | Limit | Verdict |\n\
         |---|---|---:|---:|---:|---:|---|\n",
    );
    for row in rows {
        let figure = row.measured.figure;
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} | {} | {} | {} |",
            row.measured.subject,
            figure,
            row.measured
                .base
                .map_or_else(|| "—".to_owned(), |base| value(figure, base)),
            value(figure, row.measured.head),
            change(row).unwrap_or_else(|| "—".to_owned()),
            limit(row),
            verdict(row.verdict),
        );
    }
    table
}

/// A figure's value: a count, or kB for brotli bytes, rounded up, so a size
/// over its ceiling never shows as at it.
pub(crate) fn value(figure: Figure, value: u64) -> String {
    match figure {
        Figure::Brotli => kilobytes(i128::from(value.div_ceil(10)), false),
        Figure::Instructions
        | Figure::TotalAllocations
        | Figure::TotalBytes
        | Figure::GlobalMaximumBytes
        | Figure::WireBytes
        | Figure::ResidentMemory => value.to_string(),
    }
}

/// The change as a "Performance change" section lists it.
pub(crate) fn change(row: &Row) -> Option<String> {
    let listed = row.listed_change()?;
    Some(match row.limit.increase {
        Increase::Percent(_) => format!("{}%", hundredths(listed, Sign::Always)),
        Increase::Bytes(_) => kilobytes(listed, true),
    })
}

/// Tens of bytes as kB with two decimals, signed when asked.
fn kilobytes(tens: i128, signed: bool) -> String {
    let sign = if signed {
        Sign::Always
    } else {
        Sign::WhenNegative
    };
    format!("{} kB", hundredths(tens, sign))
}

fn limit(row: &Row) -> String {
    let increase = match row.limit.increase {
        Increase::Percent(percent) => format!("+{percent}%"),
        Increase::Bytes(bytes) => format!("+{} kB", bytes / 1_000),
    };
    match row.limit.ceiling {
        Some(ceiling) => format!("{increase}, ≤ {}", value(row.measured.figure, ceiling)),
        None => increase,
    }
}

fn verdict(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Within => "within",
        Verdict::Improved => "improved",
        Verdict::New => "new, not gated",
        Verdict::Tripped => "over",
        Verdict::Accepted => "accepted",
        Verdict::OverCeiling => "over its ceiling",
    }
}
