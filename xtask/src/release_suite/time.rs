//! The timestamps and months GitHub's interface gives, as UTC: included
//! minutes reset at the start of each month, and a hosted job is counted in
//! whole minutes, each partial minute rounded up (ADRs 0108 and 0135).

use std::fmt;

use anyhow::{Context as _, Result, bail};

/// A UTC timestamp, in seconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Timestamp(i64);

/// A calendar month, in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Month {
    year: i64,
    /// From 1 to 12.
    month: i64,
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn number(text: &str, what: &str, range: std::ops::RangeInclusive<i64>) -> Result<i64> {
    let value: i64 = text
        .parse()
        .with_context(|| format!("`{text}` is not a {what}"))?;
    if !range.contains(&value) {
        bail!("{value} is not a {what}");
    }
    Ok(value)
}

impl Timestamp {
    /// Parses `YYYY-MM-DDTHH:MM:SS`, with optional fractional seconds, then
    /// `Z` or `+00:00`, as GitHub writes its timestamps.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let parsed = (|| {
            let rest = text
                .strip_suffix('Z')
                .or_else(|| text.strip_suffix("+00:00"))
                .context("not in UTC")?;
            let (date, time) = rest.split_once('T').context("no `T`")?;
            let time = time.split('.').next().unwrap_or(time);
            let mut date = date.split('-');
            let mut time = time.split(':');
            let next = |parts: &mut std::str::Split<'_, char>, what, range| {
                number(parts.next().unwrap_or_default(), what, range)
            };
            let year = next(&mut date, "year", 1970..=9999)?;
            let month = next(&mut date, "month", 1..=12)?;
            let day = next(&mut date, "day", 1..=31)?;
            let hour = next(&mut time, "hour", 0..=23)?;
            let minute = next(&mut time, "minute", 0..=59)?;
            let second = next(&mut time, "second", 0..=60)?;
            Ok::<_, anyhow::Error>(Self(
                days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second,
            ))
        })();
        parsed.with_context(|| format!("`{text}` is not a UTC timestamp"))
    }

    /// The minutes from `self` to `later`, each partial minute rounded up.
    pub(crate) fn whole_minutes_until(self, later: Self) -> u32 {
        let seconds = u64::try_from(later.0 - self.0).unwrap_or(0);
        u32::try_from(seconds.div_ceil(60)).unwrap_or(u32::MAX)
    }
}

impl Month {
    /// The month of a UTC timestamp, from its `YYYY-MM` prefix.
    pub(crate) fn of(text: &str) -> Result<Self> {
        Timestamp::parse(text)?;
        let year = number(&text[..4], "year", 1970..=9999)?;
        let month = number(&text[5..7], "month", 1..=12)?;
        Ok(Self { year, month })
    }

    pub(crate) fn next(self) -> Self {
        if self.month == 12 {
            Self {
                year: self.year + 1,
                month: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
            }
        }
    }
}

impl fmt::Display for Month {
    /// The month's first day, `YYYY-MM-01`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:04}-{:02}-01", self.year, self.month)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_minute_counts_as_a_whole_one() {
        let started = Timestamp::parse("2026-10-31T23:59:30Z").unwrap();
        let completed = Timestamp::parse("2026-11-01T00:01:31Z").unwrap();

        assert_eq!(started.whole_minutes_until(completed), 3);
    }

    #[test]
    fn december_turns_into_january_of_the_next_year() {
        let month = Month::of("2026-12-31T23:00:00Z").unwrap();

        assert_eq!(month.next().to_string(), "2027-01-01");
    }

    #[test]
    fn a_timestamp_with_fractional_seconds_and_an_offset_parses() {
        let instant = Timestamp::parse("2026-10-18T09:00:10.123+00:00").unwrap();

        assert_eq!(instant, Timestamp::parse("2026-10-18T09:00:10Z").unwrap());
    }
}
