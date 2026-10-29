//! The page suite's page events and bodies cases (ADRs 0081, 0085 and 0118,
//! ticket #703), on the Proving ground's page events page, which writes to
//! its console, requests a body, a missing page, a path a route may abort,
//! a page on another site and a host that does not resolve, opens a
//! WebSocket and throws an exception it does not catch: `page.on` listens
//! to each event of the page, which the event stream carries under its
//! credits, and a listener whose call is cancelled no longer fires; a
//! blocked, an aborted and a failed request each fail as Playwright reports
//! them; a body `page.wait_for_request_finished` kept reads whole, a
//! dropped one raises `body_unavailable` and a cut one `response_truncated`,
//! and a read of a response that never ends `deadline_exceeded` at its
//! `timeout`.

use std::collections::BTreeMap;

use goodfetch_protocol::connection::{CANCEL, DEADLINE_EXCEEDED};
use goodfetch_protocol::events::{CREDIT, EVENTS, EVENTS_SUBSCRIBE};
use goodfetch_protocol::page::events::{
    BODY_UNAVAILABLE, PAGE_ON, PAGE_WAIT_FOR_REQUEST_FINISHED, RESPONSE_BODY,
};
use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_ABORT, ROUTE_HANDLE};
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_RESPONSE;
use goodfetch_protocol::page::{PAGE_GOTO, PAGE_RELOAD, RESPONSE_TRUNCATED};
use goodfetch_protocol::settings::TIMEOUT_WAIT;
use serde_json::{Map, Value, json};

use super::navigation::url_on;
use super::{Checks, Site, finish_run, host_of, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::BROWSER_DEADLINE;

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    NetworkAndLoadEvents,
    ConsoleAndPageErrors,
    FailedRequests,
    Credits,
    Bodies,
    BodyTimeout,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-pages-network-and-load-events-carry-their-data-and-a-cancelled-listener-no-longer-fires" => {
                Self::NetworkAndLoadEvents
            }
            "console-messages-and-uncaught-exceptions-carry-their-data-from-when-a-listener-is-armed" => {
                Self::ConsoleAndPageErrors
            }
            "a-blocked-an-aborted-and-a-failed-request-each-fail-as-playwright-reports-them" => {
                Self::FailedRequests
            }
            "a-slow-reader-holds-the-page-events-stream-one-event-per-credit" => Self::Credits,
            "a-kept-body-reads-whole-a-dropped-one-is-unavailable-and-a-cut-one-is-truncated" => {
                Self::Bodies
            }
            "a-body-read-of-a-response-that-never-ends-raises-deadline-exceeded-at-its-timeout" => {
                Self::BodyTimeout
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::NetworkAndLoadEvents => network_and_load(connection, site, checks),
            Self::ConsoleAndPageErrors => console_and_page_errors(connection, site, checks),
            Self::FailedRequests => failed_requests(connection, site, checks),
            Self::Credits => credits(connection, site, checks),
            Self::Bodies => bodies(connection, site, checks),
            Self::BodyTimeout => body_timeout(connection, site, checks),
        }
    }
}

/// The credits a subscription grants at once, more than a case's events.
const CREDITS: u32 = 10_000;

/// The Connection as these cases read it: the events the case's own
/// subscription delivered so far, never another case's, the answers read
/// while waiting for another, as a Client matches each answer to its call
/// by ID, and every `route.handle` request answered by aborting the
/// request, as a handler calling `route.abort()` does.
struct Exchange<'connection> {
    connection: &'connection mut Connection,
    events: Vec<Value>,
    /// The answers read before their call was waited for, by its ID.
    answers: BTreeMap<i64, Value>,
    /// The case's subscription's call.
    subscription: Option<i64>,
}

impl<'connection> Exchange<'connection> {
    fn new(connection: &'connection mut Connection) -> Self {
        Self {
            connection,
            events: Vec::new(),
            answers: BTreeMap::new(),
            subscription: None,
        }
    }

    /// Takes in one message: a batch's events, a route's request, an
    /// answer, kept for [`answer`](Self::answer), or else the message.
    fn read(&mut self) -> Result<Option<Value>, Gone> {
        let message = self.connection.next_within(BROWSER_DEADLINE)?;
        if message["method"] == json!(ROUTE_HANDLE.name) {
            let route_id = message["params"]["route_id"].clone();
            self.connection
                .request(ROUTE_ABORT.name, &json!({ "route_id": route_id }))?;
            self.connection.reply(&message["id"], &json!({}))?;
            return Ok(None);
        }
        if message["method"] == json!(EVENTS) {
            if self
                .subscription
                .is_none_or(|id| message["params"]["id"] != json!(id))
            {
                return Ok(None);
            }
            self.events.extend(
                message["params"]["events"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
            return Ok(None);
        }
        if message.get("method").is_none()
            && let Some(id) = message["id"].as_i64()
        {
            self.answers.insert(id, message);
            return Ok(None);
        }
        Ok(Some(message))
    }

    /// The answer to the request `id`.
    fn answer(&mut self, id: i64) -> Result<Value, Gone> {
        loop {
            if let Some(answer) = self.answers.remove(&id) {
                return Ok(answer);
            }
            self.read()?;
        }
    }

    fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        let id = self.connection.request(method, params)?;
        self.answer(id)
    }

    /// Reads until an event `name` whose data `holds` arrived.
    fn until(&mut self, name: &str, holds: impl Fn(&Value) -> bool) -> Result<(), Gone> {
        while !self
            .events
            .iter()
            .any(|event| event["event"] == json!(name) && holds(&event["data"]))
        {
            self.read()?;
        }
        Ok(())
    }

    /// The data of every event `name` read so far.
    fn data(&self, name: &str) -> Vec<Value> {
        self.events
            .iter()
            .filter(|event| event["event"] == json!(name))
            .map(|event| event["data"].clone())
            .collect()
    }

    /// Subscribes to the event stream, granting `credits` at once.
    fn subscribe(&mut self, credits: u32) -> Result<i64, Gone> {
        let id = self.connection.request(EVENTS_SUBSCRIBE.name, &json!({}))?;
        self.subscription = Some(id);
        if credits > 0 {
            self.connection
                .notify(CREDIT, &json!({ "id": id, "credits": credits }))?;
        }
        Ok(id)
    }

    /// Cancels the case's subscription and finishes its Run `run_id`, whose
    /// pages' listeners then answer.
    fn end(self, run_id: &str) -> Result<(), Gone> {
        if let Some(id) = self.subscription {
            self.connection.notify(CANCEL, &json!({ "id": id }))?;
            self.connection.answer(id)?;
        }
        finish_run(self.connection, run_id)
    }

    /// Listens to `event` of `page`, answering the listening call's id.
    fn listen(&mut self, page: &Value, event: &str) -> Result<i64, Gone> {
        self.connection
            .request(PAGE_ON.name, &json!({ "page_id": page, "event": event }))
    }
}

/// Starts the Run `run_id` with `settings` and opens a page on it, when the
/// Proving ground serves the page events page.
fn opened(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<Option<(Value, String)>, Gone> {
    let url = url_on(site, "access/1", "/page-events");
    checks.check(
        "the Proving ground serves the page events page",
        url.is_some(),
    );
    let Some(url) = url else {
        return Ok(None);
    };
    start_run(connection, site, run_id, settings, checks)?;
    Ok(new_page(connection, run_id, checks)?.map(|page| (json!(page.page_id), url)))
}

fn url_of(data: &Value) -> &str {
    data["url"].as_str().unwrap_or_default()
}

/// Every network event and `load` and `domcontentloaded` carry ADR 0085's
/// data, and a listener whose call is cancelled no longer fires.
fn network_and_load(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-events";
    let Some((page, url)) = opened(connection, site, run_id, &json!({}), checks)? else {
        return Ok(());
    };
    let mut exchange = Exchange::new(connection);
    exchange.subscribe(CREDITS)?;
    let mut request_listener = 0;
    for event in [
        "request",
        "request_finished",
        "request_failed",
        "response",
        "web_socket",
        "load",
        "dom_content_loaded",
    ] {
        let id = exchange.listen(&page, event)?;
        if event == "request" {
            request_listener = id;
        }
    }
    exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    exchange.until("web_socket.close", |_| true)?;
    let requests = exchange.data("page.request");
    let document = requests.iter().find(|request| url_of(request) == url);
    checks.check(
        "the document's request is a navigation request of the page, a `GET` of a `Document`",
        document.is_some_and(|request| {
            request["page_id"] == page
                && request["method"] == json!("GET")
                && request["resource_type"] == json!("Document")
                && request["is_navigation_request"] == json!(true)
        }),
    );
    checks.check(
        "the body's request is a `Fetch` that no redirect led to",
        requests.iter().any(|request| {
            url_of(request).ends_with("/page-events/data.json")
                && request["resource_type"] == json!("Fetch")
                && request["redirect_chain"] == json!([])
        }),
    );
    let responses = exchange.data("page.response");
    let status = |suffix: &str| {
        responses
            .iter()
            .find(|exchange| url_of(&exchange["response"]).ends_with(suffix))
            .map(|exchange| exchange["response"]["status"].clone())
    };
    checks.check(
        "the responses carry their status: the document 200, the body 200, the missing page 404",
        (
            status("/page-events"),
            status("/page-events/data.json"),
            status("/page-events/missing"),
        ) == (Some(json!(200)), Some(json!(200)), Some(json!(404))),
    );
    checks.check(
        "no response says a service worker supplied it",
        responses
            .iter()
            .all(|exchange| exchange["response"]["from_service_worker"] == json!(false)),
    );
    checks.check(
        "the body's request finished",
        exchange
            .data("page.request_finished")
            .iter()
            .any(|request| url_of(request).ends_with("/page-events/data.json")),
    );
    checks.check(
        "the request to a host that does not resolve failed with `net::ERR_NAME_NOT_RESOLVED`",
        exchange.data("page.request_failed").iter().any(|request| {
            url_of(request).contains("nowhere.invalid")
                && request["failure"]["error_text"] == json!("net::ERR_NAME_NOT_RESOLVED")
        }),
    );
    checks.check(
        "the page's WebSocket is reported with its URL, its sent frame and a received one",
        exchange.data("page.web_socket").len() == 1
            && exchange.data("web_socket.frame_sent")
                == vec![json!({
                    "page_id": page,
                    "web_socket_id": exchange.data("page.web_socket")[0]["web_socket_id"],
                    "payload": "page-events hello",
                    "binary": false,
                })]
            && !exchange.data("web_socket.frame_received").is_empty(),
    );
    checks.check(
        "the document fired `domcontentloaded` and `load`",
        exchange.data("page.dom_content_loaded") == vec![json!({ "page_id": page })]
            && exchange.data("page.load") == vec![json!({ "page_id": page })],
    );
    no_longer_fires(&mut exchange, &page, &url, request_listener, checks)?;
    exchange.end(run_id)
}

/// Cancelling the `request` listener's call `listener` removes it: the
/// page's next requests are no `page.request` events, its responses still
/// are.
fn no_longer_fires(
    exchange: &mut Exchange<'_>,
    page: &Value,
    url: &str,
    listener: i64,
    checks: &mut Checks,
) -> Result<(), Gone> {
    exchange
        .connection
        .notify(CANCEL, &json!({ "id": listener }))?;
    let cancelled = exchange.answer(listener)?;
    checks.check(
        "cancelling a listener's call ends it with `call_cancelled`",
        cancelled["error"]["data"]["code"] == json!("call_cancelled"),
    );
    exchange.events.clear();
    exchange.call(PAGE_RELOAD.name, &json!({ "page_id": page }))?;
    exchange.until("page.response", |exchange| {
        url_of(&exchange["response"]) == url
    })?;
    checks.check(
        "a reload's requests are no `page.request` events once its listener is gone, its responses still are",
        exchange.data("page.request").is_empty(),
    );
    Ok(())
}

/// Console messages carry their type, text and location, an uncaught
/// exception its name, message and stack, and a listener gets none of what
/// the page wrote before it was armed.
fn console_and_page_errors(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-console";
    let Some((page, url)) = opened(connection, site, run_id, &json!({}), checks)? else {
        return Ok(());
    };
    let mut exchange = Exchange::new(connection);
    exchange.subscribe(CREDITS)?;
    exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    exchange.listen(&page, "console")?;
    exchange.listen(&page, "page_error")?;
    exchange.call(PAGE_RELOAD.name, &json!({ "page_id": page }))?;
    exchange.until("page.console", |message| {
        message["text"] == json!("page-events error")
    })?;
    exchange.until("page.page_error", |_| true)?;
    let messages = exchange.data("page.console");
    checks.check(
        "the reload's three messages arrive with their type and text, none from before the listener",
        messages
            .iter()
            .map(|message| (message["type"].clone(), message["text"].clone()))
            .collect::<Vec<_>>()
            == vec![
                (json!("log"), json!("page-events 1 true")),
                (json!("warning"), json!("page-events warning")),
                (json!("error"), json!("page-events error")),
            ],
    );
    checks.check(
        "each names the page and its location in the page's document",
        messages.iter().all(|message| {
            message["page_id"] == page
                && message["location"]["url"] == json!(url)
                && message["location"]["line_number"]
                    .as_u64()
                    .is_some_and(|line| line > 0)
        }),
    );
    let errors = exchange.data("page.page_error");
    checks.check(
        "the uncaught exception is a `TypeError` with its message and stack",
        errors.first().is_some_and(|error| {
            error["page_id"] == page
                && error["name"] == json!("TypeError")
                && error["message"] == json!("page-events uncaught")
                && error["stack"]
                    .as_str()
                    .is_some_and(|stack| stack.starts_with("TypeError: page-events uncaught\n"))
        }),
    );
    exchange.end(run_id)
}

/// A request the domain policy blocked, one a route aborted and one whose
/// host does not resolve each fail as Playwright reports them, the blocked
/// one with its Error code.
fn failed_requests(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-failures";
    let host = url_on(site, "access/1", "/")
        .as_deref()
        .and_then(host_of)
        .map(ToOwned::to_owned)
        .unwrap_or_default();
    let settings = json!({ "egress": { "allowed_domains": [host, "nowhere.invalid"] } });
    let Some((page, url)) = opened(connection, site, run_id, &settings, checks)? else {
        return Ok(());
    };
    let mut exchange = Exchange::new(connection);
    exchange.subscribe(CREDITS)?;
    exchange.listen(&page, "request_failed")?;
    exchange.connection.request(
        PAGE_ROUTE.name,
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/page-events/aborted" } }),
    )?;
    exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    exchange.until("page.request_failed", |request| {
        url_of(request).contains("nowhere.invalid")
    })?;
    let failed = exchange.data("page.request_failed");
    let failure = |part: &str| {
        failed
            .iter()
            .find(|request| url_of(request).contains(part))
            .map(|request| request["failure"].clone())
            .unwrap_or_default()
    };
    let blocked = failure("elsewhere.test");
    checks.check(
        "the request the domain policy blocked fails with `net::ERR_BLOCKED_BY_CLIENT` and `egress_refused`",
        blocked["error_text"] == json!("net::ERR_BLOCKED_BY_CLIENT")
            && blocked["error"]["code"] == json!("egress_refused"),
    );
    checks.check(
        "the request a route aborted fails with `net::ERR_FAILED`",
        failure("/page-events/aborted") == json!({ "error_text": "net::ERR_FAILED" }),
    );
    checks.check(
        "the request to a host that does not resolve fails with `net::ERR_NAME_NOT_RESOLVED`",
        failure("nowhere.invalid") == json!({ "error_text": "net::ERR_NAME_NOT_RESOLVED" }),
    );
    exchange.end(run_id)
}

/// A subscriber that grants no credits holds its stream: the page loads and
/// its events wait; each credit then sends one, in position order.
fn credits(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-credits";
    let Some((page, url)) = opened(connection, site, run_id, &json!({}), checks)? else {
        return Ok(());
    };
    let mut exchange = Exchange::new(connection);
    let subscription = exchange.subscribe(0)?;
    exchange.listen(&page, "request")?;
    let went = exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    checks.check(
        "the page loads while its events wait",
        went["result"]["response"]["status"] == json!(200) && exchange.events.is_empty(),
    );
    let mut batches = Vec::new();
    for _ in 0..2 {
        exchange
            .connection
            .notify(CREDIT, &json!({ "id": subscription, "credits": 1 }))?;
        let before = exchange.events.len();
        while exchange.events.len() == before {
            exchange.read()?;
        }
        batches.push(exchange.events.len() - before);
    }
    checks.check("each credit sends one event", batches == [1, 1]);
    exchange
        .connection
        .notify(CREDIT, &json!({ "id": subscription, "credits": CREDITS }))?;
    exchange.until("page.request", |request| {
        url_of(request).contains("nowhere.invalid")
    })?;
    let positions: Vec<u64> = exchange
        .events
        .iter()
        .filter_map(|event| event["position"].as_u64())
        .collect();
    checks.check(
        "the events arrive in the order of their positions, the document's request first",
        positions.windows(2).all(|pair| pair[0] < pair[1])
            && exchange
                .data("page.request")
                .first()
                .is_some_and(|request| url_of(request) == url),
    );
    exchange.end(run_id)
}

/// A body `page.wait_for_request_finished` kept reads whole after the page
/// navigated away; another is read on demand while the browser holds it,
/// then raises `body_unavailable` naming the waiter; a cut one raises
/// `response_truncated`.
fn bodies(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-bodies";
    let other = site.page_url.clone();
    let cut = site.named_url("fetch-edge.test", "/cut/content-length");
    let (Some(other), Some(cut)) = (other, cut) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    let Some((page, url)) = opened(connection, site, run_id, &json!({}), checks)? else {
        return Ok(());
    };
    let mut exchange = Exchange::new(connection);
    let waiting = exchange.connection.request(
        PAGE_WAIT_FOR_REQUEST_FINISHED.name,
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/page-events/data.json" } }),
    )?;
    let went = exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    let finished = exchange.answer(waiting)?;
    checks.check(
        format!(
            "`{}` armed before the navigation answers the body's request and its 200",
            PAGE_WAIT_FOR_REQUEST_FINISHED.name
        ),
        url_of(&finished["result"]["request"]).ends_with("/page-events/data.json")
            && finished["result"]["response"]["status"] == json!(200),
    );
    let body = |exchange: &mut Exchange<'_>, request_id: &Value| {
        exchange.call(
            RESPONSE_BODY.name,
            &json!({ "page_id": page, "request_id": request_id }),
        )
    };
    let document_id = went["result"]["response"]["request_id"].clone();
    let document = body(&mut exchange, &document_id)?;
    checks.check(
        "the document's body reads on demand while the browser holds it",
        handle_bytes(&document).is_some_and(|bytes| {
            String::from_utf8_lossy(&bytes).contains("<title>page-events</title>")
        }),
    );
    exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": other }))?;
    let kept = body(&mut exchange, &finished["result"]["request"]["request_id"])?;
    checks.check(
        "the kept body reads whole once the page navigated away",
        handle_bytes(&kept)
            .and_then(|bytes| serde_json::from_slice::<Map<String, Value>>(&bytes).ok())
            .is_some_and(|json| json["page_events"].is_string()),
    );
    let dropped = body(&mut exchange, &document_id)?;
    let data = &dropped["error"]["data"];
    checks.check(
        format!(
            "the document's body the browser dropped raises `{}` naming its URL",
            BODY_UNAVAILABLE.name
        ),
        data["code"] == json!(BODY_UNAVAILABLE.name) && data["url"] == json!(url),
    );
    checks.check(
        format!("its fix names `{}`", PAGE_WAIT_FOR_REQUEST_FINISHED.name),
        data["fix"]
            .as_str()
            .is_some_and(|fix| fix.contains(PAGE_WAIT_FOR_REQUEST_FINISHED.name)),
    );
    let cut_went = exchange.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page, "url": cut, "wait_until": "commit" }),
    )?;
    let cut_body = body(&mut exchange, &cut_went["result"]["response"]["request_id"])?;
    checks.check(
        format!(
            "a body cut mid-transfer raises `{}` naming its URL, never read as whole",
            RESPONSE_TRUNCATED.name
        ),
        cut_body["error"]["data"]["code"] == json!(RESPONSE_TRUNCATED.name)
            && cut_body["error"]["data"]["url"] == json!(cut),
    );
    exchange.end(run_id)
}

/// How long the body read of [`body_timeout`] waits, in milliseconds.
const BODY_TIMEOUT: u64 = 500;

/// A body read waits for its request to end only within its `timeout`
/// (ADR 0080): on the live board's server-sent events stream, whose
/// response never ends, it raises `deadline_exceeded` naming
/// `timeout.wait` at the call's own value, at the run layer.
fn body_timeout(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-body-timeout";
    let board = site.start_url("live/1").map(str::to_owned);
    checks.check(
        "the Proving ground serves the task `live/1`",
        board.is_some(),
    );
    let Some(board) = board else {
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(());
    };
    let page = json!(page.page_id);
    let mut exchange = Exchange::new(connection);
    let streaming = exchange.connection.request(
        PAGE_WAIT_FOR_RESPONSE.name,
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/events" } }),
    )?;
    exchange.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": board }))?;
    let stream = exchange.answer(streaming)?;
    let read = exchange.call(
        RESPONSE_BODY.name,
        &json!({
            "page_id": page,
            "request_id": stream["result"]["request"]["request_id"],
            "timeout": BODY_TIMEOUT,
        }),
    )?;
    let data = &read["error"]["data"];
    checks.check(
        format!(
            "a body read of the event stream, which never ends, raises `{}` past its `timeout`",
            DEADLINE_EXCEEDED.name
        ),
        data["code"] == json!(DEADLINE_EXCEEDED.name),
    );
    checks.check(
        format!(
            "its limit is `{}` at the call's own value, at the run layer",
            TIMEOUT_WAIT.name
        ),
        data["limit"] == json!({ "key": TIMEOUT_WAIT.name, "value": BODY_TIMEOUT, "layer": "run" }),
    );
    exchange.end(run_id)
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
