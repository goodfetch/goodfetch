//! The `access-policy` family on `access-policy.test`, the lab's
//! access-policy pages ported: each task crawls a three-page shelf whose
//! site states a policy, in its robots.txt (RFC 9309, with the lab's
//! groups and patterns), a `Content-Signal` line, a `TDMRep` file or header
//! (W3C's TDM Reservation Protocol), a `noai` robots meta tag, or a `402`, a `403` or a
//! challenge served with `200`. Where the policy forbids the task's purpose
//! or the site refuses it, the only right result is a reported failure, and
//! the Records read before it are tolerated; the paths the task must not
//! request are judged from the Proving ground's own observations.
//!
//! Five named hosts belong to no task. Two have robots.txt files holding the
//! product-token groups a crawl's robots.txt selection is judged on (ADRs
//! 0086 and 0090), and every other path is a page. On
//! `tokens.access-policy.test` a group for `Goodfetch-Foxhound` disallows
//! `/foxhound/`, one for the operator token `operator-bot` `/operator/`,
//! one for a bare `Foxhound` `/bare/` and the `*` group `/star/`; on
//! `star.access-policy.test` only the bare `Foxhound` and `*` groups
//! remain. A third, `crawl-delay.access-policy.test`, asks every crawler
//! for a Crawl-delay of 5 s, which a crawl's pacing is judged on. A fourth,
//! `slow-robots.access-policy.test`, answers its robots.txt, a Crawl-delay
//! of 2 s for every crawler, 1.5 s after the request arrived, so a delay
//! counted from the robots.txt request's send rather than its response's
//! arrival, or a read of a copy another reader is still fetching, shows on
//! the requests' arrivals. A fifth, `stalled-robots.access-policy.test`,
//! answers its robots.txt's head and the start of its body, the same
//! Crawl-delay, and never the rest its length declares, over HTTP/1.1, so
//! a reader of the whole file is still reading when its Session or Run
//! ends. A sixth, `markup.access-policy.test`, has no robots.txt and
//! declares its AI-use signals in its documents: `/catalog/1` declares
//! none, `/catalog/2` carries `TDMRep`'s
//! `<meta name="tdm-reservation" content="1">`,
//! `/catalog/3` an RSL `Link` header and `/catalog/4` an
//! RSL `<link>` element naming `/license.xml`, an RSL license that permits
//! `search` and prohibits `ai-all`; `/catalog/5` carries the `Link` header
//! of page 3 and sets a cookie, `visit=catalog-5`, which tells whether the
//! license read after it left from the client the document came to.

use std::time::Duration;

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

/// The named host whose robots.txt names Foxhound's token, an operator's,
/// a bare `Foxhound` and `*`.
pub(super) const TOKENS_HOST: &str = "tokens.access-policy.test";

/// The named host whose robots.txt names only a bare `Foxhound` and `*`.
pub(super) const STAR_HOST: &str = "star.access-policy.test";

/// The named host whose robots.txt asks every crawler for a Crawl-delay of
/// 5 s.
pub(super) const CRAWL_DELAY_HOST: &str = "crawl-delay.access-policy.test";

/// The named host whose robots.txt asks every crawler for a Crawl-delay of
/// 2 s, answered [`SLOW_ROBOTS_WAIT`] after the request arrived.
pub(super) const SLOW_ROBOTS_HOST: &str = "slow-robots.access-policy.test";

/// How long [`SLOW_ROBOTS_HOST`] waits before it answers its robots.txt.
const SLOW_ROBOTS_WAIT: Duration = Duration::from_millis(1_500);

/// The named host whose robots.txt sends its head and the start of its
/// body, and never [`STALLED_ROBOTS_REST`].
pub(super) const STALLED_ROBOTS_HOST: &str = "stalled-robots.access-policy.test";

/// The end of [`STALLED_ROBOTS_HOST`]'s robots.txt, which its length counts
/// and which is never sent.
const STALLED_ROBOTS_REST: &[u8] = b"# never sent\n";

/// The named host whose documents declare their AI-use signals in
/// markup.
pub(super) const MARKUP_HOST: &str = "markup.access-policy.test";

/// The RSL license [`MARKUP_HOST`]'s pages 3, 4 and 5 name.
const RSL_LICENSE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rsl xmlns="https://rslstandard.org/rsl">
  <content url="/">
    <license>
      <permits type="usage">search</permits>
      <prohibits type="usage">ai-all</prohibits>
    </license>
  </content>
</rsl>
"#;

/// The family's named hosts, which belong to no task.
pub(super) const NAMED_HOSTS: [&str; 6] = [
    TOKENS_HOST,
    STAR_HOST,
    CRAWL_DELAY_HOST,
    SLOW_ROBOTS_HOST,
    STALLED_ROBOTS_HOST,
    MARKUP_HOST,
];

/// The robots.txt of [`CRAWL_DELAY_HOST`].
const CRAWL_DELAY: &str = "User-agent: *
Crawl-delay: 5
";

/// The robots.txt of [`SLOW_ROBOTS_HOST`].
const SLOW_ROBOTS: &str = "User-agent: *
Crawl-delay: 2
";

/// The robots.txt of [`TOKENS_HOST`].
const TOKENS: &str = "User-agent: Goodfetch-Foxhound
Disallow: /foxhound/

User-agent: operator-bot
Disallow: /operator/

User-agent: Foxhound
Disallow: /bare/

User-agent: *
Disallow: /star/
";

/// The robots.txt of [`STAR_HOST`].
const STAR: &str = "User-agent: Foxhound
Disallow: /bare/

User-agent: *
Disallow: /star/
";

const CONTENT_SIGNAL: &str =
    "User-agent: *\nContent-Signal: search=yes, ai-input=yes, ai-train=no\nAllow: /\n";

/// The answer to `request` on [`MARKUP_HOST`].
fn markup(request: &Request) -> Answer {
    let page = |path: &str, head: &str| {
        Page::html(
            200,
            format!("<!doctype html><title>{path}</title>{head}<p>A page at {path}.</p>"),
        )
    };
    Answer::Page(match request.path() {
        "/license.xml" => Page::new(200, "application/rsl+xml", RSL_LICENSE),
        path @ "/catalog/1" => page(path, ""),
        path @ "/catalog/2" => page(path, "<meta name=\"tdm-reservation\" content=\"1\">"),
        path @ "/catalog/3" => page(path, "").with(
            "link",
            "</license.xml>; rel=\"license\"; type=\"application/rsl+xml\"",
        ),
        path @ "/catalog/4" => page(
            path,
            "<link rel=\"license\" type=\"application/rsl+xml\" href=\"/license.xml\">",
        ),
        path @ "/catalog/5" => page(path, "")
            .with(
                "link",
                "</license.xml>; rel=\"license\"; type=\"application/rsl+xml\"",
            )
            .with("set-cookie", "visit=catalog-5; Path=/"),
        _ => Page::text(404, "not found"),
    })
}

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
    if host == MARKUP_HOST {
        return markup(request);
    }
    if let Some(robots) = match host {
        TOKENS_HOST => Some(TOKENS),
        STAR_HOST => Some(STAR),
        CRAWL_DELAY_HOST => Some(CRAWL_DELAY),
        SLOW_ROBOTS_HOST | STALLED_ROBOTS_HOST => Some(SLOW_ROBOTS),
        _ => None,
    } {
        return match request.path() {
            "/robots.txt" if host == SLOW_ROBOTS_HOST => Answer::Later(
                SLOW_ROBOTS_WAIT,
                Box::new(Answer::Page(Page::text(200, robots))),
            ),
            "/robots.txt" if host == STALLED_ROBOTS_HOST => {
                // Past what a browser sniffs of a body before it hands the
                // response over, so the reader has the start in hand.
                let mut page = Page::text(200, format!("{robots}#{}\n", "-".repeat(1024)));
                let sent = page.body.len();
                page.body.extend_from_slice(STALLED_ROBOTS_REST);
                Answer::Stalled { page, sent }
            }
            "/robots.txt" => Answer::Page(Page::text(200, robots)),
            path => Answer::Page(Page::html(
                200,
                format!("<!doctype html><title>{path}</title><p>A page at {path}.</p>"),
            )),
        };
    }
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
