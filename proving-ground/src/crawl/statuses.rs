//! The `statuses` family on `statuses.test`: an index of articles, some
//! reached through one redirect of each kind or a chain of them; a link to
//! every HTTP error status, 402 included, each with an error page in the
//! articles' own template; soft 404s, `200` pages in that template that say
//! the article does not exist (every unknown article number is one); and
//! three redirect loops, one through two pages, one onto itself and one
//! through new URLs without end. A crawl may make a bounded number of
//! requests to the loops; each one past it is a `trap` finding.

use std::fmt::Write as _;

use hyper::StatusCode;

use super::{Counts, Page, Visit, crawl_task, document, name, numbered};
use crate::task::{Finding, Record, Task};

pub(super) const HOST: &str = "statuses.test";
pub(super) const FAMILY: &str = "statuses";

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 21;

const ARTICLES: u64 = 40;
/// Articles 1 to 30 are linked directly.
const LINKED: u64 = 30;
/// Articles 31 to 35 are linked through one redirect each, of these kinds.
const MOVED: [u16; 5] = [301, 302, 303, 307, 308];
/// Articles 36 to 40 are linked through a chain of this many redirects.
const CHAIN: u64 = 4;
/// The articles the index links that do not exist: soft 404s.
const SOFT_404S: [u64; 5] = [41, 52, 67, 88, 93];
/// The requests a task's crawl may make to the redirect loops.
const LOOP_ALLOWANCE: u64 = 100;

/// Every HTTP error status IANA registers, but 418, which it lists as
/// unused.
const ERRORS: [u16; 39] = [
    400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410, 411, 412, 413, 414, 415, 416, 417, 421,
    422, 423, 424, 425, 426, 428, 429, 431, 451, 500, 501, 502, 503, 504, 505, 506, 507, 508, 510,
    511,
];

/// What the family keeps: the requests each task host made to the loops.
#[derive(Default)]
pub(super) struct State {
    loops: Counts,
}

fn article_id(number: u64) -> String {
    format!("A-{number:04}")
}

fn title(seed: u64, number: u64) -> String {
    format!("The {}", name(seed, &[FAMILY_PLACE, number]))
}

/// An article page, in the template every page of the family uses.
fn article(id: &str, title: &str, text: &str) -> String {
    document(
        title,
        &format!(
            "<main>\n<article class=\"post\" data-id=\"{id}\">\n<h1 class=\"title\">{title}</h1>\n<p>{text}</p>\n</article>\n<a href=\"/\">All articles</a>\n</main>\n"
        ),
    )
}

pub(super) fn respond(visit: &Visit<'_>, state: &State) -> Page {
    let path = visit.request.path;
    if path == "/" {
        return Page::ok(index());
    }
    if let Some(rest) = path.strip_prefix("/loops/") {
        return loop_step(visit, state, rest);
    }
    if let Some(id) = path.strip_prefix("/articles/") {
        return match numbered(id, "A-", 4, 9999) {
            Some(number) if number <= ARTICLES => Page::ok(article(
                id,
                &title(visit.seed, number),
                "An article of the archive.",
            )),
            Some(_) => Page::ok(article(
                id,
                "Page not found",
                "This article does not exist.",
            )),
            None => Page::not_found(),
        };
    }
    if let Some(id) = path.strip_prefix("/moved/") {
        return match numbered(id, "A-", 4, LINKED + 5) {
            Some(number) if number > LINKED => Page::redirect(
                status(MOVED[usize::try_from(number - LINKED - 1).expect("invariant: below 5")]),
                &format!("/articles/{id}"),
            ),
            _ => Page::not_found(),
        };
    }
    if let Some(rest) = path.strip_prefix("/chain/") {
        let Some((id, hop)) = rest.split_once('/') else {
            return Page::not_found();
        };
        return match (
            numbered(id, "A-", 4, ARTICLES),
            super::canonical_number(hop),
        ) {
            (Some(number), Some(1)) if number > LINKED + 5 => {
                Page::redirect(StatusCode::MOVED_PERMANENTLY, &format!("/articles/{id}"))
            }
            (Some(number), Some(hop)) if number > LINKED + 5 && (2..=CHAIN).contains(&hop) => {
                Page::redirect(StatusCode::FOUND, &format!("/chain/{id}/{}", hop - 1))
            }
            _ => Page::not_found(),
        };
    }
    if let Some(code) = path.strip_prefix("/errors/") {
        return match super::canonical_number(code).and_then(|code| u16::try_from(code).ok()) {
            Some(code) if ERRORS.contains(&code) => error(code),
            _ => Page::not_found(),
        };
    }
    Page::not_found()
}

fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).expect("invariant: the family's codes are valid")
}

/// The index: every article's link, every error's, the soft 404s' and the
/// loops'.
fn index() -> String {
    let mut body = String::from("<main>\n<h1>Articles</h1>\n<ul class=\"articles\">\n");
    let mut item = |href: String, text: &str| {
        writeln!(body, "<li><a href=\"{href}\">{text}</a></li>")
            .expect("invariant: writing to a String succeeds");
    };
    for number in 1..=LINKED {
        item(format!("/articles/{}", article_id(number)), "Article");
    }
    for number in LINKED + 1..=LINKED + 5 {
        item(format!("/moved/{}", article_id(number)), "Moved article");
    }
    for number in LINKED + 6..=ARTICLES {
        item(
            format!("/chain/{}/{CHAIN}", article_id(number)),
            "Moved again",
        );
    }
    for number in SOFT_404S {
        item(format!("/articles/{}", article_id(number)), "Old article");
    }
    for code in ERRORS {
        item(format!("/errors/{code}"), "Broken link");
    }
    for start in ["a", "self", "endless/1"] {
        item(format!("/loops/{start}"), "Elsewhere");
    }
    body.push_str("</ul>\n</main>\n");
    document("Articles", &body)
}

/// One step of a redirect loop, counted against the task's allowance.
fn loop_step(visit: &Visit<'_>, state: &State, rest: &str) -> Page {
    let next = match rest {
        "a" => "/loops/b".to_owned(),
        "b" => "/loops/a".to_owned(),
        "self" => "/loops/self".to_owned(),
        _ => match rest
            .strip_prefix("endless/")
            .and_then(super::canonical_number)
        {
            Some(step) => format!("/loops/endless/{}", step.saturating_add(1)),
            None => return Page::not_found(),
        },
    };
    if state.loops.add(visit.request.host) > LOOP_ALLOWANCE {
        visit.find(Finding::Trap {
            target: visit.request.target(),
        });
    }
    let status = match rest {
        "self" => StatusCode::TEMPORARY_REDIRECT,
        _ => StatusCode::FOUND,
    };
    Page::redirect(status, &next)
}

/// The page of an HTTP error, in the articles' template, with the headers
/// its status requires.
fn error(code: u16) -> Page {
    let status = status(code);
    let reason = status.canonical_reason().unwrap_or("Error");
    let page = Page::html(
        status,
        article(
            &format!("E-{code}"),
            &format!("Error {code}: {reason}"),
            "The server could not answer this request.",
        ),
    );
    match code {
        401 => page.header("www-authenticate", "Basic realm=\"statuses\"".to_owned()),
        405 => page.header("allow", "GET, HEAD".to_owned()),
        407 => page.header("proxy-authenticate", "Basic realm=\"statuses\"".to_owned()),
        416 => page.header("content-range", "bytes */0".to_owned()),
        426 => page.header("upgrade", "HTTP/3.0".to_owned()),
        _ => page,
    }
}

pub(super) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    vec![crawl_task(
        (HOST, FAMILY),
        1,
        port,
        "/",
        "Crawl the articles from the index and return one Record per article, with the fields \
            `id` and `title`. Some links answer an HTTP error, some answer a page saying the \
            article does not exist, some redirect once or through a chain, and three redirect \
            in a loop; a crawl may make at most 100 requests to the loops.",
        (1..=ARTICLES)
            .map(|number| {
                Record::from([("id", article_id(number)), ("title", title(seed, number))])
            })
            .collect(),
    )]
}
