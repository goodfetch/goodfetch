//! The `incidents` family on `incidents.test`, the lab's HTTP incidents
//! ported: each task crawls a three-page shelf whose second page meets its
//! incident, so a tool that mishandles it can still look plausible. A 429
//! with `Retry-After` and a single 503 pass once retried; a truncated body,
//! a dropped connection, a redirect loop and a slow response never give the
//! second page in time, so the only right result is a reported failure.
//! Complete Records reported as a success after the slow page are data
//! delivered late; incomplete ones are a false success.

use std::time::Duration;

use super::shelf::{self, Shelf};
use crate::raw::{Answer, Cut, Framing, Page, Request};
use crate::task::{Expected, Kind, Task};

/// The host name the family is served on.
pub(super) const HOST: &str = "incidents.test";

const FAMILY: &str = "incidents";
const PLACE: u64 = 1620;

/// How long the slow page takes to answer.
const SLOW: Duration = Duration::from_secs(8);

/// Each task's incident, by ordinal.
#[derive(Clone, Copy)]
enum Incident {
    TooManyRequests,
    UnavailableOnce,
    Truncated,
    Dropped,
    RedirectLoop,
    Slow,
}

const INCIDENTS: [(Incident, &str); 6] = [
    (
        Incident::TooManyRequests,
        "Crawl this shelf from its first page through its `rel=\"next\"` links and return \
         every Record with its `id` and `price`. Its second page first answers `429` with \
         `Retry-After: 1`.",
    ),
    (
        Incident::UnavailableOnce,
        "Crawl this shelf from its first page through its `rel=\"next\"` links and return \
         every Record with its `id` and `price`. Its second page first answers `503`.",
    ),
    (
        Incident::Truncated,
        "Crawl this shelf from its first page through its `rel=\"next\"` links and return \
         every Record with its `id` and `price`. Its second page's body always ends before \
         its `Content-Length`; report the failure.",
    ),
    (
        Incident::Dropped,
        "Crawl this shelf from its first page through its `rel=\"next\"` links and return \
         every Record with its `id` and `price`. Its second page's connection always drops \
         mid-body, before the chunked body's last chunk; report the failure.",
    ),
    (
        Incident::RedirectLoop,
        "Crawl this shelf from its first page through its `rel=\"next\"` links and return \
         every Record with its `id` and `price`. Its second page always redirects to \
         itself; report the failure.",
    ),
    (
        Incident::Slow,
        "Crawl this shelf from its first page through its `rel=\"next\"` links and return \
         every Record with its `id` and `price`, within 5 seconds per page. Its second page \
         answers only after 8 seconds; report the failure.",
    ),
];

fn shelf(ordinal: u64) -> Shelf {
    Shelf {
        prefix: "H",
        place: PLACE,
        ordinal,
    }
}

/// The family's tasks.
pub(super) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    (1..)
        .zip(INCIDENTS)
        .map(|(ordinal, (incident, description))| {
            let refusal = !matches!(
                incident,
                Incident::TooManyRequests | Incident::UnavailableOnce
            );
            super::task(
                (FAMILY, HOST),
                ordinal,
                Kind::Crawl,
                "/catalog/1",
                description,
                Expected {
                    records: shelf(ordinal).records(seed),
                    refusal,
                    ..Expected::default()
                },
                port,
            )
        })
        .collect()
}

/// The answer to `request` on `host`, asked for its path `earlier` times
/// before.
pub(super) fn answer(seed: u64, host: &str, request: &Request, earlier: u64) -> Answer {
    let Some((ordinal, incident)) = super::ordinal(host).and_then(|ordinal| {
        let at = usize::try_from(ordinal.checked_sub(1)?).ok()?;
        INCIDENTS.get(at).map(|(incident, _)| (ordinal, *incident))
    }) else {
        return Answer::Page(Page::text(404, "not found"));
    };
    let Some(page) = shelf::page_number(request.path(), "") else {
        return Answer::Page(Page::text(404, "not found"));
    };
    let html = shelf(ordinal).plain_page(seed, page, "");
    if page != 2 {
        return Answer::Page(Page::html(200, html));
    }
    match incident {
        Incident::TooManyRequests if earlier == 0 => {
            Answer::Page(Page::text(429, "slow down").with("retry-after", "1"))
        }
        Incident::UnavailableOnce if earlier == 0 => Answer::Page(Page::text(503, "unavailable")),
        Incident::TooManyRequests | Incident::UnavailableOnce => {
            Answer::Page(Page::html(200, html))
        }
        Incident::Truncated => Answer::Cut {
            sent: shelf::before_next(&html),
            page: Page::html(200, html),
            cut: Cut::Connection,
        },
        Incident::Dropped => Answer::Cut {
            sent: shelf::before_next(&html),
            page: Page::html(200, html).framed(Framing::Chunked),
            cut: Cut::Connection,
        },
        Incident::RedirectLoop => Answer::Page(Page::text(302, "").with("location", "/catalog/2")),
        Incident::Slow => Answer::Later(SLOW, Box::new(Answer::Page(Page::html(200, html)))),
    }
}
