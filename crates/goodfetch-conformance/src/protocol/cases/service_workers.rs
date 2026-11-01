//! The page suite's service worker cases, the Page state conformance cases'
//! service worker lines on Chromium (ADR 0121, ticket #707), on the Proving
//! ground's service worker page: its worker answers one JSON call from its
//! Cache Storage, passes another on with its own fetch, and requests, as it
//! installs, a body of its site, a body on a host the Run's domain policy
//! forbids and a path a tracker list blocks. With `browser.service_workers`
//! unset, the worker's own requests pass the domain policy, the Filter
//! lists and a Session route, never a page route, and only the Run trace
//! records them; both calls arrive in the page's `response` event with
//! `from_service_worker` true, the cached one in a `page.wait_for_response`
//! armed before the navigation too, and the page reaches `networkidle`,
//! which the worker's own requests, of no page, never hold back (ticket
//! #705). With `block`, `register()` rejects as the
//! browser's own refusal does, `register` stays native, no worker controls
//! the page, and the page's own calls reach its route and the site. In a
//! headless Run, the worker's own requests show the site the user agent
//! the build shows with a window, as its pages' do (ticket #696).

use goodfetch_protocol::connection::CANCEL;
use goodfetch_protocol::events::{CREDIT, EVENTS};
use goodfetch_protocol::page::events::PAGE_ON;
use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_HANDLE, SESSION_ROUTE};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::waits::{PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_RESPONSE};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened};
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::clean_up::run_trace;
use super::launch::{IDENTITY_HOST, reports};
use super::routes::{glob, received};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::{BROWSER_DEADLINE, EngineTarget};
use crate::proving_ground::ReceivedRequests;

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    UnderTheInterception,
    Blocked,
    Identity,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-service-workers-own-requests-pass-the-domain-policy-the-filter-lists-and-session-routes-never-a-page-route" => {
                Self::UnderTheInterception
            }
            "with-service-workers-blocked-register-rejects-natively-and-the-pages-calls-reach-its-route" => {
                Self::Blocked
            }
            "a-headless-service-workers-requests-show-the-site-its-windows-identity" => {
                Self::Identity
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::UnderTheInterception => under_the_interception(connection, target, site, checks),
            Self::Blocked => blocked(connection, site, checks),
            Self::Identity => identity(connection, site, checks),
        }
    }
}

/// The credits a subscription grants at once, more than a case's events.
const CREDITS: u32 = 10_000;

/// How long the harness waits for one message while it also waits for the
/// page's report.
const POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// The timeout of the `networkidle` wait while the worker's request is
/// held: far shorter than the Proving ground's minute of hold, far longer
/// than 500 ms of quiet.
const IDLE_TIMEOUT_MS: u64 = 20_000;

/// The trace event a service worker's own request is recorded under.
const SERVICE_WORKER_REQUEST: &str = "service_worker.request";

/// The Connection as these cases read it: the events of the case's own
/// listening calls, every `route.handle` request the engine sent, each
/// answered by the handler ending without an answer, so the request falls
/// through, and the answers read before their call's was asked for.
struct Watch<'connection> {
    connection: &'connection mut Connection,
    events: Vec<Value>,
    /// The case's `page.on` calls, which carry its events.
    listeners: Vec<i64>,
    /// The params of each `route.handle` request, in arrival order.
    handled: Vec<Value>,
    answers: Vec<Value>,
}

impl<'connection> Watch<'connection> {
    fn new(connection: &'connection mut Connection) -> Self {
        Self {
            connection,
            events: Vec::new(),
            listeners: Vec::new(),
            handled: Vec::new(),
            answers: Vec::new(),
        }
    }

    /// Takes in one message within `deadline`: a route's request, a
    /// batch's events or an answer, kept, or else the message; `None` for
    /// those, or when none came.
    fn read(&mut self, deadline: std::time::Duration) -> Result<Option<Value>, Gone> {
        let message = match self.connection.next_within(deadline) {
            Ok(message) => message,
            Err(Gone::Silent(_)) if deadline == POLL => return Ok(None),
            Err(gone) => return Err(gone),
        };
        if message["method"] == json!(ROUTE_HANDLE.name) {
            self.handled.push(message["params"].clone());
            self.connection.reply(&message["id"], &json!({}))?;
            return Ok(None);
        }
        if message["method"] == json!(EVENTS) {
            if self
                .listeners
                .iter()
                .any(|id| message["params"]["id"] == json!(id))
            {
                self.events.extend(
                    message["params"]["events"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                );
            }
            return Ok(None);
        }
        if message.get("method").is_none() {
            self.answers.push(message);
            return Ok(None);
        }
        Ok(Some(message))
    }

    /// The answer to the request `id`.
    fn answer(&mut self, id: i64) -> Result<Value, Gone> {
        loop {
            if let Some(at) = self
                .answers
                .iter()
                .position(|answer| answer["id"] == json!(id))
            {
                return Ok(self.answers.remove(at));
            }
            self.read(BROWSER_DEADLINE)?;
        }
    }

    fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        let id = self.connection.request(method, params)?;
        self.answer(id)
    }

    /// Serves the engine until the page at `page` reported whether a worker
    /// controls it, or the wait runs out; its reports as `(check, observed)`.
    fn until_reported(&mut self, site: &Site, page: &str) -> Result<Vec<(String, String)>, Gone> {
        let polls = BROWSER_DEADLINE.as_millis() / POLL.as_millis();
        let mut seen = Vec::new();
        for _ in 0..polls {
            seen = site
                .proving_ground
                .observations()
                .map(|observations| reports(&observations, page))
                .unwrap_or_default();
            if seen.iter().any(|(check, _)| check == "controlled") {
                break;
            }
            self.read(POLL)?;
        }
        Ok(seen)
    }

    /// Serves the engine until the Proving ground received a request for
    /// `target` on the page's host, or the wait runs out: whether it did.
    fn until_received(&mut self, site: &Site, target: &str) -> Result<bool, Gone> {
        let polls = BROWSER_DEADLINE.as_millis() / POLL.as_millis();
        for _ in 0..polls {
            let received = site
                .proving_ground
                .received_requests()
                .unwrap_or_default()
                .of_the_case()
                .any(|request| {
                    request["host"] == json!(IDENTITY_HOST) && request["target"] == json!(target)
                });
            if received {
                return Ok(true);
            }
            self.read(POLL)?;
        }
        Ok(false)
    }

    /// Serves the engine until a `page.response` event arrived for each
    /// URL ending in one of `suffixes`, or the wait runs out: whether they
    /// all arrived. The page reports to the Proving ground on its own
    /// connection, so its report can arrive before the engine's events of
    /// the calls it made before it: only the events' arrival says they
    /// were all read.
    fn until_responded(&mut self, suffixes: &[&str]) -> Result<bool, Gone> {
        let polls = BROWSER_DEADLINE.as_millis() / POLL.as_millis();
        for _ in 0..polls {
            let responses = self.data("page.response");
            if suffixes.iter().all(|suffix| {
                responses
                    .iter()
                    .any(|exchange| url_of(&exchange["response"]).ends_with(suffix))
            }) {
                return Ok(true);
            }
            self.read(POLL)?;
        }
        Ok(false)
    }

    /// Listens to `event` of the page `page_id`, granting the listening
    /// call credits to spare.
    fn listen(&mut self, page_id: &Value, event: &str) -> Result<(), Gone> {
        let id = self
            .connection
            .request(PAGE_ON.name, &json!({ "page_id": page_id, "event": event }))?;
        self.listeners.push(id);
        self.connection
            .notify(CREDIT, &json!({ "id": id, "credits": CREDITS }))
    }

    /// Cancels the case's `calls` in flight, whose calls the Run would
    /// otherwise wait on, and finishes the Run `run_id`, serving every
    /// route's request until it answers.
    fn end(mut self, run_id: &str, calls: &[i64]) -> Result<(), Gone> {
        for id in calls {
            self.connection.notify(CANCEL, &json!({ "id": id }))?;
        }
        self.call(RUN_FINISH.name, &json!({ "run_id": run_id }))
            .map(drop)
    }

    /// The data of every event `name` read so far.
    fn data(&self, name: &str) -> Vec<Value> {
        self.events
            .iter()
            .filter(|event| event["event"] == json!(name))
            .map(|event| event["data"].clone())
            .collect()
    }

    /// The URLs of the `route.handle` requests of the route call `route`.
    fn handled_by(&self, route: i64) -> Vec<&Value> {
        self.handled
            .iter()
            .filter(|params| params["id"] == json!(route))
            .collect()
    }
}

/// The URL of `value`'s `url`.
fn url_of(value: &Value) -> &str {
    value["url"].as_str().unwrap_or_default()
}

/// What `reports` say for `check`.
fn reported<'a>(reports: &'a [(String, String)], check: &str) -> Option<&'a str> {
    reports
        .iter()
        .find(|(reported, _)| reported == check)
        .map(|(_, observed)| observed.as_str())
}

/// The service worker page's path and query for the case `case`, as its
/// reports name it, and its URL on the secure host of `site`.
fn page_of(site: &Site, case: &str) -> (String, String) {
    let path = format!("/service-worker/?case={case}");
    let url = format!("http://{IDENTITY_HOST}:{}{path}", site.port);
    (path, url)
}

/// The settings of these cases' Runs: a domain policy that allows the
/// page's own host alone.
fn settings() -> Value {
    json!({ "egress": { "allowed_domains": [IDENTITY_HOST] } })
}

/// With `browser.service_workers` unset, the worker's own requests pass
/// the domain policy, the Filter lists and a Session route, never a page
/// route; the page's events never report them and the Run trace does; the
/// calls the worker answered arrive with `from_service_worker` true.
#[expect(
    clippy::too_many_lines,
    reason = "one Run's page, its listeners, routes and waiter armed before the one navigation every check reads"
)]
fn under_the_interception(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-service-worker";
    let (path, url) = page_of(site, "allow&held");
    let query = "?case=allow&held";
    start_run(connection, site, run_id, &settings(), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let mut watch = Watch::new(connection);
    for event in ["request", "response"] {
        watch.listen(&json!(page.page_id), event)?;
    }
    let session_route = watch.connection.request(
        SESSION_ROUTE.name,
        &json!({ "session_id": page.session_id, "url": glob("**/service-worker/**") }),
    )?;
    let page_route = watch.connection.request(
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/service-worker/**") }),
    )?;
    let cached_response = watch.connection.request(
        PAGE_WAIT_FOR_RESPONSE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/service-worker/cached.json*") }),
    )?;
    let loaded = watch.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    checks.check(
        "the service worker page loads",
        loaded.get("result").is_some(),
    );
    let reports = watch.until_reported(site, &path)?;
    checks.check(
        "the page's worker registers and controls the page",
        (
            reported(&reports, "register"),
            reported(&reports, "controlled"),
        ) == (Some("registered"), Some("true")),
    );
    let own = format!("/service-worker/own.json{query}");
    let network = format!("/service-worker/network.json{query}");
    let cached = format!("/service-worker/cached.json{query}");
    let forbidden = format!("/service-worker/forbidden.json{query}");
    let tracker = format!("/service-worker/js/adobe-analytics.js{query}");
    let by_session = watch.handled_by(session_route);
    let worker_seen = |suffix: &str| {
        by_session.iter().any(|params| {
            url_of(&params["request"]).ends_with(suffix) && params.get("page_id").is_none()
        })
    };
    checks.check(
        "a Session route sees the worker's own requests, which name no page",
        worker_seen(&own) && worker_seen(&forbidden) && worker_seen(&tracker),
    );
    checks.check(
        "a Session route sees the worker's own fetch of the call it passes on",
        worker_seen(&network),
    );
    let by_page = watch.handled_by(page_route);
    checks.check(
        "a page route sees none of the worker's requests, nor a call the worker answered",
        !by_page.iter().any(|params| {
            let url = url_of(&params["request"]);
            [&own, &network, &cached, &forbidden, &tracker]
                .iter()
                .any(|suffix| url.ends_with(suffix.as_str()))
        }),
    );
    checks.check(
        "the worker's own request reaches the site once",
        received(site, IDENTITY_HOST, &own) == 1,
    );
    checks.check(
        "the worker's request to a host the domain policy forbids never reaches it",
        received(site, "elsewhere.test", &forbidden) == 0,
    );
    checks.check(
        "the worker's request a Filter list matches never reaches the site",
        received(site, IDENTITY_HOST, &tracker) == 0,
    );
    checks.check(
        "the call the worker answers from its cache reaches no network",
        received(site, IDENTITY_HOST, &cached) == 0,
    );
    let responded = watch.until_responded(&[&cached, &network])?;
    checks.check(
        "a `page.response` event arrives for both calls within the wait",
        responded,
    );
    let responses = watch.data("page.response");
    let from_worker = |suffix: &str| {
        responses.iter().any(|exchange| {
            url_of(&exchange["response"]).ends_with(suffix)
                && exchange["response"]["from_service_worker"] == json!(true)
        })
    };
    checks.check(
        "both calls arrive in the page's `response` event with `from_service_worker` true",
        from_worker(&cached) && from_worker(&network),
    );
    let held = format!("/service-worker-held.json{query}");
    waits_on_the_page(
        &mut watch,
        (cached_response, &cached, &held),
        (site, &page),
        checks,
    )?;
    let requests = watch.data("page.request");
    let count = |suffix: &str| {
        requests
            .iter()
            .filter(|request| url_of(request).ends_with(suffix))
            .count()
    };
    checks.check(
        "the page's `request` events hold its own calls and none of the worker's own requests",
        (
            count(&network),
            count(&own),
            count(&forbidden),
            count(&tracker),
        ) == (1, 0, 0, 0),
    );
    watch.end(run_id, &[session_route, page_route])?;
    let trace = run_trace(target, run_id);
    let traced = |suffix: &str| {
        trace
            .iter()
            .filter(|line| line["event"] == json!(SERVICE_WORKER_REQUEST))
            .find(|line| url_of(&line["data"]).ends_with(suffix))
            .map(|line| line["data"].clone())
    };
    checks.check(
        "the Run trace records the worker's own requests, in its Session",
        [&own, &network].iter().all(|suffix| {
            traced(suffix).is_some_and(|data| data["session_id"] == json!(page.session_id))
        }),
    );
    checks.check(
        "the Run trace records the forbidden host's request refused with `egress_refused`",
        traced(&forbidden)
            .is_some_and(|data| data["failure"]["error"]["code"] == json!("egress_refused")),
    );
    checks.check(
        "the Run trace records the tracker's request failed as `net::ERR_BLOCKED_BY_CLIENT`",
        traced(&tracker).is_some_and(|data| {
            data["failure"]["error_text"] == json!("net::ERR_BLOCKED_BY_CLIENT")
        }),
    );
    Ok(())
}

/// The response waiter `armed`, armed before the navigation, got the
/// call to `cached` the worker answered from its cache, and `page` reaches
/// `networkidle`, which the worker's own requests, of no page, never hold
/// back: while the request `held` the worker made is held in flight,
/// within a timeout shorter than its hold.
fn waits_on_the_page(
    watch: &mut Watch<'_>,
    (armed, cached, held): (i64, &str, &str),
    (site, page): (&Site, &PageOpened),
    checks: &mut Checks,
) -> Result<(), Gone> {
    let waited = watch.answer(armed)?;
    checks.check(
        "a response waiter armed before the navigation gets the cached call, from the worker",
        url_of(&waited["result"]["response"]).ends_with(cached)
            && waited["result"]["response"]["from_service_worker"] == json!(true),
    );
    let in_flight = watch.until_received(site, held)?;
    checks.check(
        format!("the worker's request {held} reached the Proving ground, which holds it"),
        in_flight,
    );
    let idle = watch.call(
        PAGE_WAIT_FOR_LOAD_STATE.name,
        &json!({ "page_id": page.page_id, "state": "networkidle", "timeout": IDLE_TIMEOUT_MS }),
    )?;
    checks.check(
        format!(
            "the page reaches `networkidle` within {IDLE_TIMEOUT_MS} ms while the worker's own request is held for a minute ({idle})"
        ),
        idle.get("result").is_some(),
    );
    Ok(())
}

/// With `browser.service_workers: block` in the Session's own settings,
/// `register()` rejects as on a failed fetch of the worker's script,
/// `register` stays native, no worker controls the page, and the page's
/// own calls reach its route and the site.
fn blocked(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-service-worker-block";
    let (path, url) = page_of(site, "block");
    let query = "?case=block";
    start_run(connection, site, run_id, &settings(), checks)?;
    let mut watch = Watch::new(connection);
    let opened = watch.call(
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": { "browser": { "service_workers": "block" } } }),
    )?;
    let session = opened["result"]["session_id"].clone();
    checks.check(
        "a Session opens with `browser.service_workers: block`",
        session.is_string(),
    );
    if !session.is_string() {
        return finish_run(watch.connection, run_id);
    }
    let paged = watch.call(SESSION_NEW_PAGE.name, &json!({ "session_id": session }))?;
    let Ok(page) = serde_json::from_value::<PageOpened>(paged["result"].clone()) else {
        checks.check("the Session opens a page", false);
        return finish_run(watch.connection, run_id);
    };
    let page_route = watch.connection.request(
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/service-worker/**") }),
    )?;
    watch.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    let reports = watch.until_reported(site, &path)?;
    checks.check(
        "`register()` rejects with a `TypeError`, as on a failed fetch of the worker's script",
        reported(&reports, "register") == Some("rejected TypeError"),
    );
    checks.check(
        "`register` is the browser's own function",
        reported(&reports, "register_native") == Some("true"),
    );
    checks.check(
        "no worker controls the page",
        reported(&reports, "controlled") == Some("false"),
    );
    let by_page = watch.handled_by(page_route);
    let routed = |suffix: &str| {
        by_page
            .iter()
            .any(|params| url_of(&params["request"]).ends_with(suffix))
    };
    let cached = format!("/service-worker/cached.json{query}");
    let network = format!("/service-worker/network.json{query}");
    checks.check(
        "the page's own calls reach its route",
        routed(&cached) && routed(&network),
    );
    checks.check(
        "the page's own calls reach the site, and the worker's never ran",
        (
            received(site, IDENTITY_HOST, &cached),
            received(site, IDENTITY_HOST, &network),
            received(
                site,
                IDENTITY_HOST,
                &format!("/service-worker/own.json{query}"),
            ),
        ) == (1, 1, 0),
    );
    watch.end(run_id, &[page_route])
}

/// The `User-Agent` of each request to `target` the Proving ground saw.
fn user_agents_of(observations: &Value, target: &str) -> Vec<String> {
    ReceivedRequests::of(observations)
        .of_the_case()
        .filter(|request| request["target"] == json!(target))
        .flat_map(|request| request["headers"].as_array().cloned().unwrap_or_default())
        .filter(|header| {
            header[0]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("user-agent"))
        })
        .filter_map(|header| header[1].as_str().map(str::to_owned))
        .collect()
}

/// In a headless Run, the requests a service worker sends itself carry the
/// user agent the build shows with a window, as its pages' do: no
/// `HeadlessChrome` reaches the site from a worker either (story 5-56).
fn identity(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-service-worker-identity";
    let (path, url) = page_of(site, "identity");
    start_run(connection, site, run_id, &settings(), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let mut watch = Watch::new(connection);
    let loaded = watch.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    checks.check(
        "the service worker page loads",
        loaded.get("result").is_some(),
    );
    let reports = watch.until_reported(site, &path)?;
    checks.check(
        "the page's worker registers and controls the page",
        (
            reported(&reports, "register"),
            reported(&reports, "controlled"),
        ) == (Some("registered"), Some("true")),
    );
    let user_agents = site
        .proving_ground
        .observations()
        .map(|observations| user_agents_of(&observations, "/service-worker/own.json?case=identity"))
        .unwrap_or_default();
    checks.check(
        "the worker's own request reached the site",
        !user_agents.is_empty(),
    );
    checks.check(
        "the worker's request shows the window's `Chrome/` user agent, with no `HeadlessChrome`",
        user_agents
            .iter()
            .all(|agent| agent.contains(" Chrome/") && !agent.contains("HeadlessChrome")),
    );
    watch.end(run_id, &[])
}
