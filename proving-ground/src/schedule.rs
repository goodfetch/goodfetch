//! The `schedule` family on `schedule.test` (ADR 0025's content that changes
//! on a known schedule, for change tracking): a board whose revision, headline
//! and price are a pure function of the seed and the time step. The time step
//! is the Proving ground's own, which whoever runs the task sets (`PUT
//! /time-step`), never the wall clock, so the board at a step is the same
//! whenever it is read (spec #152, Determinism).

use hyper::Method;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

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
        (&Method::GET, "/continuous") => Reply::html(continuous_index()),
        (&Method::GET, "/continuous/fast") => scheduled_page(seed, "fast", step, request),
        (&Method::GET, "/continuous/slow") => scheduled_page(seed, "slow", step / 3, request),
        (&Method::GET, "/continuous/static") => scheduled_page(seed, "static", 0, request),
        _ => Reply::not_found(),
    }
}

fn continuous_index() -> String {
    "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Scheduled continuous site</title></head>\n<body>\n<main>\n<h1>Scheduled pages</h1>\n<ul>\n<li><a href=\"/continuous/fast\">Fast page</a></li>\n<li><a href=\"/continuous/slow\">Slow page</a></li>\n<li><a href=\"/continuous/static\">Static page</a></li>\n</ul>\n</main>\n</body>\n</html>\n".to_owned()
}

fn scheduled_page(seed: u64, slug: &str, revision: u64, request: &Request<'_>) -> Reply {
    let etag = format!("\"{slug}-r{revision}\"");
    if request
        .headers
        .get("if-none-match")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|given| given.trim() == etag)
    {
        return Reply {
            status: hyper::StatusCode::NOT_MODIFIED,
            content_type: "text/html; charset=utf-8",
            headers: vec![("etag", etag)],
            body: hyper::body::Bytes::new(),
        };
    }
    let notice = notice(seed, revision);
    let mut reply = Reply::html(format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{slug} revision {revision}</title></head>\n<body>\n<main>\n<article data-slug=\"{slug}\" data-revision=\"{revision}\">\n<h1>{}</h1>\n<p>Price: ${}</p>\n</article>\n</main>\n</body>\n</html>\n",
        notice.headline, notice.price
    ));
    reply.headers.push(("etag", etag));
    reply
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
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    vec![Task {
        host: task_host(1),
        entry: Entry {
            id: format!("{FAMILY}/1"),
            family: FAMILY,
            kind: Kind::Watch,
            start_url: format!("{}/board", origin(&task_host(1), port)),
            description: "Watch this page while the Proving ground's time step goes from 0 to \
                the last of the task's time steps, one step after another, and return one \
                Record per revision the page shows, with the fields `id` (the revision), \
                `headline` and `price`.",
            account: None,
            time_steps: Some(WATCHED_STEPS),
        },
        expected: Expected::records(
            (0..=revision_at(WATCHED_STEPS - 1))
                .map(|revision| record(seed, revision))
                .collect(),
        ),
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

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use hyper::body::Bytes;
    use hyper::header::HeaderValue;
    use hyper::{HeaderMap, Method, StatusCode};

    use super::*;

    fn get(path: &str, if_none_match: Option<&str>) -> Reply {
        let mut headers = HeaderMap::new();
        if let Some(etag) = if_none_match {
            headers.insert(
                "if-none-match",
                HeaderValue::from_str(etag).expect("invariant: valid header value"),
            );
        }
        let empty = Bytes::new();
        let request = Request {
            method: &Method::GET,
            path,
            query: "",
            cookies: "",
            authorization: "",
            headers: &headers,
            body: &empty,
            peer: IpAddr::V4(Ipv4Addr::LOCALHOST),
            tls: None,
        };
        respond(42, 0, &request)
    }

    fn get_at(step: u64, path: &str, if_none_match: Option<&str>) -> Reply {
        let mut headers = HeaderMap::new();
        if let Some(etag) = if_none_match {
            headers.insert(
                "if-none-match",
                HeaderValue::from_str(etag).expect("invariant: valid header value"),
            );
        }
        let empty = Bytes::new();
        let request = Request {
            method: &Method::GET,
            path,
            query: "",
            cookies: "",
            authorization: "",
            headers: &headers,
            body: &empty,
            peer: IpAddr::V4(Ipv4Addr::LOCALHOST),
            tls: None,
        };
        respond(42, step, &request)
    }

    #[test]
    fn continuous_index_links_all_scheduled_pages() {
        let reply = get("/continuous", None);
        assert_eq!(reply.status, StatusCode::OK);
        let body = std::str::from_utf8(&reply.body).expect("utf-8 html");
        assert!(body.contains("/continuous/fast"));
        assert!(body.contains("/continuous/slow"));
        assert!(body.contains("/continuous/static"));
    }

    #[test]
    fn scheduled_pages_advance_by_step_and_honour_etag() {
        let fast_0 = get_at(0, "/continuous/fast", None);
        let fast_1 = get_at(1, "/continuous/fast", None);
        assert_eq!(fast_0.status, StatusCode::OK);
        assert_eq!(fast_1.status, StatusCode::OK);
        assert_ne!(fast_0.body, fast_1.body);
        let etag_0 = fast_0
            .headers
            .iter()
            .find(|(name, _)| *name == "etag")
            .map(|(_, value)| value.as_str())
            .expect("etag header");
        assert_eq!(etag_0, "\"fast-r0\"");

        // Matching ETag at step 0 returns 304 Not Modified with empty body.
        let not_modified = get_at(0, "/continuous/fast", Some(etag_0));
        assert_eq!(not_modified.status, StatusCode::NOT_MODIFIED);
        assert!(not_modified.body.is_empty());

        // Stale ETag at step 1 returns 200 OK with updated content.
        let changed = get_at(1, "/continuous/fast", Some(etag_0));
        assert_eq!(changed.status, StatusCode::OK);
        assert_eq!(changed.body, fast_1.body);

        // Slow page stays identical at steps 0..3 and changes at step 3.
        let slow_0 = get_at(0, "/continuous/slow", None);
        let slow_2 = get_at(2, "/continuous/slow", None);
        let slow_3 = get_at(3, "/continuous/slow", None);
        assert_eq!(slow_0.body, slow_2.body);
        assert_ne!(slow_0.body, slow_3.body);

        // Static page never changes across steps.
        let static_0 = get_at(0, "/continuous/static", None);
        let static_9 = get_at(9, "/continuous/static", None);
        assert_eq!(static_0.body, static_9.body);
    }
}
