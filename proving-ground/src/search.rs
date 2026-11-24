//! The `search` family on `search.test`: a search results page and the JSON
//! API of a SearXNG instance (its `/search`, which a search provider calls),
//! over articles of the seed's. The results of a query change with the time
//! step, never the wall clock, so a program that keeps an older answer misses
//! the newer results.

mod searxng;

use std::fmt::Write as _;

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request, encode};
use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on.
pub(crate) const HOST: &str = "search.test";

const FAMILY: &str = "search";

/// The only path a search is asked on.
const SEARCH_PATH: &str = "/search";

/// How many time steps the family's tasks span, from step 0.
const STEPS: u64 = 4;

/// How many results a search shows.
const RESULTS: u64 = 3;

/// The generator places of the family's values, after the family's own.
const FAMILY_PLACE: u64 = 45;
const SUBJECT_PLACE: u64 = 1;
const TITLE_PLACE: u64 = 2;
const SNIPPET_PLACE: u64 = 3;

/// The subjects articles are written on, each a query's words.
const SUBJECTS: [&str; 6] = [
    "tide mill",
    "bell foundry",
    "salt marsh",
    "paper kite",
    "stone bridge",
    "harbour crane",
];

const TITLES: [&str; 8] = [
    "A short history of the",
    "Repairing the",
    "Inside the",
    "A day at the",
    "Saving the",
    "The people behind the",
    "Measuring the",
    "Drawing the",
];

const SNIPPETS: [&str; 8] = [
    "What the keepers learned over a winter.",
    "Notes, plans and a list of what is left to do.",
    "A walk through every part, with photographs.",
    "How it was built, and why it still stands.",
    "Interviews with three generations of workers.",
    "The survey's figures, compared with the first one.",
    "Where it stands today and who looks after it.",
    "An account of the works, from start to end.",
];

/// The revision of the results at a time step: they change at every step
/// but those that leave a remainder of 2 when divided by 3.
fn revision_at(step: u64) -> u64 {
    step - step.saturating_add(1) / 3
}

/// One result of a search: an article, whose ID is the last of its URL.
struct Article {
    id: String,
    url: String,
    title: String,
    snippet: String,
}

/// The article `number` written on `SUBJECTS[subject]`.
fn article(seed: u64, subject: usize, number: u64) -> Article {
    let draw = |value, words: &[&'static str]| {
        let place = [FAMILY_PLACE, subject as u64, number, value];
        let index = generator::below(seed, &place, words.len() as u64);
        words[usize::try_from(index).expect("invariant: a word index is below 8")]
    };
    let words = SUBJECTS[subject];
    let id = format!("{}-{number}", words.replace(' ', "-"));
    Article {
        url: format!("https://articles.example/{id}"),
        id,
        title: format!("{} {words}", draw(TITLE_PLACE, &TITLES)),
        snippet: format!("No. {number}. {}", draw(SNIPPET_PLACE, &SNIPPETS)),
    }
}

/// The results of `query` at the revision `revision`, the newest article
/// first: each revision brings one article and drops the oldest. A query
/// finds the subject whose words it is, and nothing otherwise.
fn results(seed: u64, revision: u64, query: &str) -> Vec<Article> {
    let query = query.trim().to_lowercase();
    SUBJECTS
        .iter()
        .position(|words| *words == query)
        .map(|subject| {
            (1..=RESULTS)
                .rev()
                .map(|place| article(seed, subject, revision + place))
                .collect()
        })
        .unwrap_or_default()
}

/// The place in [`SUBJECTS`] of the subject the task `ordinal` searches for.
fn subject(seed: u64, ordinal: u32) -> usize {
    generator::distinct(seed, &[FAMILY_PLACE, SUBJECT_PLACE], SUBJECTS.len(), 2)
        [ordinal as usize - 1]
}

/// Answers a request on one of the family's host names at the time step
/// `step`.
pub(crate) fn respond(seed: u64, step: u64, request: &Request<'_>) -> Reply {
    match (request.method, request.path) {
        (&Method::GET, "/") => Reply::html(index(None)),
        (&Method::GET | &Method::POST, SEARCH_PATH) => {
            searxng::respond(request, |query| results(seed, revision_at(step), query))
        }
        (_, "/" | SEARCH_PATH) => {
            Reply::error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
        }
        _ => Reply::not_found(),
    }
}

/// The page that asks for a query, with the `error` a search ended in.
fn index(error: Option<&str>) -> String {
    let error = error
        .map(|error| format!("<p role=\"alert\">{}</p>\n", escape(error)))
        .unwrap_or_default();
    document("Search", &format!("{}{error}", form("")))
}

/// The results page of `query`.
fn results_page(query: &str, results: &[Article]) -> String {
    let mut main = form(query);
    if results.is_empty() {
        main.push_str("<p class=\"empty\">No results were found.</p>\n");
    }
    for Article {
        id,
        url,
        title,
        snippet,
    } in results
    {
        let _ = write!(
            main,
            "<article class=\"result\" data-id=\"{id}\">\n<h3><a href=\"{url}\">{title}</a></h3>\n<p class=\"content\">{snippet}</p>\n</article>\n"
        );
    }
    document("Search results", &main)
}

/// The search form, holding `query`.
fn form(query: &str) -> String {
    format!(
        "<form method=\"get\" action=\"{SEARCH_PATH}\" role=\"search\">\n<input type=\"search\" name=\"q\" aria-label=\"Query\" value=\"{}\">\n<button type=\"submit\">Search</button>\n</form>\n",
        escape(query)
    )
}

fn document(title: &str, main: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n{main}</main>\n</body>\n</html>\n"
    )
}

/// Text a client sent, escaped for HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The host name of the family's task `ordinal`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal, format, description| Task {
        host: task_host(ordinal),
        entry: Entry {
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind: Kind::Watch,
            start_url: format!(
                "{}{SEARCH_PATH}?q={}{format}",
                origin(&task_host(ordinal), port),
                encode(SUBJECTS[subject(seed, ordinal)])
            ),
            description,
            account: None,
            time_steps: Some(STEPS),
        },
        expected: Expected::records(records(seed, subject(seed, ordinal))),
    };
    vec![
        task(
            1,
            "",
            "Open this search results page at each of the Proving ground's time steps, from 0 \
             to the last of the task's, and return one Record per result it shows at any step, \
             with the fields `id` (the result's `data-id`), `url`, `title` and `snippet`.",
        ),
        task(
            2,
            "&format=json",
            "Ask this SearXNG search for its results at each of the Proving ground's time \
             steps, from 0 to the last of the task's, and return one Record per result it \
             answers at any step, with the fields `id` (the last part of the result's URL), \
             `url`, `title` and `snippet`.",
        ),
    ]
}

/// The Records of every result the search for `SUBJECTS[subject]` shows at a
/// step of a task.
fn records(seed: u64, subject: usize) -> Vec<Record> {
    (1..=revision_at(STEPS - 1) + RESULTS)
        .map(|number| {
            let Article {
                id,
                url,
                title,
                snippet,
            } = article(seed, subject, number);
            Record::from([
                ("id", id),
                ("url", url),
                ("title", title),
                ("snippet", snippet),
            ])
        })
        .collect()
}
