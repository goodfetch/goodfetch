//! The page suite's service worker cases, the Page state conformance cases'
//! service worker lines on Chromium (ADR 0121, ticket #707), on the Proving
//! ground's service worker page: its worker answers one JSON call from its
//! Cache Storage, passes another on with its own fetch, and requests, as it
//! installs, a body of its site, a body on a host the Run's domain policy
//! forbids and a path a tracker list blocks. With `browser.service_workers`
//! unset, the worker's own requests pass the domain policy, the Filter
//! lists and a Session route, never a page route, and only the Run trace
//! records them; both calls arrive in the page's `response` event with
//! `from_service_worker` true. With `block`, `register()` rejects as the
//! browser's own refusal does, `register` stays native, no worker controls
//! the page, and the page's own calls reach its route and the site.

use goodfetch_protocol::connection::CANCEL;
use goodfetch_protocol::events::{CREDIT, EVENTS, EVENTS_SUBSCRIBE};
use goodfetch_protocol::page::events::PAGE_ON;
use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_HANDLE, SESSION_ROUTE};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened};
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::clean_up::run_trace;
use super::launch::{IDENTITY_HOST, reports};
use super::routes::{glob, received};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::{BROWSER_DEADLINE, EngineTarget};

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    UnderTheInterception,
    Blocked,
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
        }
    }
}

/// The credits a subscription grants at once, more than a case's events.
const CREDITS: u32 = 10_000;

/// How long the harness waits for one message while it also waits for the
/// page's report.
const POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// The trace event a service worker's own request is recorded under.
const SERVICE_WORKER_REQUEST: &str = "service_worker.request";

/// The Connection as these cases read it: the events of the case's own
/// subscription, and every `route.handle` request the engine sent, each
/// answered by the handler ending without an answer, so the request falls
/// through.
struct Watch<'connection> {
    connection: &'connection mut Connection,
    events: Vec<Value>,
    subscription: Option<i64>,
    /// The params of each `route.handle` request, in arrival order.
    handled: Vec<Value>,
}

impl<'connection> Watch<'connection> {
    fn new(connection: &'connection mut Connection) -> Self {
        Self {
            connection,
            events: Vec::new(),
            subscription: None,
            handled: Vec::new(),
        }
    }

    /// Takes in one message within `deadline`: a route's request or a
    /// batch's events, or else the message; `None` for those, or when none
    /// came.
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
                .subscription
                .is_some_and(|id| message["params"]["id"] == json!(id))
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
        Ok(Some(message))
    }

    /// The answer to the request `id`.
    fn answer(&mut self, id: i64) -> Result<Value, Gone> {
        loop {
            if let Some(message) = self.read(BROWSER_DEADLINE)?
                && message.get("method").is_none()
                && message["id"] == json!(id)
            {
                return Ok(message);
            }
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

    /// Subscribes to the event stream with credits to spare.
    fn subscribe(&mut self) -> Result<(), Gone> {
        let id = self.connection.request(EVENTS_SUBSCRIBE.name, &json!({}))?;
        self.subscription = Some(id);
        self.connection
            .notify(CREDIT, &json!({ "id": id, "credits": CREDITS }))
    }

    /// Cancels the case's subscription and its `calls` in flight, whose
    /// calls the Run would otherwise wait on, and finishes the Run
    /// `run_id`, serving every route's request until it answers.
    fn end(mut self, run_id: &str, calls: &[i64]) -> Result<(), Gone> {
        for id in self.subscription.iter().chain(calls) {
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
fn under_the_interception(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-service-worker";
    let (path, url) = page_of(site, "allow");
    let query = "?case=allow";
    start_run(connection, site, run_id, &settings(), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let mut watch = Watch::new(connection);
    watch.subscribe()?;
    for event in ["request", "response"] {
        watch.connection.request(
            PAGE_ON.name,
            &json!({ "page_id": page.page_id, "event": event }),
        )?;
    }
    let session_route = watch.connection.request(
        SESSION_ROUTE.name,
        &json!({ "session_id": page.session_id, "url": glob("**/service-worker/**") }),
    )?;
    let page_route = watch.connection.request(
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/service-worker/**") }),
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
