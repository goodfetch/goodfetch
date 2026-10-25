//! Every kind of request a page makes under the run's domain policy on
//! Chromium (spec #563's stories 1 and 7; ADRs 0014 and 0121), against the
//! Proving ground's request kinds pages, whose record of requests and
//! connections is the oracle: a subresource, a navigation the page sends
//! itself, an out-of-process frame's request, a dedicated and a shared
//! worker's, a service worker's as it installs, a WebSocket and a WebRTC
//! connection to a refused host reach nothing, and the same to an allowed
//! host arrive (the paired positive control). A refusal interception judged
//! is a `page.request_failed` of the Run trace with `egress_refused`, a
//! WebSocket's included, whose kind interception cannot see.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests poll the Proving ground and read the Run trace on their own thread, never an async worker"
)]

use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::attached::{Attached, hello};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 632;

/// The host the request kinds pages are served on.
const PAGES: &str = "access.test";

/// The host of the out-of-process frame: another site than the page's,
/// which the browser resolves to loopback itself.
const FRAME: &str = "access.localhost";

/// The host each request goes to, which only the open Run allows.
const REFUSED: &str = "elsewhere.test";

/// How long a positive control waits for its request to arrive.
const ARRIVAL: Duration = Duration::from_secs(20);

/// How long a refused request has, once its page's attempt settled, to
/// arrive anyway.
const MARGIN: Duration = Duration::from_millis(750);

/// A Connection to an Attached engine, the Proving ground's hosts and
/// [`REFUSED`] mapped in each Run's `network.resolve`.
struct Program<'env> {
    engine: Attached<'env>,
    next_id: i64,
    resolve: Map<String, Value>,
}

impl<'env> Program<'env> {
    fn start(env: &'env TestEnv, cache: &Path, proving_ground: &ProvingGround) -> Self {
        let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        engine.send(&hello("collie", "0.1"));
        let _ = engine.next();
        let mut resolve: Map<String, Value> = proving_ground
            .description()
            .resolve
            .iter()
            .map(|(host, address)| (host.clone(), json!([address])))
            .collect();
        resolve.insert(
            REFUSED.to_owned(),
            json!([proving_ground.address().ip().to_string()]),
        );
        Self {
            engine,
            next_id: 0,
            resolve,
        }
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        call(&mut self.engine, self.next_id, method, params)
    }

    /// Starts the Run `run_id`, whose domain policy allows the pages' hosts
    /// and, when `open`, [`REFUSED`] too, and opens its page.
    fn page_of_run(&mut self, run_id: &str, open: bool) -> String {
        let mut allowed = vec![PAGES, FRAME];
        if open {
            allowed.push(REFUSED);
        }
        let settings = json!({
            "network": { "resolve": self.resolve },
            "egress": { "allowed_domains": allowed },
            // Each page settles within seconds; a navigation's error page
            // never does.
            "timeout": { "wait": 10_000 },
        });
        let started = self.call(
            "run.start",
            &json!({ "run_id": run_id, "settings": settings }),
        );
        assert!(started.get("result").is_some(), "{started}");
        opened(&self.call("run.new_page", &json!({ "run_id": run_id })))
            .page_id
            .0
    }

    /// Waits until the page's attempt settled: its title is `done`, or the
    /// wait ran out, as on the error page a refused navigation leaves.
    fn settle(&mut self, page: &str) {
        let _ = self.call(
            "page.wait_for_function",
            &json!({
                "page_id": page,
                "function": "() => document.title === 'done'",
                "polling_interval": 50,
            }),
        );
        std::thread::sleep(MARGIN);
    }

    fn finish(mut self, runs: &[&str]) {
        for run_id in runs {
            let _ = self.call("run.finish", &json!({ "run_id": run_id }));
        }
        let _ = self.engine.finish();
    }
}

/// How many requests to `host` whose target starts with `target` the
/// Proving ground received.
fn received(proving_ground: &ProvingGround, host: &str, target: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| {
            request["host"] == json!(host)
                && request["target"]
                    .as_str()
                    .is_some_and(|seen| seen.starts_with(target))
        })
        .count()
}

/// How many connections the Proving ground's watched target accepted.
fn target_connections(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|connection| connection["listener"] == json!("target"))
        .count()
}

/// Waits until `arrived` holds, or [`ARRIVAL`] passed; whether it held.
fn arrives(arrived: impl Fn() -> bool) -> bool {
    let until = Instant::now() + ARRIVAL;
    while Instant::now() < until {
        if arrived() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    arrived()
}

/// The Error codes of the Run trace's `page.request_failed` lines whose
/// URL is on `host`, of the Run `run_id`.
fn refusals_recorded(env: &TestEnv, run_id: &str, host: &str) -> Vec<Value> {
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
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["event"] == json!("page.request_failed"))
        .filter(|line| {
            line["data"]["url"]
                .as_str()
                .and_then(|url| url.split_once("://"))
                .is_some_and(|(_, rest)| rest.split(['/', ':', '?']).next() == Some(host))
        })
        .map(|line| {
            // A page's request carries its failure's error; the engine's
            // own record of a request no page reports is the error itself.
            let failure = &line["data"]["failure"];
            failure["error"]["code"]
                .as_str()
                .or_else(|| failure["code"].as_str())
                .map_or(Value::Null, Value::from)
        })
        .collect()
}

/// What became of one kind's request in a held Run and an open one.
#[derive(Debug, PartialEq, Eq)]
struct Attempts {
    /// How many reached the refused host from the held Run.
    held: usize,
    /// Whether one reached it from the open Run.
    open: bool,
    /// The Error codes the held Run's trace recorded for it.
    refusals: Vec<Value>,
}

/// Opens the page `path` gives on `host` in a held Run, then in an open
/// one, and counts what reached the refused host by `reached`.
fn attempt(
    host: &str,
    path: impl Fn(&ProvingGround) -> String,
    reached: impl Fn(&ProvingGround) -> usize,
) -> Attempts {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground);
    let url = proving_ground.url(host, &path(&proving_ground));

    let held = program.page_of_run("held", false);
    let _ = program.call("page.goto", &json!({ "page_id": held, "url": url }));
    program.settle(&held);
    let held_reached = reached(&proving_ground);
    let open = program.page_of_run("open", true);
    let _ = program.call("page.goto", &json!({ "page_id": open, "url": url }));
    let open_reached = arrives(|| reached(&proving_ground) > held_reached);
    program.finish(&["held", "open"]);

    Attempts {
        held: held_reached,
        open: open_reached,
        refusals: refusals_recorded(&env, "held", REFUSED),
    }
}

/// One kind's page, its request going to [`REFUSED`], a peer connection's
/// to the watched target's port.
fn kind_page(kind: &'static str) -> impl Fn(&ProvingGround) -> String {
    move |proving_ground| {
        let target = &proving_ground.description().fixtures.target;
        let turn = target.rsplit(':').next().unwrap_or_default();
        format!("/request-kinds/{kind}?to={REFUSED}&frame={FRAME}&turn={turn}")
    }
}

/// What reached the refused host at the kinds' target.
fn reached_target(kind: &str) -> impl Fn(&ProvingGround) -> usize {
    let target = format!("/request-kinds/reached?kind={kind}");
    move |proving_ground| received(proving_ground, REFUSED, &target)
}

/// What one kind's attempts should be: none held, one open, and the held
/// Run's trace recording the refusal once.
fn refused_and_recorded() -> Attempts {
    Attempts {
        held: 0,
        open: true,
        refusals: vec![json!("egress_refused")],
    }
}

/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn a_subresource_to_a_refused_host_is_refused_and_one_to_an_allowed_host_arrives() {
    let attempts = attempt(PAGES, kind_page("fetch"), reached_target("fetch"));

    assert_eq!(attempts, refused_and_recorded());
}

/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn a_navigation_the_page_sends_itself_to_a_refused_host_is_refused() {
    let attempts = attempt(PAGES, kind_page("navigation"), reached_target("navigation"));

    assert_eq!(attempts, refused_and_recorded());
}

/// An out-of-process frame's own request passes its page's interception.
///
/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn an_out_of_process_frames_request_to_a_refused_host_is_refused() {
    let attempts = attempt(PAGES, kind_page("frame"), reached_target("frame"));

    assert_eq!(attempts, refused_and_recorded());
}

/// A dedicated worker's requests pass its page's interception. Neither the
/// page's events nor the Run trace see a worker's requests, so only the
/// connection record says the refused one went nowhere.
///
/// Covers: story:4-1, operation:page.goto
#[test]
fn a_dedicated_workers_request_to_a_refused_host_is_refused() {
    let attempts = attempt(
        PAGES,
        kind_page("dedicated_worker"),
        reached_target("dedicated_worker"),
    );

    assert_eq!((attempts.held, attempts.open), (0, true));
}

/// A shared worker belongs to no page: its requests pass its Session's
/// interception, each recorded in the Run trace.
///
/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn a_shared_workers_request_to_a_refused_host_is_refused() {
    let attempts = attempt(
        PAGES,
        kind_page("shared_worker"),
        reached_target("shared_worker"),
    );

    assert_eq!(attempts, refused_and_recorded());
}

/// A service worker that fetches from a refused host as it installs: it is
/// held before its first line runs, so even that first request is refused,
/// as a page's subresource would be.
///
/// Covers: story:4-7, operation:page.goto
#[test]
fn a_service_worker_fetching_from_a_refused_host_as_it_installs_is_refused() {
    let attempts = attempt(
        FRAME,
        |_| "/service-worker/".to_owned(),
        |proving_ground| received(proving_ground, REFUSED, "/service-worker/forbidden.json"),
    );

    assert_eq!((attempts.held, attempts.open), (0, true));
}

/// No interception sees a WebSocket's handshake: the browser holds it to
/// the domain policy where it connects, and the engine records the refusal
/// as a `page.request_failed` of the Run trace.
///
/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn a_websocket_to_a_refused_host_is_blocked_and_recorded_as_a_failed_request() {
    let attempts = attempt(PAGES, kind_page("websocket"), reached_target("websocket"));

    assert_eq!(attempts, refused_and_recorded());
}

/// A peer connection's TURN server over TCP: no interception sees it, so
/// the browser holds it to the domain policy where it connects, and sends
/// no UDP a policy could not judge.
///
/// Covers: story:4-1, operation:page.goto
#[test]
fn a_webrtc_connection_to_a_refused_host_reaches_nothing() {
    let attempts = attempt(PAGES, kind_page("webrtc"), target_connections);

    assert_eq!((attempts.held, attempts.open), (0, true));
}
