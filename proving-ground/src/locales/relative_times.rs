//! Notices whose times are relative to the `Date` each is served with, a
//! fixed one of the seed's, next to a Sunday the clocks of Paris change on
//! (ADR 0123). A count of days is taken from the date in Paris, so it spans
//! a day of 23 or 25 hours, and may name a time the clocks skip or show
//! twice; a count of hours is exact.

use std::time::SystemTime;

use time::{Date, Duration, Month, OffsetDateTime, UtcOffset};

use super::FAMILY_PLACE;
use crate::exchange::Reply;
use crate::generator;
use crate::task::Record;

/// The path of the index of the notices.
pub(super) const INDEX_PATH: &str = "/notices";

pub(super) const DESCRIPTION: &str = "Crawl the notices this index links, whose times are in \
    the time zone `Europe/Paris` and relative to the moment each notice was served, its \
    response's `Date` header. Return one Record per notice with the fields `id` (the article's \
    `data-id`), `day` (its day, as a date such as `2024-01-31`), `posted` and `seen` (its \
    times, as RFC 3339 with Paris's offset then, such as `2024-01-31T09:05:00+01:00`).";

/// The generator places of a notice's values, after the family's, this
/// submodule's and the notice's own.
const PLACE: u64 = 2;
const YEAR_PLACE: u64 = 1;
const MINUTE_PLACE: u64 = 2;

/// The years drawn from, each with the day of March and the day of October
/// that Paris's clocks change on, at 01:00 UTC: the month's last Sunday
/// (Directive 2000/84/EC).
const CHANGES: [(i32, u8, u8); 11] = [
    (2015, 29, 25),
    (2016, 27, 30),
    (2017, 26, 29),
    (2018, 25, 28),
    (2019, 31, 27),
    (2020, 29, 25),
    (2021, 28, 31),
    (2022, 27, 30),
    (2023, 26, 29),
    (2024, 31, 27),
    (2025, 30, 26),
];

/// The change of the clocks a notice stands next to.
#[derive(Clone, Copy)]
enum Change {
    /// 02:00 becomes 03:00 on a Sunday of March.
    Spring,
    /// 03:00 becomes 02:00 on a Sunday of October.
    Autumn,
}

/// A wall-clock time in Paris next to a change: its day counted from the
/// Sunday of the change, its hour, and the hours it is ahead of UTC.
#[derive(Clone, Copy)]
struct Wall {
    day: i64,
    hour: u8,
    ahead: i8,
}

const fn wall(day: i64, hour: u8, ahead: i8) -> Wall {
    Wall { day, hour, ahead }
}

/// A notice: when it is served, the phrases its times show, and what they
/// resolve to, worked out by hand.
struct Notice {
    id: &'static str,
    change: Change,
    served: Wall,
    posted: (&'static str, Wall),
    seen: (&'static str, Wall),
}

/// What every notice's day shows: the day before the one it is served on.
const DAY: &str = "yesterday";

static NOTICES: [Notice; 4] = [
    // Served on Monday in Paris and on Sunday in UTC: 48 hours before are on
    // Friday, and the exact 24 hours end in Saturday's winter time.
    Notice {
        id: "spring",
        change: Change::Spring,
        served: wall(1, 0, 2),
        posted: ("2 days ago", wall(-1, 0, 1)),
        seen: ("24 hours ago", wall(-1, 23, 1)),
    },
    // The time of day two days before is one the clocks skip, so it is moved
    // past the hour skipped.
    Notice {
        id: "spring-skipped",
        change: Change::Spring,
        served: wall(2, 2, 2),
        posted: ("2 days ago", wall(0, 3, 2)),
        seen: ("48 hours ago", wall(0, 1, 1)),
    },
    // Served on the Sunday itself, 24 hours after a time of that same day.
    Notice {
        id: "autumn",
        change: Change::Autumn,
        served: wall(0, 23, 1),
        posted: ("2 days ago", wall(-2, 23, 2)),
        seen: ("24 hours ago", wall(0, 0, 2)),
    },
    // The time of day one day before is one the clocks show twice, so it is
    // the first of the two, and 24 hours before is the second.
    Notice {
        id: "autumn-repeated",
        change: Change::Autumn,
        served: wall(1, 2, 1),
        posted: ("1 day ago", wall(0, 2, 2)),
        seen: ("24 hours ago", wall(0, 2, 1)),
    },
];

/// A notice's times for a seed.
struct Drawn {
    sunday: Date,
    minute: u8,
}

impl Drawn {
    fn at(&self, wall: Wall) -> OffsetDateTime {
        let offset = UtcOffset::from_hms(wall.ahead, 0, 0)
            .expect("invariant: Paris is at most two hours ahead of UTC");
        self.day(wall)
            .with_hms(wall.hour, self.minute, 0)
            .expect("invariant: an hour below 24 and a minute below 60")
            .assume_offset(offset)
    }

    fn day(&self, wall: Wall) -> Date {
        self.sunday + Duration::days(wall.day)
    }
}

/// The values a seed gives the notice `index` of [`NOTICES`].
fn drawn(seed: u64, index: usize) -> Drawn {
    let below =
        |value, bound| generator::below(seed, &[FAMILY_PLACE, PLACE, index as u64, value], bound);
    let year = usize::try_from(below(YEAR_PLACE, CHANGES.len() as u64))
        .expect("invariant: an index of the years");
    let (year, march, october) = CHANGES[year];
    let (month, day) = match NOTICES[index].change {
        Change::Spring => (Month::March, march),
        Change::Autumn => (Month::October, october),
    };
    Drawn {
        sunday: Date::from_calendar_date(year, month, day)
            .expect("invariant: every day of the table is in its month"),
        minute: u8::try_from(below(MINUTE_PLACE, 60)).expect("invariant: a minute is below 60"),
    }
}

/// A date as `2024-01-31`.
fn date(date: Date) -> String {
    let (year, month, day) = date.to_calendar_date();
    format!("{year}-{:02}-{day:02}", u8::from(month))
}

/// A time as RFC 3339 with its own offset.
fn time(time: OffsetDateTime) -> String {
    let (hour, minute, second) = time.to_hms();
    format!(
        "{}T{hour:02}:{minute:02}:{second:02}+{:02}:00",
        date(time.date()),
        time.offset().whole_hours()
    )
}

impl Notice {
    fn path(&self) -> String {
        format!("{INDEX_PATH}/{}", self.id)
    }

    fn record(&self, drawn: &Drawn) -> Record {
        Record::from([
            ("id", self.id.to_owned()),
            ("day", date(drawn.day(self.served) - Duration::DAY)),
            ("posted", time(drawn.at(self.posted.1))),
            ("seen", time(drawn.at(self.seen.1))),
        ])
    }

    fn reply(&self, drawn: &Drawn) -> Reply {
        let id = self.id;
        let (posted, seen) = (self.posted.0, self.seen.0);
        let mut reply = Reply::html(format!(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{id}</title></head>\n<body>\n<main>\n<article data-id=\"{id}\">\n<h1>{id}</h1>\n<dl>\n<dt>Day</dt><dd class=\"day\">{DAY}</dd>\n<dt>Posted</dt><dd class=\"posted\">{posted}</dd>\n<dt>Seen</dt><dd class=\"seen\">{seen}</dd>\n</dl>\n<p>All times are Paris time.</p>\n</article>\n</main>\n</body>\n</html>\n"
        ));
        reply.headers.push((
            "date",
            httpdate::fmt_http_date(SystemTime::from(drawn.at(self.served))),
        ));
        reply
    }
}

/// Answers a request for the index or one of its notices.
pub(super) fn respond(seed: u64, path: &str) -> Option<Reply> {
    if path == INDEX_PATH {
        return Some(super::index("Notices", NOTICES.iter().map(Notice::path)));
    }
    NOTICES
        .iter()
        .enumerate()
        .find(|(_, notice)| notice.path() == path)
        .map(|(index, notice)| notice.reply(&drawn(seed, index)))
}

/// The expected Record of each notice.
pub(super) fn records(seed: u64) -> Vec<Record> {
    NOTICES
        .iter()
        .enumerate()
        .map(|(index, notice)| notice.record(&drawn(seed, index)))
        .collect()
}
