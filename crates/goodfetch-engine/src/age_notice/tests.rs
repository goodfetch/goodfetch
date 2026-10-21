//! The age notice (ADR 0030) in process: its rule, a pure function of the
//! release date and a reading of the `Clock`'s wall time (spec #213: rules
//! that depend on time need no clock), and its once-a-day record.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_test_support::TestEnv;
use jiff::civil::{Date, date};
use rstest::rstest;

use super::{AgeNotice, is_old};
use crate::home::Home;
use crate::product::{Channel, Identity};

/// Midday UTC of `day`, as the wall clock reads it.
fn at(day: Date) -> SystemTime {
    let midday = day
        .at(12, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp();
    SystemTime::from(midday)
}

/// More than three months past the release date, by the calendar, in UTC.
///
/// Covers: story:3-97
#[rstest]
#[case::the_release_day(date(2026, 6, 1), date(2026, 6, 1), false)]
#[case::three_months_to_the_day(date(2026, 6, 1), date(2026, 9, 1), false)]
#[case::the_day_after(date(2026, 6, 1), date(2026, 9, 2), true)]
#[case::the_end_of_a_long_month(date(2025, 11, 30), date(2026, 2, 28), false)]
#[case::past_the_end_of_a_short_month(date(2025, 11, 30), date(2026, 3, 1), true)]
#[case::a_year_on(date(2026, 6, 1), date(2027, 6, 1), true)]
#[case::a_clock_set_before_the_release(date(2026, 6, 1), date(2025, 1, 1), false)]
fn an_executable_is_old_more_than_three_months_past_its_release_date(
    #[case] released: Date,
    #[case] today: Date,
    #[case] old: bool,
) {
    assert_eq!(is_old(released, at(today)), old);
}

fn identity() -> Identity {
    Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    }
}

/// An old executable's notice comes once a day: its first command of a
/// day prints it, and the others that day print nothing.
///
/// Covers: story:3-97
#[test]
fn an_old_executable_prints_its_notice_once_a_day() {
    let env = TestEnv::new();
    let today = at(date(2026, 10, 21));
    let notice = AgeNotice::new(&Home::new(env.folder().join("notice home")), identity());

    let first = notice.due(date(2026, 6, 1), today).unwrap();
    let again = notice
        .due(date(2026, 6, 1), today + Duration::from_secs(60 * 60))
        .unwrap();
    let next_day = notice
        .due(date(2026, 6, 1), today + Duration::from_secs(24 * 60 * 60))
        .unwrap();

    assert_eq!(
        (first, again, next_day.is_some()),
        (
            Some("goodfetch-collie 1.4.0 was released on 2026-06-01, more than three months ago; newer versions carry security fixes and a current Chrome. Upgrade with `cargo install goodfetch-collie --locked --features executable`, or turn this notice off by setting `version.age_notice` to `false` in user configuration.".to_owned()),
            None,
            true
        )
    );
}

/// An executable within three months of its release prints nothing, and
/// records nothing.
///
/// Covers: story:3-97
#[test]
fn a_recent_executable_prints_no_notice() {
    let env = TestEnv::new();
    let home = Home::new(env.folder().join("notice home"));

    let due = AgeNotice::new(&home, identity())
        .due(date(2026, 6, 1), at(date(2026, 7, 1)))
        .unwrap();

    assert_eq!((due, home.root().exists()), (None, false));
}

/// `version.age_notice = false` in user configuration turns the notice
/// off (ADR 0030).
///
/// Covers: story:3-97, setting:version.age_notice
#[test]
fn the_age_notice_setting_turns_it_off() {
    let env = TestEnv::new();
    let home = Home::new(env.folder().join("notice home"));
    let major = home.root().join("1");
    std::fs::create_dir_all(&major).unwrap();
    std::fs::write(
        major.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.version]\nage_notice = false\n",
    )
    .unwrap();

    let due = AgeNotice::new(&home, identity())
        .due(date(2026, 6, 1), at(date(2026, 10, 21)))
        .unwrap();

    assert_eq!(due, None);
}
