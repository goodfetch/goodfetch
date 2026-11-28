//! Every kind of request a page makes under the run's domain policy on
//! Chromium (spec #563's stories 1 and 7; ADRs 0014 and 0121), against the
//! Proving ground's request kinds pages, whose record of requests and
//! connections is the oracle: a subresource, a navigation the page sends
//! itself, an out-of-process frame's request, a dedicated and a shared
//! worker's, a service worker's as it installs, a WebSocket and a WebRTC
//! connection to a refused host reach nothing, and the same to an allowed
//! host arrive (the paired positive control). A refusal interception judged
//! is a `page.request_failed` of the Run trace with `egress_refused`, a
//! WebSocket's included, whose kind interception cannot see. The SSRF
//! table's runner for these kinds (ADR 0067) makes each kind's requests to
//! the table's hosts from one page.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests poll the Proving ground and read the Run trace on their own thread, never an async worker"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::RUN_TRACE_REQUESTS;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    Answer, BROWSERS_PATH_VARIABLE, Outcome, ProvingGround, Refusal, Runner, SsrfCase, SsrfTable,
    TestEnv, browser_cache,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::page::{BROWSER_DEADLINE, opened};
use crate::proxy::PASSWORD_VARIABLE;
use goodfetch_test_client::{Attached, hello};

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
struct Program {
    engine: Attached,
    next_id: i64,
    resolve: Map<String, Value>,
}

impl Program {
    fn start(env: &TestEnv, cache: &Path, proving_ground: &ProvingGround) -> Self {
        Self::start_with_env(env, cache, proving_ground, &[])
    }

    fn start_with_env(
        env: &TestEnv,
        cache: &Path,
        proving_ground: &ProvingGround,
        extra_env: &[(&str, &str)],
    ) -> Self {
        let mut engine = Attached::start_with(env, COLLIE, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
            for (name, value) in extra_env {
                command.env(name, value);
            }
        });
        engine.send(&hello(Product::Collie));
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
        self.engine.ask(self.next_id, method, params)
    }

    /// Starts the Run `run_id`, whose domain policy allows the pages' hosts
    /// and, when `open`, [`REFUSED`] too, and opens its page (in a Session
    /// carrying `session_settings` when given).
    fn page_of_run_with_session(
        &mut self,
        run_id: &str,
        open: bool,
        session_settings: Option<&Value>,
    ) -> String {
        let mut allowed = vec![PAGES, FRAME];
        if open {
            allowed.push(REFUSED);
        }
        let settings = json!({
            "network": { "resolve": self.resolve },
            "egress": {
                "allowed_domains": allowed,
                "private_ranges": self.resolve.keys().collect::<Vec<_>>(),
            },
            // Each page settles within seconds; a navigation's error page
            // never does.
            "timeout": { "wait": 10_000 },
        });
        let started = self.call(
            "run.start",
            &json!({ "run_id": run_id, "settings": settings }),
        );
        assert!(started.get("result").is_some(), "{started}");
        if let Some(session_settings) = session_settings {
            let session = self.call(
                "run.new_session",
                &json!({ "run_id": run_id, "settings": session_settings }),
            );
            assert!(session.get("result").is_some(), "{session}");
            let session_id = session["result"]["session_id"].clone();
            opened(&self.call("session.new_page", &json!({ "session_id": session_id })))
                .page_id
                .0
        } else {
            opened(&self.call("run.new_page", &json!({ "run_id": run_id })))
                .page_id
                .0
        }
    }

    /// The Run settings an SSRF case's requests go under: its domain policy,
    /// the pages' hosts allowed beside it, its lookups in `network.resolve`,
    /// `public` the Proving ground's address, and the Proving ground's
    /// hosts and the `listed` names of the positive controls in
    /// `egress.private_ranges`.
    fn case_settings(
        &self,
        case: &SsrfCase,
        proving_ground: &ProvingGround,
        listed: &[&str],
    ) -> Value {
        let public = proving_ground.address().ip().to_string();
        let mut resolve = self.resolve.clone();
        for lookup in &case.lookups {
            let answers: Vec<String> = lookup
                .answers
                .iter()
                .map(|answer| match answer {
                    Answer::Public => public.clone(),
                    Answer::Address(address) => address.to_string(),
                })
                .collect();
            resolve.insert(lookup.name.clone(), json!(answers));
        }
        let (mut allowed, blocked) = case.domains.as_ref().map_or_else(
            || (vec!["*".to_owned()], Vec::new()),
            |domains| (domains.allowed.clone(), domains.blocked.clone()),
        );
        allowed.extend([PAGES.to_owned(), FRAME.to_owned()]);
        let mut ranges: Vec<String> = self.resolve.keys().cloned().collect();
        ranges.extend(listed.iter().map(|name| (*name).to_owned()));
        json!({
            "network": { "resolve": resolve },
            "egress": {
                "allowed_domains": allowed,
                "blocked_domains": blocked,
                "private_ranges": ranges,
            },
            "timeout": { "wait": 10_000 },
        })
    }

    /// Starts the Run `run_id` under `settings` and opens its page.
    fn page_with(&mut self, run_id: &str, settings: &Value) -> String {
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
        .filter(|request| request.host == host && request.target.starts_with(target))
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

/// What became of one kind's request in a held Run and an open one.
#[derive(Debug, PartialEq, Eq)]
struct Attempts {
    /// How many reached the refused host from the held Run.
    held: usize,
    /// Whether one reached it from the open Run.
    open: bool,
    /// The `(Error code, rule)` pairs the held Run's trace recorded for it.
    refusals: Vec<(Value, Value)>,
    /// Upstream proxy relays to [`REFUSED`] recorded during the held Run.
    held_relays: Vec<String>,
    /// Upstream proxy relays to [`REFUSED`] recorded during the open Run.
    open_relays: Vec<String>,
}

/// Names of Proving ground proxies that relayed a connection whose destination
/// host is [`REFUSED`] (matching authority `host:port` or URL host, never query
/// parameters of the page URL).
fn relayed_to_refused(proving_ground: &ProvingGround) -> Vec<String> {
    let authority_prefix = format!("{REFUSED}:");
    proving_ground.observations().unwrap()["relays"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|relay| {
            relay["target"].as_str().is_some_and(|target| {
                target.starts_with(&authority_prefix) || host_of(target).as_deref() == Some(REFUSED)
            })
        })
        .filter_map(|relay| relay["proxy"].as_str().map(str::to_owned))
        .collect()
}

/// Opens the page `path` gives on `host` in a held Run, then in an open
/// one, and counts what reached the refused host by `reached`.
fn attempt(
    host: &str,
    path: impl Fn(&ProvingGround) -> String,
    reached: impl Fn(&ProvingGround) -> usize,
) -> Attempts {
    attempt_with_session(host, path, reached, |_| (Vec::new(), None))
}

/// Like [`attempt`], configuring each Session's own settings and extra
/// engine environment variables from the Proving ground's fixtures.
fn attempt_with_session(
    host: &str,
    path: impl Fn(&ProvingGround) -> String,
    reached: impl Fn(&ProvingGround) -> usize,
    session: impl FnOnce(&ProvingGround) -> (Vec<(String, String)>, Option<Value>),
) -> Attempts {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let (extra_env, session_settings) = session(&proving_ground);
    let env_refs: Vec<(&str, &str)> = extra_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut program = Program::start_with_env(&env, &cache, &proving_ground, &env_refs);
    let url = proving_ground.url(host, &path(&proving_ground));

    let held = program.page_of_run_with_session("held", false, session_settings.as_ref());
    let _ = program.call("page.goto", &json!({ "page_id": held, "url": url }));
    program.settle(&held);
    let held_reached = reached(&proving_ground);
    let held_relays = relayed_to_refused(&proving_ground);
    let open = program.page_of_run_with_session("open", true, session_settings.as_ref());
    let _ = program.call("page.goto", &json!({ "page_id": open, "url": url }));
    let open_reached = arrives(|| reached(&proving_ground) > held_reached);
    let open_relays = relayed_to_refused(&proving_ground);
    program.finish(&["held", "open"]);

    let (_, recorded) = refusals_from(&env, "held", 0);
    let refusals = recorded
        .into_iter()
        .filter(|(recorded_host, _, _)| recorded_host == REFUSED)
        .map(|(_, code, rule)| (code, rule))
        .collect();

    Attempts {
        held: held_reached,
        open: open_reached,
        refusals,
        held_relays,
        open_relays,
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

/// Checks that the held Run reached nothing and recorded only
/// `egress_refused` / `domain_policy` refusals, while the paired open Run
/// reached the target.
fn assert_refused_and_recorded(attempts: &Attempts, exact_one: bool) {
    let expected = (json!("egress_refused"), json!("domain_policy"));
    assert_eq!((attempts.held, attempts.open), (0, true), "{attempts:?}");
    if exact_one {
        assert_eq!(attempts.refusals, vec![expected], "{attempts:?}");
    } else {
        assert!(
            !attempts.refusals.is_empty() && attempts.refusals.iter().all(|r| *r == expected),
            "{attempts:?}"
        );
    }
}

/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn a_navigation_the_page_sends_itself_to_a_refused_host_is_refused() {
    let attempts = attempt(PAGES, kind_page("navigation"), reached_target("navigation"));

    assert_refused_and_recorded(&attempts, true);
}

/// A peer connection's TURN server over TCP: no interception sees it, so
/// the egress relay holds it to the domain policy where it connects,
/// records the refusal as `page.request_failed` with `egress_refused` and
/// `rule: "domain_policy"`, and sends no UDP a policy could not judge
/// (ticket 2366).
///
/// Covers: story:4-1, error:egress_refused, operation:page.goto
#[test]
fn a_webrtc_connection_to_a_refused_host_reaches_nothing() {
    let attempts = attempt(PAGES, kind_page("webrtc"), target_connections);

    assert_refused_and_recorded(&attempts, false);
}

/// A Session with its own HTTP or SOCKS5 proxy still chains through the
/// egress relay, which holds its WebSocket and WebRTC TURN-over-TCP
/// connections to the domain policy and records each refusal as
/// `page.request_failed` with `egress_refused` and `rule: "domain_policy"`,
/// while the paired open Run reaches the Proving ground through that proxy
/// (ticket 2366).
///
/// Covers: story:4-1, error:egress_refused, setting:proxy.server, operation:run.new_session, operation:page.goto
#[cfg(unix)]
#[rstest]
#[case::http_websocket("http", "open", false, "websocket")]
#[case::http_webrtc("http", "open", false, "webrtc")]
/// Covers: setting:proxy.username, setting:proxy.password
#[case::socks5_websocket("socks5", "socks5", true, "websocket")]
/// Covers: setting:proxy.username, setting:proxy.password
#[case::socks5_webrtc("socks5", "socks5", true, "webrtc")]
fn a_session_with_its_own_proxy_holds_websocket_and_webrtc_to_the_domain_policy(
    #[case] scheme: &str,
    #[case] proxy_name: &str,
    #[case] credentials: bool,
    #[case] kind: &'static str,
) {
    let oracle: fn(&ProvingGround) -> usize = match kind {
        "webrtc" => target_connections,
        _ => |pg| received(pg, REFUSED, "/request-kinds/reached?kind=websocket"),
    };
    let attempts = attempt_with_session(PAGES, kind_page(kind), oracle, |proving_ground| {
        let fixtures = &proving_ground.description().fixtures;
        let mut proxy = json!({ "server": format!("{scheme}://{}", fixtures.proxies[proxy_name]) });
        let mut extra_env = Vec::new();
        if credentials {
            proxy["username"] = json!(fixtures.proxy_credentials.username);
            proxy["password"] = json!(format!("env:{PASSWORD_VARIABLE}"));
            extra_env.push((
                PASSWORD_VARIABLE.to_owned(),
                fixtures.proxy_credentials.password.clone(),
            ));
        }
        (extra_env, Some(json!({ "proxy": proxy })))
    });

    assert_refused_and_recorded(&attempts, kind == "websocket");
    assert!(attempts.held_relays.is_empty(), "{attempts:?}");
    assert!(
        attempts.open_relays.iter().any(|name| name == proxy_name),
        "{attempts:?}"
    );
}

/// The value of the first `rule` field in `value`, at any depth.
fn rule_in(value: &Value) -> Option<Value> {
    match value {
        Value::Object(fields) => fields
            .get("rule")
            .cloned()
            .or_else(|| fields.values().find_map(rule_in)),
        Value::Array(items) => items.iter().find_map(rule_in),
        _ => None,
    }
}

/// The host of `url` as the WHATWG parser writes it, an IPv6 address in
/// brackets.
fn host_of(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    Some(match url.host()? {
        url::Host::Ipv6(address) => format!("[{address}]"),
        host => host.to_string(),
    })
}

/// The refusals the Run trace of `run_id` recorded from its line `from` on,
/// as `(host, Error code, rule)`.
fn refusals_from(env: &TestEnv, run_id: &str, from: usize) -> (usize, Vec<(String, Value, Value)>) {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_REQUESTS);
    let lines: Vec<Value> = std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    let refusals = lines
        .iter()
        .skip(from)
        .filter(|line| {
            line["event"] == json!("page.request_failed")
                || (line["event"] == json!("service_worker.request")
                    && !line["data"]["failure"].is_null())
        })
        .filter_map(|line| {
            let host = host_of(line["data"]["url"].as_str()?)?;
            let failure = &line["data"]["failure"];
            let code = failure["error"]["code"]
                .as_str()
                .or_else(|| failure["code"].as_str())
                .map_or(Value::Null, Value::from);
            Some((host, code, rule_in(failure).unwrap_or(Value::Null)))
        })
        .collect();
    (lines.len(), refusals)
}

/// The hosts of the requests for the kinds' target that reached the
/// Proving ground, from its request `from` on.
fn reached_from(proving_ground: &ProvingGround, kind: &str, from: usize) -> (usize, Vec<String>) {
    let target = format!("/request-kinds/reached?kind={kind}");
    let requests = proving_ground.received_requests().unwrap();
    let reached = requests
        .every()
        .skip(from)
        .filter(|request| request.target.starts_with(&target))
        .map(|request| request.host.to_ascii_lowercase())
        .collect();
    (requests.every().count(), reached)
}

/// The SSRF table's thin runner on Chromium (ADR 0067) for each kind of
/// request a page makes (ADR 0014): every address case and domain policy
/// case, as the request kinds page makes it to the case's host (as the
/// WHATWG parser writes it) on the Proving ground's port, is refused with
/// `egress_refused` and its rule, recorded in the Run trace as a
/// `page.request_failed`, and never reaches the Proving ground. The page
/// makes one kind's requests to many hosts at once, each host once per
/// page; one Run per domain policy and mapping, a case's lookups in its
/// `network.resolve`, finished once its cases are done, so the test holds
/// one browser and one page at a time: the Resource governor admits no
/// page past its floor, one, while the host is under pressure (ADR 0040).
/// The positive control, an allowed host, reaches the Proving ground. The
/// cases a kind never brings to the engine ([`Unjudged`]) reach nothing,
/// with no refusal recorded.
///
/// Covers: story:4-1, story:4-7, story:4-89, error:egress_refused, operation:page.goto
#[rstest]
#[case::subresource("fetch", PAGES, Unjudged::Unspecified)]
#[case::out_of_process_frame("frame", PAGES, Unjudged::Unparsed)]
#[case::dedicated_worker("dedicated_worker", PAGES, Unjudged::Unspecified)]
#[case::shared_worker("shared_worker", PAGES, Unjudged::Unspecified)]
#[case::service_worker("service_worker", FRAME, Unjudged::Unparsed)]
#[case::websocket("websocket", PAGES, Unjudged::Unparsed)]
fn every_address_case_of_the_ssrf_table_is_refused_for_each_kind_of_request_on_chromium(
    #[case] kind: &str,
    #[case] page_host: &str,
    #[case] unjudged: Unjudged,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let public = proving_ground.address().ip();
    let runner = Runner::committed(
        "chromium_request_kinds",
        file!(),
        "every_address_case_of_the_ssrf_table_is_refused_for_each_kind_of_request_on_chromium",
    )
    .unwrap();
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "an_allowed_host"
        family = "domain_policy"
        url = "http://elsewhere.test/"
        target = "public"
        refusal = { code = "egress_refused", rule = "domain_policy" }
        "#,
    )
    .unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground);
    // Each case's settings and host; the cases of one settings share a Run,
    // and each page of it makes one request per host.
    let cases: Vec<&SsrfCase> = controls.cases.iter().chain(runner.cases()).collect();
    let mut groups: Vec<(Value, Vec<(&SsrfCase, String)>)> = Vec::new();
    for case in &cases {
        let Some(host) = host_of(&case.url) else {
            continue;
        };
        let settings = program.case_settings(case, &proving_ground, &[]);
        match groups.iter_mut().find(|(shared, _)| *shared == settings) {
            Some((_, members)) => members.push((case, host)),
            None => groups.push((settings, vec![(case, host)])),
        }
    }
    let mut outcomes: BTreeMap<String, Outcome> = BTreeMap::new();
    let mut waves = 0;
    for (index, (settings, mut members)) in groups.into_iter().enumerate() {
        let run_id = format!("run-{index}");
        let page = program.page_with(&run_id, &settings);
        while !members.is_empty() {
            let mut hosts = BTreeSet::new();
            let (wave, rest): (Vec<_>, Vec<_>) = members
                .into_iter()
                .partition(|(_, host)| hosts.insert(host.clone()));
            members = rest;
            let (lines, _) = refusals_from(&env, &run_id, 0);
            let (requested, _) = reached_from(&proving_ground, kind, 0);
            let to: Vec<String> = wave
                .iter()
                .map(|(_, host)| format!("to={}", encode(host)))
                .collect();
            waves += 1;
            let url = proving_ground.url(
                page_host,
                &format!(
                    "/request-kinds/{kind}?{}&frame={FRAME}&wave={waves}",
                    to.join("&")
                ),
            );
            let _ = program.call("page.goto", &json!({ "page_id": page, "url": url }));
            program.settle(&page);
            let (_, refusals) = refusals_from(&env, &run_id, lines);
            let (_, reached) = reached_from(&proving_ground, kind, requested);
            for (case, host) in wave {
                let refusal = refusals
                    .iter()
                    .find(|(refused, code, _)| *refused == host && *code == json!("egress_refused"))
                    .and_then(|(_, _, rule)| serde_json::from_value(rule.clone()).ok())
                    .map(|rule| Refusal::EgressRefused { rule });
                let arrived = reached
                    .iter()
                    .filter(|arrived| {
                        arrived.trim_end_matches(&format!(":{}", proving_ground.address().port()))
                            == host.to_ascii_lowercase()
                    })
                    .count();
                outcomes.insert(
                    case.name.clone(),
                    Outcome {
                        refusal,
                        connections: vec![public; arrived],
                    },
                );
            }
        }
        let _ = program.call("run.finish", &json!({ "run_id": run_id }));
    }
    program.finish(&[]);

    let result = runner.run(&controls.cases, public, |case| {
        outcomes.remove(&case.name).unwrap_or_default()
    });

    let unrecorded: Vec<_> = runner
        .cases()
        .iter()
        .filter(|case| unjudged.holds(case))
        .map(|case| (case.name.clone(), None, false))
        .collect();
    assert_eq!(
        result.map_err(|failed| {
            let let_through: Vec<_> = failed
                .let_through
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect();
            (failed.no_control, failed.held, let_through)
        }),
        Err((false, Vec::new(), unrecorded))
    );
}

/// The table's cases a kind of request never brings to the engine, which
/// then records no refusal of theirs: each still reaches nothing.
#[derive(Clone, Copy, Debug)]
enum Unjudged {
    /// The URLs the WHATWG parser refuses, which no page can request.
    Unparsed,
    /// Those, and the unspecified addresses `0.0.0.0` and `::`, which
    /// Chromium refuses to a page's or a dedicated or shared worker's
    /// `fetch` itself, before its proxy, the Session's egress relay, sees
    /// the request.
    Unspecified,
}

impl Unjudged {
    /// Whether `case` is one of them.
    fn holds(self, case: &SsrfCase) -> bool {
        let unparsed = host_of(&case.url).is_none();
        let unspecified = case
            .target_address(PUBLIC_STAND_IN)
            .is_some_and(|target| target.to_canonical().is_unspecified());
        match self {
            Self::Unparsed => unparsed,
            Self::Unspecified => unparsed || unspecified,
        }
    }
}

/// An address that stands for a public one, for reading a case's target.
const PUBLIC_STAND_IN: std::net::IpAddr =
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 31, 196, 1));

/// `text` percent-encoded for a query's value.
fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}
