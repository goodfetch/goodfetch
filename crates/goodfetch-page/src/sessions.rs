//! The engine's Sessions and their pages (spec #577, ticket #697; ADRs
//! 0021, 0078, 0092 and 0102). Each Session is its own browser context, with
//! its own cookies and storage, in the one browser the engine launches for
//! its Run (ADR 0037 / SA-09, SA-10): the Run's default Session, which its
//! first `run.new_page` opens with the Run's settings and every later one
//! shares, and the Sessions `run.new_session` opens. A Session is `Open`
//! once opened and `Closed` once `session.close` closed it: its pages and
//! its context close, its pages' admissions go back to the Resource
//! governor, and a later call on it or one of its pages raises `closed`,
//! while closing it again changes nothing. Each page holds one admission of
//! the governor's `browser_page` kind while it is open (ADR 0040).
//!
//! The Run's state keeps each Session it opened and whether it is closed, so
//! a start resuming the Run under its ID, in this engine or another, finds
//! its Sessions under the same IDs: the first call that reaches the resumed
//! Run reopens them from its run state, each making its browser context
//! again on its next page. A Session's ID names its Run, so a call naming a
//! Session of a Run this engine has not reached yet reaches it. When the
//! Run stops, its Sessions, its pages and its browser go with it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::session::SessionOpened;
use goodfetch_protocol::page::{PageId, PageOpened, SessionId};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::{
    SettingLimit, SettingsLayer, TIMEOUT_ACTION, TIMEOUT_NAVIGATION,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::driver::{LocalBrowser, PageDriver, SessionDriver};
use crate::interception::Interception;

/// What the page crate adds to every run file: each Session the Run opened,
/// by its position among them, whether it is the Run's default Session, and
/// whether it is closed.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "page-sessions",
    statements: &[
        "CREATE TABLE IF NOT EXISTS page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL) STRICT",
    ],
};

/// A navigation's deadline when the Run's settings name none: 30 s.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The guard's window when the Run's settings name none: 30 s, the
/// setting's default.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// What separates a Session's Run ID from its position in its ID; a
/// position is digits alone, so the last one splits the ID.
const SESSION_MARK: &str = ".session-";

/// The engine's Sessions and their pages, which every call of every
/// Connection shares. Clones share them.
#[derive(Clone)]
pub(crate) struct Sessions {
    state: Arc<Mutex<State>>,
    pins: Vec<PinnedArtifact>,
    interception: Interception,
}

#[derive(Default)]
struct State {
    runs: BTreeMap<RunId, RunSessions>,
    sessions: BTreeMap<SessionId, Arc<Session>>,
    pages: BTreeMap<PageId, OpenPage>,
    opened_pages: u64,
}

/// A live Run's Sessions.
struct RunSessions {
    /// The browser its Sessions share, launched by the first that needs it.
    browser: Arc<LocalBrowser>,
    /// The highest position a Session of the Run took.
    last: u32,
    /// The position of its default Session, once opened.
    default: Option<u32>,
    /// The positions of its closed Sessions.
    closed: BTreeSet<u32>,
}

/// One open Session.
struct Session {
    run: Run,
    position: u32,
    driver: SessionDriver<Arc<LocalBrowser>>,
}

/// One open page.
#[derive(Clone)]
pub(crate) struct OpenPage {
    pub(crate) run: Run,
    /// Its Session.
    pub(crate) session: SessionId,
    pub(crate) driver: PageDriver,
    /// Its Run's `timeout.action` and where it was set.
    pub(crate) action_timeout: (SettingLimit, Duration),
    /// Its Run's clock.
    pub(crate) clock: Clock,
    /// Its admission of the governor's `browser_page` kind, given back once
    /// the page leaves and no call holds it.
    _admission: Arc<Permit>,
}

/// The ID of the Session at `position` of the Run `run`.
fn session_id(run: &RunId, position: u32) -> SessionId {
    SessionId(format!("{run}{SESSION_MARK}{position}"))
}

/// The Run and the position a Session's ID names, if it is one the engine
/// gives.
fn parse_session_id(session: &SessionId) -> Option<(RunId, u32)> {
    let (run, position) = session.0.rsplit_once(SESSION_MARK)?;
    if position.is_empty() || !position.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((RunId::parse(run).ok()?, position.parse().ok()?))
}

/// `closed` for the Session `session`.
fn session_closed(session: &SessionId) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the Session `{}` is closed, or its Run ended", session.0),
        &json!({ "session_id": session }),
    )
}

/// `closed` for the page `page`.
fn page_closed(page: &PageId) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the page `{}` is closed, or its Run ended", page.0),
        &json!({ "page_id": page }),
    )
}

/// The failure of a read or write of `run`'s state.
fn unavailable(run: &Run, error: &impl std::fmt::Display, context: &CallContext) -> CallError {
    goodfetch_engine::run::run_error(
        RunError::Unavailable {
            run_id: run.id().clone(),
            folder: run.folder().to_owned(),
            error: error.to_string(),
        },
        context.identity(),
    )
}

impl Sessions {
    /// The engine's Sessions, whose browsers come from `pins`, under
    /// `interception`.
    pub(crate) fn new(pins: Vec<PinnedArtifact>, interception: Interception) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            pins,
            interception,
        }
    }

    /// Opens a Session of the live Run `run_id`: its browser context is made
    /// now, launching the Run's browser when no Session has.
    pub(crate) async fn new_session(
        &self,
        run_id: &RunId,
        context: &CallContext,
    ) -> Result<SessionOpened, CallError> {
        let run = live(run_id, context).await?;
        let (position, browser) = {
            let mut state = self.state.lock().await;
            let runs = self.reach(&mut state, &run, context).await?;
            runs.last += 1;
            (runs.last, Arc::clone(&runs.browser))
        };
        let driver = SessionDriver::new(browser);
        driver.open().await?;
        self.record(&run, position, false, context).await?;
        let session = session_id(run.id(), position);
        self.state.lock().await.sessions.insert(
            session.clone(),
            Arc::new(Session {
                run,
                position,
                driver,
            }),
        );
        self.interception.open_session(&session);
        Ok(SessionOpened {
            session_id: session,
        })
    }

    /// Opens a page in the default Session of the live Run `run_id`, opened
    /// now when the Run has none; `closed` once it is closed.
    pub(crate) async fn new_default_page(
        &self,
        run_id: &RunId,
        context: &CallContext,
    ) -> Result<PageOpened, CallError> {
        let run = live(run_id, context).await?;
        let session = {
            let mut state = self.state.lock().await;
            let runs = self.reach(&mut state, &run, context).await?;
            let position = match runs.default {
                Some(position) if runs.closed.contains(&position) => {
                    return Err(CallError::new(
                        &CLOSED,
                        format!(
                            "the default Session of the Run `{}` is closed: open a Session of its own with `run.new_session`",
                            run.id()
                        ),
                        &json!({ "session_id": session_id(run.id(), position) }),
                    ));
                }
                Some(position) => position,
                None => {
                    runs.last += 1;
                    runs.default = Some(runs.last);
                    let position = runs.last;
                    let browser = Arc::clone(&runs.browser);
                    self.record(&run, position, true, context).await?;
                    let session = session_id(run.id(), position);
                    state.sessions.insert(
                        session.clone(),
                        Arc::new(Session {
                            run: run.clone(),
                            position,
                            driver: SessionDriver::new(browser),
                        }),
                    );
                    self.interception.open_session(&session);
                    position
                }
            };
            session_id(run.id(), position)
        };
        self.new_page(&session, context).await
    }

    /// Opens a page in the Session `session`, once the governor admits one
    /// more browser page; `closed` when the Session is closed.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(session_id = %session.0, page_id = tracing::field::Empty))]
    pub(crate) async fn new_page(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<PageOpened, CallError> {
        let open = self.session(session, context).await?;
        let run = open.run.clone();
        let (navigation_timeout, action_timeout) = timeouts(&run);
        let admission = run.governor().admit(WorkKind::BrowserPage).await;
        let driver = open.driver.new_page(navigation_timeout).await?;
        let page = {
            let mut state = self.state.lock().await;
            state.opened_pages += 1;
            PageId(format!("page-{}", state.opened_pages))
        };
        self.interception
            .start(&page, session, &driver, &run)
            .await?;
        let mut state = self.state.lock().await;
        if !state.sessions.contains_key(session) {
            drop(state);
            self.interception.close(&[page], session);
            return Err(session_closed(session));
        }
        tracing::Span::current().record("page_id", page.0.as_str());
        state.pages.insert(
            page.clone(),
            OpenPage {
                clock: run.clock(),
                run,
                session: session.clone(),
                driver,
                action_timeout,
                _admission: Arc::new(admission),
            },
        );
        Ok(PageOpened {
            page_id: page,
            session_id: session.clone(),
        })
    }

    /// Closes the Session `session`: its pages and its browser context
    /// close, and its pages' admissions go back; a Session closed already
    /// stays as it is.
    pub(crate) async fn close(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let open = match self.session(session, context).await {
            Ok(open) => open,
            Err(error) => {
                return if self.was_closed(session).await {
                    Ok(())
                } else {
                    Err(error)
                };
            }
        };
        let pages = {
            let mut state = self.state.lock().await;
            if state.sessions.remove(session).is_none() {
                return Ok(());
            }
            if let Some(runs) = state.runs.get_mut(open.run.id()) {
                runs.closed.insert(open.position);
            }
            let pages: Vec<PageId> = state
                .pages
                .iter()
                .filter(|(_, page)| &page.session == session)
                .map(|(page_id, _)| page_id.clone())
                .collect();
            for page in &pages {
                state.pages.remove(page);
            }
            pages
        };
        self.interception.close(&pages, session);
        open.driver.close().await;
        let position = open.position;
        open.run
            .state()
            .write(move |connection| {
                connection
                    .execute(
                        "UPDATE page_sessions SET closed = 1 WHERE position = ?1",
                        [position],
                    )
                    .map(drop)
            })
            .await
            .map_err(|error| unavailable(&open.run, &error, context))
    }

    /// The open page `page_id`, or `closed`.
    pub(crate) async fn page(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        self.state
            .lock()
            .await
            .pages
            .get(page_id)
            .cloned()
            .ok_or_else(|| page_closed(page_id))
    }

    /// The open Session `session`, reaching its Run first when this engine
    /// has not yet; `closed` when it is closed or its Run is not live.
    async fn session(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<Arc<Session>, CallError> {
        if let Some(open) = self.state.lock().await.sessions.get(session) {
            return Ok(Arc::clone(open));
        }
        let Some((run_id, _)) = parse_session_id(session) else {
            return Err(session_closed(session));
        };
        let Ok(run) = context.runs().live(run_id).await else {
            return Err(session_closed(session));
        };
        let mut state = self.state.lock().await;
        self.reach(&mut state, &run, context).await?;
        state
            .sessions
            .get(session)
            .cloned()
            .ok_or_else(|| session_closed(session))
    }

    /// Whether `session` names a closed Session of a Run this engine holds.
    async fn was_closed(&self, session: &SessionId) -> bool {
        let Some((run_id, position)) = parse_session_id(session) else {
            return false;
        };
        self.state
            .lock()
            .await
            .runs
            .get(&run_id)
            .is_some_and(|runs| runs.closed.contains(&position))
    }

    /// The Sessions of the live Run `run`, reopened from its run state the
    /// first time this engine reaches it; they leave when it stops.
    async fn reach<'state>(
        &self,
        state: &'state mut State,
        run: &Run,
        context: &CallContext,
    ) -> Result<&'state mut RunSessions, CallError> {
        if !state.runs.contains_key(run.id()) {
            let recorded = run
                .state()
                .read(|connection| {
                    let mut statement = connection.prepare(
                        "SELECT position, is_default, closed FROM page_sessions ORDER BY position",
                    )?;
                    let rows = statement.query_map([], |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    })?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()
                })
                .await
                .map_err(|error| unavailable(run, &error, context))?;
            let browser = Arc::new(LocalBrowser::new(
                run.clone(),
                context.runs().home().clone(),
                Pins {
                    pinned_by: PinnedBy {
                        product: context.identity().product,
                        product_version: context.identity().product_version.clone(),
                    },
                    artifacts: self.pins.clone(),
                },
            ));
            let mut runs = RunSessions {
                browser: Arc::clone(&browser),
                last: 0,
                default: None,
                closed: BTreeSet::new(),
            };
            for (position, is_default, closed) in recorded {
                runs.last = runs.last.max(position);
                if is_default {
                    runs.default = Some(position);
                }
                if closed {
                    runs.closed.insert(position);
                } else {
                    let session = session_id(run.id(), position);
                    self.interception.open_session(&session);
                    state.sessions.insert(
                        session,
                        Arc::new(Session {
                            run: run.clone(),
                            position,
                            driver: SessionDriver::new(Arc::clone(&browser)),
                        }),
                    );
                }
            }
            state.runs.insert(run.id().clone(), runs);
            self.leave_with(run);
        }
        Ok(state
            .runs
            .get_mut(run.id())
            .expect("invariant: the Run's Sessions were just reached"))
    }

    /// Takes the Run's Sessions and pages out once it stops.
    fn leave_with(&self, run: &Run) {
        let sessions = self.clone();
        let stopping = run.cancellation().clone();
        let run_id = run.id().clone();
        run.spawn(async move {
            stopping.cancelled().await;
            let mut state = sessions.state.lock().await;
            state.runs.remove(&run_id);
            let gone: Vec<SessionId> = state
                .sessions
                .iter()
                .filter(|(_, session)| session.run.id() == &run_id)
                .map(|(id, _)| id.clone())
                .collect();
            let mut pages: BTreeMap<SessionId, Vec<PageId>> = BTreeMap::new();
            state.pages.retain(|page_id, page| {
                if page.run.id() == &run_id {
                    pages
                        .entry(page.session.clone())
                        .or_default()
                        .push(page_id.clone());
                    false
                } else {
                    true
                }
            });
            for session in &gone {
                state.sessions.remove(session);
            }
            drop(state);
            for session in &gone {
                let pages = pages.remove(session).unwrap_or_default();
                sessions.interception.close(&pages, session);
            }
        });
    }

    /// Records the Session at `position` of `run` in its run state.
    async fn record(
        &self,
        run: &Run,
        position: u32,
        is_default: bool,
        context: &CallContext,
    ) -> Result<(), CallError> {
        run.state()
            .write(move |connection| {
                connection
                    .execute(
                        "INSERT INTO page_sessions (position, is_default, closed) VALUES (?1, ?2, 0)",
                        rusqlite::params![position, is_default],
                    )
                    .map(drop)
            })
            .await
            .map_err(|error| unavailable(run, &error, context))
    }
}

/// The live Run `run_id`, or `run_not_found`.
async fn live(run_id: &RunId, context: &CallContext) -> Result<Run, CallError> {
    context.runs().live(run_id.clone()).await.map_err(|error| {
        CallError::new(
            &RUN_NOT_FOUND,
            format!("No live Run has the Run ID `{run_id}`: {error}"),
            &RunDetails {
                run_id: run_id.clone(),
                path: None,
            },
        )
    })
}

/// The Run's `timeout.navigation`, and its `timeout.action` with where it
/// was set.
fn timeouts(run: &Run) -> (Duration, (SettingLimit, Duration)) {
    let navigation = run
        .settings()
        .and_then(|settings| settings.resolved().duration(TIMEOUT_NAVIGATION.name))
        .unwrap_or(NAVIGATION_TIMEOUT);
    let action = run
        .settings()
        .and_then(|settings| Deadline::of(settings.resolved(), &TIMEOUT_ACTION))
        .map_or_else(
            || {
                let limit = SettingLimit {
                    key: TIMEOUT_ACTION.name.to_owned(),
                    value: json!(ACTION_TIMEOUT.as_millis()),
                    layer: SettingsLayer::Default,
                };
                (limit, ACTION_TIMEOUT)
            },
            |deadline| (deadline.limit().clone(), deadline.duration()),
        );
    (navigation, action)
}
