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
//!    on: each hop of a redirect, a refresh or a script is a request of its
//!    own and is judged again; a hop the browser sends no request for (a
//!    redirect's `Location` or a scheduled navigation to a scheme other
//!    than HTTP's) is refused as it shows ([`hops`]).
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
//! the page, raises ([`Interception::take_failure`]); one that fails on a
//! service worker's request, which belongs to no page, leaves it on its
//! Session, for the next call on any of its pages.
//!
//! A request the engine sends itself from a page's Session, as
//! `page.request` does through the browser-fetch tier, passes steps 3 to 8
//! as the page's Session chose them ([`Interception::engine_requests`]), never a
//! route.
//!
//! Step 4's Filter lists are off for a page while it loads once more with
//! blocking off, and for every page's requests from a document on a domain
//! its Run learned blocking off for (ADR 0047), keyed as ADR 0004 keys a
//! domain; the never-block list and `browser.block_resources` stay. Each
//! page counts the requests the lists blocked, which tell a navigation the
//! lists touched.
//!
//! Each out-of-process frame of a page, and each shared worker of a
//! Session, is held as a service worker is, before it runs: a frame's
//! requests pass its page's pipeline, a shared worker's its Session's.
//!
//! Size: the one table of the pages, Sessions and workers the interception
//! holds, and how each is put under it; the pipeline, its steps, routes,
//! answers, hops and pending navigations are in their own modules.
#![expect(
    clippy::disallowed_types,
    reason = "routes are armed and answers delivered as their call arrives, before any `.await`: each lock guards one table or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

mod answers;
mod clean_up;
mod hops;
mod navigations;
mod pipeline;
mod routes;
mod steps;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use goodfetch_cdp::NetError;
use goodfetch_cdp::generated::{fetch, network};
use goodfetch_clean_up_rules::matching::{Choices, ResourceKind};
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_engine::CallError;
use goodfetch_engine::client::Client;
use goodfetch_engine::egress::{self, Navigator, Policy, RequestKind};
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{RunSettings, Values};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::route::{RouteId, UrlMatcher};
use goodfetch_protocol::page::{
    BROWSER_BLOCK_RESOURCES, BROWSER_SERVICE_WORKERS, NAVIGATION_FAILED, NavigationFailedDetails,
    PageId, SessionId,
};
use goodfetch_protocol::run::warning::BLOCK_RESOURCES_VISIBLE;
use goodfetch_protocol::run::{RunId, RunWarning};
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use self::answers::Answers;
pub(crate) use self::answers::{Answer, Fulfillment, Overrides, Reply};
use self::clean_up::CleanUp;
pub(crate) use self::clean_up::SESSION_KEYS;
use self::navigations::{NavigatingPage, Pending};
use self::pipeline::{PageContext, Sender};
pub(crate) use self::routes::Armed;
use self::routes::{Route, RouteTable};
use crate::borrowed::Lookups;
use crate::clean_up_versions::Versions;
use crate::driver::{Child, ChildKind, OnChild, OnWorker, PageDriver, ServiceWorkers};
use crate::page_events::Refusals;
use crate::url_pattern::UrlPattern;

/// `browser.service_workers`'s value that keeps every service worker from
/// running.
const BLOCK: &str = "block";

/// A kind of worker that belongs to no page.
#[derive(Clone, Copy, Debug)]
enum Worker {
    /// A site's service worker (ADR 0121).
    Service,
    /// A site's shared worker.
    Shared,
}

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
    /// The domains each live Run learned blocking off for, while any of its
    /// pages holds them.
    learned: Mutex<BTreeMap<RunId, Weak<Learned>>>,
    /// The cosmetic rules each live Run's served documents are read with,
    /// held until the Run ends: a Run whose pages a fetch tier serves may
    /// hold no Session that keeps its Filter lists loaded, and without
    /// this each served document would compile them again.
    served: Mutex<BTreeMap<RunId, ServedCosmetic>>,
}

/// The cosmetic rules of one live Run's served documents, made once.
struct ServedCosmetic {
    /// Cancelled once the Run ends.
    live: tokio_util::sync::CancellationToken,
    cosmetic: Arc<tokio::sync::OnceCell<Option<Cosmetic>>>,
}

impl Shared {
    /// The domains `run` learned blocking off for, which its pages share.
    fn learned(&self, run: &RunId) -> Arc<Learned> {
        let mut learned = lock(&self.learned);
        if let Some(held) = learned.get(run).and_then(Weak::upgrade) {
            return held;
        }
        learned.retain(|_, held| held.strong_count() > 0);
        let held = Arc::new(Learned::default());
        learned.insert(run.clone(), Arc::downgrade(&held));
        held
    }
}

/// The domains a Run learned blocking off for (ADR 0047), each keyed as
/// ADR 0004 keys a domain.
#[derive(Default)]
struct Learned(Mutex<BTreeSet<String>>);

impl Learned {
    /// Whether blocking was learned off for the domain of `url`.
    fn holds(&self, url: &str) -> bool {
        let learned = lock(&self.0);
        !learned.is_empty() && domain_key(url).is_some_and(|domain| learned.contains(&domain))
    }
}

/// The domain of `url` as ADR 0004 keys it: the registrable domain of its
/// host under the full Public Suffix List, private entries included, or
/// its address; none for a URL without a host.
pub(crate) fn domain_key(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    Some(match url.host()? {
        url::Host::Domain(host) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            match psl::domain(host.as_bytes()) {
                Some(domain) => String::from_utf8_lossy(domain.as_bytes()).into_owned(),
                None => host,
            }
        }
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    })
}

/// What step 4's Filter lists do for a page's document (ADR 0047).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lists {
    /// No Filter list is on for its Session.
    Off,
    /// Its Session's lists are on, but its Run learned blocking off for the
    /// document's domain.
    LearnedOff,
    /// Its Session's lists are on.
    On,
}

#[derive(Default)]
struct Tables {
    pages: BTreeMap<PageId, Arc<PageState>>,
    sessions: BTreeMap<SessionId, Arc<RouteTable>>,
    /// Each Session's pending navigations, which its service workers'
    /// requests may carry.
    pending: BTreeMap<SessionId, Arc<Pending>>,
    /// The Sessions whose Run was warned that `browser.block_resources`
    /// shows.
    warned: BTreeSet<SessionId>,
    /// The Sessions on a Borrowed browser, each with the lookups its
    /// requests' hosts are resolved through.
    interception_only: BTreeMap<SessionId, Arc<Lookups>>,
    /// Each Session's own failure slot: where a Session route's handler
    /// that failed on a service worker's request leaves its failure, for
    /// the next call on any page of the Session.
    failures: BTreeMap<SessionId, LeftFailure>,
}

/// Where a handler's failure waits for the call it blocked, or else the
/// next call: the first one left, until a call takes it.
pub(super) type LeftFailure = Arc<Mutex<Option<CallError>>>;

/// One page's side of the interception, or one service worker's: its
/// routes, its Session's, and the first failure a handler left for the
/// next call on the page, or for a worker on any page of its Session.
pub(super) struct PageState {
    routes: Arc<RouteTable>,
    session: Arc<RouteTable>,
    /// Where its handlers' failures wait: the page's own slot, or a
    /// worker's Session's.
    failure: LeftFailure,
    /// Its Session's slot, which a call on the page takes from after the
    /// page's own.
    session_failure: LeftFailure,
    /// The cosmetic rules its Session's Filter lists hold, when any is on.
    cosmetic: Option<Cosmetic>,
    /// Steps 3 to 8 as its Session chose them, for the engine's own
    /// requests from the Session.
    engine_requests: EngineRequests,
    /// Its Session's pending navigations.
    pending: Arc<Pending>,
    /// What a service worker's request carrying one of its navigations is
    /// admitted and refused through; none for a service worker's own.
    navigating: Option<NavigatingPage>,
    /// Who asked for the navigation call going on, while it may load a
    /// `file:` URL.
    files: Mutex<Option<Navigator>>,
    /// Whether its Session turns any Filter list on.
    lists_on: bool,
    /// Whether it is loading once more with blocking off.
    retrying: AtomicBool,
    /// How many of its requests the Filter lists blocked.
    lists_blocked: AtomicU64,
    /// The domains its Run learned blocking off for.
    learned: Arc<Learned>,
    /// What it does with each document its main frame receives, when a
    /// program opened it; none for a service worker's own.
    on_document: Option<Arc<dyn OnDocument>>,
}

/// What a page a program opened does with each document its main frame
/// receives from the site, whatever started its navigation: its Session's
/// side reads its origin's robots.txt for it (ADR 0086). A handler's page
/// has none: its navigation gate judges its documents (ADR 0087).
pub(crate) trait OnDocument: Send + Sync {
    /// The main frame received the document at `url`, its response arrived;
    /// `steps` are its Session's steps 3 to 8 for the engine's own requests.
    fn received(&self, url: &str, steps: &EngineRequests);
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
            egress: RequestKind::Page,
        };
        match steps::send_on(&request, &self.policy, &self.blocking, true) {
            Ok(steps::Sent::Continue(command)) => Ok(command),
            Ok(steps::Sent::StandIn(_)) | Err(steps::Stopped::Blocked { .. }) => {
                Err(CallError::new(
                    &NAVIGATION_FAILED,
                    format!(
                        "{url} was blocked by the Session's Filter lists or `browser.block_resources`"
                    ),
                    &NavigationFailedDetails {
                        url: Some(url.to_owned()),
                        error: Some(NetError::BlockedByClient.name().to_owned()),
                    },
                ))
            }
            Err(steps::Stopped::Refused(refused)) => Err(refused.error()),
        }
    }
}

/// The cosmetic rules a page's Session holds (ADR 0047): the embedded
/// Filter lists, shared, and which of them its settings turn on; live
/// hiding and extraction-side removal read them.
#[derive(Clone)]
pub(crate) struct Cosmetic {
    /// The Filter lists of the version the page's Run holds to.
    pub(crate) lists: Arc<FilterLists>,
    /// That version's digest, which keys their selectors in the engine's
    /// cache of compilations.
    pub(crate) version: String,
    /// What the Session's settings choose of them.
    pub(crate) choices: Choices,
}

impl Cosmetic {
    /// The cosmetic rules of step 4's `blocking`, its Filter lists of the
    /// version `version`, when any list is on.
    fn of(blocking: &steps::Blocking, version: Option<String>) -> Option<Self> {
        blocking
            .lists
            .as_ref()
            .zip(version)
            .map(|(lists, version)| Self {
                lists: Arc::clone(lists),
                version,
                choices: blocking.choices.clone(),
            })
    }
}

/// What a page's Session chose of the Clean-up rules beside its requests'
/// blocking: its cosmetic rules, and its `live_view.hiding`.
pub(crate) struct PageCleanUp {
    /// Its cosmetic rules, when any Filter list is on.
    pub(crate) cosmetic: Option<Cosmetic>,
    /// Its `live_view.hiding`: `auto`, `off` or `always`.
    pub(crate) live_hiding: String,
}

/// A page the interception holds: what its Session chose of the Clean-up
/// rules, what the interception does with each of its children before it
/// runs, and how many of its requests, its out-of-process frames'
/// included, wait on a route handler now.
pub(crate) struct HeldPage {
    pub(crate) clean_up: PageCleanUp,
    pub(crate) children: OnChild,
    pub(crate) routed: watch::Receiver<usize>,
}

impl PageState {
    /// Leaves `error` for the call waiting on the request, or else the next
    /// call on the page, unless an earlier failure waits already.
    fn leave(&self, error: CallError) {
        lock(&self.failure).get_or_insert(error);
    }

    /// Whether step 4's Filter lists are off for a request sent from the
    /// document at `source`: while the page loads once more with blocking
    /// off, or on a domain its Run learned blocking off for.
    fn lists_off(&self, source: &str) -> bool {
        self.lists_on && (self.retrying.load(Ordering::Acquire) || self.learned.holds(source))
    }

    /// Counts a request the Filter lists blocked.
    fn blocked_by_lists(&self) {
        self.lists_blocked.fetch_add(1, Ordering::AcqRel);
    }
}

/// A page loading with blocking off, until it goes (ADR 0047).
pub(crate) struct ListsOff(Option<Arc<PageState>>);

impl Drop for ListsOff {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            state.retrying.store(false, Ordering::Release);
        }
    }
}

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Interception {
    /// The versions of the Clean-up rules the engine's Runs hold to, which
    /// choose each new Run's (ADR 0047).
    pub(crate) fn versions(&self) -> Arc<Versions> {
        self.shared.clean_up.versions()
    }

    /// Opens the Session `session`'s route table, once.
    pub(crate) fn open_session(&self, session: &SessionId) {
        lock(&self.shared.tables)
            .sessions
            .entry(session.clone())
            .or_default();
    }

    /// Holds the Session `session`, on a Borrowed browser, to the policy by
    /// interception alone (ADR 0056): no relay is under that browser, so
    /// each request's host is resolved by the engine through `lookups`,
    /// the Run's `network.resolve` mapping first, and its answers judged
    /// before it is sent on.
    pub(crate) fn hold_by_interception_only(&self, session: &SessionId, lookups: Lookups) {
        lock(&self.shared.tables)
            .interception_only
            .insert(session.clone(), Arc::new(lookups));
    }

    /// Puts the page `page_id` of the Session `session`, which `driver`
    /// drives for `run`, under the interception: every request of its
    /// target is paused and passes the pipeline from now on, its Clean-up
    /// rules chosen by `own`, the Session's own settings, over the Run's,
    /// each refusal left in `refusals` for the request's failure to carry,
    /// each document its main frame receives handed to `on_document`. It
    /// answers what the Session chose of the Clean-up rules beside its
    /// requests' blocking, and what becomes of the page's children: each
    /// out-of-process frame's requests pass the page's pipeline too, from
    /// before it runs. The responses a response observer keeps are the
    /// page's events' to pass on, never the interception's.
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
        (refusals, on_document): (Refusals, Option<Arc<dyn OnDocument>>),
    ) -> Result<HeldPage, CallError> {
        let policy = policy_of(run)?;
        let resolved = run.settings().map(RunSettings::resolved);
        let choices = clean_up::choices(resolved, own);
        let blocked = (!choices.block_resources.is_empty())
            .then(|| clean_up::effective(resolved, own, BROWSER_BLOCK_RESOURCES.name).to_string());
        let (blocking, version) = self.shared.clean_up.blocking(choices, run).await;
        let clean_up = PageCleanUp {
            cosmetic: Cosmetic::of(&blocking, version),
            live_hiding: clean_up::live_hiding(resolved, own),
        };
        if let Some(blocked) = blocked {
            self.warn_of_blocked_resources(session, run, &blocked).await;
        }
        let (state, lookups) = {
            let mut tables = lock(&self.shared.tables);
            let lookups = tables.interception_only.get(session).cloned();
            let pending = Arc::clone(tables.pending.entry(session.clone()).or_default());
            let session_failure = Arc::clone(tables.failures.entry(session.clone()).or_default());
            let session = Arc::clone(tables.sessions.entry(session.clone()).or_default());
            let state = Arc::new(PageState {
                routes: Arc::default(),
                session,
                pending,
                navigating: Some(NavigatingPage {
                    driver: driver.clone(),
                    refusals: refusals.clone(),
                }),
                failure: LeftFailure::default(),
                session_failure,
                files: Mutex::new(None),
                lists_on: blocking.lists.is_some(),
                retrying: AtomicBool::new(false),
                lists_blocked: AtomicU64::new(0),
                learned: self.shared.learned(run.id()),
                cosmetic: clean_up.cosmetic.clone(),
                engine_requests: EngineRequests {
                    policy: policy.clone(),
                    blocking: blocking.clone(),
                },
                on_document,
            });
            tables.pages.insert(page_id.clone(), Arc::clone(&state));
            (state, lookups)
        };
        let events = driver.events();
        pause_requests(driver).await?;
        let workers =
            match clean_up::effective(resolved, own, BROWSER_SERVICE_WORKERS.name).as_str() {
                Some(BLOCK) => ServiceWorkers::Block,
                _ => ServiceWorkers::Allow(self.workers_of(
                    (session, Worker::Service),
                    driver,
                    run,
                    (policy.clone(), lookups.clone()),
                    blocking.clone(),
                )),
            };
        driver.shared_workers(self.workers_of(
            (session, Worker::Shared),
            driver,
            run,
            (policy.clone(), lookups.clone()),
            blocking.clone(),
        ));
        // Chosen before the page loads anything, so no worker of its sites
        // runs before the interception holds it.
        driver.service_workers(workers).await?;
        // The page's own requests and its frames' are counted while a route
        // handler holds them; a worker's are not the page's.
        let routed = Arc::new(watch::Sender::new(0));
        let answers = self.shared.answers.counting(Arc::clone(&routed));
        let children = Self::children_of(
            page_id,
            (&state, answers.clone()),
            driver,
            run,
            (policy.clone(), lookups.clone()),
            blocking.clone(),
        );
        let context = Arc::new(
            PageContext::new(
                driver.clone(),
                Sender::Page(page_id.clone()),
                state,
                answers,
                policy,
                blocking,
                refusals,
                run.clone(),
            )
            .with_lookups(lookups),
        );
        run.spawn(pipeline::dispatch(events, context));
        Ok(HeldPage {
            clean_up,
            children,
            routed: routed.subscribe(),
        })
    }

    /// What the page `page_id`, whose side of the interception is `state`,
    /// whose route answers count its held requests as `answers` does, and
    /// whose driver is `page`, does with each of its children before it
    /// runs: an out-of-process frame's requests pause on its own target and
    /// pass the page's pipeline, under the page's routes, its Session's and
    /// the same steps, each refused one recorded in the Run trace, since
    /// the page's events never see them; a frame whose requests cannot be
    /// paused never runs. A dedicated worker's requests pass its page's
    /// own pipeline already.
    fn children_of(
        page_id: &PageId,
        (state, answers): (&Arc<PageState>, Answers),
        page: &PageDriver,
        run: &Run,
        (policy, lookups): (Policy, Option<Arc<Lookups>>),
        blocking: steps::Blocking,
    ) -> OnChild {
        let (page_id, state, page, run) = (
            page_id.clone(),
            Arc::clone(state),
            page.clone(),
            run.clone(),
        );
        Arc::new(move |child: &Child| {
            let frame = (child.kind == ChildKind::Frame)
                .then(|| (page.on_child(child.session.clone()), child.url.clone()));
            let (page_id, state, answers, run) = (
                page_id.clone(),
                Arc::clone(&state),
                answers.clone(),
                run.clone(),
            );
            let (policy, blocking, lookups) = (policy.clone(), blocking.clone(), lookups.clone());
            Box::pin(async move {
                let Some((frame, url)) = frame else {
                    return true;
                };
                let events = frame.events();
                let enabled = frame
                    .command(&fetch::EnableParams {
                        patterns: Some(patterns()),
                        handle_auth_requests: frame.answers_authentication().then_some(true),
                    })
                    .await;
                if let Err(failure) = enabled {
                    tracing::warn!(%failure, "an out-of-process frame's requests could not be paused, so it does not run");
                    return false;
                }
                // Its WebSockets, which no interception sees, are judged as
                // they open.
                if let Err(failure) = frame.command(&network::EnableParams::default()).await {
                    tracing::debug!(%failure, "an out-of-process frame went before its WebSockets were followed");
                }
                let context = Arc::new(
                    PageContext::new(
                        frame,
                        Sender::Frame(page_id),
                        state,
                        answers,
                        policy,
                        blocking,
                        Refusals::default(),
                        run.clone(),
                    )
                    .with_lookups(lookups)
                    .with_document(&url),
                );
                run.spawn(pipeline::dispatch(events, context));
                true
            })
        })
    }

    /// What the Session `session`, whose page `page` of `run` its context
    /// chose for, does with each service or shared worker of its sites,
    /// as `worker` says, before it runs (ADRs 0014 and 0121): its requests
    /// pause and pass the pipeline as a page's do, under the Session's
    /// routes alone and the same domain policy and Filter lists, until the
    /// worker goes; a service worker's are each recorded in the Run trace,
    /// a shared worker's when refused.
    fn workers_of(
        &self,
        (session, worker): (&SessionId, Worker),
        page: &PageDriver,
        run: &Run,
        (policy, lookups): (Policy, Option<Arc<Lookups>>),
        blocking: steps::Blocking,
    ) -> OnWorker {
        // The browser keeps this choice, so it holds no driver of the
        // browser and nothing that must end with the Run before the
        // browser does.
        let shared = Arc::clone(&self.shared);
        let (session, egress, run) = (session.clone(), page.egress_handle(), run.clone());
        // A worker's requests take its Session's pages' value: their gate
        // admits the navigations the worker carries.
        let value = page.value().clone();
        Arc::new(move |driver: PageDriver, gone: CancellationToken| {
            let (shared, session, egress, run, value) = (
                Arc::clone(&shared),
                session.clone(),
                egress.clone(),
                run.clone(),
                value.clone(),
            );
            let (policy, blocking, lookups) = (policy.clone(), blocking.clone(), lookups.clone());
            Box::pin(async move {
                let (routes, pending, session_failure) = {
                    let mut tables = lock(&shared.tables);
                    let Some(routes) = tables.sessions.get(&session).cloned() else {
                        return false;
                    };
                    (
                        routes,
                        tables.pending.get(&session).cloned().unwrap_or_default(),
                        Arc::clone(tables.failures.entry(session.clone()).or_default()),
                    )
                };
                let log = Arc::new(RequestLog::new(
                    run.owner().child(),
                    run.folder(),
                    run.secrets().mask().clone(),
                    run.metrics().counted_work().clone(),
                ));
                let driver = driver.with_egress(egress).with_value(value, run.clock());
                let events = driver.events();
                if let Err(error) = pause_requests(&driver).await {
                    tracing::warn!(%error, ?worker, "a worker's requests could not be paused, so it does not run");
                    return false;
                }
                let state = Arc::new(PageState {
                    routes: Arc::default(),
                    session: routes,
                    pending,
                    navigating: None,
                    // A worker belongs to no page: its handlers' failures
                    // wait on its Session.
                    failure: Arc::clone(&session_failure),
                    session_failure,
                    files: Mutex::new(None),
                    lists_on: blocking.lists.is_some(),
                    retrying: AtomicBool::new(false),
                    lists_blocked: AtomicU64::new(0),
                    learned: shared.learned(run.id()),
                    cosmetic: None,
                    engine_requests: EngineRequests {
                        policy: policy.clone(),
                        blocking: blocking.clone(),
                    },
                    on_document: None,
                });
                let context = Arc::new(
                    PageContext::new(
                        driver,
                        match worker {
                            Worker::Service => Sender::ServiceWorker { session, log },
                            Worker::Shared => Sender::SharedWorker,
                        },
                        state,
                        shared.answers.clone(),
                        policy,
                        blocking,
                        Refusals::default(),
                        run.clone(),
                    )
                    .with_lookups(lookups),
                );
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

    /// What step 4's Filter lists do for the page `page_id`'s document at
    /// `url`; [`Lists::Off`] for a page the interception does not hold.
    pub(crate) fn lists_for(&self, page_id: &PageId, url: &str) -> Lists {
        let Some(state) = lock(&self.shared.tables).pages.get(page_id).cloned() else {
            return Lists::Off;
        };
        if !state.lists_on {
            Lists::Off
        } else if state.learned.holds(url) {
            Lists::LearnedOff
        } else {
            Lists::On
        }
    }

    /// How many of the page `page_id`'s requests the Filter lists blocked
    /// so far.
    pub(crate) fn lists_blocked(&self, page_id: &PageId) -> u64 {
        lock(&self.shared.tables)
            .pages
            .get(page_id)
            .map_or(0, |state| state.lists_blocked.load(Ordering::Acquire))
    }

    /// Turns the Filter lists off for every request of the page `page_id`,
    /// as it loads once more with blocking off, until the [`ListsOff`]
    /// answered goes.
    pub(crate) fn lists_off(&self, page_id: &PageId) -> ListsOff {
        let state = lock(&self.shared.tables).pages.get(page_id).cloned();
        if let Some(state) = &state {
            state.retrying.store(true, Ordering::Release);
        }
        ListsOff(state)
    }

    /// Learns blocking off for the domain of `url` for every page of the
    /// page `page_id`'s Run (ADR 0047), and answers that domain.
    pub(crate) fn learn_off(&self, page_id: &PageId, url: &str) -> Option<String> {
        let domain = domain_key(url)?;
        let state = lock(&self.shared.tables).pages.get(page_id).cloned()?;
        lock(&state.learned.0).insert(domain.clone());
        Some(domain)
    }

    /// The cosmetic rules of the page `page_id`'s Session, when any of its
    /// Filter lists is on.
    pub(crate) fn cosmetic(&self, page_id: &PageId) -> Option<Cosmetic> {
        let state = lock(&self.shared.tables).pages.get(page_id).cloned()?;
        state.cosmetic.clone()
    }

    /// The cosmetic rules a document a fetch tier served `run` is read with,
    /// as in every tier, HTTP included (ADR 0047): those of the Run's
    /// Filter lists, when any is on, of the version it holds to.
    ///
    /// They are made on the Run's first served document and held until the
    /// Run ends, so its Filter lists are compiled once for it.
    pub(crate) async fn served_cosmetic(&self, run: &Run) -> Option<Cosmetic> {
        let cell = {
            let mut served = lock(&self.shared.served);
            served.retain(|_, held| !held.live.is_cancelled());
            Arc::clone(
                &served
                    .entry(run.id().clone())
                    .or_insert_with(|| ServedCosmetic {
                        live: run.cancellation().clone(),
                        cosmetic: Arc::default(),
                    })
                    .cosmetic,
            )
        };
        cell.get_or_init(|| async {
            let choices = clean_up::choices(
                run.settings().map(RunSettings::resolved),
                &Values::default(),
            );
            let (blocking, version) = self.shared.clean_up.blocking(choices, run).await;
            Cosmetic::of(&blocking, version)
        })
        .await
        .clone()
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

    /// Steps 3 to 8 for the engine's own requests from a Session of `run`
    /// whose own settings are `own`, as its pages' would be: its domain
    /// policy, and the Filter lists and `browser.block_resources` it
    /// chose, for a request no page of the Session sends.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a domain list of the Run's that holds no host.
    pub(crate) async fn engine_requests_of(
        &self,
        run: &Run,
        own: &Values,
    ) -> Result<EngineRequests, CallError> {
        let policy = policy_of(run)?;
        let choices = clean_up::choices(run.settings().map(RunSettings::resolved), own);
        let (blocking, _) = self.shared.clean_up.blocking(choices, run).await;
        Ok(EngineRequests { policy, blocking })
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

    /// Lets the main frame of the page `page_id` load a `file:` URL, as
    /// `navigator` asked, until the [`FilesAllowed`] answered goes: the
    /// navigation call's hops are then judged as the call's (the `file:`
    /// rule, spec #564).
    pub(crate) fn allow_files(&self, page_id: &PageId, navigator: Navigator) -> FilesAllowed {
        let state = lock(&self.shared.tables).pages.get(page_id).cloned();
        if let Some(state) = &state {
            *lock(&state.files) = Some(navigator);
        }
        FilesAllowed(state)
    }

    /// Judges whether the `javascript:` URL `url` may run in the main frame
    /// of the page `page_id`, whose document is at `document_url`: only
    /// where the page's domain policy allows that document's origin (ADR
    /// 0014).
    ///
    /// # Errors
    ///
    /// `egress_refused` when the policy refuses it, and `closed` for a page
    /// the interception does not hold.
    pub(crate) fn judge_javascript(
        &self,
        page_id: &PageId,
        url: &str,
        document_url: &str,
    ) -> Result<(), CallError> {
        let requests = self.engine_requests(page_id)?;
        egress::judge_javascript(url, document_url, &requests.policy)
            .map_err(|refused| refused.error())
    }

    /// The failure a handler left on the page `page_id` for its next call,
    /// or else on its Session (a Session route's, on a service worker's
    /// request), taken.
    pub(crate) fn take_failure(&self, page_id: &PageId) -> Option<CallError> {
        let state = lock(&self.shared.tables).pages.get(page_id).cloned()?;
        let own = lock(&state.failure).take();
        own.or_else(|| lock(&state.session_failure).take())
    }

    /// Takes the pages `pages` out of the interception, as each ends: their
    /// own routes' subscriptions end, and a navigation pending on one,
    /// whose `load` never comes now, gives back the permit a service
    /// worker's request took for it. Their Session's routes stay with its
    /// other pages.
    pub(crate) fn close_pages(&self, pages: &[PageId]) {
        let mut tables = lock(&self.shared.tables);
        for page in pages {
            if let Some(state) = tables.pages.remove(page) {
                state.routes.close();
                state.pending.ended(page);
            }
        }
    }

    /// Takes the Session `session` and its pages `pages` out of the
    /// interception, as the Session or its Run ends: every route of theirs
    /// ends its subscription.
    pub(crate) fn close_session(&self, session: &SessionId, pages: &[PageId]) {
        self.close_pages(pages);
        let mut tables = lock(&self.shared.tables);
        if let Some(table) = tables.sessions.remove(session) {
            table.close();
        }
        tables.pending.remove(session);
        tables.failures.remove(session);
        tables.interception_only.remove(session);
    }
}

/// The main frame of a page loading `file:` URLs for a navigation call,
/// until it goes.
pub(crate) struct FilesAllowed(Option<Arc<PageState>>);

impl Drop for FilesAllowed {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            *lock(&state.files) = None;
        }
    }
}

/// Pauses the requests of `driver`'s page or service worker at
/// [`patterns`]: an HTTP or HTTPS proxy's 407 reaches the engine too, which
/// answers it with the Session's credentials (SA-22).
///
/// # Errors
///
/// The code of opening a page when the browser refuses.
async fn pause_requests(driver: &PageDriver) -> Result<(), CallError> {
    driver
        .opening_command(&fetch::EnableParams {
            patterns: Some(patterns()),
            handle_auth_requests: driver.answers_authentication().then_some(true),
        })
        .await
        .map(drop)
}

/// What a target's requests pause at: every request before it leaves, and
/// a document's response, whose redirect the egress policy judges (a
/// `Location` to a scheme no request may use sends no request of its own)
/// and a navigation gate judges before the page renders it.
fn patterns() -> Vec<fetch::RequestPattern> {
    let stage = |resource_type, request_stage| fetch::RequestPattern {
        url_pattern: Some("*".to_owned()),
        resource_type,
        request_stage: Some(request_stage),
    };
    vec![
        stage(None, fetch::RequestStage::Request),
        stage(
            Some(network::ResourceType::Document),
            fetch::RequestStage::Response,
        ),
    ]
}

/// The domain policy of `run`'s pages, every host's when it has no
/// settings.
///
/// # Errors
///
/// `setting_invalid` for a domain list of the Run's that holds no host.
fn policy_of(run: &Run) -> Result<Policy, CallError> {
    match run.settings() {
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
    })
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
