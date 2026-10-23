//! The interception (spec #577; ADRs 0014, 0047, 0085, 0087, 0104, 0117):
//! one pipeline every request of a page passes, in one written order, the
//! same on every browser. On Chromium the `Fetch` domain pauses each request
//! of the page's target before it leaves; on Firefox and `WebKit`,
//! Playwright's routing will (A05-45). The steps, each at its place in
//! [`pipeline`]:
//!
//! Steps 3 to 8, for what the last route sends on, are [`steps`]'s.
//!
//! 1. A crawl request's own document fetch has its Hooks applied already
//!    (ADR 0104); nothing here runs a Hook.
//! 2. Page routes, later first, then Session routes; `fulfill` and `abort`
//!    end here.
//! 3. The egress check and the domain policy, on what the last route sends
//!    on.
//! 4. The never-block list, the Filter lists and `browser.block_resources`.
//! 5. The Response cache and a replaying Run's Response recording.
//! 6. The page's navigation gate, then its lease check.
//! 7. The engine's own additions, last, so no route reads them: the
//!    Session's cookies, Secret placeholders, the Session's credentials.
//! 8. The network, through the Session's proxy if it has one: an HTTP or
//!    HTTPS proxy's 407 is answered with the Session's credentials once per
//!    request, and a second 407, or one with no credentials to give, fails
//!    the page's navigation with `proxy_auth_failed` (ADR 0053).
//!
//! A route therefore sees exactly what the interception sees, and adding
//! one changes neither caching nor which requests are intercepted.
//!
//! A site's service worker on Chromium (ADR 0121) runs only once the
//! interception holds it, as its Session's browser context chose when the
//! Session's first page opened (`browser.service_workers`): under `allow`
//! its own requests pass the same pipeline as a page's, under the Session's
//! routes alone, since a worker belongs to no page, each recorded in the
//! Run trace; under `block` none runs. A page request the worker's fetch
//! handler answers never reaches the page's interception: what the worker
//! sends on for it is its own request. Routes
//! are armed when their call arrives ([`Interception::arm`]), and each
//! answer is delivered to the request it names when its call arrives
//! ([`Interception::answer`]), so a handler's answer is always read before
//! its reply that it ended. A handler that fails leaves its failure on its
//! page, which the call waiting on the request, or else the next call on
//! the page, raises ([`Interception::take_failure`]).
//!
//! A request the engine sends itself from a page's Session, as
//! `page.request` does through the browser-fetch tier, passes steps 3 to 8
//! as the page's Session chose them ([`Interception::engine_requests`]), never a
//! route.
#![expect(
    clippy::disallowed_types,
    reason = "routes are armed and answers delivered as their call arrives, before any `.await`: each lock guards one table or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

mod answers;
mod clean_up;
mod pipeline;
mod routes;
mod steps;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use goodfetch_cdp::generated::{fetch, network};
use goodfetch_clean_up_rules::matching::{Choices, ResourceKind};
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_engine::CallError;
use goodfetch_engine::client::Client;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{RunSettings, Values};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::route::{RouteId, UrlMatcher};
use goodfetch_protocol::page::{
    BROWSER_BLOCK_RESOURCES, BROWSER_SERVICE_WORKERS, NAVIGATION_FAILED, NavigationFailedDetails,
    PageId, SessionId,
};
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::BLOCK_RESOURCES_VISIBLE;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};
use tokio_util::sync::CancellationToken;

use self::answers::Answers;
pub(crate) use self::answers::{Answer, Fulfillment, Overrides, Reply};
use self::clean_up::CleanUp;
pub(crate) use self::clean_up::SESSION_KEYS;
use self::pipeline::{Observing, PageContext, Sender};
pub(crate) use self::routes::Armed;
use self::routes::{Route, RouteTable};
use crate::driver::{OnServiceWorker, PageDriver, ServiceWorkers};
use crate::page_events::Refusals;
use crate::page_value::ResponseObserver;
use crate::proxy::SessionProxy;
use crate::url_pattern::UrlPattern;

/// `browser.service_workers`'s value that keeps every service worker from
/// running.
const BLOCK: &str = "block";

/// Whose requests a route answers.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Scope<'a> {
    /// One page's.
    Page(&'a PageId),
    /// Every page's of a Session, after their own routes.
    Session(&'a SessionId),
}

/// The engine's interception: every open page's and Session's routes, the
/// failures their handlers left on their pages, and the requests their
/// handlers are answering. Clones share it.
#[derive(Clone, Default)]
pub(crate) struct Interception {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    tables: Mutex<Tables>,
    answers: Answers,
    /// The Clean-up rules step 4 matches, loaded once for every Session.
    clean_up: CleanUp,
}

#[derive(Default)]
struct Tables {
    pages: BTreeMap<PageId, Arc<PageState>>,
    sessions: BTreeMap<SessionId, Arc<RouteTable>>,
    /// The Sessions whose Run was warned that `browser.block_resources`
    /// shows.
    warned: BTreeSet<SessionId>,
}

/// One page's side of the interception: its routes, its Session's, and the
/// first failure a handler left for the next call on the page.
pub(super) struct PageState {
    routes: Arc<RouteTable>,
    session: Arc<RouteTable>,
    failure: Mutex<Option<CallError>>,
    /// The cosmetic rules its Session's Filter lists hold, when any is on.
    cosmetic: Option<Cosmetic>,
    /// Steps 3 to 8 as its Session chose them, for the engine's own
    /// requests from the Session.
    engine_requests: EngineRequests,
}

/// Steps 3 to 8 of a page's pipeline, as its Session chose them, for a
/// request the engine sends itself from the page's Session: the
/// browser-fetch tier's, which `page.request` sends (ADRs 0020 and 0121).
/// No route sees such a request, as none sees Playwright's
/// `APIRequestContext`'s.
#[derive(Clone)]
pub(crate) struct EngineRequests {
    policy: Policy,
    blocking: steps::Blocking,
}

impl EngineRequests {
    /// Steps 3 to 7 for the request `paused`, sent with `overrides`, and
    /// step 8's command, which pauses its response; its source is its own
    /// URL, since no document of the page sent it.
    ///
    /// # Errors
    ///
    /// `egress_refused` when the domain policy refuses it, and
    /// `navigation_failed` naming `net::ERR_BLOCKED_BY_CLIENT` when the
    /// Filter lists or `browser.block_resources` block it: it goes no
    /// further, and its caller fails it.
    pub(crate) fn send_on(
        &self,
        paused: &fetch::RequestPausedEvent,
        overrides: &Overrides,
    ) -> Result<fetch::ContinueRequestParams, CallError> {
        let url = overrides.url.as_deref().unwrap_or(&paused.request.url);
        let request = steps::Paused {
            id: &paused.request_id,
            url,
            method: overrides
                .method
                .as_deref()
                .unwrap_or(&paused.request.method),
            overrides,
            kind: ResourceKind::Xhr,
            source_url: url,
        };
        match steps::send_on(&request, &self.policy, &self.blocking, true) {
            Ok(steps::Sent::Continue(command)) => Ok(command),
            Ok(steps::Sent::StandIn(_)) | Err(steps::Stopped::Blocked) => Err(CallError::new(
                &NAVIGATION_FAILED,
                format!(
                    "{url} was blocked by the Session's Filter lists or `browser.block_resources`"
                ),
                &NavigationFailedDetails {
                    url: url.to_owned(),
                    error: Some(BLOCKED_BY_CLIENT.to_owned()),
                },
            )),
            Err(steps::Stopped::Refused(refused)) => Err(refused.error()),
        }
    }
}

/// The browser's network error for a request a blocker failed.
const BLOCKED_BY_CLIENT: &str = "net::ERR_BLOCKED_BY_CLIENT";

/// The cosmetic rules a page's Session holds (ADR 0047): the embedded
/// Filter lists, shared, and which of them its settings turn on; live
/// hiding and extraction-side removal read them.
#[derive(Clone)]
pub(crate) struct Cosmetic {
    /// The embedded Filter lists.
    pub(crate) lists: Arc<FilterLists>,
    /// What the Session's settings choose of them.
    pub(crate) choices: Choices,
}

/// What a page's Session chose of the Clean-up rules beside its requests'
/// blocking: its cosmetic rules, and its `live_view.hiding`.
pub(crate) struct PageCleanUp {
    /// Its cosmetic rules, when any Filter list is on.
    pub(crate) cosmetic: Option<Cosmetic>,
    /// Its `live_view.hiding`: `auto`, `off` or `always`.
    pub(crate) live_hiding: String,
}

impl PageState {
    /// Leaves `error` for the call waiting on the request, or else the next
    /// call on the page, unless an earlier failure waits already.
    fn leave(&self, error: CallError) {
        lock(&self.failure).get_or_insert(error);
    }
}

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Interception {
    /// Opens the Session `session`'s route table, once.
    pub(crate) fn open_session(&self, session: &SessionId) {
        lock(&self.shared.tables)
            .sessions
            .entry(session.clone())
            .or_default();
    }

    /// Puts the page `page_id` of the Session `session`, which `driver`
    /// drives for `run`, under the interception: every request of its
    /// target is paused and passes the pipeline from now on, its Clean-up
    /// rules chosen by `own`, the Session's own settings, over the Run's,
    /// each refusal left in `refusals` for the request's failure to carry,
    /// each response `observer` keeps passed to it, a service worker's of
    /// the Session included. It answers what the Session chose of the
    /// Clean-up rules beside its requests' blocking.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a domain list of the Run's that holds no host,
    /// and the code of opening a page when the browser refuses to pause its
    /// requests.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the page's requests paused unanswered.
    pub(crate) async fn start(
        &self,
        page_id: &PageId,
        session: &SessionId,
        driver: &PageDriver,
        run: &Run,
        own: &Values,
        refusals: Refusals,
        observer: Option<Arc<dyn ResponseObserver>>,
    ) -> Result<PageCleanUp, CallError> {
        let policy = match run.settings() {
            Some(settings) => Policy::of(settings.resolved()),
            None => Policy::new(&["*"], &[] as &[&str]),
        }
        .map_err(|invalid| {
            CallError::new(
                &SETTING_INVALID,
                format!(
                    "`{}` holds `{}`, which is no host",
                    invalid.setting, invalid.entry
                ),
                &SettingInvalidDetails {
                    key: invalid.setting.to_owned(),
                    layer: None,
                    reason: format!("`{}` is no host", invalid.entry),
                },
            )
        })?;
        let resolved = run.settings().map(RunSettings::resolved);
        let choices = clean_up::choices(resolved, own);
        let blocked = (!choices.block_resources.is_empty())
            .then(|| clean_up::effective(resolved, own, BROWSER_BLOCK_RESOURCES.name).to_string());
        let blocking = self.shared.clean_up.blocking(choices, run.owner()).await;
        let clean_up = PageCleanUp {
            cosmetic: blocking.lists.as_ref().map(|lists| Cosmetic {
                lists: Arc::clone(lists),
                choices: blocking.choices.clone(),
            }),
            live_hiding: clean_up::live_hiding(resolved, own),
        };
        if let Some(blocked) = blocked {
            self.warn_of_blocked_resources(session, run, &blocked).await;
        }
        let state = {
            let mut tables = lock(&self.shared.tables);
            let session = Arc::clone(tables.sessions.entry(session.clone()).or_default());
            let state = Arc::new(PageState {
                routes: Arc::default(),
                session,
                failure: Mutex::new(None),
                cosmetic: clean_up.cosmetic.clone(),
                engine_requests: EngineRequests {
                    policy: policy.clone(),
                    blocking: blocking.clone(),
                },
            });
            tables.pages.insert(page_id.clone(), Arc::clone(&state));
            state
        };
        let events = driver.events().await;
        let observing = observer.map(Observing::of);
        let patterns = patterns(driver.value().gate().is_some(), observing.is_some());
        driver
            .opening_command(&fetch::EnableParams {
                patterns: Some(patterns),
                // An HTTP or HTTPS proxy's 407 reaches the engine, which
                // answers it with the Session's credentials (SA-22).
                handle_auth_requests: driver
                    .proxy()
                    .is_some_and(SessionProxy::answers_authentication)
                    .then_some(true),
            })
            .await?;
        let workers =
            match clean_up::effective(resolved, own, BROWSER_SERVICE_WORKERS.name).as_str() {
                Some(BLOCK) => ServiceWorkers::Block,
                _ => ServiceWorkers::Allow(self.service_workers_of(
                    session,
                    driver,
                    run,
                    policy.clone(),
                    blocking.clone(),
                    observing.clone(),
                )),
            };
        // Chosen before the page loads anything, so no worker of its sites
        // runs before the interception holds it.
        driver.service_workers(workers).await?;
        let context = Arc::new(PageContext::new(
            driver.clone(),
            Sender::Page(page_id.clone()),
            state,
            self.shared.answers.clone(),
            policy,
            blocking,
            refusals,
            observing,
            run.clone(),
        ));
        run.spawn(pipeline::dispatch(events, context));
        Ok(clean_up)
    }

    /// What the Session `session`, whose page `page` of `run` its context
    /// chose for, does with each service worker of its sites before it
    /// runs (ADR 0121): its requests pause and pass the pipeline as a
    /// page's do, under the Session's routes alone and the same domain
    /// policy and Filter lists, each recorded in the Run trace, until the
    /// worker goes.
    fn service_workers_of(
        &self,
        session: &SessionId,
        page: &PageDriver,
        run: &Run,
        policy: Policy,
        blocking: steps::Blocking,
        observing: Option<Observing>,
    ) -> OnServiceWorker {
        // The browser keeps this choice, so it holds no driver of the
        // browser and nothing that must end with the Run before the
        // browser does.
        let shared = Arc::clone(&self.shared);
        let (session, proxy, run) = (session.clone(), page.proxy_handle(), run.clone());
        Arc::new(move |worker: PageDriver, gone: CancellationToken| {
            let (shared, session, proxy, run) = (
                Arc::clone(&shared),
                session.clone(),
                proxy.clone(),
                run.clone(),
            );
            let (policy, blocking, observing) =
                (policy.clone(), blocking.clone(), observing.clone());
            Box::pin(async move {
                let Some(routes) = lock(&shared.tables).sessions.get(&session).cloned() else {
                    return false;
                };
                let log = Arc::new(RequestLog::new(run.owner().child(), run.folder()));
                let worker = worker.with_proxy(proxy);
                let events = worker.events().await;
                let enabled = worker
                    .opening_command(&fetch::EnableParams {
                        patterns: Some(patterns(false, observing.is_some())),
                        handle_auth_requests: worker
                            .proxy()
                            .is_some_and(SessionProxy::answers_authentication)
                            .then_some(true),
                    })
                    .await;
                if let Err(error) = enabled {
                    tracing::warn!(%error, "a service worker's requests could not be paused, so it does not run");
                    return false;
                }
                let state = Arc::new(PageState {
                    routes: Arc::default(),
                    session: routes,
                    failure: Mutex::new(None),
                    cosmetic: None,
                    engine_requests: EngineRequests {
                        policy: policy.clone(),
                        blocking: blocking.clone(),
                    },
                });
                let context = Arc::new(PageContext::new(
                    worker,
                    Sender::ServiceWorker { session, log },
                    state,
                    shared.answers.clone(),
                    policy,
                    blocking,
                    Refusals::default(),
                    observing,
                    run.clone(),
                ));
                run.spawn(async move {
                    tokio::select! {
                        () = gone.cancelled() => {}
                        () = pipeline::dispatch(events, context) => {}
                    }
                });
                true
            })
        })
    }

    /// The cosmetic rules of the page `page_id`'s Session, when any of its
    /// Filter lists is on.
    pub(crate) fn cosmetic(&self, page_id: &PageId) -> Option<Cosmetic> {
        let state = lock(&self.shared.tables).pages.get(page_id).cloned()?;
        state.cosmetic.clone()
    }

    /// Warns `run`, once for the Session `session`, that its
    /// `browser.block_resources`, `blocked`, blocks loads the site can see missing,
    /// since few people block images or fonts (ADRs 0047, 0117). It warns
    /// in `standard` and `stealth`, never in `identified`; until area 8's
    /// Access mode setting, every Session is `standard`.
    async fn warn_of_blocked_resources(&self, session: &SessionId, run: &Run, blocked: &str) {
        if !lock(&self.shared.tables).warned.insert(session.clone()) {
            return;
        }
        let warning = RunWarning {
            code: BLOCK_RESOURCES_VISIBLE.to_owned(),
            message: format!(
                "The Session's `{}` blocks {blocked}: a site can see the missing loads, which few people block.",
                BROWSER_BLOCK_RESOURCES.name
            ),
            sessions: vec![session.0.clone()],
            folder: None,
        };
        if let Err(error) = run.warn(warning).await {
            tracing::warn!(%error, "the warning of `browser.block_resources` was not kept");
        }
    }

    /// Arms a route of `scope` for the requests whose URL `matcher`
    /// matches, whose handler `client` answers under the call `call`'s id.
    /// It is armed when this returns, so a later call's requests reach it;
    /// the [`Armed`] value removes it when dropped.
    ///
    /// # Errors
    ///
    /// `pattern_unsupported` for a pattern the matcher cannot read, and
    /// `closed` for a page or Session the interception does not hold.
    pub(crate) fn arm(
        &self,
        scope: Scope<'_>,
        matcher: &UrlMatcher,
        client: &Client,
        call: &RequestId,
    ) -> Result<Armed, CallError> {
        let pattern = match matcher {
            UrlMatcher::Glob { pattern } => UrlPattern::glob(pattern)?,
            UrlMatcher::Regex { pattern } => UrlPattern::regex(pattern)?,
        };
        let tables = lock(&self.shared.tables);
        let table = match scope {
            Scope::Page(page) => tables
                .pages
                .get(page)
                .map(|state| Arc::clone(&state.routes)),
            Scope::Session(session) => tables.sessions.get(session).map(Arc::clone),
        }
        .ok_or_else(|| closed(scope))?;
        drop(tables);
        Ok(table.arm(Route::new(pattern, client.clone(), call.clone())))
    }

    /// Delivers `answer` to the request `route_id` names, now, and answers
    /// once the pipeline acted on it.
    ///
    /// # Errors
    ///
    /// `route_already_handled` for a request answered already or whose
    /// handler ended, and `closed` for an ID the engine never gave.
    pub(crate) fn answer(
        &self,
        route_id: &RouteId,
        answer: Answer,
    ) -> Result<impl Future<Output = Result<Reply, CallError>> + use<>, CallError> {
        self.shared.answers.deliver(route_id, answer)
    }

    /// Steps 3 to 8 of the page `page_id`'s pipeline, for a request the
    /// engine sends itself from its Session.
    ///
    /// # Errors
    ///
    /// `closed` for a page the interception does not hold.
    pub(crate) fn engine_requests(&self, page_id: &PageId) -> Result<EngineRequests, CallError> {
        lock(&self.shared.tables)
            .pages
            .get(page_id)
            .map(|state| state.engine_requests.clone())
            .ok_or_else(|| closed(Scope::Page(page_id)))
    }

    /// The failure a handler left on the page `page_id` for its next call,
    /// taken.
    pub(crate) fn take_failure(&self, page_id: &PageId) -> Option<CallError> {
        let state = lock(&self.shared.tables).pages.get(page_id).cloned()?;
        lock(&state.failure).take()
    }

    /// Takes the pages `pages` and the Session `session` out of the
    /// interception, as their Run stops: their routes' subscriptions end.
    pub(crate) fn close(&self, pages: &[PageId], session: &SessionId) {
        let mut tables = lock(&self.shared.tables);
        for page in pages {
            if let Some(state) = tables.pages.remove(page) {
                state.routes.close();
            }
        }
        if let Some(table) = tables.sessions.remove(session) {
            table.close();
        }
    }
}

/// What a target's requests pause at: every request before it leaves, a
/// document's response too under a navigation gate, which judges it before
/// the page renders it, and every response for a response observer, which
/// keeps the ones it matches.
fn patterns(gated: bool, observed: bool) -> Vec<fetch::RequestPattern> {
    let stage = |resource_type, request_stage| fetch::RequestPattern {
        url_pattern: Some("*".to_owned()),
        resource_type,
        request_stage: Some(request_stage),
    };
    let mut patterns = vec![stage(None, fetch::RequestStage::Request)];
    if observed {
        patterns.push(stage(None, fetch::RequestStage::Response));
    } else if gated {
        patterns.push(stage(
            Some(network::ResourceType::Document),
            fetch::RequestStage::Response,
        ));
    }
    patterns
}

/// `closed` for a page or Session the interception does not hold.
fn closed(scope: Scope<'_>) -> CallError {
    let (message, details) = match scope {
        Scope::Page(page) => (
            format!("the page `{}` is closed, or its Run ended", page.0),
            serde_json::json!({ "page_id": page }),
        ),
        Scope::Session(session) => (
            format!("the Session `{}` is closed, or its Run ended", session.0),
            serde_json::json!({ "session_id": session }),
        ),
    };
    CallError::new(&goodfetch_protocol::connection::CLOSED, message, &details)
}
