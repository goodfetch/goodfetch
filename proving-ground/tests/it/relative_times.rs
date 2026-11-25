//! The `locales` family's notices, whose times are relative to the `Date`
//! each is served with, on either side of a change of the clocks in Paris.
//!
//! The known-correct results come from reading each notice over plain HTTP
//! and resolving its phrases here: an exact duration is subtracted from the
//! instant, a count of days from the date in Paris (ADR 0123).
//! Covers: story:2-33

use std::time::UNIX_EPOCH;

use rstest::rstest;
use serde_json::{Value, json};
use time::{Date, Month, Weekday};

use crate::proving_ground::{Answer, ProvingGround, submission};
use crate::reading::{every, first, origin, start_url, task};

const SEED: u64 = 1540;
const TASK: &str = "locales/2";

const HOUR: i64 = 3_600;
const DAY: i64 = 24 * HOUR;

/// Seconds since 1970, shown as a calendar day and a second of it.
fn civil(seconds: i64) -> (Date, i64) {
    let epoch = Date::from_calendar_date(1970, Month::January, 1).expect("a date");
    let days = i32::try_from(seconds.div_euclid(DAY)).expect("a day");
    let date = Date::from_julian_day(epoch.to_julian_day() + days).expect("a date");
    (date, seconds.rem_euclid(DAY))
}

fn seconds(date: Date, second: i64) -> i64 {
    let epoch = Date::from_calendar_date(1970, Month::January, 1).expect("a date");
    i64::from(date.to_julian_day() - epoch.to_julian_day()) * DAY + second
}

/// The instant the clocks change in the European Union: 01:00 UTC on the last
/// Sunday of the month (Directive 2000/84/EC).
fn change(year: i32, month: Month) -> i64 {
    let mut date = Date::from_calendar_date(year, month, 31).expect("a date");
    while date.weekday() != Weekday::Sunday {
        date = date.previous_day().expect("a date");
    }
    seconds(date, HOUR)
}

/// Paris's offset from UTC at an instant.
fn offset(instant: i64) -> i64 {
    let year = civil(instant).0.year();
    let summer = change(year, Month::March)..change(year, Month::October);
    if summer.contains(&instant) {
        2 * HOUR
    } else {
        HOUR
    }
}

fn paris(instant: i64) -> (Date, i64) {
    civil(instant + offset(instant))
}

/// The instant of a wall-clock time in Paris: the later reading of a time
/// the clocks skip, the earlier of one they show twice.
fn instant(date: Date, second: i64) -> i64 {
    let wall = seconds(date, second);
    let readings = [2 * HOUR, HOUR].map(|offset| wall - offset);
    readings
        .into_iter()
        .find(|reading| paris(*reading) == (date, second))
        .unwrap_or(readings[1])
}

fn printed(instant: i64) -> String {
    let (date, second) = paris(instant);
    format!(
        "{}T{:02}:{:02}:{:02}+0{}:00",
        day(date),
        second / HOUR,
        second % HOUR / 60,
        second % 60,
        offset(instant) / HOUR
    )
}

fn day(date: Date) -> String {
    format!(
        "{}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn days_before(date: Date, days: i32) -> Date {
    Date::from_julian_day(date.to_julian_day() - days).expect("a date")
}

/// How a program resolves a notice: from the instant it reads the notice's
/// times as relative to, the day "yesterday" is and the instant a count of
/// days ago is.
struct Reading {
    anchor: fn(&Answer) -> i64,
    yesterday: fn(i64) -> Date,
    days_ago: fn(i64, i32) -> i64,
}

fn served(page: &Answer) -> i64 {
    let date = page.header("date").expect("a `Date` header");
    let time = httpdate::parse_http_date(date).expect("an HTTP date");
    let since = time.duration_since(UNIX_EPOCH).expect("after 1970");
    i64::try_from(since.as_secs()).expect("a time")
}

fn right() -> Reading {
    Reading {
        anchor: served,
        yesterday: |anchor| days_before(paris(anchor).0, 1),
        days_ago: |anchor, days| {
            let (date, second) = paris(anchor);
            instant(days_before(date, days), second)
        },
    }
}

/// Counts every day as 24 hours.
fn whole_days() -> Reading {
    Reading {
        days_ago: |anchor, days| anchor - i64::from(days) * DAY,
        ..right()
    }
}

/// Takes yesterday in UTC.
fn utc_days() -> Reading {
    Reading {
        yesterday: |anchor| days_before(civil(anchor).0, 1),
        ..right()
    }
}

/// Reads the skipped time before the change and the repeated one after it.
fn other_side() -> Reading {
    Reading {
        days_ago: |anchor, days| {
            let (date, second) = paris(anchor);
            let wall = seconds(days_before(date, days), second);
            let readings = [HOUR, 2 * HOUR].map(|offset| wall - offset);
            readings
                .into_iter()
                .find(|reading| paris(*reading) == (days_before(date, days), second))
                .unwrap_or(readings[1])
        },
        ..right()
    }
}

/// Reads the times as relative to another clock, such as the machine's own.
fn another_clock() -> Reading {
    Reading {
        anchor: |_| 2_000_000_000,
        ..right()
    }
}

fn notices(proving_ground: &ProvingGround) -> Vec<Answer> {
    let task = task(proving_ground, TASK);
    let index = proving_ground.get(start_url(&task));
    assert_eq!(index.status, 200, "{}", index.body);
    every(r#"<a href="([^"]+)""#, &index.body)
        .iter()
        .map(|path| proving_ground.get(&format!("{}{path}", origin(start_url(&task)))))
        .collect()
}

fn read(proving_ground: &ProvingGround, reading: &Reading) -> Vec<Value> {
    notices(proving_ground)
        .iter()
        .map(|page| {
            let shown = |class| first(&format!(r#"class="{class}">([^<]+)<"#), &page.body);
            let count = |class| -> i32 {
                first(r"^(\d+) (?:hours|days?) ago$", &shown(class))
                    .parse()
                    .expect("a count")
            };
            let anchor = (reading.anchor)(page);
            assert_eq!(shown("day"), "yesterday");
            json!({
                "id": first(r#"data-id="([^"]+)""#, &page.body),
                "day": day((reading.yesterday)(anchor)),
                "posted": printed((reading.days_ago)(anchor, count("posted"))),
                "seen": printed(anchor - i64::from(count("seen")) * HOUR),
            })
        })
        .collect()
}

#[rstest]
#[case::one_seed(SEED)]
#[case::another_seed(SEED + 1)]
#[case::a_third_seed(SEED + 2)]
fn times_resolved_at_the_date_served_are_verified(#[case] seed: u64) {
    let proving_ground = ProvingGround::start(seed);
    let records = read(&proving_ground, &right());

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer} {records:?}");
}

#[rstest]
#[case::days_of_24_hours(whole_days())]
#[case::days_in_utc(utc_days())]
#[case::the_other_side_of_the_change(other_side())]
#[case::another_clock(another_clock())]
fn times_resolved_otherwise_are_refuted(#[case] reading: Reading) {
    let proving_ground = ProvingGround::start(SEED);
    let records = read(&proving_ground, &reading);

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn a_notice_carries_one_date_the_same_whenever_it_is_read() {
    let dates = |proving_ground: &ProvingGround| -> Vec<i64> {
        notices(proving_ground).iter().map(served).collect()
    };
    let first = ProvingGround::start(SEED);

    assert_eq!(dates(&first), dates(&first));
    assert_eq!(dates(&first), dates(&ProvingGround::start(SEED)));
    assert_ne!(dates(&first), dates(&ProvingGround::start(SEED + 1)));
    for page in notices(&first) {
        assert_eq!(page.every_header("date").len(), 1);
    }
}

#[test]
fn the_notices_stand_on_both_sides_of_both_changes() {
    let proving_ground = ProvingGround::start(SEED);
    let records = read(&proving_ground, &right());

    let offsets: Vec<String> = records
        .iter()
        .map(|record| {
            let posted = record["posted"].as_str().expect("a time");
            posted[posted.len() - 6..].to_owned()
        })
        .collect();

    assert_eq!(offsets, ["+01:00", "+02:00", "+02:00", "+02:00"]);
}
