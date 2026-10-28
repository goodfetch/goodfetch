//! The `access` family on `access.test`: the first rung of ADR 0035's Access
//! family, on the local copy with no outside network. Two pages one Session
//! fetches, over HTTP and in the browser, whose fingerprints the server
//! compares; a self-hosted bot probe in the manner of `BotD` and `CreepJS`; a
//! probe of the debugging protocol's side effects; the version check; the
//! block-detection corpus; and pages no task names, which the engine's own
//! tests read: the identity a page reads of its browser, every client hint
//! asked for, and a 100 ms timer that reports whether a page behind another
//! tab kept it (spec #577, ticket #696); a page's events (#703); a page
//! a service worker controls (#707); and a page that opens each JavaScript
//! dialog and one that opens a popup (#719); and a page whose own script
//! defines a global and records an init script's mark (#704); and one page
//! per kind of request a page makes, which makes one of that kind to the
//! host it names (#632). Its answers name the build's age beside the
//! Access dimensions a result was run under, so every result is reported per
//! Fingerprint method, engine, Access mode and build age, against stock
//! headless and headful Chrome as controls.

mod consistency;
mod corpus;
mod versions;

use hyper::{Method, StatusCode};
use serde::Serialize;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::observation::{self, Observations, Recorded, Report};
use crate::oracle::{self, Answer, Finding};
use crate::push::Push;
use crate::task::{self, Entry, Expected, Kind, Record, Task};
use versions::BuildAge;

/// The host name the family is served on.
pub(crate) const HOST: &str = "access.test";

/// A host name the family also answers on, which a browser treats as a
/// secure context over plain HTTP, resolving every `*.localhost` name to
/// loopback itself: there `navigator.userAgentData` and the client hints
/// exist, which the identity page reads and asks for.
const LOCAL_HOST: &str = "access.localhost";

pub(crate) const FAMILY: &str = "access";

/// How many tasks the family has.
const TASKS: u32 = 5;

const CONSISTENCY_TASK: &str = "access/1";
const BOT_PROBE_TASK: &str = "access/2";
const CDP_PROBE_TASK: &str = "access/3";
const VERSION_TASK: &str = "access/4";

/// The pages the consistency task's Session fetches.
const CONSISTENCY_PAGES: [&str; 2] = ["/consistency/1", "/consistency/2"];
const BOT_PROBE: &str = "/probe";
const CDP_PROBE: &str = "/cdp-probe";
const VERSION: &str = "/version";
const IDENTITY: &str = "/identity";
const TIMER: &str = "/timer";
const PAGE_EVENTS: &str = "/page-events";
/// The JSON body the page events page requests, also at the path a
/// program may abort.
const PAGE_EVENTS_DATA: [&str; 2] = ["/page-events/data.json", "/page-events/aborted"];
/// The WebSocket the page events page opens.
const PAGE_EVENTS_SOCKET: &str = "/page-events/socket";
/// The dialogs page, the popup page and the page its popup loads.
const DIALOGS: &str = "/dialogs";
const POPUP: &str = "/popup";
const POPUP_OPENED: &str = "/popup/opened";
/// The page functions page.
const PAGE_FUNCTIONS: &str = "/page-functions";
/// The service worker page and its worker's script.
const SERVICE_WORKER: &str = "/service-worker/";
const SERVICE_WORKER_SCRIPT: &str = "/service-worker/sw.js";
/// The JSON calls the service worker page and its worker make.
const SERVICE_WORKER_CALLS: [&str; 3] = [
    "/service-worker/own.json",
    "/service-worker/cached.json",
    "/service-worker/network.json",
];
/// The path the service worker requests that EasyPrivacy blocks.
const SERVICE_WORKER_TRACKER: &str = "/service-worker/js/adobe-analytics.js";
/// The request the service worker makes, when its page asks, that the
/// Proving ground holds in flight [`SERVICE_WORKER_HOLD`], far past
/// `networkidle`'s 500 ms of quiet, so a wait that counted a worker's own
/// requests would not end while it is held (#707).
const SERVICE_WORKER_HELD: &str = "/service-worker-held.json";
/// How long [`SERVICE_WORKER_HELD`] is held.
const SERVICE_WORKER_HOLD: std::time::Duration = std::time::Duration::from_secs(60);
/// The request kinds pages, one per kind under this prefix, and their
/// worker's script, service worker's script and cross-site frame.
const REQUEST_KINDS: &str = "/request-kinds/";
const REQUEST_KINDS_WORKER: &str = "/request-kinds/worker.js";
const REQUEST_KINDS_SERVICE_WORKER: &str = "/request-kinds/sw.js";
const REQUEST_KINDS_FRAME: &str = "/request-kinds/frame-document";
/// The kinds of request the request kinds pages make (ADR 0014).
const REQUEST_KIND_NAMES: [&str; 8] = [
    "fetch",
    "navigation",
    "frame",
    "dedicated_worker",
    "shared_worker",
    "service_worker",
    "websocket",
    "webrtc",
];

/// The client hints the identity page asks for, beyond those a browser
/// sends unasked, so the requests its script makes carry every one.
const CLIENT_HINTS: &str = "Sec-CH-UA-Full-Version-List, Sec-CH-UA-Platform-Version, Sec-CH-UA-Arch, Sec-CH-UA-Bitness, Sec-CH-UA-Model, Sec-CH-UA-WoW64, Sec-CH-UA-Form-Factors";

/// The Record the version check's result is.
const VERSION_RECORD: &str = "version-check";

const BOT_PROBE_SCRIPT: &str = include_str!("../pages/access/bot-probe.js");
const CDP_PROBE_SCRIPT: &str = include_str!("../pages/access/cdp-probe.js");
const IDENTITY_SCRIPT: &str = include_str!("../pages/access/identity.js");
const TIMER_SCRIPT: &str = include_str!("../pages/access/timer.js");
const PAGE_EVENTS_SCRIPT: &str = include_str!("../pages/access/page-events.js");
const DIALOGS_SCRIPT: &str = include_str!("../pages/access/dialogs.js");
const POPUP_SCRIPT: &str = include_str!("../pages/access/popup.js");
const PAGE_FUNCTIONS_SCRIPT: &str = include_str!("../pages/access/page-functions.js");
const SERVICE_WORKER_PAGE_SCRIPT: &str = include_str!("../pages/access/service-worker.js");
const SERVICE_WORKER_WORKER_SCRIPT: &str = include_str!("../pages/access/service-worker-sw.js");
const REQUEST_KINDS_SCRIPT: &str = include_str!("../pages/access/request-kinds.js");
const REQUEST_KINDS_WORKER_SCRIPT: &str = include_str!("../pages/access/request-kinds-worker.js");
const REQUEST_KINDS_SERVICE_WORKER_SCRIPT: &str =
    include_str!("../pages/access/request-kinds-sw.js");
const REQUEST_KINDS_FRAME_SCRIPT: &str = include_str!("../pages/access/request-kinds-frame.js");

/// The generator places of the family's values: the family's own first, then
/// 0 and the value's.
const FAMILY_PLACE: u64 = 4;
const CODE_PLACE: u64 = 1;

/// Every host name the family is served on, each with its task.
pub(crate) fn hosts() -> Vec<(String, Option<String>)> {
    let mut hosts = task::family_hosts(FAMILY, HOST, TASKS);
    hosts.push((LOCAL_HOST.to_owned(), None));
    hosts
}

/// The code a page of the family shows beside its Record's ID, from the
/// seed, so a Record proves its page was read.
fn code(seed: u64, record: &str) -> String {
    let place = record.bytes().fold(0_u64, |place, byte| {
        place.wrapping_mul(131).wrapping_add(u64::from(byte))
    });
    format!(
        "{:08x}",
        generator::below(seed, &[FAMILY_PLACE, 0, CODE_PLACE, place], 1 << 32)
    )
}

/// A Record of the family: its ID and its page's code.
fn record(seed: u64, id: &str) -> Record {
    Record::from([("id", id.to_owned()), ("code", code(seed, id))])
}

/// Answers a request to one of the family's host names: its page and what
/// the site records of it, if anything; `None` when `host` is not the
/// family's.
pub(crate) fn respond(
    seed: u64,
    host: &str,
    request: &Request<'_>,
) -> Option<(Reply, Option<Recorded>)> {
    if host != LOCAL_HOST && !task::serves(host, HOST, TASKS) {
        return None;
    }
    let path = request.path;
    Some(match *request.method {
        Method::GET if path == CONSISTENCY_PAGES[0] => (
            Reply::html(page(
                seed,
                "consistency-1",
                &format!(
                    "<a class=\"next\" href=\"{}\">The next page</a>\n",
                    CONSISTENCY_PAGES[1]
                ),
            )),
            None,
        ),
        Method::GET if path == CONSISTENCY_PAGES[1] => {
            (Reply::html(page(seed, "consistency-2", "")), None)
        }
        Method::GET if path == BOT_PROBE => (
            Reply::html(page(
                seed,
                "probe",
                &probe_script("probe", BOT_PROBE_SCRIPT),
            )),
            None,
        ),
        Method::GET if path == CDP_PROBE => (
            Reply::html(page(
                seed,
                "cdp-probe",
                &probe_script("cdp", CDP_PROBE_SCRIPT),
            )),
            None,
        ),
        Method::GET if path == VERSION => (Reply::html(version_page()), None),
        Method::GET if path == IDENTITY => {
            let mut reply = Reply::html(page(seed, "identity", &reporting(IDENTITY_SCRIPT)));
            reply.headers.push(("accept-ch", CLIENT_HINTS.to_owned()));
            (reply, None)
        }
        Method::GET if path == TIMER => (
            Reply::html(page(seed, "timer", &reporting(TIMER_SCRIPT))),
            None,
        ),
        Method::GET if path == PAGE_EVENTS => {
            (scripted(seed, "page-events", PAGE_EVENTS_SCRIPT), None)
        }
        Method::GET if PAGE_EVENTS_DATA.contains(&path) => (
            Reply::json(
                StatusCode::OK,
                &serde_json::json!({ "page_events": code(seed, "page-events") }),
            ),
            None,
        ),
        Method::GET if path == DIALOGS => (
            Reply::html(page(seed, "dialogs", &reporting(DIALOGS_SCRIPT))),
            None,
        ),
        Method::GET if path == POPUP => (scripted(seed, "popup", POPUP_SCRIPT), None),
        Method::GET if path == PAGE_FUNCTIONS => (
            scripted(seed, "page-functions", PAGE_FUNCTIONS_SCRIPT),
            None,
        ),
        Method::GET if path == POPUP_OPENED => (Reply::html(page(seed, "popup-opened", "")), None),
        Method::GET if path == SERVICE_WORKER => (
            Reply::html(page(
                seed,
                "service-worker",
                &reporting(SERVICE_WORKER_PAGE_SCRIPT),
            )),
            None,
        ),
        Method::GET if path == SERVICE_WORKER_SCRIPT => {
            (script_reply(SERVICE_WORKER_WORKER_SCRIPT), None)
        }
        Method::GET if SERVICE_WORKER_CALLS.contains(&path) => (
            Reply::json(StatusCode::OK, &serde_json::json!({ "path": path })),
            None,
        ),
        Method::GET if path == SERVICE_WORKER_TRACKER => (script_reply("void 0;\n"), None),
        Method::GET if path == SERVICE_WORKER_HELD => (
            Reply::json(StatusCode::OK, &serde_json::json!({ "held": true })),
            None,
        ),
        Method::GET if let Some(reply) = request_kinds(seed, path) => (reply, None),
        Method::GET => (corpus::page(path).unwrap_or_else(Reply::not_found), None),
        Method::POST if path == "/reports" => match Report::parse(request.body) {
            Some(report) => (Reply::no_content(), Some(Recorded::Report(report))),
            None => (
                Reply::error(StatusCode::BAD_REQUEST, "malformed_report"),
                None,
            ),
        },
        _ => (Reply::not_found(), None),
    })
}

/// A request kinds page, its worker's or service worker's script or its
/// frame, for `path`.
fn request_kinds(seed: u64, path: &str) -> Option<Reply> {
    if path == REQUEST_KINDS_WORKER {
        return Some(script_reply(REQUEST_KINDS_WORKER_SCRIPT));
    }
    if path == REQUEST_KINDS_SERVICE_WORKER {
        return Some(script_reply(REQUEST_KINDS_SERVICE_WORKER_SCRIPT));
    }
    if path == REQUEST_KINDS_FRAME {
        return Some(scripted(
            seed,
            "request-kinds-frame",
            REQUEST_KINDS_FRAME_SCRIPT,
        ));
    }
    path.strip_prefix(REQUEST_KINDS)
        .is_some_and(|kind| REQUEST_KIND_NAMES.contains(&kind))
        .then(|| scripted(seed, "request-kinds", REQUEST_KINDS_SCRIPT))
}

/// The connection a request to `name` and `path` asks the server to hold
/// open: the page events page's WebSocket.
pub(crate) fn push(name: &str, method: &Method, path: &str) -> Option<Push> {
    (*method == Method::GET
        && path == PAGE_EVENTS_SOCKET
        && (name == LOCAL_HOST || task::serves(name, HOST, TASKS)))
    .then_some(Push::Socket)
}

/// How long the server holds a request for `path` on `name` before it
/// answers, if it holds it: the service worker's held request.
pub(crate) fn hold(name: &str, path: &str) -> Option<std::time::Duration> {
    (path == SERVICE_WORKER_HELD && (name == LOCAL_HOST || task::serves(name, HOST, TASKS)))
        .then_some(SERVICE_WORKER_HOLD)
}

/// `script`, served as JavaScript.
fn script_reply(script: &'static str) -> Reply {
    Reply {
        status: StatusCode::OK,
        content_type: "text/javascript; charset=utf-8",
        headers: Vec::new(),
        body: script.into(),
    }
}

/// A page of the family: its Record, and `more` after it.
fn page(seed: u64, id: &str, more: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{id}</title></head>\n<body>\n<main>\n<article class=\"record\" data-id=\"{id}\">\n<p>Code: <span class=\"code\">{}</span></p>\n</article>\n{more}</main>\n</body>\n</html>\n",
        code(seed, id)
    )
}

/// A page of the family whose script is `script`, inline.
fn scripted(seed: u64, id: &str, script: &str) -> Reply {
    Reply::html(page(seed, id, &format!("<script>\n{script}</script>\n")))
}

/// A probe's script, which reports to its host under `check`.
fn probe_script(check: &str, script: &str) -> String {
    format!("<script data-reports=\"/reports\" data-check=\"{check}\">\n{script}</script>\n")
}

/// A script that reports to its host under checks of its own.
fn reporting(script: &str) -> String {
    format!("<script data-reports=\"/reports\">\n{script}</script>\n")
}

/// The version check's page: what the tool is asked to report, and the date
/// it is graded at.
fn version_page() -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Version check</title></head>\n<body>\n<main>\n<h1>Version check</h1>\n<p>Report the Chrome major this browser claims and the version check's level for it, as of <time datetime=\"{as_of}\">{as_of}</time>.</p>\n</main>\n</body>\n</html>\n",
        as_of = versions::AS_OF
    )
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let origin = |ordinal| task::origin(&task::task_host(ordinal, HOST), port);
    let records = |ids: &[&str]| Expected::records(ids.iter().map(|id| record(seed, id)).collect());
    let task = |ordinal, kind, path: &str, description, expected| Task {
        host: task::task_host(ordinal, HOST),
        entry: Entry {
            account: None,
            time_steps: None,
            id: format!("{FAMILY}/{ordinal}"),
            family: FAMILY,
            kind,
            start_url: format!("{}{path}", origin(ordinal)),
            description,
        },
        expected,
    };
    vec![
        task(
            1,
            Kind::Extract,
            CONSISTENCY_PAGES[0],
            "Fetch this page over HTTP and the page it links to in the browser, in one \
                Session, and return each page's Record with the fields `id` and `code`. The \
                server compares the fingerprints of the two requests.",
            records(&["consistency-1", "consistency-2"]),
        ),
        task(
            2,
            Kind::Extract,
            BOT_PROBE,
            "Open this page in the browser and extract its Record with the fields `id` and \
                `code`. Its script reports the signals a bot detector reads.",
            records(&["probe"]),
        ),
        task(
            3,
            Kind::Extract,
            CDP_PROBE,
            "Open this page in the browser and extract its Record with the fields `id` and \
                `code`. Its script reports the side effects of driving the browser over its \
                debugging protocol.",
            records(&["cdp-probe"]),
        ),
        task(
            4,
            Kind::Extract,
            VERSION,
            "Open this page and return one Record with the fields `id` (`version-check`), \
                `major` (the Chrome major the browser claims) and `level` (`current`, `info`, \
                `notice` or `warning`, the version check's level as of the page's date). The \
                expected Record is graded from the version the request to this page claims.",
            Expected::default(),
        ),
        task(
            5,
            Kind::Crawl,
            corpus::INDEX,
            "Fetch each page this index links and return one Record per page with the fields \
                `id` (the page's path) and `cause` (its Block cause: `challenge`, `captcha`, \
                `rate_limit`, `ip_block`, `decoy`, `payment_required`, or `none` when the page \
                is no block).",
            Expected::records(
                corpus::causes()
                    .map(|(path, cause)| {
                        Record::from([("id", path.to_owned()), ("cause", cause.to_owned())])
                    })
                    .collect(),
            ),
        ),
    ]
}

/// The version check's expected result now: its Record graded from the
/// Chrome major the first request to its page claimed; `None` for another
/// task, or before such a request.
pub(crate) fn expected_now(task: &str, observations: &Observations) -> Option<Expected> {
    if task != VERSION_TASK {
        return None;
    }
    let major = claimed_majors(observations, task, VERSION)
        .into_iter()
        .next()?;
    Some(Expected::records(vec![Record::from([
        ("id", VERSION_RECORD.to_owned()),
        ("major", major.to_string()),
        ("level", versions::age(major).level.as_str().to_owned()),
    ])]))
}

/// The Chrome major each request to `task`'s `page` claimed, in order.
fn claimed_majors(observations: &Observations, task: &str, page: &str) -> Vec<u32> {
    observations
        .requests_of(task)
        .iter()
        .filter(|seen| seen.target == page)
        .filter_map(|seen| seen.fingerprint.user_agent.as_deref())
        .filter_map(versions::claimed_major)
        .collect()
}

/// The oracle's answer for the family: the answer, and the age of the build
/// the submission named.
#[derive(Serialize)]
pub(crate) struct AccessAnswer {
    #[serde(flatten)]
    answer: Answer,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_age: Option<BuildAge>,
}

/// Judges the family's `answer` for `task` from the Proving ground's own
/// record: the fingerprints of the consistency task's requests, whether a
/// probe's script ran, the user agent the bot probe was fetched with, and
/// whether the version check's page was fetched at all.
pub(crate) fn judge(task: &str, answer: Answer, observations: &Observations) -> AccessAnswer {
    let mut findings = Vec::new();
    match task {
        CONSISTENCY_TASK => {
            let requests = observations.requests_of(task);
            let pages: Vec<_> = requests
                .iter()
                .filter(|seen| CONSISTENCY_PAGES.contains(&seen.target.as_str()))
                .collect();
            findings.extend(consistency::findings(&pages));
        }
        BOT_PROBE_TASK | CDP_PROBE_TASK => {
            let page = if task == BOT_PROBE_TASK {
                BOT_PROBE
            } else {
                CDP_PROBE
            };
            let ran = observations
                .reports_of(task)
                .iter()
                .any(|report| report.check == observation::RAN && report.page == page);
            if !ran {
                findings.push(Finding {
                    check: "probe_not_run".to_owned(),
                    at: page.to_owned(),
                    expected: None,
                    observed: serde_json::Value::Null,
                });
            }
            if task == BOT_PROBE_TASK {
                let headless = observations.requests_of(task).iter().any(|seen| {
                    seen.target == page
                        && seen
                            .fingerprint
                            .user_agent
                            .as_deref()
                            .is_some_and(|agent| agent.contains("HeadlessChrome"))
                });
                if headless {
                    findings.push(Finding {
                        check: "probe".to_owned(),
                        at: page.to_owned(),
                        expected: None,
                        observed: "headless_user_agent".into(),
                    });
                }
            }
        }
        VERSION_TASK if claimed_majors(observations, task, VERSION).is_empty() => {
            findings.push(Finding {
                check: "version_not_claimed".to_owned(),
                at: VERSION.to_owned(),
                expected: None,
                observed: serde_json::Value::Null,
            });
        }
        _ => {}
    }
    let build_age = answer
        .access()
        .and_then(oracle::AccessDimensions::chromium_major)
        .map(versions::age);
    AccessAnswer {
        answer: answer.refuted_by(findings),
        build_age,
    }
}
