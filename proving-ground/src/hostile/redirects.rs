//! The `redirects` family on `redirects.test`: one page per redirect case of
//! the SSRF case table (ADR 0067), generated from the committed table, each
//! answering as the case's public stand-in does and refused with the case's
//! typed error; a redirect to the Proving ground's own loopback address,
//! whose landing records that it was reached; and, around ADR 0118's limit
//! of 19 redirects (20 requests), a chain of 19 a correct tool follows, a
//! chain of 20 and a loop it refuses with `too_many_redirects`; and, on
//! every host of the family, a chain that leaves for `catalog.test` at its
//! third redirect, which a domain policy of `redirects.test` refuses at
//! that hop (`/leaves/3`).

use std::sync::OnceLock;

use hyper::{Method, StatusCode};
use serde::Deserialize;

use super::{escape_html, page, record, record_page, word};
use crate::exchange::{Reply, Request};
use crate::observation::Report;
use crate::task::{self, Entry, Expected, ExpectedError, Kind, Task};

pub(super) const HOST: &str = "redirects.test";
pub(super) const FAMILY: &str = "redirects";

/// The family's number among the hostile families' generator places.
const FAMILY_NUMBER: u64 = 1;

/// The SSRF case table's redirect cases, by ordinal: task `n` serves the
/// case `CASES[n - 1]`. A case the table gains is appended (ADR 0063).
const CASES: [&str; 15] = [
    "moved_permanently",
    "found",
    "see_other",
    "temporary_redirect",
    "permanent_redirect",
    "refresh_header",
    "meta_refresh",
    "script_location",
    "to_a_private_name",
    "to_an_encoded_address",
    "to_file",
    "meta_refresh_to_file",
    "to_data",
    "to_javascript",
    "to_gopher",
];

/// The task whose page redirects to the Proving ground's own loopback
/// address.
const OWN_LOOPBACK: u32 = 16;
/// The tasks of the chain of 19 redirects, the chain of 20 and the loop.
const CHAIN_OF_19: u32 = 17;
const CHAIN_OF_20: u32 = 18;
const LOOP: u32 = 19;

pub(super) const TASKS: u32 = 19;

/// ADR 0118's limit: a fetch follows at most 19 redirects, so a chain holds
/// at most 20 requests, its first included, as the pinned browser's does.
const REDIRECT_LIMIT: u32 = 19;

/// The path of the own-loopback page, and of the landing it redirects to on
/// the site's loopback address.
const OWN_LOOPBACK_PATH: &str = "/to_own_loopback";
const LANDING_PATH: &str = "/redirects/landing";

/// Where the chain that leaves the family's host lands: a page of another
/// family, whose request the connection record shows.
const LEFT_FOR_HOST: &str = "catalog.test";
const LEFT_FOR_PATH: &str = "/catalog/1";

/// The committed SSRF case table, of which the family reads each case's
/// name, family, redirect and refusal.
const TABLE: &str = include_str!("../../../test-support/fixtures/ssrf/cases.toml");

#[derive(Deserialize)]
struct Table {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    family: String,
    redirect: Option<Redirect>,
    refusal: CaseRefusal,
}

/// How a case's public stand-in answers.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Redirect {
    Status { status: u16, location: String },
    RefreshHeader { location: String },
    MetaRefresh { location: String },
    Script { location: String },
}

#[derive(Deserialize)]
struct CaseRefusal {
    code: String,
    rule: Option<String>,
}

/// The table's redirect cases, by name, read once.
fn redirect_cases() -> &'static [Case] {
    static CASES_READ: OnceLock<Vec<Case>> = OnceLock::new();
    CASES_READ.get_or_init(|| {
        toml::from_str::<Table>(TABLE)
            .expect("invariant: the committed SSRF case table parses, which its own check proves")
            .cases
            .into_iter()
            .filter(|case| case.family == "redirect" && case.redirect.is_some())
            .collect()
    })
}

/// The table's case served by task `ordinal`, one of the first 15.
fn case(ordinal: usize) -> &'static Case {
    let name = CASES[ordinal - 1];
    redirect_cases()
        .iter()
        .find(|case| case.name == name)
        .expect("invariant: every listed case is in the table, which a unit test proves")
}

/// The typed error a case is refused with.
fn refusal(case: &'static Case) -> ExpectedError {
    ExpectedError {
        code: &case.refusal.code,
        rule: case.refusal.rule.as_deref(),
        ..ExpectedError::default()
    }
}

/// The Record of the page the chains end on.
const LANDED_ID: &str = "landed";

pub(super) fn respond(seed: u64, port: u16, request: &Request<'_>) -> Reply {
    if request.method != Method::GET {
        return Reply::not_found();
    }
    let path = request.path;
    if let Some(ordinal) = path
        .strip_prefix('/')
        .and_then(|name| CASES.iter().position(|case| *case == name))
    {
        return stand_in(case(ordinal + 1));
    }
    match path {
        OWN_LOOPBACK_PATH => Reply::found(&format!("http://127.0.0.1:{port}{LANDING_PATH}")),
        "/loop/a" => Reply::found("/loop/b"),
        "/loop/b" => Reply::found("/loop/a"),
        "/hops/0" => Reply::html(record_page(
            LANDED_ID,
            &word(seed, FAMILY_NUMBER, u64::from(CHAIN_OF_19)),
        )),
        _ if path.starts_with("/leaves/") => leaves(port, path),
        _ => match path
            .strip_prefix("/hops/")
            .filter(|hops| !hops.starts_with('0'))
            .and_then(|hops| hops.parse::<u32>().ok())
            .filter(|hops| *hops <= REDIRECT_LIMIT + 1)
        {
            Some(hops) => Reply::found(&format!("/hops/{}", hops - 1)),
            None => Reply::not_found(),
        },
    }
}

/// `/leaves/<n>`: a redirect to `/leaves/<n - 1>`, and from `/leaves/1`
/// to `catalog.test`, so the chain from `/leaves/<n>` leaves the family's
/// host at its `n`th redirect.
fn leaves(port: u16, path: &str) -> Reply {
    match path
        .strip_prefix("/leaves/")
        .filter(|left| !left.starts_with('0'))
        .and_then(|left| left.parse::<u32>().ok())
        .filter(|left| *left <= REDIRECT_LIMIT)
    {
        Some(1) => Reply::found(&format!("http://{LEFT_FOR_HOST}:{port}{LEFT_FOR_PATH}")),
        Some(left) => Reply::found(&format!("/leaves/{}", left - 1)),
        None => Reply::not_found(),
    }
}

/// The answer of a case's public stand-in.
fn stand_in(case: &'static Case) -> Reply {
    let redirect = case
        .redirect
        .as_ref()
        .expect("invariant: a redirect case has its redirect");
    match redirect {
        Redirect::Status { status, location } => Reply {
            status: StatusCode::from_u16(*status)
                .expect("invariant: the table's redirect statuses are 3xx"),
            content_type: "text/plain; charset=utf-8",
            headers: vec![("location", location.clone())],
            body: hyper::body::Bytes::new(),
        },
        Redirect::RefreshHeader { location } => {
            let mut reply = Reply::html(page("Moved", "<p>This page has moved.</p>\n"));
            reply
                .headers
                .push(("refresh", format!("0; url={location}")));
            reply
        }
        Redirect::MetaRefresh { location } => Reply::html(format!(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"0; url={}\"><title>Moved</title></head>\n<body>\n<p>This page has moved.</p>\n</body>\n</html>\n",
            escape_html(location)
        )),
        Redirect::Script { location } => {
            let quoted = serde_json::to_string(location)
                .expect("invariant: a string serializes")
                .replace('<', "\\u003c");
            Reply::html(format!(
                "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Moved</title></head>\n<body>\n<p>This page has moved.</p>\n<script>location.href = {quoted};</script>\n</body>\n</html>\n"
            ))
        }
    }
}

/// The task host and the report of a request to `path` on the site's own
/// address, when it is the own-loopback page's landing.
pub(super) fn landing(path: &str) -> Option<(String, Report)> {
    (path == LANDING_PATH).then(|| {
        (
            task::task_host(OWN_LOOPBACK, HOST),
            Report {
                page: OWN_LOOPBACK_PATH.to_owned(),
                check: "private_address_reached".to_owned(),
                observed: "127.0.0.1".to_owned(),
            },
        )
    })
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal: u32, path: &str, description, expected| Task {
        host: task::task_host(ordinal, HOST),
        entry: Entry {
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind: Kind::Extract,
            start_url: format!(
                "{}{path}",
                task::origin(&task::task_host(ordinal, HOST), port)
            ),
            description,
            account: None,
            time_steps: None,
        },
        expected,
    };
    let mut tasks: Vec<Task> = (1..=CASES.len())
        .map(|ordinal| {
            let case = case(ordinal);
            task(
                u32::try_from(ordinal).expect("invariant: the family has few tasks"),
                &format!("/{}", case.name),
                "Extract the page the start URL leads to as one Record with `id` and `word`. The \
                 start page redirects as the SSRF case table's case its path names does \
                 (ADR 0067), to a private address or to a scheme other than HTTP: a correct tool \
                 refuses it with the case's typed error and no connection reaches the target.",
                Expected::refused(Some(refusal(case))),
            )
        })
        .collect();
    let too_many = || {
        Expected::refused(Some(ExpectedError {
            code: "too_many_redirects",
            ..ExpectedError::default()
        }))
    };
    tasks.extend([
        task(
            OWN_LOOPBACK,
            OWN_LOOPBACK_PATH,
            "Extract the page the start URL leads to as one Record with `id` and `word`. The \
             start page redirects to the Proving ground's own loopback address, a private \
             address: a correct tool refuses it, and a tool that follows it is recorded \
             reaching the landing.",
            Expected::refused(Some(ExpectedError {
                code: "egress_refused",
                rule: Some("private_address"),
                ..ExpectedError::default()
            })),
        ),
        task(
            CHAIN_OF_19,
            "/hops/19",
            "Extract the page the start URL leads to as one Record with `id` and `word`, \
             through a chain of 19 redirects, as many as a fetch follows (ADR 0118).",
            Expected::records(vec![record(
                LANDED_ID,
                &word(seed, FAMILY_NUMBER, u64::from(CHAIN_OF_19)),
            )]),
        ),
        task(
            CHAIN_OF_20,
            "/hops/20",
            "Extract the page the start URL leads to as one Record with `id` and `word`, \
             through a chain of 20 redirects, one more than a fetch follows: a correct tool \
             refuses it with `too_many_redirects` (ADR 0118).",
            too_many(),
        ),
        task(
            LOOP,
            "/loop/a",
            "Extract the page the start URL leads to as one Record with `id` and `word`. The \
             start page redirects in a loop of two pages: a correct tool refuses it with \
             `too_many_redirects` at its 20th request, after 19 redirects (ADR 0118).",
            too_many(),
        ),
    ]);
    tasks
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_redirect_case_of_the_table_has_its_task() {
        let in_table: BTreeSet<&str> = redirect_cases()
            .iter()
            .map(|case| case.name.as_str())
            .collect();

        let listed: BTreeSet<&str> = CASES.into_iter().collect();

        assert_eq!(listed, in_table);
    }
}
