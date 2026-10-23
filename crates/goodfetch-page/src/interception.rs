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
//! 8. The network.
//!
//! A route therefore sees exactly what the interception sees, and adding
//! one changes neither caching nor which requests are intercepted. Routes
//! are armed when their call arrives ([`Interception::arm`]), and each
//! answer is delivered to the request it names when its call arrives
//! ([`Interception::answer`]), so a handler's answer is always read before
//! its reply that it ended. A handler that fails leaves its failure on its
//! page, which the call waiting on the request, or else the next call on
//! the page, raises ([`Interception::take_failure`]).
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

use goodfetch_cdp::generated::fetch;
use goodfetch_clean_up_rules::matching::Choices;
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_engine::CallError;
use goodfetch_engine::client::Client;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{RunSettings, Values};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::route::{RouteId, UrlMatcher};
use goodfetch_protocol::page::{BROWSER_BLOCK_RESOURCES, PageId, SessionId};
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::BLOCK_RESOURCES_VISIBLE;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};

use self::answers::Answers;
pub(crate) use self::answers::{Answer, Fulfillment, Overrides, Reply};
use self::clean_up::CleanUp;
pub(crate) use self::clean_up::SESSION_KEYS;
use self::pipeline::PageContext;
pub(crate) use self::routes::Armed;
use self::routes::{Route, RouteTable};
use crate::driver::PageDriver;
use crate::url_pattern::UrlPattern;

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
}

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
    /// rules chosen by `own`, the Session's own settings, over the Run's.
    /// It answers what the Session chose of the Clean-up rules beside its
    /// requests' blocking.
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
            });
            tables.pages.insert(page_id.clone(), Arc::clone(&state));
            state
        };
        let events = driver.events().await;
        driver
            .opening_command(&fetch::EnableParams {
                patterns: Some(vec![fetch::RequestPattern {
                    url_pattern: Some("*".to_owned()),
                    resource_type: None,
                    request_stage: Some(fetch::RequestStage::Request),
                }]),
                handle_auth_requests: None,
            })
            .await?;
        let context = Arc::new(PageContext::new(
            driver.clone(),
            page_id.clone(),
            state,
            self.shared.answers.clone(),
            policy,
            blocking,
            RequestLog::new(run.owner().child(), run.folder()),
            run.clone(),
        ));
        run.spawn(pipeline::dispatch(events, context));
        Ok(clean_up)
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
