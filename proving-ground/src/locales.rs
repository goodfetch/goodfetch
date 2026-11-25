//! The `locales` family on `locales.test` (ADR 0123): pages whose values read
//! right only in the language each declares and at the date each is served
//! with. [`document_language`] declares the language in a meta tag, in a
//! header or on the root element; [`relative_times`] shows times relative to
//! a fixed `Date`, on either side of a change of the clocks.

mod document_language;
mod relative_times;

use std::fmt::Write as _;

use hyper::Method;

use crate::exchange::{Reply, Request};
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on.
pub(crate) const HOST: &str = "locales.test";

const FAMILY: &str = "locales";

/// The generator place of the family's values, followed by its submodule's.
const FAMILY_PLACE: u64 = 44;

/// Answers a request on one of the family's host names.
pub(crate) fn respond(seed: u64, request: &Request<'_>) -> Reply {
    if request.method != Method::GET {
        return Reply::not_found();
    }
    document_language::respond(seed, request.path)
        .or_else(|| relative_times::respond(seed, request.path))
        .unwrap_or_else(Reply::not_found)
}

/// An index page titled `title`, linking each page of `paths` by its path.
fn index(title: &str, paths: impl Iterator<Item = String>) -> Reply {
    let links = paths.fold(String::new(), |mut links, path| {
        let _ = writeln!(links, "<li><a href=\"{path}\">{path}</a></li>");
        links
    });
    Reply::html(format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n<h1>{title}</h1>\n<ul>\n{links}</ul>\n</main>\n</body>\n</html>\n"
    ))
}

/// The host name of the family's task `ordinal`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal, path, description, records: Vec<Record>| Task {
        host: task_host(ordinal),
        entry: Entry {
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind: Kind::Crawl,
            start_url: format!("{}{path}", origin(&task_host(ordinal), port)),
            description,
            account: None,
            time_steps: None,
        },
        expected: Expected::records(records),
    };
    vec![
        task(
            1,
            document_language::INDEX_PATH,
            document_language::DESCRIPTION,
            document_language::records(seed),
        ),
        task(
            2,
            relative_times::INDEX_PATH,
            relative_times::DESCRIPTION,
            relative_times::records(seed),
        ),
    ]
}
