//! The service-worker Challenge (#1539): an article behind a check that a
//! service worker of the page must answer. Its script registers the worker
//! and posts the check; the worker, once it controls the page, sends the
//! check itself with the proof its own script holds. A client that blocks
//! service workers, or sends its requests past them, posts the check without
//! the proof.

use hyper::body::Bytes;
use hyper::{Method, StatusCode};

use super::{FAMILY_PLACE, SERVICE_WORKER_PLACE};
use crate::exchange::{Reply, Request};
use crate::generator;
use crate::observation::{self, Recorded, Report};

/// The page's path.
pub(super) const PATH: &str = "/service-worker";

/// The path of the worker's script, whose scope holds the page.
const WORKER_PATH: &str = "/service-worker.js";

/// The path the check is posted to.
const CHECK_PATH: &str = "/service-worker/check";

/// The cookie that carries a passed check.
const COOKIE: &str = "service_worker";

/// The header a browser sends as it fetches a service worker's script.
const WORKER_REQUEST: &str = "service-worker";

/// The header the worker sends its proof in.
const PROOF_HEADER: &str = "x-service-worker";

/// The values drawn at the Challenge's generator place.
const PROOF_VALUE: u64 = 0;
const COOKIE_VALUE: u64 = 1;

/// The page's script and the worker's, which holds the proof in place of
/// [`PROOF_MARK`].
const PAGE_SCRIPT: &str = include_str!("../../pages/protection/service-worker-challenge.js");
const WORKER_SCRIPT: &str = include_str!("../../pages/protection/service-worker.js");
const PROOF_MARK: &str = "@PROOF@";

fn drawn(seed: u64, value: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &[FAMILY_PLACE, 0, SERVICE_WORKER_PLACE, value])
    )
}

/// Whether `request` carries the header `name` with the value `value`.
fn carries(request: &Request<'_>, name: &str, value: &str) -> bool {
    request
        .headers
        .get(name)
        .is_some_and(|sent| sent.as_bytes() == value.as_bytes())
}

/// Answers a request for the page, whose article is `article`, for its
/// worker or for its check, with what the site records of it; `None` for any
/// other.
pub(super) fn respond(
    seed: u64,
    request: &Request<'_>,
    article: impl FnOnce() -> String,
) -> Option<(Reply, Option<Recorded>)> {
    let clearance = drawn(seed, COOKIE_VALUE);
    Some(match (request.method, request.path) {
        (&Method::GET, PATH) if request.cookie(COOKIE) == Some(clearance.as_str()) => {
            (Reply::html(article()), None)
        }
        (&Method::GET, PATH) => (Reply::html_with_status(StatusCode::FORBIDDEN, page()), None),
        (&Method::GET, WORKER_PATH) if carries(request, WORKER_REQUEST, "script") => {
            (worker(seed), None)
        }
        (&Method::GET, WORKER_PATH) => (
            Reply::error(StatusCode::FORBIDDEN, "not_a_service_worker"),
            None,
        ),
        (&Method::POST, CHECK_PATH) => {
            let passed = carries(request, PROOF_HEADER, &drawn(seed, PROOF_VALUE));
            let reply = if passed {
                Reply::no_content().set_cookie(COOKIE, &clearance)
            } else {
                Reply::error(StatusCode::FORBIDDEN, "no_service_worker")
            };
            (reply, Some(checked(passed)))
        }
        _ => return None,
    })
}

/// What the site records of a check: that the page's script ran, and whether
/// a service worker answered it. It is no finding either way, so a client
/// that passes after a refused check is judged by its Records alone.
fn checked(passed: bool) -> Recorded {
    Recorded::Report(Report {
        page: PATH.to_owned(),
        check: observation::RAN.to_owned(),
        observed: if passed {
            "service_worker"
        } else {
            "no_service_worker"
        }
        .to_owned(),
    })
}

/// The worker's script for `seed`.
fn worker(seed: u64) -> Reply {
    Reply {
        status: StatusCode::OK,
        content_type: "text/javascript; charset=utf-8",
        headers: Vec::new(),
        body: Bytes::from(WORKER_SCRIPT.replace(PROOF_MARK, &drawn(seed, PROOF_VALUE))),
    }
}

/// The page of a client no service worker vouched for.
fn page() -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Just a moment...</title></head>\n<body>\n<main id=\"service-worker\">\n<h1>Checking your browser before accessing this site.</h1>\n<p>This takes a moment and needs service workers.</p>\n</main>\n<script>\n{PAGE_SCRIPT}</script>\n</body>\n</html>\n"
    )
}
