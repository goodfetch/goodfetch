//! Each measured figure against its limits: its increase over the base's
//! figure, which a pull request may accept, and its ceiling, which nothing
//! passes. A figure with no base is reported, never gated; an improvement
//! is reported, never gated.

use super::budget;
use super::figure::{Figure, Measured, away_from_zero};

/// How far a figure may grow over the base's before it trips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Increase {
    Percent(u64),
    Bytes(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limit {
    pub(crate) increase: Increase,
    /// The largest value the figure may reach, whatever the pull request
    /// says: its ceiling of `budget::ceilings`, when one names the figure.
    pub(crate) ceiling: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Within,
    /// Below the base's figure: reported, never gated.
    Improved,
    /// No figure of the base to compare with: reported, never gated.
    New,
    /// Past its increase threshold, with no acceptance yet.
    Tripped,
    /// Past its increase threshold, listed in the pull request's accepted
    /// "Performance change" section.
    Accepted,
    /// Over its ceiling.
    OverCeiling,
}

pub(crate) struct Row {
    pub(crate) measured: Measured,
    pub(crate) limit: Limit,
    pub(crate) verdict: Verdict,
}

impl Row {
    /// The head's growth over the base's figure, when the base has one.
    pub(crate) fn change(&self) -> Option<i128> {
        self.measured
            .base
            .map(|base| i128::from(self.measured.head) - i128::from(base))
    }

    /// The change in the unit the threshold uses, rounded away from zero
    /// so the figure a "Performance change" section copies is never under
    /// the measured one: hundredths of a percent, or bytes in tens (a
    /// hundredth of a kB).
    pub(crate) fn listed_change(&self) -> Option<i128> {
        let change = self.change()?;
        let base = i128::from(self.measured.base?);
        match self.limit.increase {
            Increase::Percent(_) if base == 0 => None,
            Increase::Percent(_) => Some(away_from_zero(change * 10_000, base)),
            Increase::Bytes(_) => Some(away_from_zero(change, 10)),
        }
    }
}

/// The limits a figure is held to.
pub(crate) fn limit(measured: &Measured) -> Limit {
    let increase = match measured.figure {
        Figure::Brotli => Increase::Bytes(budget::BUNDLE_INCREASE_BYTES),
        Figure::Instructions
        | Figure::TotalAllocations
        | Figure::TotalBytes
        | Figure::GlobalMaximumBytes
        | Figure::WireBytes
        | Figure::ResidentMemory => Increase::Percent(budget::INCREASE_PERCENT),
    };
    Limit {
        increase,
        ceiling: budget::figure_ceiling(&measured.subject, measured.figure),
    }
}

pub(crate) fn row(measured: Measured) -> Row {
    let limit = limit(&measured);
    let verdict = verdict(&measured, limit);
    Row {
        measured,
        limit,
        verdict,
    }
}

fn verdict(measured: &Measured, limit: Limit) -> Verdict {
    if limit.ceiling.is_some_and(|ceiling| measured.head > ceiling) {
        return Verdict::OverCeiling;
    }
    let Some(base) = measured.base else {
        return Verdict::New;
    };
    let tripped = match limit.increase {
        Increase::Percent(percent) => {
            u128::from(measured.head) * 100 > u128::from(base) * u128::from(100 + percent)
        }
        Increase::Bytes(bytes) => measured.head > base.saturating_add(bytes),
    };
    if tripped {
        Verdict::Tripped
    } else if measured.head < base {
        Verdict::Improved
    } else {
        Verdict::Within
    }
}
