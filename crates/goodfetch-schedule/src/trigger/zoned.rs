//! Connects `jiff::Zoned` to `croner`'s [`CronDateTime`] trait so cron
//! evaluation runs in any IANA time zone with exact DST gap and fold
//! resolution, without pulling a second date-time library (ADR 0036).

use std::cmp::Ordering;

use croner::errors::CronError;
use croner::time::{CivilDate, CivilDateTime, CivilTime, CronDateTime, Resolution, Weekday};
use jiff::civil::DateTime;
use jiff::tz::AmbiguousOffset;
use jiff::{SignedDuration, Zoned};

/// A [`Zoned`] instant implementing [`CronDateTime`] for `croner`.
#[derive(Clone, Debug)]
pub(super) struct ZonedInstant(pub(super) Zoned);

fn to_civil(datetime: DateTime) -> CivilDateTime {
    CivilDateTime::new(
        CivilDate::from_parts_unchecked(
            i32::from(datetime.year()),
            u32::try_from(datetime.month()).expect("invariant: jiff month is 1..=12"),
            u32::try_from(datetime.day()).expect("invariant: jiff day is 1..=31"),
        ),
        CivilTime::from_parts_unchecked(
            u32::try_from(datetime.hour()).expect("invariant: jiff hour is 0..=23"),
            u32::try_from(datetime.minute()).expect("invariant: jiff minute is 0..=59"),
            u32::try_from(datetime.second()).expect("invariant: jiff second is 0..=59"),
        ),
    )
}

fn to_jiff(civil: CivilDateTime) -> Result<DateTime, CronError> {
    let year = i16::try_from(civil.year()).map_err(|_| CronError::InvalidDate)?;
    let month = i8::try_from(civil.month()).map_err(|_| CronError::InvalidDate)?;
    let day = i8::try_from(civil.day()).map_err(|_| CronError::InvalidDate)?;
    let hour = i8::try_from(civil.hour()).map_err(|_| CronError::InvalidTime)?;
    let minute = i8::try_from(civil.minute()).map_err(|_| CronError::InvalidTime)?;
    let second = i8::try_from(civil.second()).map_err(|_| CronError::InvalidTime)?;
    DateTime::new(year, month, day, hour, minute, second, 0).map_err(|_| CronError::InvalidDate)
}

fn to_weekday(weekday: jiff::civil::Weekday) -> Weekday {
    let days = u32::try_from(weekday.to_sunday_zero_offset())
        .expect("invariant: jiff sunday-zero offset is 0..=6");
    Weekday::from_days_from_sunday(days)
}

impl CronDateTime for ZonedInstant {
    fn to_civil(&self) -> CivilDateTime {
        to_civil(self.0.datetime())
    }

    fn civil_weekday(&self) -> Weekday {
        to_weekday(self.0.weekday())
    }

    fn resolve_civil(&self, civil: CivilDateTime) -> Result<Resolution<Self>, CronError> {
        let datetime = to_jiff(civil)?;
        let ambiguous = self.0.time_zone().to_ambiguous_zoned(datetime);
        Ok(match ambiguous.offset() {
            AmbiguousOffset::Unambiguous { .. } => Resolution::Single(Self(
                ambiguous
                    .unambiguous()
                    .map_err(|_| CronError::InvalidTime)?,
            )),
            AmbiguousOffset::Fold { .. } => {
                let earlier = ambiguous
                    .clone()
                    .earlier()
                    .map_err(|_| CronError::InvalidTime)?;
                let later = ambiguous.later().map_err(|_| CronError::InvalidTime)?;
                Resolution::Ambiguous(Self(earlier), Self(later))
            }
            AmbiguousOffset::Gap { .. } => Resolution::Gap,
        })
    }

    fn checked_add_seconds(&self, seconds: i64) -> Option<Self> {
        self.0
            .checked_add(SignedDuration::from_secs(seconds))
            .ok()
            .map(Self)
    }

    fn cmp_instant(&self, other: &Self) -> Ordering {
        self.0.timestamp().cmp(&other.0.timestamp())
    }
}
