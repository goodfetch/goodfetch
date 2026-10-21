//! The bounds of a Limit's period (ADR 0041): a day, a week from Monday or
//! a month, from one local midnight to the next in the Limit's own IANA
//! time zone, never the host's. A day whose midnight the zone skips starts
//! at the first moment after the gap.

use goodfetch_protocol::run::{Period, PeriodLength};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};

/// One period: its first moment, and the first moment of the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodBounds {
    /// When it starts.
    pub start: Timestamp,
    /// When the next one starts.
    pub end: Timestamp,
}

/// The period of `period` that holds `now`.
///
/// # Errors
///
/// When this engine does not know the period's time zone.
pub fn bounds(period: &Period, now: Timestamp) -> Result<PeriodBounds, jiff::Error> {
    let zone = TimeZone::get(&period.time_zone)?;
    let today = now.to_zoned(zone.clone()).date();
    let (first, next) = match period.every {
        PeriodLength::Day => (today, today.tomorrow()?),
        PeriodLength::Week => {
            let monday =
                today.checked_sub(i64::from(today.weekday().to_monday_zero_offset()).days())?;
            (monday, monday.checked_add(1.week())?)
        }
        PeriodLength::Month => {
            let first = today.first_of_month();
            (first, first.checked_add(1.month())?)
        }
    };
    Ok(PeriodBounds {
        start: first.to_zoned(zone.clone())?.timestamp(),
        end: next.to_zoned(zone)?.timestamp(),
    })
}

#[cfg(test)]
mod tests;
