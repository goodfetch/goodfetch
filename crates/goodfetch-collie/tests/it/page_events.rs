//! A page's events and the bodies of its responses on Collie's executable
//! over stdio (spec #577, ticket #703; ADRs 0081 and 0085): `page.on`
//! listens to an event of a page under Playwright's name, the engine
//! publishes it on the event stream, which `events.subscribe` reads under
//! its credits (ADR 0024), and a listener whose call is cancelled no longer
//! fires. The Proving ground's page events page does what each event
//! reports. Each test has its own Goodfetch home, Proving ground and
//! Browser cache (ADRs 0059 and 0094).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run trace and page body files on its own thread, never an async worker"
)]

use std::collections::BTreeMap;

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::attached::Attached;
use crate::page::{opened, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 703;

/// The credits a subscription grants at once, more than these tests' events.
const CREDITS: u32 = 10_000;

/// An Attached engine, the events its subscription read so far and the
/// answers read before their call was waited for, by its ID, as a Client
/// matches each answer to its call.
pub(crate) struct Exchange<'env> {
    pub(crate) engine: Attached<'env>,
    events: Vec<Value>,
    answers: BTreeMap<i64, Value>,
    next_id: i64,
}

impl<'env> Exchange<'env> {
    pub(crate) fn new(engine: Attached<'env>) -> Self {
        Self {
            engine,
            events: Vec::new(),
            answers: BTreeMap::new(),
            next_id: 100,
        }
    }

    /// Sends a request and returns its ID.
    pub(crate) fn request(&mut self, method: &str, params: &Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    fn notify(&mut self, method: &str, params: &Value) {
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Takes in one message: the events of a batch, an answer, kept for
    /// [`answer`](Self::answer), or else the message.
    pub(crate) fn read(&mut self) -> Option<Value> {
        let message = self.engine.next().expect("the engine answered");
        if message["method"] == json!("route.handle") {
            // The harness's route aborts every request it is asked about,
            // then ends.
            let route_id = message["params"]["route_id"].clone();
            self.request("route.abort", &json!({ "route_id": route_id }));
            self.engine
                .send(&json!({ "jsonrpc": "2.0", "id": message["id"], "result": {} }));
            return None;
        }
        if message["method"] == json!("$/events") {
            self.events.extend(
                message["params"]["events"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
            return None;
        }
        if message.get("method").is_none()
            && let Some(id) = message["id"].as_i64()
        {
            self.answers.insert(id, message);
            return None;
        }
        Some(message)
    }

    /// The answer to the request `id`, whenever it came.
    pub(crate) fn answer(&mut self, id: i64) -> Value {
        loop {
            if let Some(answer) = self.answers.remove(&id) {
                return answer;
            }
            let _ = self.read();
        }
    }

    pub(crate) fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.request(method, params);
        self.answer(id)
    }

    /// Reads until an event `name` whose data `holds` arrived.
    pub(crate) fn until(&mut self, name: &str, holds: impl Fn(&Value) -> bool) {
        while !self
            .events
            .iter()
            .any(|event| event["event"] == json!(name) && holds(&event["data"]))
        {
            let _ = self.read();
        }
    }

    /// The data of every event `name` read so far.
    pub(crate) fn data(&self, name: &str) -> Vec<Value> {
        self.events
            .iter()
            .filter(|event| event["event"] == json!(name))
            .map(|event| event["data"].clone())
            .collect()
    }

    /// Subscribes to the event stream with credits for every event.
    pub(crate) fn subscribe(&mut self) -> i64 {
        let id = self.request("events.subscribe", &json!({}));
        self.notify("$/credit", &json!({ "id": id, "credits": CREDITS }));
        id
    }

    /// Listens to `event` of `page`, answering the listening call's ID.
    pub(crate) fn listen(&mut self, page: &str, event: &str) -> i64 {
        self.request("page.on", &json!({ "page_id": page, "event": event }))
    }
}

/// Starts an engine and its Run `run_id` with `settings`, and opens a page.
fn page_of<'env>(
    env: &'env TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Map<String, Value>,
) -> (Exchange<'env>, String) {
    let cache = browser_cache(env).unwrap();
    let engine = started_with(env, &cache, proving_ground, run_id, settings, |_| {});
    let mut exchange = Exchange::new(engine);
    let page = opened(&exchange.call("run.new_page", &json!({ "run_id": run_id })));
    (exchange, page.page_id.0)
}

/// The first of `requests` to `url`.
fn with_url(requests: &[Value], url: &str) -> Value {
    requests
        .iter()
        .find(|request| request["url"] == json!(url))
        .cloned()
        .unwrap_or_default()
}

/// The status of the first of `responses` from `url`.
fn status(responses: &[Value], url: &str) -> Option<Value> {
    responses
        .iter()
        .find(|exchange| exchange["response"]["url"] == json!(url))
        .map(|exchange| exchange["response"]["status"].clone())
}

fn urls(data: &[Value]) -> Vec<String> {
    data.iter()
        .filter_map(|request| request["url"].as_str().map(ToOwned::to_owned))
        .collect()
}

/// The network events and `load` and `domcontentloaded`, `request` first.
const NETWORK_AND_LOAD: [&str; 7] = [
    "request",
    "request_finished",
    "request_failed",
    "response",
    "web_socket",
    "load",
    "dom_content_loaded",
];

/// Cancels the `request` listener's call `listener`, which removes it, and
/// reloads the page: answers the cancelled call and the `page.request`
/// events of the reload, none once the listener is gone, while its
/// responses still come.
fn cancelled(
    exchange: &mut Exchange<'_>,
    listener: i64,
    page: &str,
    url: &str,
) -> (Value, Vec<Value>) {
    exchange.notify("$/cancel", &json!({ "id": listener }));
    let cancelled = exchange.answer(listener);
    exchange.events.clear();
    exchange.call("page.reload", &json!({ "page_id": page }));
    exchange.until("page.response", |exchange| {
        exchange["response"]["url"] == json!(url)
    });
    (cancelled, exchange.data("page.request"))
}

/// The network events of the page events page each carry ADR 0085's data:
/// every request with its method, resource type and navigation flag, the
/// responses with their status, the finished requests, the failed one with
/// the browser's error, the WebSocket with its frames and close, and the
/// document's `domcontentloaded` and `load`; a listener whose call is
/// cancelled no longer fires.
/// Covers: story:5-48, operation:page.on, event:page.request, event:page.request_finished, event:page.request_failed, event:page.response, event:page.web_socket, event:web_socket.frame_sent, event:web_socket.frame_received, event:web_socket.close, event:page.load, event:page.dom_content_loaded
#[test]
fn a_pages_network_and_load_events_carry_their_data_and_a_cancelled_listener_no_longer_fires() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "events", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");
    let data = proving_ground.url("access.test", "/page-events/data.json");
    let missing = proving_ground.url("access.test", "/page-events/missing");

    exchange.subscribe();
    let listening = NETWORK_AND_LOAD.map(|event| exchange.listen(&page, event));
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    exchange.until("web_socket.close", |_| true);
    let requests = exchange.data("page.request");
    let responses = exchange.data("page.response");
    let finished = exchange.data("page.request_finished");
    let failed = exchange.data("page.request_failed");
    let sockets = exchange.data("page.web_socket");
    let sent = exchange.data("web_socket.frame_sent");
    let received = exchange.data("web_socket.frame_received");
    let loads = (
        exchange.data("page.dom_content_loaded"),
        exchange.data("page.load"),
    );

    let (cancelled, after_cancel) = cancelled(&mut exchange, listening[0], &page, &url);
    let _ = exchange.call("run.finish", &json!({ "run_id": "events" }));
    let _ = exchange.engine.finish();

    let (document, fetched) = (with_url(&requests, &url), with_url(&requests, &data));
    let status_of = |wanted: &str| status(&responses, wanted);
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    assert_eq!(
        (
            (
                document["page_id"].clone(),
                document["method"].clone(),
                document["resource_type"].clone(),
                document["is_navigation_request"].clone(),
            ),
            (
                fetched["resource_type"].clone(),
                fetched["is_navigation_request"].clone(),
                fetched["redirect_chain"].clone(),
            ),
            (status_of(&url), status_of(&data), status_of(&missing)),
            responses
                .iter()
                .all(|exchange| exchange["response"]["from_service_worker"] == json!(false)),
            urls(&finished).contains(&data),
            failed
                .iter()
                .find(|request| request["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("nowhere.invalid")))
                .map(|request| request["failure"]["error_text"].clone()),
        ),
        (
            (json!(page), json!("GET"), json!("Document"), json!(true)),
            (json!("Fetch"), json!(false), json!([])),
            (Some(json!(200)), Some(json!(200)), Some(json!(404))),
            true,
            true,
            Some(json!("net::ERR_NAME_NOT_RESOLVED")),
        ),
        "{requests:?} {responses:?} {failed:?}"
    );
    assert_eq!(
        (
            sockets
                .iter()
                .map(|socket| socket["url"]
                    .as_str()
                    .unwrap_or_default()
                    .ends_with("/page-events/socket"))
                .collect::<Vec<_>>(),
            sent.iter()
                .map(|frame| (frame["payload"].clone(), frame["binary"].clone()))
                .collect::<Vec<_>>(),
            !received.is_empty(),
            (
                loads.0.iter().any(|fired| fired["page_id"] == json!(page)),
                loads.1.iter().any(|fired| fired["page_id"] == json!(page)),
            ),
            cancelled["error"]["data"]["code"].clone(),
            after_cancel,
        ),
        (
            vec![true],
            vec![(json!("page-events hello"), json!(false))],
            true,
            (true, true),
            json!("call_cancelled"),
            Vec::<Value>::new(),
        ),
        "{sockets:?} {sent:?}"
    );
}

/// The page's console messages carry their type, text and location, and an
/// exception it did not catch its name, message and stack; a listener
/// armed after the page wrote to its console gets none of what it wrote
/// before (ADR 0081: armed when called).
/// Covers: story:5-48, operation:page.on, event:page.console, event:page.page_error
#[test]
fn console_messages_and_uncaught_exceptions_carry_their_data_from_when_a_listener_is_armed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "console", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");

    exchange.subscribe();
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    exchange.listen(&page, "console");
    exchange.listen(&page, "page_error");
    exchange.call("page.reload", &json!({ "page_id": page }));
    exchange.until("page.console", |message| {
        message["text"] == json!("page-events error")
    });
    exchange.until("page.page_error", |_| true);
    let messages = exchange.data("page.console");
    let errors = exchange.data("page.page_error");
    let _ = exchange.call("run.finish", &json!({ "run_id": "console" }));
    let _ = exchange.engine.finish();

    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    assert_eq!(
        (
            messages
                .iter()
                .map(|message| (message["type"].clone(), message["text"].clone()))
                .collect::<Vec<_>>(),
            messages
                .iter()
                .all(|message| message["page_id"] == json!(page)
                    && message["location"]["url"] == json!(url)
                    && message["location"]["line_number"]
                        .as_u64()
                        .is_some_and(|line| line > 0)),
            errors.first().map(|error| (
                error["page_id"].clone(),
                error["name"].clone(),
                error["message"].clone(),
                error["stack"]
                    .as_str()
                    .is_some_and(|stack| stack.starts_with("TypeError: page-events uncaught\n")),
            )),
        ),
        (
            vec![
                (json!("log"), json!("page-events 1 true")),
                (json!("warning"), json!("page-events warning")),
                (json!("error"), json!("page-events error")),
            ],
            true,
            Some((
                json!(page),
                json!("TypeError"),
                json!("page-events uncaught"),
                true
            )),
        ),
        "{messages:?} {errors:?}"
    );
}

/// The bytes of the page body file a handle names.
fn body_of(handle: &Value) -> Vec<u8> {
    std::fs::read(handle["path"].as_str().unwrap_or_default()).unwrap_or_default()
}

/// A response a `page.wait_for_request_finished` waiter answered keeps its
/// whole body, even once the page navigated away; another response's body
/// is read on demand while the browser holds it, and once it dropped it,
/// as after the next navigation, `response.body` raises `body_unavailable`
/// whose fix names the waiter; a body cut mid-transfer raises
/// `response_truncated`, never returned as whole (ADRs 0085 and 0118).
/// Covers: story:5-46, operation:response.body, operation:page.wait_for_request_finished, error:body_unavailable, error:response_truncated, setting:timeout.wait
#[test]
fn a_kept_body_reads_whole_a_dropped_one_is_unavailable_and_a_cut_one_is_truncated() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "bodies", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");
    let other = proving_ground.url("catalog.test", "/catalog/1");
    let cut = proving_ground.url("fetch-edge.test", "/cut/content-length");

    let waiting = exchange.request(
        "page.wait_for_request_finished",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/page-events/data.json" } }),
    );
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    let finished = exchange.answer(waiting);
    let data_id = finished["result"]["request"]["request_id"].clone();
    let document_id = went["result"]["response"]["request_id"].clone();
    let document_body = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": document_id }),
    );
    exchange.call("page.goto", &json!({ "page_id": page, "url": other }));
    let kept = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": data_id }),
    );
    let dropped = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": document_id }),
    );
    let cut_went = exchange.call(
        "page.goto",
        &json!({ "page_id": page, "url": cut, "wait_until": "commit" }),
    );
    let cut_body = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": cut_went["result"]["response"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "bodies" }));
    let _ = exchange.engine.finish();

    let kept_json: Value = serde_json::from_slice(&body_of(&kept["result"])).unwrap_or_default();
    assert_eq!(
        (
            finished["result"]["response"]["status"].clone(),
            finished["result"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/page-events/data.json")),
            kept_json["page_events"].is_string(),
            String::from_utf8_lossy(&body_of(&document_body["result"]))
                .contains("<title>page-events</title>"),
            dropped["error"]["data"]["code"].clone(),
            dropped["error"]["data"]["url"].clone(),
            dropped["error"]["data"]["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("page.wait_for_request_finished")),
            cut_went["result"]["response"]["status"].clone(),
            cut_body["error"]["data"]["code"].clone(),
            cut_body["error"]["data"]["url"].clone(),
        ),
        (
            json!(200),
            true,
            true,
            true,
            json!("body_unavailable"),
            json!(url),
            true,
            json!(200),
            json!("response_truncated"),
            json!(cut),
        ),
        "{finished} {kept} {document_body} {dropped} {cut_went} {cut_body}"
    );
}

/// A response's body is one the engine itself reads (ticket #644): past
/// the Run's `network.max_body_size`, `response.body` raises
/// `response_limit_exceeded` naming the URL, the setting and its value,
/// for a kept body as for one read on demand; under the default limit the
/// same bodies read whole, as the test above shows.
/// Covers: operation:response.body, error:response_limit_exceeded, setting:network.max_body_size
#[test]
fn a_body_past_network_max_body_size_raises_response_limit_exceeded() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut settings = Map::new();
    settings.insert("network".to_owned(), json!({ "max_body_size": 16 }));
    let (mut exchange, page) = page_of(&env, &proving_ground, "limited", &settings);
    let url = proving_ground.url("access.test", "/page-events");

    let waiting = exchange.request(
        "page.wait_for_request_finished",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/page-events/data.json" } }),
    );
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    let finished = exchange.answer(waiting);
    let document = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": went["result"]["response"]["request_id"] }),
    );
    let kept = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": finished["result"]["request"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "limited" }));
    let _ = exchange.engine.finish();

    let refusal = |answer: &Value| {
        let data = &answer["error"]["data"];
        (
            data["code"].clone(),
            data["setting"].clone(),
            data["limit"].clone(),
        )
    };
    let expected = (
        json!("response_limit_exceeded"),
        json!("network.max_body_size"),
        json!(16),
    );
    assert_eq!(
        (refusal(&document), refusal(&kept)),
        (expected.clone(), expected),
        "{document} {kept}"
    );
    assert_eq!(document["error"]["data"]["url"], json!(url), "{document}");
}

/// The lines of the Run trace's request log.
fn request_log(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let file = env
        .home()
        .join("0.0")
        .join("runs")
        .join(run_id)
        .join("trace")
        .join("requests.jsonl");
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// A request the domain policy blocked, one a route aborted and one whose
/// host does not resolve are each a `page.request_failed` carrying the
/// browser's error as Playwright's `request.failure()` gives it, the
/// blocked one also its Error code, which the Run trace records since no
/// call waited on it; a finished request carries no failure.
/// Covers: story:5-48, operation:page.on, event:page.request_failed, event:page.request_finished, error:egress_refused
#[test]
fn a_blocked_an_aborted_and_a_failed_request_each_fail_as_playwright_reports_them() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut settings = Map::new();
    settings.insert(
        "egress".to_owned(),
        json!({ "allowed_domains": ["access.test", "nowhere.invalid"] }),
    );
    let (mut exchange, page) = page_of(&env, &proving_ground, "failures", &settings);
    let url = proving_ground.url("access.test", "/page-events");

    exchange.subscribe();
    exchange.listen(&page, "request_failed");
    exchange.listen(&page, "request_finished");
    exchange.request(
        "page.route",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/page-events/aborted" } }),
    );
    exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    exchange.until("page.request_failed", |request| {
        request["url"]
            .as_str()
            .is_some_and(|url| url.contains("nowhere.invalid"))
    });
    let failed = exchange.data("page.request_failed");
    let finished = exchange.data("page.request_finished");
    let _ = exchange.call("run.finish", &json!({ "run_id": "failures" }));
    let _ = exchange.engine.finish();

    let failure_of = |part: &str| {
        failed
            .iter()
            .find(|request| {
                request["url"]
                    .as_str()
                    .is_some_and(|url| url.contains(part))
            })
            .map(|request| request["failure"].clone())
            .unwrap_or_default()
    };
    let blocked = failure_of("elsewhere.test");
    let logged = request_log(&env, "failures");
    assert_eq!(
        (
            blocked["error_text"].clone(),
            blocked["error"]["code"].clone(),
            blocked["error"]["rule"].clone(),
            failure_of("/page-events/aborted"),
            failure_of("nowhere.invalid"),
            finished.iter().any(|request| request["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/page-events/data.json"))
                && request.get("failure").is_none()),
            logged
                .iter()
                .map(|line| (
                    line["event"].clone(),
                    line["data"]["page_id"].clone(),
                    line["data"]["url"]
                        .as_str()
                        .is_some_and(|url| url.contains("elsewhere.test")),
                    line["data"]["failure"]["error"]["code"].clone(),
                ))
                .collect::<Vec<_>>(),
        ),
        (
            json!("net::ERR_BLOCKED_BY_CLIENT"),
            json!("egress_refused"),
            json!("domain_policy"),
            json!({ "error_text": "net::ERR_FAILED" }),
            json!({ "error_text": "net::ERR_NAME_NOT_RESOLVED" }),
            true,
            vec![(
                json!("page.request_failed"),
                json!(page),
                true,
                json!("egress_refused")
            )],
        ),
        "{failed:?} {logged:?}"
    );
}

/// A page's events travel under the subscription's credits (ADR 0024): a
/// subscriber that grants none holds its stream, so the page loads and
/// its events wait unsent; each credit then sends one event, in the order
/// of their positions, and the rest follow once it grants them.
/// Covers: story:5-48, operation:page.on, operation:events.subscribe, event:page.request
#[test]
fn a_slow_reader_holds_the_page_events_stream_and_reads_one_event_per_credit() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "credits", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");

    let subscription = exchange.request("events.subscribe", &json!({}));
    exchange.listen(&page, "request");
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    let held = exchange.events.len();
    let mut batches = Vec::new();
    for _ in 0..2 {
        exchange.notify("$/credit", &json!({ "id": subscription, "credits": 1 }));
        let before = exchange.events.len();
        while exchange.events.len() == before {
            let _ = exchange.read();
        }
        batches.push(exchange.events.len() - before);
    }
    exchange.notify(
        "$/credit",
        &json!({ "id": subscription, "credits": CREDITS }),
    );
    exchange.until("page.request", |request| {
        request["url"]
            .as_str()
            .is_some_and(|url| url.contains("nowhere.invalid"))
    });
    let positions: Vec<u64> = exchange
        .events
        .iter()
        .filter_map(|event| event["position"].as_u64())
        .collect();
    let requested = urls(&exchange.data("page.request"));
    let events = exchange.events.clone();
    let _ = exchange.call("run.finish", &json!({ "run_id": "credits" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            went["result"]["response"]["status"].clone(),
            held,
            batches,
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            requested.first().cloned(),
            requested.len() >= 6,
        ),
        (json!(200), 0, vec![1, 1], true, Some(url), true),
        "{events:?}"
    );
}

/// `response body` through the command generated from the catalog: no
/// engine holds the page it names, so the call fails with `closed`.
/// Covers: command:response.body
#[test]
fn response_body_of_a_page_no_engine_holds_fails_with_closed() {
    let env = TestEnv::new();
    let checked = goodfetch_test_support::cli(&env, env!("CARGO_BIN_EXE_goodfetch-collie"))
        .args([
            "response",
            "body",
            "--page-id",
            "page-1",
            "--request-id",
            "1.1",
            "--json",
        ])
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            checked.documents[0]["error"]["code"].clone(),
            checked.documents[0]["error"]["operation"].clone(),
        ),
        (Some(3), json!("closed"), json!("response.body"))
    );
}
