//! The `schedule` family on `schedule.test` (ADR 0025's content that changes
//! on a known schedule, for change tracking): a board whose revision, headline
//! and price are a pure function of the seed and the time step. The time step
//! is the Proving ground's own, which whoever runs the task sets (`PUT
//! /time-step`), never the wall clock, so the board at a step is the same
//! whenever it is read (spec #152, Determinism).

use hyper::Method;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task};

/// The host name the family is served on.
pub(crate) const HOST: &str = "schedule.test";

const FAMILY: &str = "schedule";

/// How many time steps the watch task spans, from step 0.
const WATCHED_STEPS: u64 = 6;

/// The generator places of the family's values: the family's own first, then
/// the revision's number and the value's.
const FAMILY_PLACE: u64 = 4;
const ADJECTIVE_PLACE: u64 = 1;
const NOUN_PLACE: u64 = 2;
const PRICE_PLACE: u64 = 3;

const ADJECTIVES: [&str; 8] = [
    "Spring",
    "Summer",
    "Autumn",
    "Winter",
    "Weekend",
    "Midweek",
    "Holiday",
    "Clearance",
];
const NOUNS: [&str; 8] = [
    "sale",
    "offer",
    "special",
    "deal",
    "bargain",
    "markdown",
    "discount",
    "promotion",
];

/// The board's revision at a time step: it changes at every step but those
/// that leave a remainder of 2 when divided by 3, so the board stays as it
/// was at steps 2, 5, 8 and so on.
fn revision_at(step: u64) -> u64 {
    step - step.saturating_add(1) / 3
}

/// One revision of the board.
struct Notice {
    id: String,
    headline: String,
    /// In dollars, with two decimals.
    price: String,
}

fn notice(seed: u64, revision: u64) -> Notice {
    let draw = |value, bound| {
        usize::try_from(generator::below(
            seed,
            &[FAMILY_PLACE, revision, value],
            bound,
        ))
        .expect("invariant: a word index is below 8")
    };
    let cents = 100 + generator::below(seed, &[FAMILY_PLACE, revision, PRICE_PLACE], 9_900);
    Notice {
        id: format!("R-{revision}"),
        headline: format!(
            "{} {}",
            ADJECTIVES[draw(ADJECTIVE_PLACE, 8)],
            NOUNS[draw(NOUN_PLACE, 8)]
        ),
        price: format!("{}.{:02}", cents / 100, cents % 100),
    }
}

/// Answers a request on the family's host name at the time step `step`.
pub(crate) fn respond(seed: u64, step: u64, request: &Request<'_>) -> Reply {
    match (request.method, request.path) {
        (&Method::GET, "/board") => Reply::html(board(&notice(seed, revision_at(step)))),
        _ => Reply::not_found(),
    }
}

fn board(notice: &Notice) -> String {
    let Notice {
        id,
        headline,
        price,
    } = notice;
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Notice board</title></head>\n<body>\n<main>\n<article class=\"notice\" data-revision=\"{id}\">\n<h1 class=\"headline\">{headline}</h1>\n<p>Price: <span class=\"price\" data-price=\"{price}\">${price}</span></p>\n<p class=\"revision\">Revision {id}</p>\n</article>\n</main>\n</body>\n</html>\n"
    )
}

/// The host name of the family's task `ordinal`, on which the task's pages
/// are served: `1.schedule.test` for `schedule/1`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    vec![Task {
        host: task_host(1),
        entry: Entry {
            id: format!("{FAMILY}/1"),
            family: FAMILY,
            kind: Kind::Watch,
            start_url: format!("http://{}:{port}/board", task_host(1)),
            description: "Watch this page while the Proving ground's time step goes from 0 to \
                the last of the task's time steps, one step after another, and return one \
                Record per revision the page shows, with the fields `id` (the revision), \
                `headline` and `price`.",
            account: None,
            time_steps: Some(WATCHED_STEPS),
        },
        expected: Expected {
            records: (0..=revision_at(WATCHED_STEPS - 1))
                .map(|revision| record(seed, revision))
                .collect(),
            effects: Vec::new(),
        },
    }]
}

/// The expected Record of a revision: its fields as the board shows them, the
/// price without its currency sign.
fn record(seed: u64, revision: u64) -> Record {
    let Notice {
        id,
        headline,
        price,
    } = notice(seed, revision);
    Record::from([("id", id), ("headline", headline), ("price", price)])
}
