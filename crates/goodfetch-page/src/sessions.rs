//! The engine's Sessions and their pages (spec #577, ticket #697; ADRs
//! 0021, 0078, 0092 and 0102). Each Session is its own browser context, with
//! its own cookies and storage, in a browser of its Run's browser pool
//! (`pool`; ADR 0037 / SA-09, SA-10): the Run's default Session, which its
//! first `run.new_page` opens with the Run's settings and every later one
//! shares, and the Sessions `run.new_session` opens. A Session is `Open`
//! once opened and `Closed` once `session.close` closed it: its pages and
//! its context close, its pages' admissions go back to the Resource
//! governor, and a later call on it or one of its pages raises `closed`,
//! while closing it again changes nothing. Each page holds one admission of
//! the governor's `browser_page` kind while it is open (ADR 0040). Each
//! Session draws its identity seed from the Run's `Entropy` when it opens,
//! kept in run state for the Fingerprint method that seeds a browser with
//! it, so recycling never changes it (ADRs 0040 and 0059).
//!
//! The Run's state keeps each Session it opened and whether it is closed, so
//! a start resuming the Run under its ID, in this engine or another, finds
//! its Sessions under the same IDs: the first call that reaches the resumed
//! Run reopens them from its run state, each making its browser context
//! again on its next page. A Session's ID names its Run, so a call naming a
//! Session of a Run this engine has not reached yet reaches it. When the
//! Run stops, its Sessions, its pages and its browsers go with it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::run::settings_error;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::settings::{SettingsError, Values};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::session::{NewSessionParams, SessionOpened};
use goodfetch_protocol::page::{PageId, PageOpened, SessionId};
use goodfetch_protocol::run::{Lifecycle, RUN_ENDED, RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SESSION_SOURCE, SettingLimit,
    SettingsLayer, TIMEOUT_ACTION, TIMEOUT_NAVIGATION,
};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::driver::PageDriver;
use crate::hiding::{Hiding, PageHiding};
use crate::interception::{Interception, SESSION_KEYS};
use crate::pool::{InBrowser, Placement, Pool, Seat};

/// What the page crate adds to every run file: each Session the Run opened,
/// by its position among them, whether it is the Run's default Session,
/// whether it is closed, its own settings, as a JSON object by key, and its
/// identity seed, its 64 bits as SQLite's signed integer.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "page-sessions",
    statements: &[
        "CREATE TABLE IF NOT EXISTS page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL, identity_seed INTEGER NOT NULL) STRICT",
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
    hiding: Hiding,
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
    /// Its browsers, which its Sessions share.
    pool: Pool,
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
    /// Its seat in its Run's browser pool.
    seat: Seat,
    /// Its own settings, which its pages take over the Run's.
    settings: Values,
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
    /// Its live hiding, when it hides anything.
    pub(crate) hiding: Option<Arc<PageHiding>>,
    /// Its admission of the governor's `browser_page` kind, given back once
    /// the page leaves and no call holds it.
    _admission: Arc<Permit>,
    /// Its place among its browser's open pages, left at the same moment.
    _in_browser: Arc<InBrowser>,
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
            hiding: Hiding::default(),
        }
    }

    /// Opens a Session of the live Run `params` names, from its own source,
    /// else the Run's, under its own settings: its browser context is made
    /// now, launching the Run's browser when no Session has.
    pub(crate) async fn new_session(
        &self,
        params: NewSessionParams,
        context: &CallContext,
    ) -> Result<SessionOpened, CallError> {
        let run = live(&params.run_id, context).await?;
        let settings = own_settings(&params.settings, &run, context)?;
        opening_source(&run, params.source)?;
        let (position, seat) = {
            let mut state = self.state.lock().await;
            let runs = self.reach(&mut state, &run, context).await?;
            runs.last += 1;
            (runs.last, runs.pool.seat(Placement::Shared))
        };
        seat.open().await?;
        self.record(&run, position, false, &settings, context)
            .await?;
        let session = session_id(run.id(), position);
        self.state.lock().await.sessions.insert(
            session.clone(),
            Arc::new(Session {
                run,
                position,
                seat,
                settings,
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
        opening_source(&run, None)?;
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
                    let seat = runs.pool.seat(Placement::Shared);
                    self.record(&run, position, true, &Values::default(), context)
                        .await?;
                    let session = session_id(run.id(), position);
                    state.sessions.insert(
                        session.clone(),
                        Arc::new(Session {
                            run: run.clone(),
                            position,
                            seat,
                            settings: Values::default(),
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
        let (driver, in_browser) = open.seat.new_page(navigation_timeout).await?;
        let page = {
            let mut state = self.state.lock().await;
            state.opened_pages += 1;
            PageId(format!("page-{}", state.opened_pages))
        };
        let clean_up = self
            .interception
            .start(&page, session, &driver, &run, &open.settings)
            .await?;
        let hiding = match self
            .hiding
            .start(&page, session, &driver, &run, &clean_up)
            .await
        {
            Ok(hiding) => hiding,
            Err(error) => {
                self.interception
                    .close(std::slice::from_ref(&page), session);
                return Err(error);
            }
        };
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
                hiding,
                _admission: Arc::new(admission),
                _in_browser: Arc::new(in_browser),
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
        self.hiding.close(session).await;
        open.seat.close().await;
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
                        "SELECT position, is_default, closed, settings FROM page_sessions ORDER BY position",
                    )?;
                    let rows = statement.query_map([], |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, bool>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    })?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()
                })
                .await
                .map_err(|error| unavailable(run, &error, context))?;
            let pool = Pool::new(
                run.clone(),
                context.runs().home().clone(),
                Pins {
                    pinned_by: PinnedBy {
                        product: context.identity().product,
                        product_version: context.identity().product_version.clone(),
                    },
                    artifacts: self.pins.clone(),
                },
            );
            let mut runs = RunSessions {
                pool: pool.clone(),
                last: 0,
                default: None,
                closed: BTreeSet::new(),
            };
            for (position, is_default, closed, settings) in recorded {
                runs.last = runs.last.max(position);
                if is_default {
                    runs.default = Some(position);
                }
                if closed {
                    runs.closed.insert(position);
                } else {
                    let settings = recorded_settings(&settings, run, context)?;
                    let session = session_id(run.id(), position);
                    self.interception.open_session(&session);
                    state.sessions.insert(
                        session,
                        Arc::new(Session {
                            run: run.clone(),
                            position,
                            seat: pool.seat(Placement::Shared),
                            settings,
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
                sessions.hiding.close(session).await;
            }
        });
    }

    /// Records the Session at `position` of `run`, with its own `settings`
    /// and the identity seed it draws from the Run's `Entropy`, in its run
    /// state.
    async fn record(
        &self,
        run: &Run,
        position: u32,
        is_default: bool,
        settings: &Values,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let settings = Value::Object(
            settings
                .keys()
                .filter_map(|key| Some((key.to_owned(), settings.get(key)?.clone())))
                .collect(),
        )
        .to_string();
        let seed = i64::from_le_bytes(run.fork_entropy().await.next_u64().to_le_bytes());
        run.state()
            .write(move |connection| {
                connection
                    .execute(
                        "INSERT INTO page_sessions (position, is_default, closed, settings, identity_seed) VALUES (?1, ?2, 0, ?3, ?4)",
                        rusqlite::params![position, is_default, settings, seed],
                    )
                    .map(drop)
            })
            .await
            .map_err(|error| unavailable(run, &error, context))
    }
}

/// Reads a Session's own `settings` (ADRs 0083 and 0092): the Clean-up
/// rules' keys, which its pages take over the Run's (ADR 0047), and
/// `setting_locked` for one the Run's layers lock. Any other key is
/// refused with `setting_not_applicable`: `session.source`, which the
/// call's own `source` sets, first; any other, since no other setting takes
/// effect on one Session apart from the Run's yet, so none is ignored.
fn own_settings(
    settings: &Map<String, Value>,
    run: &Run,
    context: &CallContext,
) -> Result<Values, CallError> {
    if settings.is_empty() {
        return Ok(Values::default());
    }
    let values = Values::read(SettingsLayer::Run, settings, context.identity().product)
        .map_err(|error| settings_error(&error, context.identity()))?;
    let refused = if values.get(SESSION_SOURCE.name).is_some() {
        Some((
            SESSION_SOURCE.name,
            "pass it as `run.new_session`'s own `source`",
        ))
    } else {
        values
            .keys()
            .find(|key| !SESSION_KEYS.contains(key))
            .map(|key| (key, "set it on the Run, whose Sessions all take it"))
    };
    if let Some((key, fix)) = refused {
        return Err(settings_error(
            &SettingsError::NotApplicable {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: fix.to_owned(),
            },
            context.identity(),
        ));
    }
    let locked = values.keys().find_map(|key| {
        let effective = run.settings()?.resolved().get(key)?;
        effective.locked_by.map(|layer| SettingLimit {
            key: key.to_owned(),
            value: effective.value.clone(),
            layer,
        })
    });
    match locked {
        Some(limit) => Err(settings_error(
            &SettingsError::Locked {
                key: limit.key.clone(),
                layer: SettingsLayer::Run,
                fix: format!(
                    "leave `{}` out of the Session's settings: the `{}` layer locks it for the Run",
                    limit.key,
                    limit.layer.as_str()
                ),
                limit,
            },
            context.identity(),
        )),
        None => Ok(values),
    }
}

/// The own settings `recorded`, as the run state of `run` holds them, of a
/// Session reopened from it.
fn recorded_settings(
    recorded: &str,
    run: &Run,
    context: &CallContext,
) -> Result<Values, CallError> {
    let nested: Map<String, Value> =
        serde_json::from_str(recorded).map_err(|error| unavailable(run, &error, context))?;
    Values::read(SettingsLayer::Run, &nested, context.identity().product)
        .map_err(|error| unavailable(run, &error, context))
}

/// Checks the Session source a Session of `run` opens from: `own`, the
/// call's own `source` when it gave one (`null` for a fresh Session), else
/// the Run's `session.source`. A fresh Session is all this engine opens:
/// any source kind raises `capability_unavailable` naming it, until the
/// ticket of that kind builds its reader (ADR 0056).
#[expect(
    clippy::option_option,
    reason = "a setting's three states (ADR 0083): left out inherits, `null` clears, a value sets"
)]
fn opening_source(run: &Run, own: Option<Option<SessionSource>>) -> Result<(), CallError> {
    let source = match own {
        Some(own) => own.map(|source| (source, SettingsLayer::Run)),
        None => run
            .settings()
            .and_then(|settings| settings.resolved().get(SESSION_SOURCE.name))
            .and_then(|effective| {
                serde_json::from_value::<SessionSource>(effective.value.clone())
                    .ok()
                    .map(|source| (source, effective.layer))
            }),
    };
    match source {
        None => Ok(()),
        Some((source, layer)) => {
            let kind = source.kind();
            Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "This engine opens no Session from a `{kind}` Session source, which `session.source` names in {}: open a fresh Session with `source: null`, or leave `session.source` unset.",
                    layer.as_str()
                ),
                &CapabilityUnavailableDetails {
                    key: SESSION_SOURCE.name.to_owned(),
                    layer,
                    kind: kind.to_owned(),
                    fix: "`source: null`".to_owned(),
                },
            ))
        }
    }
}

/// The live Run `run_id`; `run_ended` for one that ended `finished`, which
/// takes no new work (ADR 0102), and `run_not_found` for any other.
async fn live(run_id: &RunId, context: &CallContext) -> Result<Run, CallError> {
    let error = match context.runs().live(run_id.clone()).await {
        Ok(run) => return Ok(run),
        Err(error) => error,
    };
    let details = RunDetails {
        run_id: run_id.clone(),
        path: None,
    };
    if finished(run_id, context).await {
        return Err(CallError::new(
            &RUN_ENDED,
            format!(
                "The Run `{run_id}` ended `finished` and takes no new work: start a Run under a new Run ID."
            ),
            &details,
        ));
    }
    Err(CallError::new(
        &RUN_NOT_FOUND,
        format!("No live Run has the Run ID `{run_id}`: {error}"),
        &details,
    ))
}

/// Whether the Run `run_id`'s committed state says it ended `finished`.
async fn finished(run_id: &RunId, context: &CallContext) -> bool {
    let Ok(mut reader) = context.runs().reader(run_id.clone()).await else {
        return false;
    };
    reader
        .read(|connection| {
            connection.query_row("SELECT lifecycle FROM run", [], |row| {
                row.get::<_, String>(0)
            })
        })
        .await
        .is_ok_and(|lifecycle| lifecycle == Lifecycle::Finished.as_str())
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
