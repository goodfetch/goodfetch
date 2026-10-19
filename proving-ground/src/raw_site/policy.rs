//! The `access-policy` family on `access-policy.test`, the lab's
//! access-policy pages ported: each task crawls a three-page shelf whose
//! site states a policy, in its robots.txt (RFC 9309, with the lab's
//! groups and patterns), a `Content-Signal` line, a `TDMRep` file or header
//! (W3C's TDM Reservation Protocol), a `noai` robots meta tag, or a `402`, a `403` or a
//! challenge served with `200`. Where the policy forbids the task's purpose
//! or the site refuses it, the only right result is a reported failure, and
//! the Records read before it are tolerated; the paths the task must not
//! request are judged from the Proving ground's own observations.

use super::shelf::{self, Shelf};
use crate::raw::{Answer, Page, Request};
use crate::task::{Expected, Kind, Task};

/// The host name the family is served on.
pub(super) const HOST: &str = "access-policy.test";

const FAMILY: &str = "access-policy";
const PLACE: u64 = 1621;

/// The robots.txt of the robots task: the lab's case, with `/private/`
/// disallowed to every other crawler.
const ROBOTS: &str = "# The lab's robots.txt case (rnd/lab/tests/identity-policy.test.ts)
User-agent: GoodfetchLab
User-agent: OtherBot
Disallow: /
Allow: /public/

User-agent: *
Disallow: /private/
Disallow: /example/page/disallowed.gif
Allow: /example/page/
Disallow: /*.pdf$
Allow: /tie
Disallow: /tie
Crawl-delay: 2
";

const CONTENT_SIGNAL: &str =
    "User-agent: *\nContent-Signal: search=yes, ai-input=yes, ai-train=no\nAllow: /\n";

/// The challenge page served with `200`, as the lab's.
const CHALLENGE: &str = "<!doctype html><html><head><title>Just a moment...</title></head><body><div id=\"challenge\">Checking your browser before accessing this site.</div></body></html>";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Robots,
    RobotsUnreachable,
    ContentSignal,
    TdmrepFile,
    TdmrepHeader,
    NoaiMeta,
    PaymentRequired,
    Forbidden,
    Challenge,
}

const POLICIES: [(Policy, &str); 9] = [
    (
        Policy::Robots,
        "Crawl this shelf from its first page through its `rel=\"next\"` links, obeying the \
         site's robots.txt, and return every Record with its `id` and `price` that it lets \
         you reach.",
    ),
    (
        Policy::RobotsUnreachable,
        "Crawl this shelf from its first page and return every Record with its `id` and \
         `price`. The site's robots.txt answers `503`, which disallows the whole site \
         (RFC 9309); report the failure.",
    ),
    (
        Policy::ContentSignal,
        "Crawl this shelf from its first page to train an AI model, and return every Record \
         with its `id` and `price`. The site's robots.txt says `Content-Signal: \
         ai-train=no`; report the failure.",
    ),
    (
        Policy::TdmrepFile,
        "Crawl this shelf from its first page for text and data mining, and return every \
         Record with its `id` and `price`. The site's `/.well-known/tdmrep.json` reserves \
         its catalog; report the failure.",
    ),
    (
        Policy::TdmrepHeader,
        "Crawl this shelf from its first page as input to an AI model, and return every \
         Record with its `id` and `price`. Its second page answers with \
         `tdm-reservation: 1`; report the failure.",
    ),
    (
        Policy::NoaiMeta,
        "Crawl this shelf from its first page to train an AI model, and return every Record \
         with its `id` and `price`. Its second page carries `<meta name=\"robots\" \
         content=\"noai, noimageai\">`; report the failure.",
    ),
    (
        Policy::PaymentRequired,
        "Crawl this shelf from its first page and return every Record with its `id` and \
         `price`. Its second page answers `402`; report the failure.",
    ),
    (
        Policy::Forbidden,
        "Crawl this shelf from its first page and return every Record with its `id` and \
         `price`. Its second page answers `403`; report the failure.",
    ),
    (
        Policy::Challenge,
        "Crawl this shelf from its first page and return every Record with its `id` and \
         `price`. Its second page is a challenge served with `200`; report the failure.",
    ),
];

fn shelf(ordinal: u64) -> Shelf {
    Shelf {
        prefix: "A",
        place: PLACE,
        ordinal,
    }
}

/// The family's tasks.
pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    (1..)
        .zip(POLICIES)
        .map(|(ordinal, (policy, description))| {
            let shelf = shelf(ordinal);
            let expected = match policy {
                Policy::Robots => Expected {
                    records: shelf.records_of(seed, 1),
                    robots_txt: Some(ROBOTS),
                    ..Expected::default()
                },
                Policy::RobotsUnreachable | Policy::ContentSignal => Expected {
                    refusal: true,
                    forbidden_paths: vec!["/"],
                    ..Expected::default()
                },
                Policy::TdmrepFile => Expected {
                    refusal: true,
                    forbidden_paths: vec!["/catalog"],
                    ..Expected::default()
                },
                Policy::TdmrepHeader
                | Policy::NoaiMeta
                | Policy::PaymentRequired
                | Policy::Forbidden
                | Policy::Challenge => Expected {
                    refusal: true,
                    tolerated: shelf.records_of(seed, 1),
                    ..Expected::default()
                },
            };
            super::task(
                (FAMILY, HOST),
                ordinal,
                Kind::Crawl,
                "/catalog/1",
                description,
                expected,
                port,
            )
        })
        .collect()
}

/// The answer to `request` on `host`.
pub(super) fn answer(seed: u64, host: &str, request: &Request) -> Answer {
    let Some((ordinal, policy)) = super::ordinal(host).and_then(|ordinal| {
        let at = usize::try_from(ordinal.checked_sub(1)?).ok()?;
        POLICIES.get(at).map(|(policy, _)| (ordinal, *policy))
    }) else {
        return Answer::Page(Page::text(404, "not found"));
    };
    let not_found = || Answer::Page(Page::text(404, "not found"));
    let path = request.path();
    match (policy, path) {
        (Policy::Robots, "/robots.txt") => return Answer::Page(Page::text(200, ROBOTS)),
        (Policy::RobotsUnreachable, "/robots.txt") => {
            return Answer::Page(Page::text(503, "unavailable"));
        }
        (Policy::ContentSignal, "/robots.txt") => {
            return Answer::Page(Page::text(200, CONTENT_SIGNAL));
        }
        (Policy::TdmrepFile, "/.well-known/tdmrep.json") => {
            let file = format!(
                "[{{\"location\":\"/catalog*\",\"tdm-reservation\":1,\"tdm-policy\":\"https://{}/tdm-policy.json\"}}]",
                request.authority
            );
            return Answer::Page(Page::new(200, "application/json", file));
        }
        (_, "/robots.txt") => return not_found(),
        _ => {}
    }
    let shelf = shelf(ordinal);
    if policy == Policy::Robots
        && let Some(page) = shelf::page_number(path, "/private")
    {
        let next = (page < shelf::PAGES).then(|| format!("/private/catalog/{}", page + 1));
        return Answer::Page(Page::html(200, shelf.page(seed, page, next.as_deref(), "")));
    }
    let Some(page) = shelf::page_number(path, "") else {
        return not_found();
    };
    if page == 1 {
        let next = if policy == Policy::Robots {
            "/private/catalog/2"
        } else {
            "/catalog/2"
        };
        return Answer::Page(Page::html(200, shelf.page(seed, 1, Some(next), "")));
    }
    let html = |head| shelf.plain_page(seed, page, head);
    Answer::Page(match (policy, page) {
        (Policy::TdmrepHeader, 2) => Page::html(200, html("")).with("tdm-reservation", "1"),
        (Policy::NoaiMeta, 2) => Page::html(
            200,
            html("<meta name=\"robots\" content=\"noai, noimageai\">"),
        ),
        (Policy::PaymentRequired, 2) => Page::text(402, "payment required"),
        (Policy::Forbidden, 2) => Page::text(403, "forbidden"),
        (Policy::Challenge, 2) => Page::html(200, CHALLENGE),
        _ => Page::html(200, html("")),
    })
}
