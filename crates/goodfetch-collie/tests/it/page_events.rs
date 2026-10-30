//! A page's events and the bodies of its responses on Collie's executable
//! over stdio (spec #577, ticket #703; ADRs 0081 and 0085): `page.on`
//! listens to an event of a page under Playwright's name, and the engine
//! sends each one on that call itself, under the call's own credits (ADR
//! 0024), never on the event stream `events.subscribe` reads; a listener
//! whose call is cancelled no longer fires. The Proving ground's page
//! events pages do what each event reports: its forms page posts a sign-in
//! form to a redirect, and its framed page holds an iframe on another
//! site. Each test has its own Goodfetch home, Proving ground and Browser
//! cache (ADRs 0059 and 0094).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run trace and page body files on its own thread, never an async worker"
)]

use std::collections::BTreeMap;
use std::process::Command;

use goodfetch_test_support::{
    Canaries, Canary, Output, ProvingGround, SecretKind, TestEnv, browser_cache,
};
use serde_json::{Map, Value, json};

use crate::attached::Attached;
use crate::page::{opened, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 703;

/// The credits a call grants at once, more than these tests' events.
const CREDITS: u32 = 10_000;

/// How many of a page's events wait unsent for one listener before it
/// ends with `page_events_lost`, as `page.on`'s description says.
const LISTENER_LIMIT: usize = 1_024;

/// An Attached engine, the events its calls carried so far, every call's
/// and each call's apart, and the answers read before their call was
/// waited for, by its ID, as a Client matches each answer to its call.
pub(crate) struct Exchange<'env> {
    pub(crate) engine: Attached<'env>,
    events: Vec<Value>,
    /// The events of each call, by its ID: a `page.on` call's, or the
    /// subscription's to the event stream.
    of_call: BTreeMap<i64, Vec<Value>>,
    answers: BTreeMap<i64, Value>,
    next_id: i64,
}

impl<'env> Exchange<'env> {
    pub(crate) fn new(engine: Attached<'env>) -> Self {
        Self {
            engine,
            events: Vec::new(),
            of_call: BTreeMap::new(),
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
            let events = message["params"]["events"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(call) = message["params"]["id"].as_i64() {
                self.of_call
                    .entry(call)
                    .or_default()
                    .extend(events.iter().cloned());
            }
            self.events.extend(events);
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

    /// The events the call `call` carried so far.
    fn of(&self, call: i64) -> &[Value] {
        self.of_call.get(&call).map_or(&[], Vec::as_slice)
    }

    /// Reads until the call `call` carried an event `name` whose data
    /// `holds`.
    fn until_on(&mut self, call: i64, name: &str, holds: impl Fn(&Value) -> bool) {
        while !self
            .of(call)
            .iter()
            .any(|event| event["event"] == json!(name) && holds(&event["data"]))
        {
            let _ = self.read();
        }
    }

    /// Subscribes to the event stream with credits for every event.
    pub(crate) fn subscribe(&mut self) -> i64 {
        let id = self.request("events.subscribe", &json!({}));
        self.notify("$/credit", &json!({ "id": id, "credits": CREDITS }));
        id
    }

    /// Listens to `event` of `page`, granting the listening call credits
    /// for every event, and answers its ID.
    pub(crate) fn listen(&mut self, page: &str, event: &str) -> i64 {
        self.listen_with(page, event, CREDITS)
    }

    /// Listens to `event` of `page`, granting the listening call `credits`
    /// at once, and answers its ID.
    fn listen_with(&mut self, page: &str, event: &str, credits: u32) -> i64 {
        let id = self.request("page.on", &json!({ "page_id": page, "event": event }));
        if credits > 0 {
            self.notify("$/credit", &json!({ "id": id, "credits": credits }));
        }
        id
    }
}

/// Starts an engine and its Run `run_id` with `settings`, and opens a page.
fn page_of<'env>(
    env: &'env TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Map<String, Value>,
) -> (Exchange<'env>, String) {
    page_started(env, proving_ground, run_id, settings, |_| {})
}

/// As [`page_of`], the engine's command changed by `change` first.
fn page_started<'env>(
    env: &'env TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Map<String, Value>,
    change: impl FnOnce(&mut Command),
) -> (Exchange<'env>, String) {
    let cache = browser_cache(env).unwrap();
    let engine = started_with(env, &cache, proving_ground, run_id, settings, change);
    let mut exchange = Exchange::new(engine);
    let page = opened(&exchange.call("run.new_page", &json!({ "run_id": run_id })));
    (exchange, page.page_id.0)
}

/// The first of `requests` whose URL ends with `suffix`.
fn ending_with(requests: &[Value], suffix: &str) -> Value {
    requests
        .iter()
        .find(|request| {
            request["url"]
                .as_str()
                .is_some_and(|url| url.ends_with(suffix))
        })
        .cloned()
        .unwrap_or_default()
}

/// The bytes a request's `post_data` holds, decoded from its Base64.
fn decoded(request: &Value) -> Vec<u8> {
    request["post_data"]
        .as_str()
        .and_then(|encoded| data_encoding::BASE64.decode(encoded.as_bytes()).ok())
        .unwrap_or_default()
}

/// Every `post_data` anywhere in `value`, decoded, one after another.
fn every_post_data(value: &Value, into: &mut Vec<u8>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if key == "post_data"
                    && let Some(encoded) = inner.as_str()
                {
                    into.extend(
                        data_encoding::BASE64
                            .decode(encoded.as_bytes())
                            .unwrap_or_default(),
                    );
                    into.push(b'\n');
                }
                every_post_data(inner, into);
            }
        }
        Value::Array(values) => {
            for inner in values {
                every_post_data(inner, into);
            }
        }
        _ => {}
    }
}

/// Fills `value` into the field `selector` of `page`.
fn fill(exchange: &mut Exchange<'_>, page: &str, selector: &str, value: &str) -> Value {
    exchange.call(
        "locator.fill",
        &json!({ "locator": { "page_id": page, "selector": selector }, "value": value }),
    )
}

/// Clicks the element `selector` of `page`.
fn click(exchange: &mut Exchange<'_>, page: &str, selector: &str) -> Value {
    exchange.call(
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": selector } }),
    )
}

/// Arms a `page.wait_for_request_finished` of `page` for the URLs `glob`
/// matches, answering the waiter's call ID.
fn finished_waiter(exchange: &mut Exchange<'_>, page: &str, glob: &str) -> i64 {
    exchange.request(
        "page.wait_for_request_finished",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": glob } }),
    )
}

/// Whether `request` names its headers in lowercase, as Playwright gives
/// them, its `user-agent` among them.
fn lowercase_headers(request: &Value) -> bool {
    request["headers"].as_object().is_some_and(|headers| {
        headers.contains_key("user-agent")
            && headers
                .keys()
                .all(|name| *name == name.to_ascii_lowercase())
    })
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

/// The network events of the page events page each carry ADR 0085's data
/// with Playwright's values: every request with its method, its headers
/// named in lowercase, its resource type in lowercase and its navigation
/// flag, the responses with their status, the finished requests, the failed one with
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
                lowercase_headers(&document),
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
            (
                json!(page),
                json!("GET"),
                json!("document"),
                json!(true),
                true
            ),
            (json!("fetch"), json!(false), json!([])),
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
/// whole body, even once the page navigated away (ADR 0085).
/// Covers: story:5-46, operation:response.body, operation:page.wait_for_request_finished
#[test]
fn a_body_a_finished_waiter_answered_reads_whole_once_the_page_navigated_away() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "kept", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");
    let other = proving_ground.url("catalog.test", "/catalog/1");
    let waiting = finished_waiter(&mut exchange, &page, "**/page-events/data.json");
    exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    let finished = exchange.answer(waiting);
    exchange.call("page.goto", &json!({ "page_id": page, "url": other }));

    let kept = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": finished["result"]["request"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "kept" }));
    let _ = exchange.engine.finish();

    let kept_json: Value = serde_json::from_slice(&body_of(&kept["result"])).unwrap_or_default();
    assert_eq!(
        (
            finished["result"]["response"]["status"].clone(),
            finished["result"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/page-events/data.json")),
            kept_json["page_events"].is_string(),
        ),
        (json!(200), true, true),
        "{finished} {kept}"
    );
}

/// A response no waiter kept has its body read on demand while the browser
/// holds it (ADR 0085).
/// Covers: story:5-46, operation:response.body
#[test]
fn a_body_no_waiter_kept_reads_on_demand_while_the_browser_holds_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "held", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));

    let read = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": went["result"]["response"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "held" }));
    let _ = exchange.engine.finish();

    assert!(
        String::from_utf8_lossy(&body_of(&read["result"])).contains("<title>page-events</title>"),
        "{went} {read}"
    );
}

/// Once the browser dropped a body no waiter kept, as after the next
/// navigation, `response.body` raises `body_unavailable`, whose fix names
/// the waiter that would have kept it (ADR 0085).
/// Covers: story:5-46, operation:response.body, error:body_unavailable
#[test]
fn a_body_the_browser_dropped_raises_body_unavailable_naming_the_waiter() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "dropped", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");
    let other = proving_ground.url("catalog.test", "/catalog/1");
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    exchange.call("page.goto", &json!({ "page_id": page, "url": other }));

    let dropped = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": went["result"]["response"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "dropped" }));
    let _ = exchange.engine.finish();

    let data = &dropped["error"]["data"];
    assert_eq!(
        (
            data["code"].clone(),
            data["url"].clone(),
            data["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("page.wait_for_request_finished")),
        ),
        (json!("body_unavailable"), json!(url), true),
        "{dropped}"
    );
}

/// A body cut mid-transfer raises `response_truncated`, never returned as
/// whole (ADR 0118).
/// Covers: operation:response.body, error:response_truncated
#[test]
fn a_body_cut_mid_transfer_raises_response_truncated() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "cut", &Map::new());
    let cut = proving_ground.url("fetch-edge.test", "/cut/content-length");
    let went = exchange.call(
        "page.goto",
        &json!({ "page_id": page, "url": cut, "wait_until": "commit" }),
    );

    let read = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": went["result"]["response"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "cut" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            went["result"]["response"]["status"].clone(),
            read["error"]["data"]["code"].clone(),
            read["error"]["data"]["url"].clone(),
        ),
        (json!(200), json!("response_truncated"), json!(cut)),
        "{went} {read}"
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

/// A page's events travel under the listening call's own credits (ADR
/// 0024): a listener whose Client grants none holds them, so the page
/// loads and its events wait unsent; each credit then sends one event, in
/// the order of the call's own positions, from 1, and the rest follow once
/// it grants them.
/// Covers: story:5-48, operation:page.on, event:page.request
#[test]
fn a_slow_listener_holds_its_page_events_and_reads_one_event_per_credit() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "credits", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");

    let listener = exchange.listen_with(&page, "request", 0);
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    let held = exchange.events.len();
    let mut batches = Vec::new();
    for _ in 0..2 {
        exchange.notify("$/credit", &json!({ "id": listener, "credits": 1 }));
        let before = exchange.events.len();
        while exchange.events.len() == before {
            let _ = exchange.read();
        }
        batches.push(exchange.events.len() - before);
    }
    exchange.notify("$/credit", &json!({ "id": listener, "credits": CREDITS }));
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
            positions.first().copied(),
            positions.windows(2).all(|pair| pair[0] + 1 == pair[1]),
            requested.first().cloned(),
            requested.len() >= 6,
        ),
        (json!(200), 0, vec![1, 1], Some(1), true, Some(url), true),
        "{events:?}"
    );
}

/// A page's events go to the `page.on` call that listens to them, and to
/// no subscription to the event stream, which any Client of the engine may
/// hold: the stream carries the Run's own events, and none of the page's
/// (ADR 0085's #703 note).
/// Covers: story:5-48, operation:page.on, operation:events.subscribe, event:page.request
#[test]
fn a_pages_events_go_only_to_the_call_that_listens_never_the_event_stream() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "private", &Map::new());
    let url = proving_ground.url("access.test", "/page-events");
    let subscription = exchange.subscribe();
    let listener = exchange.listen(&page, "request");

    exchange.call("page.goto", &json!({ "page_id": page, "url": url }));
    exchange.until_on(listener, "page.request", |request| {
        request["url"]
            .as_str()
            .is_some_and(|url| url.contains("nowhere.invalid"))
    });
    let finished = exchange.call("run.finish", &json!({ "run_id": "private" }));
    exchange.until_on(subscription, "run.axes_changed", |run| {
        run["lifecycle"] == json!("finished")
    });
    let streamed: Vec<Value> = exchange
        .of(subscription)
        .iter()
        .map(|event| event["event"].clone())
        .collect();
    let requested = urls(&exchange.data("page.request"));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            finished["result"]["lifecycle"].clone(),
            requested.contains(&url),
            streamed
                .iter()
                .filter(|name| name.as_str().is_some_and(
                    |name| name.starts_with("page.") || name.starts_with("web_socket.")
                ))
                .count(),
        ),
        (json!("finished"), true, 0),
        "{streamed:?}"
    );
}

/// A listener whose Client grants too few credits holds at most 1,024 of
/// the page's events; past them it ends, sending those it held, in order,
/// then raising `page_events_lost`, while the page and another listener,
/// whose Client reads, go on losing none (ADR 0024: the engine never waits
/// on a Client).
/// Covers: story:5-48, operation:page.on, event:page.console, error:page_events_lost
#[test]
fn a_listener_whose_client_reads_too_slowly_ends_with_page_events_lost() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "flood", &Map::new());
    let starved = exchange.listen_with(&page, "console", 0);
    let reading = exchange.listen(&page, "console");
    let flood = "() => { for (let i = 0; i < 1100; i += 1) console.log(`flood ${i}`); }";

    exchange.call(
        "page.evaluate",
        &json!({ "page_id": page, "function": flood }),
    );
    exchange.until_on(reading, "page.console", |message| {
        message["text"] == json!("flood 1099")
    });
    exchange.notify("$/credit", &json!({ "id": starved, "credits": CREDITS }));
    let ended = exchange.answer(starved);
    let held: Vec<u64> = exchange
        .of(starved)
        .iter()
        .filter_map(|event| event["position"].as_u64())
        .collect();
    let read = exchange.of(reading).len();
    let _ = exchange.call("run.finish", &json!({ "run_id": "flood" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            ended["error"]["data"]["code"].clone(),
            (LISTENER_LIMIT..1_100).contains(&held.len()),
            held.iter()
                .copied()
                .eq(1..=u64::try_from(held.len()).unwrap()),
            read,
        ),
        (json!("page_events_lost"), true, true, 1_100),
        "{ended} {}",
        held.len()
    );
}

/// A form posted to a redirect is two hops, each its own request under its
/// own `request_id`, as Playwright's `redirectedFrom()` and `redirectedTo()`
/// tell them apart: the POST carries its headers in lowercase and its
/// urlencoded body, its response the 302 with its status text, and its
/// `page.wait_for_request_finished` waiter answers at that hop, whose body
/// `response.body` refuses with `body_unavailable`; the GET the redirect
/// led to names the POST as `redirected_from` and its URL in its
/// `redirect_chain` (ADR 0085's #703 note).
/// Covers: story:5-48, operation:page.on, event:page.request, event:page.response, event:page.request_finished, operation:page.wait_for_request_finished, operation:response.body, error:body_unavailable
#[test]
fn a_form_posted_to_a_redirect_is_two_hops_each_with_its_own_request_and_data() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "redirect", &Map::new());
    let forms = proving_ground.url("access.test", "/page-events/forms");
    let session = proving_ground.url("access.test", "/page-events/session");
    let home = proving_ground.url("access.test", "/page-events/home");
    for event in ["request", "response", "request_finished"] {
        exchange.listen(&page, event);
    }
    exchange.call("page.goto", &json!({ "page_id": page, "url": forms }));
    fill(&mut exchange, &page, "input[name=password]", "pw");
    let waiting = finished_waiter(&mut exchange, &page, "**/page-events/session");

    click(&mut exchange, &page, "#sign-in-submit");
    let hop = exchange.answer(waiting)["result"].clone();
    exchange.until("page.response", |exchange| {
        exchange["response"]["url"] == json!(home)
    });
    let hop_body = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": hop["request"]["request_id"] }),
    );
    let requests = exchange.data("page.request");
    let (posted, led_to) = (with_url(&requests, &session), with_url(&requests, &home));
    let responses = exchange.data("page.response");
    let finished = urls(&exchange.data("page.request_finished"));
    let _ = exchange.call("run.finish", &json!({ "run_id": "redirect" }));
    let _ = exchange.engine.finish();

    let hop_id = hop["request"]["request_id"].clone();
    assert_eq!(
        (
            (
                posted["request_id"].clone(),
                posted["method"].clone(),
                posted["headers"]["content-type"].clone(),
                String::from_utf8_lossy(&decoded(&posted)).into_owned(),
            ),
            (
                hop["response"]["status"].clone(),
                hop["response"]["status_text"].clone(),
                hop["response"]["request_id"].clone(),
                hop_body["error"]["data"]["code"].clone(),
            ),
            (
                led_to["redirected_from"].clone(),
                led_to["request_id"] != hop_id,
                led_to["method"].clone(),
                led_to["redirect_chain"].clone(),
            ),
            (
                status(&responses, &session),
                status(&responses, &home),
                finished.contains(&session) && finished.contains(&home),
            ),
        ),
        (
            (
                hop_id.clone(),
                json!("POST"),
                json!("application/x-www-form-urlencoded"),
                "user=jb&password=pw".to_owned(),
            ),
            (
                json!(302),
                json!("Found"),
                hop_id.clone(),
                json!("body_unavailable")
            ),
            (hop_id, true, json!("GET"), json!([session])),
            (Some(json!(302)), Some(json!(200)), true),
        ),
        "{hop} {hop_body} {requests:?}"
    );
}

/// The variable the canary test's key-encryption key is read from.
const KEY_VARIABLE: &str = "GF_TEST_PAGE_EVENTS_KEY";

/// A Secret a program types into a form, a value holding marks a browser
/// escapes as it encodes a form, is masked as `%run_state.key_reference%`
/// inside every request body and URL the engine returns, sends as an event
/// or keeps: the urlencoded POST a form submits, the GET a search form
/// sends, and the POST a script sends that the domain policy refuses into
/// the Run trace. A canary planted as the value is searched in every
/// reply, every event, every `post_data` decoded from its Base64, and
/// every file of its Goodfetch home, the Run trace and logs among them, but
/// the Browser cache, which holds no output (spec #576's story 84; ADR 0092).
/// Covers: story:5-48, operation:page.on, event:page.request, event:page.request_failed, operation:page.wait_for_request_finished
#[test]
fn a_secret_typed_into_a_form_is_masked_in_every_request_body_and_url_the_engine_gives() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let value = format!("gfcanary-password-{:016x}&! *~/+=", env.seed());
    let mut canaries = Canaries::new(env.seed());
    canaries.add(Canary::known("typed", SecretKind::TypedPassword, &value));
    env.user_configuration(&format!(
        "run_state.key_reference = \"env:{KEY_VARIABLE}\"\n"
    ));
    let mut settings = Map::new();
    settings.insert(
        "egress".to_owned(),
        json!({ "allowed_domains": ["access.test"] }),
    );
    let (mut exchange, page) =
        page_started(&env, &proving_ground, "masked", &settings, |command| {
            command.env(KEY_VARIABLE, &value);
        });
    let forms = proving_ground.url("access.test", "/page-events/forms");
    exchange.listen(&page, "request");
    exchange.listen(&page, "request_failed");
    let mut replies = vec![exchange.call("page.goto", &json!({ "page_id": page, "url": forms }))];
    replies.push(fill(&mut exchange, &page, "input[name=password]", &value));
    replies.push(fill(&mut exchange, &page, "input[name=q]", &value));
    replies.push(click(&mut exchange, &page, "#away"));
    exchange.until("page.request_failed", |request| {
        request["url"]
            .as_str()
            .is_some_and(|url| url.contains("elsewhere.test"))
    });
    let searched = finished_waiter(&mut exchange, &page, "**/page-events/search?*");
    replies.push(click(&mut exchange, &page, "#search-submit"));
    replies.push(exchange.answer(searched));
    replies.push(exchange.call("page.goto", &json!({ "page_id": page, "url": forms })));
    replies.push(fill(&mut exchange, &page, "input[name=password]", &value));
    let signed_in = finished_waiter(&mut exchange, &page, "**/page-events/session");

    replies.push(click(&mut exchange, &page, "#sign-in-submit"));
    replies.push(exchange.answer(signed_in));
    replies.push(exchange.call("run.finish", &json!({ "run_id": "masked" })));
    let events = Value::Array(exchange.events.clone());
    let requests = exchange.data("page.request");
    let (rest, _) = exchange.engine.finish();
    replies.extend(rest);

    let logged = Value::Array(request_log(&env, "masked"));
    let mut bodies = Vec::new();
    for value in [&Value::Array(replies.clone()), &events, &logged] {
        every_post_data(value, &mut bodies);
    }
    let (replies_bytes, events_bytes) = (
        serde_json::to_vec(&replies).unwrap(),
        serde_json::to_vec(&events).unwrap(),
    );
    let leaks: Vec<String> = canaries
        .leaks(&[
            Output::Bytes {
                name: "replies",
                bytes: &replies_bytes,
            },
            Output::Bytes {
                name: "events",
                bytes: &events_bytes,
            },
            Output::Bytes {
                name: "decoded post_data",
                bytes: &bodies,
            },
            Output::Path(env.home()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{:?} in {} at {}", hit.encoding, hit.output, hit.offset))
        .collect();
    let posted = ending_with(&requests, "/page-events/session");
    let search = requests
        .iter()
        .find(|request| {
            request["url"]
                .as_str()
                .is_some_and(|url| url.contains("/page-events/search?"))
        })
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        (
            leaks,
            String::from_utf8_lossy(&decoded(&posted)).into_owned(),
            search["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("?q=%run_state.key_reference%")),
            logged.as_array().is_some_and(|lines| lines
                .iter()
                .any(|line| line["data"]["post_data"].is_string())),
        ),
        (
            Vec::<String>::new(),
            "user=jb&password=%run_state.key_reference%".to_owned(),
            true,
            true,
        ),
        "{requests:?} {logged}"
    );
}

/// An iframe on another site, which the browser runs out of process,
/// reports to its page's listeners: its document is a navigation request
/// of the page, the body of its own fetch reads through the waiter that
/// kept it, read on the frame's own session, and its console message and
/// uncaught exception reach the page's `console` and `page_error`
/// listeners (ADR 0085's #703 note; Playwright reports a page's frames as
/// the page).
/// Covers: story:5-48, operation:page.on, event:page.request, event:page.console, event:page.page_error, operation:page.wait_for_request_finished, operation:response.body
#[test]
fn an_out_of_process_frames_requests_body_console_and_exception_reach_its_page() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page) = page_of(&env, &proving_ground, "framed", &Map::new());
    let framed = proving_ground.url("access.test", "/page-events/framed");
    for event in ["request", "console", "page_error"] {
        exchange.listen(&page, event);
    }
    let waiting = finished_waiter(&mut exchange, &page, "**/page-events/data.json");

    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": framed }));
    let finished = exchange.answer(waiting);
    let body = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": finished["result"]["request"]["request_id"] }),
    );
    exchange.until("page.page_error", |error| {
        error["message"] == json!("page-events frame uncaught")
    });
    let frame_document = ending_with(&exchange.data("page.request"), "/page-events/frame");
    let messages: Vec<Value> = exchange
        .data("page.console")
        .iter()
        .map(|message| message["text"].clone())
        .collect();
    let _ = exchange.call("run.finish", &json!({ "run_id": "framed" }));
    let _ = exchange.engine.finish();

    let read: Value = serde_json::from_slice(&body_of(&body["result"])).unwrap_or_default();
    assert_eq!(
        (
            went["result"]["response"]["status"].clone(),
            finished["result"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("http://access.localhost:")),
            read["page_events"].is_string(),
            (
                frame_document["resource_type"].clone(),
                frame_document["is_navigation_request"].clone(),
            ),
            messages,
        ),
        (
            json!(200),
            true,
            true,
            (json!("document"), json!(true)),
            vec![json!("page-events frame")],
        ),
        "{finished} {body} {frame_document}"
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
