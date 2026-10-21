//! A Limit's period bounds through their interface (spec #213's Seam 4),
//! in the zones of test support's shared table (ADR 0073): offsets of
//! −3:30, +5:45 and +12:45, a 30-minute daylight saving shift, and the days
//! of a gap and of an overlap. Each expected bound was worked out by hand
//! from the zone's rules; none depends on the host's zone.
//!
//! Covers: story:3-68

use goodfetch_protocol::run::{Period, PeriodLength};
use goodfetch_test_support::{TimeZoneCase, time_zones};
use jiff::Timestamp;
use jiff::civil::Time;
use rstest::rstest;

use super::{PeriodBounds, bounds};

/// The shared table's case of the zone `name`, which every case here takes
/// its zone from.
fn shared(name: &str) -> TimeZoneCase {
    time_zones()
        .into_iter()
        .find(|zone| zone.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the shared table of zones"))
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn period(every: PeriodLength, zone: &TimeZoneCase) -> Period {
    Period {
        every,
        time_zone: zone.name.clone(),
    }
}

#[rstest]
#[case::a_day_in_utc(
    "UTC",
    PeriodLength::Day,
    "2026-03-10T15:00:00Z",
    "2026-03-10T00:00:00Z",
    "2026-03-11T00:00:00Z"
)]
#[case::a_day_at_minus_three_thirty(
    "America/St_Johns",
    PeriodLength::Day,
    "2026-01-15T02:00:00Z",
    "2026-01-14T03:30:00Z",
    "2026-01-15T03:30:00Z"
)]
#[case::a_day_at_plus_five_forty_five(
    "Asia/Kathmandu",
    PeriodLength::Day,
    "2026-01-15T20:00:00Z",
    "2026-01-15T18:15:00Z",
    "2026-01-16T18:15:00Z"
)]
#[case::a_month_at_plus_twelve_forty_five(
    "Pacific/Chatham",
    PeriodLength::Month,
    "2026-07-01T12:00:00Z",
    "2026-06-30T11:15:00Z",
    "2026-07-31T11:15:00Z"
)]
#[case::a_day_whose_clock_goes_back_thirty_minutes(
    "Australia/Lord_Howe",
    PeriodLength::Day,
    "2026-04-05T06:00:00Z",
    "2026-04-04T13:00:00Z",
    "2026-04-05T13:30:00Z"
)]
#[case::a_week_from_monday(
    "Europe/Paris",
    PeriodLength::Week,
    "2026-04-01T10:00:00Z",
    "2026-03-29T22:00:00Z",
    "2026-04-05T22:00:00Z"
)]
fn a_period_runs_from_one_local_midnight_to_the_next_in_its_own_zone(
    #[case] zone: &str,
    #[case] every: PeriodLength,
    #[case] now: &str,
    #[case] start: &str,
    #[case] end: &str,
) {
    let zone = shared(zone);

    let found = bounds(&period(every, &zone), at(now)).unwrap();

    assert_eq!(
        found,
        PeriodBounds {
            start: at(start),
            end: at(end)
        }
    );
}

#[rstest]
#[case::the_gap_day_lasts_twenty_three_hours(
    "Europe/Paris",
    "2026-03-28T23:00:00Z",
    "2026-03-29T22:00:00Z"
)]
fn a_day_whose_clock_skips_an_hour_is_shorter(
    #[case] zone: &str,
    #[case] start: &str,
    #[case] end: &str,
) {
    let zone = shared(zone);
    let gap = zone.dst_gap.unwrap();
    let noon = gap
        .to_datetime(Time::constant(12, 0, 0, 0))
        .in_tz(&zone.name)
        .unwrap()
        .timestamp();

    let found = bounds(&period(PeriodLength::Day, &zone), noon).unwrap();

    assert_eq!(
        found,
        PeriodBounds {
            start: at(start),
            end: at(end)
        }
    );
}

#[rstest]
#[case::the_overlap_day_lasts_twenty_five_hours(
    "America/New_York",
    "2026-11-01T04:00:00Z",
    "2026-11-02T05:00:00Z"
)]
fn a_day_whose_clock_repeats_an_hour_is_longer(
    #[case] zone: &str,
    #[case] start: &str,
    #[case] end: &str,
) {
    let zone = shared(zone);
    let overlap = zone.dst_overlap.unwrap();
    let noon = overlap
        .to_datetime(Time::constant(12, 0, 0, 0))
        .in_tz(&zone.name)
        .unwrap()
        .timestamp();

    let found = bounds(&period(PeriodLength::Day, &zone), noon).unwrap();

    assert_eq!(
        found,
        PeriodBounds {
            start: at(start),
            end: at(end)
        }
    );
}

#[test]
fn a_zone_this_engine_does_not_know_has_no_period() {
    let unknown = Period {
        every: PeriodLength::Day,
        time_zone: "Mars/Olympus_Mons".to_owned(),
    };

    assert!(bounds(&unknown, at("2026-03-10T15:00:00Z")).is_err());
}
