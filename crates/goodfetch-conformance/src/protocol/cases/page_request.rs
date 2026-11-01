//! The page suite's `page.request` cases (ADRs 0020, 0085, 0092, 0118,
//! 0119 and 0121, ticket #706): Playwright's request API on a page's
//! Session, sent from inside the Session's browser, judged by its answers,
//! the bodies `response.body` reads of them and the Proving ground's own
//! record of the requests it received. A 429 is a response, its body kept,
//! never a Block cause. The fetch edge lines (ADR 0118): `max_redirects`
//! follows its count, 0 answering the 3xx, refuses one above 19 with
//! `setting_invalid` and raises `too_many_redirects` past it, and a cut
//! body raises `response_truncated` from `response.body`. The response
//! decoding lines (ADR 0119): a body's bytes are kept as the site sent
//! them, after any content coding, so a `Shift_JIS` text reads in its own
//! encoding and JSON after a BOM reads, and a content coding that fails to
//! decode raises `navigation_failed`. Pitfall H20: a cookie a page sets
//! is sent by `page.request` and the next navigation of its Session, never
//! by another Session's. The request passes the page's domain policy, and
//! no service worker answers it, on a page one controls.
//!
//! The fetch edge pages are reached over plain HTTP/1.1, since the browser
//! does not trust the Proving ground's authority until the engine's TLS
//! relay does (ADR 0042).

use goodfetch_protocol::egress::EGRESS_REFUSED;
use goodfetch_protocol::page::PAGE_GOTO;
use goodfetch_protocol::page::events::RESPONSE_BODY;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{RESPONSE_TRUNCATED, TOO_MANY_REDIRECTS};
use goodfetch_protocol::settings::SETTING_INVALID;
use serde_json::{Value, json};

use super::hostile_input;
use super::launch::{IDENTITY_HOST, reports_once};
use super::navigation::{call, failed_with, open, url_on};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// One `page.request` case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    TooManyRequestsIsAResponse,
    RedirectsFollowTheirCount,
    CutBodyIsTruncated,
    BodiesKeepTheirBytes,
    FailedCodingFails,
    CookiesStayInTheirSession,
    DomainPolicyJudgesIt,
    NoServiceWorkerAnswersIt,
    /// A hostile-input case, a route's among them.
    HostileInput(hostile_input::Case),
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "page-request-answers-a-429-as-a-response-and-keeps-its-body" => {
                Self::TooManyRequestsIsAResponse
            }
            "page-request-follows-its-max-redirects-and-refuses-a-count-above-19" => {
                Self::RedirectsFollowTheirCount
            }
            "a-page-requests-cut-body-raises-response-truncated" => Self::CutBodyIsTruncated,
            "page-request-keeps-the-bytes-of-a-shift-jis-text-and-of-json-after-a-bom" => {
                Self::BodiesKeepTheirBytes
            }
            "a-page-request-whose-content-coding-fails-to-decode-raises-navigation-failed" => {
                Self::FailedCodingFails
            }
            "cookies-a-page-sets-are-sent-by-page-request-and-the-next-navigation-of-its-session-alone" => {
                Self::CookiesStayInTheirSession
            }
            "page-request-passes-the-pages-domain-policy" => Self::DomainPolicyJudgesIt,
            "no-service-worker-answers-a-page-request" => Self::NoServiceWorkerAnswersIt,
            _ => return hostile_input::Case::from_id(id).map(Self::HostileInput),
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::TooManyRequestsIsAResponse => too_many_requests(connection, site, checks),
            Self::RedirectsFollowTheirCount => redirect_counts(connection, site, checks),
            Self::CutBodyIsTruncated => cut_body(connection, site, checks),
            Self::BodiesKeepTheirBytes => kept_bytes(connection, site, checks),
            Self::FailedCodingFails => failed_coding(connection, site, checks),
            Self::CookiesStayInTheirSession => cookies(connection, site, checks),
            Self::DomainPolicyJudgesIt => domain_policy(connection, site, checks),
            Self::NoServiceWorkerAnswersIt => service_worker(connection, site, checks),
            Self::HostileInput(case) => case.run(connection, target, site, checks),
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

/// The code a failure answer names.
fn code_of(answer: &Value) -> &Value {
    &answer["error"]["data"]["code"]
}

/// `max_redirects` follows at most its count: 0 answers the 3xx itself, a
/// chain within the count answers its last response, one past it raises
/// `too_many_redirects` naming the first URL, the hops followed and the
/// option, and a count above 19, the browser-fetch tier's, is refused with
/// `setting_invalid` before anything is sent (ADR 0118).
fn redirect_counts(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-request-redirects";
    let chain = site.named_url("fetch-edge.test", "/redirect/hops/5");
    let Some(page) = open(connection, site, run_id, &[&chain], checks)? else {
        return Ok(());
    };
    let chain = chain.unwrap_or_default();
    for count in [20, 21] {
        let refused = request(
            connection,
            &page,
            &json!({ "url": chain, "max_redirects": count }),
        )?;
        checks.check(
            format!(
                "`max_redirects: {count}` is refused with `{}` naming the option",
                SETTING_INVALID.name
            ),
            code_of(&refused) == &json!(SETTING_INVALID.name)
                && refused["error"]["data"]["key"] == json!("max_redirects"),
        );
    }
    let answered = |answer: &Value, path: &str| {
        answer["result"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with(path))
    };
    let unfollowed = request(
        connection,
        &page,
        &json!({ "url": chain, "max_redirects": 0 }),
    )?;
    checks.check(
        "`max_redirects: 0` answers the 3xx itself",
        unfollowed["result"]["status"] == json!(302)
            && answered(&unfollowed, "/redirect/hops/5")
            && unfollowed["result"]["headers"]["location"] == json!("/redirect/hops/4"),
    );
    let within = request(
        connection,
        &page,
        &json!({ "url": chain, "max_redirects": 5 }),
    )?;
    checks.check(
        "a chain of 5 redirects within `max_redirects: 5` answers its last response",
        within["result"]["status"] == json!(200) && answered(&within, "/redirect/hops/0"),
    );
    let followed_by_default = request(connection, &page, &json!({ "url": chain }))?;
    checks.check(
        "with no `max_redirects`, the chain of 5 is followed",
        followed_by_default["result"]["status"] == json!(200),
    );
    let past = request(
        connection,
        &page,
        &json!({ "url": chain, "max_redirects": 3 }),
    )?;
    let data = &past["error"]["data"];
    checks.check(
        format!(
            "a chain past `max_redirects: 3` raises `{}`",
            TOO_MANY_REDIRECTS.name
        ),
        code_of(&past) == &json!(TOO_MANY_REDIRECTS.name),
    );
    checks.check(
        "it names the first URL and the 3 hops followed",
        data["url"] == json!(chain) && data["hops"].as_array().map(Vec::len) == Some(3),
    );
    checks.check(
        "it names the option and its count as the limit",
        data["limit"] == json!({ "kind": "option", "name": "max_redirects", "count": 3 }),
    );
    finish_run(connection, run_id)
}

/// A body cut by a length mismatch, a chunked stream ending early or a gzip
/// stream cut before its `Content-Length` is never read as whole: the
/// request answers its response, and `response.body` raises
/// `response_truncated` naming the URL and the bytes received and expected
/// (ADR 0118).
fn cut_body(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-request-cut";
    let cuts = [
        (
            "a body shorter than its `Content-Length`",
            "/cut/content-length",
            true,
        ),
        (
            "a chunked body without its last chunk",
            "/cut/chunked",
            false,
        ),
        (
            "a gzip body cut before its `Content-Length`",
            "/cut/gzip-content-length",
            true,
        ),
    ]
    .map(|(what, path, declared)| (what, site.named_url("fetch-edge.test", path), declared));
    let urls: Vec<&Option<String>> = cuts.iter().map(|(_, url, _)| url).collect();
    let Some(page) = open(connection, site, run_id, &urls, checks)? else {
        return Ok(());
    };
    for (what, url, declared) in &cuts {
        let url = url.clone().unwrap_or_default();
        let answer = request(connection, &page, &json!({ "url": url }))?;
        checks.check(
            format!("{what}: `page.request` answers its 200"),
            answer["result"]["status"] == json!(200),
        );
        let body = body_of(connection, &page, &answer)?;
        let data = body
            .as_ref()
            .err()
            .map_or(&Value::Null, |failed| &failed["error"]["data"]);
        checks.check(
            format!(
                "{what}: `response.body` raises `{}` naming the URL, never the body as whole",
                RESPONSE_TRUNCATED.name
            ),
            data["code"] == json!(RESPONSE_TRUNCATED.name) && data["url"] == json!(url),
        );
        let (received, expected) = (data["received"].as_u64(), data["expected"].as_u64());
        checks.check(
            format!("{what}: it names the bytes received and those expected"),
            if *declared {
                matches!((received, expected), (Some(received), Some(expected)) if received < expected)
            } else {
                received.is_some() && data["expected"].is_null()
            },
        );
    }
    finish_run(connection, run_id)
}

/// 日本語 in `Shift_JIS`.
const SHIFT_JIS: [u8; 6] = [0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA];

/// A body's bytes are kept as the site sent them: a `Shift_JIS` text, never
/// re-encoded, so it reads in the encoding its response names, as `text()`
/// does; JSON after a UTF-8 BOM, which reads once the BOM is passed over, as
/// `json()` does; and a document, which names no decoded encoding, since no
/// page shows it (ADR 0119).
fn kept_bytes(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-request-bytes";
    let text = site.named_url("fetch-edge.test", "/text/shift-jis");
    let json = site.named_url("fetch-edge.test", "/json/bom");
    let document = url_on(site, "encodings/1", "/pages/shift-jis");
    let Some(page) = open(connection, site, run_id, &[&text, &json, &document], checks)? else {
        return Ok(());
    };
    let text = request(connection, &page, &json!({ "url": text }))?;
    checks.check(
        "the Shift_JIS text's response names its charset",
        text["result"]["headers"]["content-type"] == json!("text/plain; charset=Shift_JIS"),
    );
    let text_body = body_of(connection, &page, &text)?;
    checks.check(
        "its body holds the Shift_JIS bytes the site sent, never re-encoded",
        text_body.as_deref() == Ok(SHIFT_JIS.as_slice()),
    );
    let json = request(connection, &page, &json!({ "url": json }))?;
    let json_body = body_of(connection, &page, &json)?;
    checks.check(
        "the JSON body keeps its BOM and reads as JSON past it",
        json_body.as_deref().is_ok_and(|bytes| {
            bytes
                .strip_prefix(b"\xEF\xBB\xBF".as_slice())
                .and_then(|json| serde_json::from_slice::<Value>(json).ok())
                == Some(json!({ "bom": true }))
        }),
    );
    let document = request(connection, &page, &json!({ "url": document }))?;
    checks.check(
        "a document's response names no decoded encoding, since no page shows it",
        document["result"]["status"] == json!(200)
            && document["result"]
                .get("encoding")
                .is_none_or(Value::is_null),
    );
    finish_run(connection, run_id)
}

/// A body whose listed content coding fails to decode raises
/// `navigation_failed` with the browser's `net::ERR_CONTENT_DECODING_FAILED`,
/// a plain body labelled gzip and an invalid gzip header alike (ADRs 0118
/// and 0119).
fn failed_coding(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-request-codings";
    let plain_as_gzip = url_on(site, "content-coding/1", "/page")
        .as_deref()
        .map(plain_http);
    let invalid_header = url_on(site, "content-coding/2", "/page")
        .as_deref()
        .map(plain_http);
    let Some(page) = open(
        connection,
        site,
        run_id,
        &[&plain_as_gzip, &invalid_header],
        checks,
    )?
    else {
        return Ok(());
    };
    for (what, url) in [
        (
            "`page.request` of a plain body labelled gzip",
            plain_as_gzip,
        ),
        (
            "`page.request` of a body whose gzip header is invalid",
            invalid_header,
        ),
    ] {
        let answer = request(connection, &page, &json!({ "url": url }))?;
        failed_with(&answer, what, DECODING_FAILED, checks);
    }
    finish_run(connection, run_id)
}

/// The network error of a body whose listed coding does not decode.
const DECODING_FAILED: &str = "net::ERR_CONTENT_DECODING_FAILED";

/// A task's URL over plain HTTP/1.1, which the browser reaches without
/// trusting the Proving ground's authority.
fn plain_http(url: &str) -> String {
    url.replacen("https://", "http://", 1)
}

/// Opens a Session of the Run `run_id` and a page in it, answering the
/// page's ID.
fn session_page(
    connection: &mut Connection,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    let id = connection.request(RUN_NEW_SESSION.name, &json!({ "run_id": run_id }))?;
    let opened = connection.browser_answer(id)?.message;
    let session = opened["result"]["session_id"].clone();
    let id = connection.request(SESSION_NEW_PAGE.name, &json!({ "session_id": session }))?;
    let paged = connection.browser_answer(id)?.message;
    let page = paged["result"]["page_id"].clone();
    checks.check(
        format!("`{}` opens a page of a new Session", SESSION_NEW_PAGE.name),
        session.is_string() && page.is_string(),
    );
    Ok(page.is_string().then_some(page))
}

/// What the Proving ground received for each request to `host` whose
/// target starts with `prefix`, in arrival order: its target and the
/// cookies it sent.
fn received(site: &Site, host: &str, prefix: &str) -> Vec<(Value, Value)> {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| {
            request["host"] == json!(host)
                && request["target"]
                    .as_str()
                    .is_some_and(|target| target.starts_with(prefix))
        })
        .map(|request| (request["target"].clone(), request["cookies"].clone()))
        .collect()
}

/// Pitfall H20: a cookie a page sets is sent by `page.request` and by the
/// next navigation in the same Session, and by no other Session of the
/// Run; a cookie `page.request`'s response sets is the Session's too, sent
/// by its next navigation, as the Proving ground's record of the cookies
/// each request sent shows.
fn cookies(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-request-cookies";
    let url = |path: &str| site.named_url("scope.test", path).unwrap_or_default();
    start_run(connection, site, run_id, &json!({}), checks)?;
    let first = session_page(connection, run_id, checks)?;
    let second = session_page(connection, run_id, checks)?;
    let (Some(first), Some(second)) = (first, second) else {
        return finish_run(connection, run_id);
    };
    call(
        connection,
        PAGE_GOTO.name,
        &first,
        &json!({ "url": url("/storage?mark=first") }),
    )?;
    let sent = request(
        connection,
        &first,
        &json!({ "url": url("/about?by=first-request") }),
    )?;
    call(
        connection,
        PAGE_GOTO.name,
        &first,
        &json!({ "url": url("/about?by=first-navigation") }),
    )?;
    request(
        connection,
        &second,
        &json!({ "url": url("/about?by=second-request") }),
    )?;
    request(
        connection,
        &second,
        &json!({ "url": url("/storage?mark=second") }),
    )?;
    call(
        connection,
        PAGE_GOTO.name,
        &second,
        &json!({ "url": url("/about?by=second-navigation") }),
    )?;
    checks.check(
        "`page.request` from the Session answers 200",
        sent["result"]["status"] == json!(200),
    );
    let mark = |mark: &str| json!([["mark", mark]]);
    checks.check(
        "the cookie a page set is sent by `page.request` and the next navigation of its Session, never by another Session, which keeps the cookie its own `page.request` got",
        received(site, "scope.test", "/about?by=")
            == vec![
                (json!("/about?by=first-request"), mark("first")),
                (json!("/about?by=first-navigation"), mark("first")),
                (json!("/about?by=second-request"), json!([])),
                (json!("/about?by=second-navigation"), mark("second")),
            ],
    );
    finish_run(connection, run_id)
}

/// `page.request` passes the page's interception: a host the Run's domain
/// policy refuses raises `egress_refused`, and the site never receives it.
fn domain_policy(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-request-domain-policy";
    let allowed = site.named_url("fetch-edge.test", "/target");
    let refused = site.named_url("scope.test", "/about?by=refused-request");
    let settings = json!({ "egress": { "allowed_domains": ["fetch-edge.test"] } });
    start_run(connection, site, run_id, &settings, checks)?;
    let Some(page) = new_page(connection, run_id, checks)?.map(|page| json!(page.page_id)) else {
        return finish_run(connection, run_id);
    };
    let answered = request(connection, &page, &json!({ "url": allowed }))?;
    checks.check(
        "`page.request` to a host the domain policy allows answers 200",
        answered["result"]["status"] == json!(200),
    );
    let stopped = request(connection, &page, &json!({ "url": refused }))?;
    checks.check(
        format!(
            "`page.request` to a host the domain policy refuses raises `{}`",
            EGRESS_REFUSED.name
        ),
        code_of(&stopped) == &json!(EGRESS_REFUSED.name),
    );
    checks.check(
        "the refused request never reaches the site",
        received(site, "scope.test", "/about?by=refused-request").is_empty(),
    );
    finish_run(connection, run_id)
}

/// No service worker answers `page.request`: on a page a worker controls,
/// which answers `cached.json` from its own cache, the request reaches the
/// site and answers the site's body (ADR 0121).
fn service_worker(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-request-service-worker";
    let path = "/service-worker/?case=page-request";
    let origin = format!("http://{IDENTITY_HOST}:{}", site.port);
    let cached = "/service-worker/cached.json?case=page-request";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)?.map(|page| json!(page.page_id)) else {
        return finish_run(connection, run_id);
    };
    call(
        connection,
        PAGE_GOTO.name,
        &page,
        &json!({ "url": format!("{origin}{path}") }),
    )?;
    let reports = reports_once(site, path, |seen| {
        seen.iter().any(|(check, _)| check == "controlled")
    })
    .unwrap_or_default();
    checks.check(
        "a service worker registers and controls the page",
        reports.contains(&("register".to_owned(), "registered".to_owned()))
            && reports.contains(&("controlled".to_owned(), "true".to_owned())),
    );
    checks.check(
        "the worker answered the page's own fetch of `cached.json`, which never reached the site",
        received(site, IDENTITY_HOST, cached).is_empty(),
    );
    let answer = request(
        connection,
        &page,
        &json!({ "url": format!("{origin}{cached}") }),
    )?;
    checks.check(
        "`page.request` of `cached.json` answers the site's response, which no service worker supplied",
        answer["result"]["status"] == json!(200)
            && answer["result"]["from_service_worker"] == json!(false),
    );
    let body = body_of(connection, &page, &answer)?;
    checks.check(
        "its body is the site's, never the worker's cached one",
        body.ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            == Some(json!({ "path": "/service-worker/cached.json" })),
    );
    checks.check(
        "the request reached the site",
        received(site, IDENTITY_HOST, cached).len() == 1,
    );
    finish_run(connection, run_id)
}
