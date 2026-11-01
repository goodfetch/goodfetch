//! ADR 0067's hostile input beside SSRF, each page served with its expected
//! result, which names what a correct tool refuses, and with which typed
//! error where one is decided, or keeps: the `redirects` family (every
//! redirect of the SSRF case table, a redirect to the Proving ground's own
//! loopback address, chains around ADR 0118's limit of 20 and a loop), the
//! `headers` family (oversized, repeated and malformed heads, and attempts to
//! inject a header), the `downloads` family (page-suggested download names
//! that try to leave the download folder) and the `bombs` family
//! (decompression bombs past the engine's ratio limit). A page a correct
//! tool keeps shows one Record, `id` and `word`, drawn from the seed.

mod bombs;
mod downloads;
mod headers;
mod redirects;

use hyper::StatusCode;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::observation::{Recorded, Report};
use crate::task::{self, Record, Task};

/// The generator place of the families' values.
const FAMILY_PLACE: u64 = 31;

const WORDS: [&str; 16] = [
    "amber", "basalt", "cedar", "dune", "ember", "fjord", "garnet", "harbor", "iris", "juniper",
    "kelp", "lichen", "marble", "nectar", "orchid", "pebble",
];

/// The word of the Record `id` of the family numbered `family`, for `seed`.
fn word(seed: u64, family: u64, ordinal: u64) -> String {
    let index = generator::below(seed, &[FAMILY_PLACE, family, ordinal], WORDS.len() as u64);
    WORDS[usize::try_from(index).expect("invariant: a word index is below 16")].to_owned()
}

/// The Record a kept page shows.
fn record(id: &str, word: &str) -> Record {
    Record::from([("id", id.to_owned()), ("word", word.to_owned())])
}

/// A whole HTML page.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n{body}</main>\n</body>\n</html>\n"
    )
}

/// The page that shows a kept page's Record.
fn record_page(id: &str, word: &str) -> String {
    page(
        id,
        &format!(
            "<article class=\"record\" data-id=\"{id}\">\n<h1 class=\"word\">{word}</h1>\n</article>\n"
        ),
    )
}

/// `text` escaped for an HTML attribute or text.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Every host name the families are served on, each with its task.
pub(crate) fn hosts() -> Vec<(String, Option<String>)> {
    let mut hosts = task::family_hosts(redirects::FAMILY, redirects::HOST, redirects::TASKS);
    hosts.extend(task::family_hosts(
        headers::FAMILY,
        headers::HOST,
        headers::TASKS,
    ));
    hosts.extend(task::family_hosts(
        downloads::FAMILY,
        downloads::HOST,
        downloads::TASKS,
    ));
    hosts.extend(task::family_hosts(bombs::FAMILY, bombs::HOST, bombs::TASKS));
    hosts
}

/// The families' tasks for `seed`, their start URLs on `port`. Ordinals are
/// append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let mut tasks = redirects::tasks(seed, port);
    tasks.extend(headers::tasks(seed, port));
    tasks.extend(downloads::tasks(seed, port));
    tasks.extend(bombs::tasks(seed, port));
    tasks
}

/// Answers a request to one of the families' host names, `host`, given the
/// port the site is reached on: its page and what the site records of it, if
/// anything; `None` when `host` is not theirs.
pub(crate) fn respond(
    seed: u64,
    port: u16,
    host: &str,
    request: &Request<'_>,
) -> Option<(Reply, Option<Recorded>)> {
    if task::serves(host, redirects::HOST, redirects::TASKS) {
        return Some((redirects::respond(seed, port, request), None));
    }
    if task::serves(host, headers::HOST, headers::TASKS) {
        return Some(headers::respond(seed, request));
    }
    if task::serves(host, downloads::HOST, downloads::TASKS) {
        return Some((downloads::respond(seed, host, request), None));
    }
    if task::serves(host, bombs::HOST, bombs::TASKS) {
        return Some((bombs::respond(seed, host, request), None));
    }
    None
}

/// The bytes of a whole response whose head hyper cannot write, such as one
/// with a NUL in a header value, for a request to `path` on `host`; `None`
/// for any other request, which hyper answers.
pub(crate) fn raw_response(seed: u64, host: &str, path: &str) -> Option<Vec<u8>> {
    task::serves(host, headers::HOST, headers::TASKS)
        .then(|| headers::raw_response(seed, path))
        .flatten()
}

/// Whether `host`, without its port, is one whose pages may need their own
/// bytes, so a TLS connection to it is offered HTTP/1.1 alone.
pub(crate) fn http1_only(host: &str) -> bool {
    task::serves(host, headers::HOST, headers::TASKS)
}

/// Answers a request on a host name the site does not serve, such as its own
/// loopback address, to the landing of a redirect to a private address:
/// the landing, and the report the site records for the task whose page
/// redirected there, on that task's host name; `None` for another path.
pub(crate) fn private_landing(path: &str) -> Option<(Reply, String, Report)> {
    let (task_host, report) = redirects::landing(path)?;
    Some((
        Reply::html_with_status(
            StatusCode::OK,
            page(
                "Reached",
                "<p>A redirect to a private address was followed.</p>\n",
            ),
        ),
        task_host,
        report,
    ))
}
