//! The page suite's `page.request` cases (ADRs 0020, 0085, 0092, 0118,
//! 0119 and 0121, ticket #706): Playwright's request API on a page's
//! Session, sent from inside the Session's browser, judged by its answers,
//! the bodies `response.body` reads of them and the Proving ground's own
//! record of the requests it received. A 429 is a response, its body kept,
//! never a Block cause.
//!
//! The fetch edge pages are reached over plain HTTP/1.1, since the browser
//! does not trust the Proving ground's authority until the engine's TLS
//! relay does (ADR 0042).

use goodfetch_protocol::page::events::RESPONSE_BODY;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use serde_json::{Value, json};

use super::navigation::{call, open};
use super::{Checks, Site, finish_run};
use crate::protocol::connection::{Connection, Gone};

/// One `page.request` case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    TooManyRequestsIsAResponse,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "page-request-answers-a-429-as-a-response-and-keeps-its-body" => {
                Self::TooManyRequestsIsAResponse
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::TooManyRequestsIsAResponse => too_many_requests(connection, site, checks),
        }
    }
}

/// Sends `params` with `page.request` from `page`, and answers the engine's
/// whole answer.
fn request(connection: &mut Connection, page: &Value, params: &Value) -> Result<Value, Gone> {
    call(connection, PAGE_REQUEST.name, page, params)
}

/// The bytes of the body `response.body` reads of the response a
/// `page.request` answer holds, or its whole failure answer.
fn body_of(
    connection: &mut Connection,
    page: &Value,
    answer: &Value,
) -> Result<Result<Vec<u8>, Value>, Gone> {
    let read = call(
        connection,
        RESPONSE_BODY.name,
        page,
        &json!({ "request_id": answer["result"]["request_id"] }),
    )?;
    Ok(handle_bytes(&read).ok_or(read))
}

/// The bytes of the page body file a `response.body` answer's handle names,
/// read on the same machine as the engine.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness reads a body's file on its own thread, outside any async worker"
)]
fn handle_bytes(answer: &Value) -> Option<Vec<u8>> {
    std::fs::read(answer["result"]["path"].as_str()?).ok()
}

/// A 429 from the site is the request's response, with its status and its
/// `Retry-After`, whose body reads whole: no Block cause is judged on it
/// (ADR 0092).
fn too_many_requests(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-request-429";
    let limited = site.named_url("fetch-edge.test", "/retry-after/429/long");
    let Some(page) = open(connection, site, run_id, &[&limited], checks)? else {
        return Ok(());
    };
    let limited = limited.unwrap_or_default();
    let answer = request(connection, &page, &json!({ "url": limited }))?;
    let response = &answer["result"];
    checks.check(
        "`page.request` to a 429 answers a response, never a failure",
        answer.get("error").is_none() && response.is_object(),
    );
    checks.check(
        "its status is 429 and its URL the request's",
        response["status"] == json!(429) && response["url"] == json!(limited),
    );
    checks.check(
        "it carries the site's `Retry-After`",
        response["headers"]["retry-after"] == json!("3600"),
    );
    checks.check(
        "it says no service worker supplied it",
        response["from_service_worker"] == json!(false),
    );
    let body = body_of(connection, &page, &answer)?;
    checks.check(
        "its kept body reads whole through `response.body`",
        body.as_deref() == Ok(b"unavailable".as_slice()),
    );
    finish_run(connection, run_id)
}
