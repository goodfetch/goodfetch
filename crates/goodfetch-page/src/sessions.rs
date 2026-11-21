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
//!
//! Each Session and each page changes state only through its table
//! (`lifecycle`, spec #594, ADR 0128), and every call on it runs through it
//! (`life`). Each page is followed until it ends without the engine closing
//! its browser (ADR 0113): a target the browser destroyed is closed in the
//! browser, and emits `close`, its Session and other pages staying; a
//! crashed renderer, or a browser that exited non-zero or by a signal,
//! crashes the page, which emits `crash` and counts in Usage (ADR 0040), and
//! the browser's crash crashes its Session too; a browser that exited with
//! status 0 quit, which ends its Session. Closing a Session ends its calls
//! in flight with `call_cancelled` naming the program; a crashed Session's
//! close releases what is left and leaves it `Crashed`.
//!
//! Size: one table, the engine's Sessions with their opening, closing and
//! reopening from run state, each of which reads and changes that table
//! under its one lock; a page's end and the tables of states are in their
//! own modules.

mod ends;
mod life;
mod lifecycle;
mod memory;
pub(crate) mod robots_txt;
mod session_pages;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::Command as _;
use goodfetch_cdp::generated::target;
use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::egress::CdpUrlOrigin;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::local_addresses::Unavailable;
use goodfetch_engine::request_signing::{KeyError, RequestSigner};
use goodfetch_engine::run::{secret_error, settings_error};
use goodfetch_engine::run_key::Sealed;
use goodfetch_engine::run_state::{RunStateError, RunStateTables, SecretColumn, run_row};
use goodfetch_engine::runs::{Run, RunError, RunKeyError};
use goodfetch_engine::secrets::Purpose;
use goodfetch_engine::settings::{RunSettings, SettingsError, Values, setting};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::{Authority, SettingScope};
use goodfetch_protocol::connection::{CLOSED, CancelledBy, ClosedValue};
use goodfetch_protocol::egress::{EGRESS_LOCAL_ADDRESSES, LocalAddressFailure};
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::access_mode::{IDENTIFIED_SIGNATURE_AGENT, IDENTIFIED_SIGNING_KEY};
use goodfetch_protocol::page::proxy::PROXY_PROVIDER;
use goodfetch_protocol::page::session::{NewSessionParams, SessionOpened};
use goodfetch_protocol::page::{PageId, PageOpened, SessionId};
use goodfetch_protocol::run::{Lifecycle, RUN_ENDED, RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::setting_scopes;
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, RunOrigin, SESSION_SOURCE, SettingLimit,
    SettingsLayer, TIMEOUT_ACTION, TIMEOUT_WAIT,
};
use goodfetch_protocol::usage::PROXY_BYTES;
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, OnceCell};

pub(crate) use self::life::page_closed;
use self::life::{Ending, PageLife, SessionLife, session_closed};
pub(crate) use self::lifecycle::PageEvent;
use self::lifecycle::{
    Attributes, Owner, PageEffect, SessionEffect, SessionEvent, SessionState, Source,
};
use crate::access_mode::{self, BrowserFamily, SessionIdentityPlan, SessionSettings, SigningPlan};
use crate::borrowed::{CdpUrl, resolve_of};
use crate::consent::Consent;
use crate::dialogs::Dialogs;
use crate::document_request::{DocumentRequest, Unread};
use crate::driver::{PageDriver, SessionEgress, Visibility, counted_as_opening};
use crate::egress_relay::{self, EgressRelay};
use crate::errors::{cancelled, invalid_transition};
use crate::hiding::{Hiding, PageHiding};
use crate::interception::{HeldBrowser, Interception, Lookups};
use crate::launch::Launcher;
use crate::page_events;
use crate::page_functions::{FrameScripts, Injections};
use crate::page_value::{HandlerPages, PageValue, PooledSession};
use crate::pool::{InBrowser, LocalFailure, Placement, Pool, Seat};
use crate::proxy::{Opening, SessionProxy};
use crate::recovery::Checking;
use crate::trace;

/// What the page crate adds to every run file: each Session the Run opened,
/// by its position among them, whether it is the Run's default Session,
/// whether it is closed, its own settings, as a JSON object by key, its
/// identity seed, its 64 bits as SQLite's signed integer, and, for one on a
/// Borrowed browser, its `cdp_url` and who gave it, so a resumed Run
/// reopens it on that browser (`source`, null for a fresh Session), sealed
/// under the Run's data key ([`SOURCE`]).
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "page-sessions",
    statements: &[
        "CREATE TABLE IF NOT EXISTS page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL, identity_seed INTEGER NOT NULL, source BLOB) STRICT",
    ],
};

/// A Session's `source` is secret: a `cdp_url` names the one browser it
/// reaches and may carry its provider's access token.
const SOURCE: SecretColumn = SecretColumn {
    table: "page_sessions",
    column: "source",
};

/// What separates a Session's Run ID from its position in its ID; a
/// position is digits alone, so the last one splits the ID.
const SESSION_MARK: &str = ".session-";

/// The engine's Sessions and their pages, which every call of every
/// Connection shares. Clones share them.
#[derive(Clone)]
pub(crate) struct Sessions {
    state: Arc<Mutex<State>>,
    pins: Vec<PinnedArtifact>,
    /// How each Run's browsers start.
    launcher: Launcher,
    interception: Interception,
    /// The Consent prompt runner every page starts.
    consent: Consent,
    hiding: Hiding,
    /// Each open page's events, which calls reach as they arrive.
    events: page_events::Table,
    /// Each open page's dialogs and their handlers.
    dialogs: Dialogs,
    /// What each page carries for the Product that hands pages to
    /// handlers: its value and its response observer (ADR 0087).
    handler_pages: Arc<dyn HandlerPages>,
    /// How many calls listen to each Session's `page` event.
    page_listeners: session_pages::PageListeners,
}

#[derive(Default)]
struct State {
    runs: BTreeMap<RunId, RunSessions>,
    sessions: BTreeMap<SessionId, Arc<Session>>,
    pages: BTreeMap<PageId, OpenPage>,
    /// The crashed pages a close released, and those the Resource governor
    /// ended past their memory cap, by their Run, whose later calls raise
    /// `page_crashed` or `page_memory_exceeded` again until their Run stops.
    crashed: BTreeMap<PageId, (RunId, PageLife)>,
    opened_pages: u64,
}

/// A live Run's Sessions.
struct RunSessions {
    /// Its browsers, which its Sessions share.
    pool: Pool,
    /// The highest position a Session of the Run took.
    last: u32,
    /// The position of its default Session, set once it opened: the call
    /// that opens it holds the Run's other first pages until it has.
    default: Arc<OnceCell<u32>>,
    /// The positions of its closed Sessions.
    closed: BTreeSet<u32>,
    /// The position of each Session a crawl's Session pool opened, by the
    /// pool's own ID for it, set once it opened.
    pooled: BTreeMap<SessionId, Arc<OnceCell<u32>>>,
}

/// One Session this engine holds: opening, open, or crashed.
struct Session {
    run: Run,
    position: u32,
    /// Its seat in its Run's browser pool.
    seat: Seat,
    /// Its own settings, which its pages take over the Run's.
    settings: Values,
    /// Where it is, through its table.
    life: SessionLife,
    /// Its one proxy, if it has one.
    proxy: Option<Arc<SessionProxy>>,
    /// The egress relay its browser reaches the network through, unless
    /// `security.raw_cdp_egress_proxy` is off or it is on a Borrowed
    /// browser.
    relay: Option<Arc<EgressRelay>>,
    /// Set once its Provenance records how its network is held: by its
    /// egress relay, or by interception alone.
    egress_marked: OnceCell<()>,
    /// Why a Session reopened from run state cannot open a page, such as a
    /// proxy that did not open: raised by each page it is asked for, while
    /// the Run's other Sessions go on.
    reopen_failed: Option<CallError>,
    /// On a Borrowed browser, the main frames of every page it held, whose
    /// popups alone are its own; none elsewhere, where its browser context
    /// holds only its pages.
    held_frames: Option<session_pages::HeldFrames>,
    /// Its marks of main-world injections in Provenance, each once.
    injections: Injections,
}

/// A fresh Session a program opened, in a browser the engine launched.
const FRESH: Attributes = Attributes {
    source: Source::Launched,
    owner: Owner::Program,
};

/// A Session a program opened on a Borrowed browser, from a `cdp_url`.
const BORROWED: Attributes = Attributes {
    source: Source::Borrowed,
    owner: Owner::Program,
};

/// A fresh Session a crawl's Session pool opened, in a browser the engine
/// launched.
const POOL: Attributes = Attributes {
    source: Source::Launched,
    owner: Owner::Pool,
};

/// A Session a crawl's Session pool opened on a Borrowed browser.
const POOL_BORROWED: Attributes = Attributes {
    source: Source::Borrowed,
    owner: Owner::Pool,
};

/// Who opens a Session no `run.new_session` call opens.
#[derive(Clone, Copy)]
enum Opener {
    /// The Run's first `run.new_page`: its default Session, kept in run
    /// state.
    Default,
    /// A crawl's Session pool, which keeps its Sessions itself: a resumed
    /// Run reopens none of them, the pool opening each again at its next
    /// page. Its egress relay binds the rotation position the pool gave
    /// it, when it gave one.
    Pool { address_ordinal: Option<u64> },
}

/// The Provenance action of a Session held to the policy by interception
/// alone.
const INTERCEPTION_ONLY: &str = "interception_only";

/// The Provenance action of a Session whose browser reaches the network
/// through its egress relay.
const EGRESS_RELAY: &str = "egress_relay";

/// One page this engine holds: open, or crashed.
#[derive(Clone)]
pub(crate) struct OpenPage {
    pub(crate) run: Run,
    /// Its Session.
    pub(crate) session: SessionId,
    /// Its own settings, which it takes over its Session's and its Run's.
    pub(crate) settings: Values,
    pub(crate) driver: PageDriver,
    /// Where it is, through its table: each call on it runs through it.
    pub(crate) life: PageLife,
    /// Its Run's `timeout.action` and where it was set.
    pub(crate) action_timeout: (SettingLimit, Duration),
    /// Its Run's clock.
    pub(crate) clock: Clock,
    /// Its live hiding, when it hides anything.
    pub(crate) hiding: Option<Arc<PageHiding>>,
    /// Its documents' hiding sheets, which the Consent prompt runner writes
    /// into too.
    pub(crate) sheets: Arc<crate::hiding::Sheets>,
    /// How its Session's pages are judged: its Page check, when it declares
    /// one, and whether it may hold a login.
    pub(crate) page_check: Arc<Checking>,
    /// Its init scripts as its out-of-process iframes take them.
    pub(crate) frame_scripts: FrameScripts,
    /// Its Session's marks of main-world injections in Provenance.
    pub(crate) injections: Injections,
    /// Its admission of the governor's `browser_page` kind, given back once
    /// the page leaves and no call holds it.
    _admission: Arc<Permit>,
    /// Its place among its browser's open pages, left at the same moment.
    in_browser: Arc<InBrowser>,
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

/// The failure of a read or write of `run`'s state.
fn unavailable(run: &Run, error: &impl std::fmt::Display, context: &CallContext) -> CallError {
    goodfetch_engine::run::run_error(
        RunError::Unavailable {
            run_id: run.id().clone(),
            folder: run.folder().to_owned(),
            error: error.to_string(),
        },
        context,
    )
}

/// Refuses to open a page whose value's lease ended, before any command
/// leaves for it, the one that opens its target included (ADR 0087): the
/// lease check's own error.
fn lease_holds(value: &PageValue) -> Result<(), CallError> {
    match value.lease() {
        Some(lease) => lease
            .check()
            .map_err(|ended| lease.withheld(ended, target::CreateTargetParams::METHOD)),
        None => Ok(()),
    }
}

/// What the engine opens a page of its own in a Session for
/// ([`Sessions::engine_page`]), which says what a move of the Session then
/// would race: storage a program's navigation has yet to write is not carried
/// (ADR 0040).
#[derive(Clone, Copy, Debug)]
pub(crate) enum EngineWork {
    /// A request the program or a crawl sent: a move races any navigation
    /// in flight on the Session's pages, from its call to its page's `load`.
    Request,
    /// Reading for a document that just arrived in one of the Session's
    /// pages, whose scripts may still be writing, whether or not its
    /// navigation call has answered: a move always races them.
    ArrivingDocument,
}

/// A page of a Session that the engine drives itself
/// ([`Sessions::engine_page`]): closed, and its place in its browser given
/// back, once its caller closes it or drops it, a cancelled call's
/// included, so no tab stays open in the Session's browser.
pub(crate) struct EnginePage {
    driver: PageDriver,
    /// Its place in its browser, until it closes.
    in_browser: Option<InBrowser>,
    /// The Run its close runs on when it is dropped.
    run: Run,
}

impl EnginePage {
    /// Its driver.
    pub(crate) fn driver(&self) -> &PageDriver {
        &self.driver
    }

    /// Closes it now.
    pub(crate) async fn close(mut self) {
        if let Some(in_browser) = self.in_browser.take() {
            self.driver.close().await;
            drop(in_browser);
        }
    }
}

impl Drop for EnginePage {
    fn drop(&mut self) {
        if let Some(in_browser) = self.in_browser.take() {
            let driver = self.driver.clone();
            self.run.spawn(async move {
                driver.close().await;
                drop(in_browser);
            });
        }
    }
}

impl Sessions {
    /// The engine's Sessions, whose browsers `launcher` starts from `pins`,
    /// under `interception`, their pages' events in `events` and their
    /// dialogs in `dialogs`, each page's value and response observer from
    /// `handler_pages`.
    pub(crate) fn new(
        (pins, launcher): (Vec<PinnedArtifact>, Launcher),
        interception: Interception,
        events: page_events::Table,
        dialogs: Dialogs,
        handler_pages: Arc<dyn HandlerPages>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            pins,
            launcher,
            interception,
            consent: Consent::default(),
            hiding: Hiding::default(),
            events,
            dialogs,
            handler_pages,
            page_listeners: session_pages::PageListeners::default(),
        }
    }

    /// The request a Session's browser sends for a document, read from
    /// the build it would launch, pinned or installed, for `run`.
    pub(crate) async fn document_request(
        &self,
        run: &Run,
    ) -> Result<Option<DocumentRequest>, Unread> {
        DocumentRequest::of_browser(&self.pins, run).await
    }

    /// The consent walls the pages' Consent prompt runners left for each
    /// page's next call.
    pub(crate) fn consent_walls(&self) -> &crate::consent::Walls {
        self.consent.walls()
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
        let named = named_source(&run, params.source, context);
        let identity = access_mode_opens(&settings, &run, named.as_ref(), context)?;
        let borrowed = opening_source(named)?;
        no_local_address_on(&run, source_of(borrowed.as_ref()))?;
        let signing = open_signing(&run, identity.signing.as_ref(), context).await?;
        self.reach(&run, context).await?;
        let (position, pool) = taken(&mut *self.state.lock().await, &run)?;
        let proxy = open_proxy(&run, &settings, context, position).await?;
        no_proxy_under(borrowed.as_ref(), proxy.as_deref())?;
        let relay = open_relay(
            &run,
            proxy.as_ref(),
            borrowed.is_some(),
            &pool,
            rotation_ordinal(position),
            context,
        )
        .await?;
        let attributes = if borrowed.is_some() { BORROWED } else { FRESH };
        // It goes into the table under the lock, while its Run is there:
        // once the Run stops, its clean-up has taken the Run out, and a
        // Session put in after it would hold the Run forever (#1763).
        let (session, life, open) = {
            let mut state = self.state.lock().await;
            let runs = reached(&mut state, &run)?;
            let seat = match &borrowed {
                Some(cdp_url) => runs.pool.borrowed_seat(cdp_url.clone()),
                None => runs
                    .pool
                    .seat(Placement::Shared, egress(proxy.as_ref(), relay.as_ref())),
            };
            let session = session_id(run.id(), position);
            let life = SessionLife::new(session.clone(), attributes, SessionState::Opening);
            let open = Arc::new(Session {
                run: run.clone(),
                position,
                seat,
                settings: settings.clone(),
                life: life.clone(),
                proxy,
                egress_marked: OnceCell::new(),
                relay,
                reopen_failed: None,
                held_frames: borrowed
                    .as_ref()
                    .map(|_| session_pages::HeldFrames::default()),
                injections: Injections::default(),
            });
            state.sessions.insert(session.clone(), Arc::clone(&open));
            self.open_in_interception(&session, &open, borrowed.as_ref(), signing);
            (session, life, open)
        };
        if let Err(error) = open.seat.open().await {
            life.open_failed(error.clone())?;
            self.state.lock().await.sessions.remove(&session);
            self.interception.close_session(&session, &[]);
            return Err(error);
        }
        match life.apply(SessionEvent::SourceReady, None)? {
            SessionEffect::Opened => {}
            // A `close()` while it opened abandoned the open (S3).
            _ => return Err(cancelled(CancelledBy::Program, ClosedValue::Session)),
        }
        self.record(&run, position, false, &settings, borrowed.as_ref(), context)
            .await?;
        mark_egress(&open, &session).await;
        Ok(SessionOpened {
            session_id: session,
        })
    }

    /// Opens a page in the default Session of the live Run `run_id`, opened
    /// now when the Run has none; `closed` once it is closed.
    pub(crate) async fn new_default_page(
        &self,
        run_id: &RunId,
        settings: &Map<String, Value>,
        context: &CallContext,
    ) -> Result<PageOpened, CallError> {
        let run = live(run_id, context).await?;
        let named = named_source(&run, None, context);
        let identity = access_mode_opens(&Values::default(), &run, named.as_ref(), context)?;
        let borrowed = opening_source(named)?;
        self.reach(&run, context).await?;
        let default = Arc::clone(&reached(&mut *self.state.lock().await, &run)?.default);
        let position = *default
            .get_or_try_init(|| {
                self.open_held(
                    &run,
                    borrowed.as_ref(),
                    identity.signing.as_ref(),
                    Opener::Default,
                    context,
                )
            })
            .await?;
        if reached(&mut *self.state.lock().await, &run)?
            .closed
            .contains(&position)
        {
            return Err(CallError::new(
                &CLOSED,
                format!(
                    "the default Session of the Run `{}` is closed: open a Session of its own with `run.new_session`",
                    run.id()
                ),
                &json!({ "session_id": session_id(run.id(), position) }),
            ));
        }
        let session = session_id(run.id(), position);
        self.new_page(&session, settings, context).await
    }

    /// Opens the default Session of `run`, or a Session its crawl's Session
    /// pool holds, as `opener` says, on the Borrowed browser at `borrowed`
    /// if there is one, signing as `signing` plans when it is `identified`,
    /// and answers its position: its signing key is resolved, and its proxy
    /// and the default Session's record in run state are opened and
    /// written, outside the table's lock.
    async fn open_held(
        &self,
        run: &Run,
        borrowed: Option<&CdpUrl>,
        signing: Option<&SigningPlan>,
        opener: Opener,
        context: &CallContext,
    ) -> Result<u32, CallError> {
        no_local_address_on(run, source_of(borrowed))?;
        let signing = open_signing(run, signing, context).await?;
        let (position, pool) = taken(&mut *self.state.lock().await, run)?;
        let proxy = open_proxy(run, &Values::default(), context, position).await?;
        no_proxy_under(borrowed, proxy.as_deref())?;
        // A pool Session's relay binds the position its pool gave it, which
        // its HTTP requests leave from too (ADR 0044's #682 note).
        let address_ordinal = match opener {
            Opener::Pool {
                address_ordinal: Some(ordinal),
            } => ordinal,
            Opener::Default | Opener::Pool { .. } => rotation_ordinal(position),
        };
        let relay = open_relay(
            run,
            proxy.as_ref(),
            borrowed.is_some(),
            &pool,
            address_ordinal,
            context,
        )
        .await?;
        let seat = match borrowed {
            Some(cdp_url) => pool.borrowed_seat(cdp_url.clone()),
            None => pool.seat(Placement::Shared, egress(proxy.as_ref(), relay.as_ref())),
        };
        if let Opener::Default = opener {
            self.record(run, position, true, &Values::default(), borrowed, context)
                .await?;
        }
        let attributes = match (opener, borrowed.is_some()) {
            (Opener::Default, false) => FRESH,
            (Opener::Default, true) => BORROWED,
            (Opener::Pool { .. }, false) => POOL,
            (Opener::Pool { .. }, true) => POOL_BORROWED,
        };
        let session = session_id(run.id(), position);
        // The Run may have stopped while its record was written: a Session
        // goes into the table only while its Run is there, under the lock
        // its clean-up takes it out under (#1763).
        let held = Arc::new(Session {
            run: run.clone(),
            position,
            seat,
            settings: Values::default(),
            life: SessionLife::new(session.clone(), attributes, SessionState::Open),
            proxy,
            egress_marked: OnceCell::new(),
            relay,
            reopen_failed: None,
            held_frames: borrowed.map(|_| session_pages::HeldFrames::default()),
            injections: Injections::default(),
        });
        let mut state = self.state.lock().await;
        reached(&mut state, run)?;
        state.sessions.insert(session.clone(), Arc::clone(&held));
        self.open_in_interception(&session, &held, borrowed, signing);
        Ok(position)
    }

    /// Opens a page in the Session `session`, once the governor admits one
    /// more browser page; `closed` when the Session is closed.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(session_id = %session.0, page_id = tracing::field::Empty))]
    pub(crate) async fn new_page(
        &self,
        session: &SessionId,
        settings: &Map<String, Value>,
        context: &CallContext,
    ) -> Result<PageOpened, CallError> {
        let open = self.session(session, context).await?;
        if let Some(failed) = &open.reopen_failed {
            return Err(failed.clone());
        }
        let settings = page_settings(settings, &open.run, context)?;
        open.life.call().await?;
        let run = open.run.clone();
        if let Some(proxy) = &open.proxy {
            proxy.warn_if_outlived(&run, session).await;
        }
        let value = self.handler_pages.value(run.id(), session);
        lease_holds(&value)?;
        let admission = run.governor().admit(WorkKind::BrowserPage).await;
        // A Session closed while the page waited, or whose browser went
        // under it, raises what its table says now (#697, S8, S9).
        let (driver, in_browser) = match open.seat.new_page(Visibility::Visible).await {
            Ok(opened) => opened,
            Err(error) => return Err(open.life.settled(error).await),
        };
        let driver = driver
            .with_value(value, run.clock())
            .with_reader(self.handler_pages.reader(run.id(), session));
        let open_session = Arc::clone(&open);
        // Its set-up is the rest of its opening (ADR 0140's C7).
        let page = counted_as_opening(self.adopt(session_pages::Adopting {
            observer: self.handler_pages.observer(run.id(), session),
            settings,
            open,
            driver,
            in_browser,
            admission,
            events: context.events().clone(),
        }))
        .await?;
        tracing::Span::current().record("page_id", page.0.as_str());
        mark_egress(&open_session, session).await;
        Ok(PageOpened {
            page_id: page,
            session_id: session.clone(),
        })
    }

    /// Closes the Session `session`: its pages and its browser context
    /// close, and its pages' admissions go back; a Session closed already,
    /// or ended with its Run, stays as it is (ADR 0078).
    pub(crate) async fn close(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let open = match self.session(session, context).await {
            Ok(open) => open,
            Err(error) => {
                return if self.was_closed(session).await
                    || ended_with_its_run(session, context).await
                {
                    Ok(())
                } else {
                    Err(error)
                };
            }
        };
        let effect = open.life.apply(SessionEvent::Close, None)?;
        self.end_session(&open, session, effect, context).await
    }

    /// Opens a page in the Session a crawl's Session pool names `pooled`
    /// in the live Run `run_id`, opening that Session first when it has no
    /// browser context yet: from its source, `Some(None)` for a fresh one,
    /// or else from the Run's `session.source` (ADR 0093), its egress relay
    /// bound to the rotation position the pool gave it. The Session is the
    /// pool's (ADR 0088): only [`Sessions::retire`] ends it, besides its
    /// Run; `closed` once it is retired.
    pub(crate) async fn new_pool_page(
        &self,
        run_id: &RunId,
        pooled: &PooledSession,
        context: &CallContext,
    ) -> Result<PageOpened, CallError> {
        let session = self.pool_session(run_id, pooled, context).await?;
        self.new_page(&session, &Map::new(), context).await
    }

    /// The Session a crawl's Session pool names `pooled` in the live Run
    /// `run_id`, opened as [`Sessions::new_pool_page`] opens it when it has
    /// no browser context yet.
    pub(crate) async fn pool_session(
        &self,
        run_id: &RunId,
        pooled: &PooledSession,
        context: &CallContext,
    ) -> Result<SessionId, CallError> {
        let run = live(run_id, context).await?;
        let named = named_source(&run, pooled.source.clone(), context);
        let identity = access_mode_opens(&Values::default(), &run, named.as_ref(), context)?;
        let borrowed = opening_source(named)?;
        self.reach(&run, context).await?;
        let cell = Arc::clone(
            reached(&mut *self.state.lock().await, &run)?
                .pooled
                .entry(pooled.id.clone())
                .or_default(),
        );
        let opener = Opener::Pool {
            address_ordinal: pooled.address_ordinal,
        };
        let position = *cell
            .get_or_try_init(|| {
                self.open_held(
                    &run,
                    borrowed.as_ref(),
                    identity.signing.as_ref(),
                    opener,
                    context,
                )
            })
            .await?;
        Ok(session_id(run.id(), position))
    }

    /// The Run of the open Session `session` and its own settings, which
    /// its pages take over the Run's; `closed` once it closed.
    pub(crate) async fn own_settings(
        &self,
        session: &SessionId,
    ) -> Result<(Run, Values), CallError> {
        self.state
            .lock()
            .await
            .sessions
            .get(session)
            .map(|open| (open.run.clone(), open.settings.clone()))
            .ok_or_else(|| session_closed(session))
    }

    /// Retires the Session a crawl's Session pool names `pooled` in the
    /// Run `run_id` (spec #594's `Retire`): its pages and its browser
    /// context close, as `session.close` closes them, and the pool opens
    /// its replacement as a new Session. A Session the pool never opened a
    /// browser context for, one already closing or closed (a `Retire`
    /// racing a `Close`) and a Run no longer live change nothing.
    ///
    /// # Errors
    ///
    /// `internal`, for a Session the table does not let the pool retire,
    /// such as one on a Borrowed browser.
    pub(crate) async fn retire(
        &self,
        run_id: &RunId,
        pooled: &SessionId,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let position = {
            let state = self.state.lock().await;
            state
                .runs
                .get(run_id)
                .and_then(|runs| runs.pooled.get(pooled))
                .and_then(|cell| cell.get().copied())
        };
        let Some(position) = position else {
            return Ok(());
        };
        let session = session_id(run_id, position);
        let Some(open) = self.state.lock().await.sessions.get(&session).cloned() else {
            return Ok(());
        };
        let effect = open.life.apply(SessionEvent::Retire, None)?;
        self.end_session(&open, &session, effect, context).await
    }

    /// Ends the Session `open`, whose table answered `effect` to its
    /// `Close` or its `Retire`: its pages, then its browser context, close.
    async fn end_session(
        &self,
        open: &Arc<Session>,
        session: &SessionId,
        effect: SessionEffect,
        context: &CallContext,
    ) -> Result<(), CallError> {
        if let Some(proxy) = &open.proxy {
            proxy.warn_if_outlived(&open.run, session).await;
        }
        let crashed = match effect {
            // Already closing or closed: `close()` changes nothing (C).
            SessionEffect::Nothing => return Ok(()),
            SessionEffect::AbandonOpen
            | SessionEffect::CloseChildren
            | SessionEffect::EndAttachAndClose => false,
            // A crashed Session releases what is left and stays `Crashed`
            // (S22).
            SessionEffect::Release => true,
            SessionEffect::Serve
            | SessionEffect::Wait
            | SessionEffect::Opened
            | SessionEffect::FailOpen
            | SessionEffect::Lend
            | SessionEffect::EndAttach
            | SessionEffect::Quit
            | SessionEffect::Crash
            | SessionEffect::Suspend
            | SessionEffect::EndAttachAndSuspend
            | SessionEffect::Unreachable
            | SessionEffect::Resume
            | SessionEffect::Reopen => {
                return Err(invalid_transition(
                    "session",
                    &format!("{:?}", open.life.state()),
                    &format!("Close or Retire (effect {effect:?})"),
                ));
            }
        };
        let mut closing = Vec::new();
        // Calls in flight on its pages end first (S7, P8); a crashed page
        // releases what is left (P33).
        for (page_id, page) in self.pages_of(session).await {
            match page.life.end(&Ending::Closed(ClosedValue::Session))? {
                PageEffect::Close => closing.push((page_id, page)),
                effect => {
                    self.page_ended(&page_id, &open.run, &page.life, effect)
                        .await;
                }
            }
        }
        open.seat.close().await;
        for (page_id, page) in &closing {
            // The context's disposal destroyed each target: the close the
            // engine sent is confirmed (P29).
            let effect = page.life.end(&Ending::Destroyed)?;
            self.page_ended(page_id, &open.run, &page.life, effect)
                .await;
        }
        if crashed {
            self.hiding.close(session).await;
            return Ok(());
        }
        open.life.apply(SessionEvent::ChildrenClosed, None)?;
        self.session_ended(open, session)
            .await
            .map_err(|error| unavailable(&open.run, &error, context))
    }

    /// The pages the Session `session` holds, by ID.
    async fn pages_of(&self, session: &SessionId) -> Vec<(PageId, OpenPage)> {
        self.state
            .lock()
            .await
            .pages
            .iter()
            .filter(|(_, page)| &page.session == session)
            .map(|(page_id, page)| (page_id.clone(), page.clone()))
            .collect()
    }

    /// The Session `session` ended: it leaves, whatever pages it still
    /// held go, and its run state records it closed. A Session of a
    /// crawl's Session pool, which the run file never records, writes
    /// nothing: at `batch`, its retirement is no barrier that commits the
    /// crawl's open batch (#2168).
    async fn session_ended(
        &self,
        open: &Session,
        session: &SessionId,
    ) -> Result<(), RunStateError> {
        let pages = {
            let mut state = self.state.lock().await;
            state.sessions.remove(session);
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
        self.events.close(&pages);
        self.dialogs.close(&pages);
        self.interception.close_session(session, &pages);
        self.hiding.close(session).await;
        if open.life.pooled() {
            return Ok(());
        }
        let position = open.position;
        open.run
            .state()
            .write(move |connection| {
                connection
                    .prepare_cached("UPDATE page_sessions SET closed = 1 WHERE position = ?1")?
                    .execute([position])
                    .map(drop)
            })
            .await
    }

    /// Opens a page of the open Session `session` that the engine drives
    /// itself, apart from the Session's pages: no call names it, no route
    /// or event of theirs sees it, and it holds no admission of the
    /// governor's. The browser-fetch tier sends `page.request` from one,
    /// in the Session's browser context, so its cookies are the Session's
    /// (ADR 0020). It carries `value`, the value of the page it acts for.
    /// It closes once its caller is done with it, or drops it; `closed`
    /// when the Session is. It counts toward its browser's limits, and moves
    /// the Session off a recycled browser unless a move would race what
    /// `work` says (ADR 0040).
    pub(crate) async fn engine_page(
        &self,
        session: &SessionId,
        value: &PageValue,
        work: EngineWork,
    ) -> Result<EnginePage, CallError> {
        let (open, races) = {
            let state = self.state.lock().await;
            let open = state
                .sessions
                .get(session)
                .cloned()
                .ok_or_else(|| session_closed(session))?;
            let races = match work {
                EngineWork::ArrivingDocument => true,
                EngineWork::Request => {
                    self.interception.navigation_pending(session)
                        || state
                            .pages
                            .values()
                            .any(|page| page.session == *session && page.life.navigating())
                }
            };
            (open, races)
        };
        if let Some(failed) = &open.reopen_failed {
            return Err(failed.clone());
        }
        open.life.call().await?;
        lease_holds(value)?;
        let (driver, in_browser) = open.seat.new_engine_page(races).await?;
        Ok(EnginePage {
            driver: driver.with_value(value.clone(), open.run.clock()),
            in_browser: Some(in_browser),
            run: open.run.clone(),
        })
    }

    /// Whether the open Session `session` serves the engine's own page for
    /// a page carrying `value` now, with no wait: it is open and serves
    /// calls, and `value`'s lease holds. A fetch from its engine's page that
    /// failed while it no longer did failed in the engine, not at the site.
    pub(crate) async fn serves(&self, session: &SessionId, value: &PageValue) -> bool {
        let serving = self
            .state
            .lock()
            .await
            .sessions
            .get(session)
            .is_some_and(|open| open.reopen_failed.is_none() && open.life.serves());
        serving && lease_holds(value).is_ok()
    }

    /// The page `page_id` this engine holds; `page_crashed` again for a
    /// crashed one a close released, else `closed`.
    pub(crate) async fn page(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        let state = self.state.lock().await;
        if let Some(page) = state.pages.get(page_id) {
            return Ok(page.clone());
        }
        Err(state
            .crashed
            .get(page_id)
            .map_or_else(|| page_closed(page_id), |(_, life)| life.crashed()))
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
        self.reach(&run, context).await?;
        self.state
            .lock()
            .await
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

    /// Reaches the live Run `run`: its Sessions, reopened from its run state
    /// the first time this engine reaches it, leave when it stops. The run
    /// state is read, and each reopened Session's proxy opened, outside the
    /// table's lock, so no other Run's call waits on them; a call that
    /// reached the Run first in the meantime keeps its own.
    async fn reach(&self, run: &Run, context: &CallContext) -> Result<(), CallError> {
        if self.state.lock().await.runs.contains_key(run.id()) {
            return Ok(());
        }
        let recorded = recorded_sessions(run, context).await?;
        let (pool, losses) = Pool::new(
            run.clone(),
            self.launcher.clone(),
            context.engine().home().clone(),
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
            default: Arc::new(OnceCell::new()),
            closed: BTreeSet::new(),
            pooled: BTreeMap::new(),
        };
        let mut reopened = Vec::new();
        for (position, is_default, closed, settings, source) in recorded {
            runs.last = runs.last.max(position);
            if is_default {
                runs.default = Arc::new(OnceCell::new_with(Some(position)));
            }
            if closed {
                runs.closed.insert(position);
                continue;
            }
            let settings = recorded_settings(&settings, run, context)?;
            let borrowed = match source {
                Some(sealed) => Some(opened_source(&sealed, position, run, context).await?),
                None => None,
            };
            let (proxy, relay, reopen_failed) =
                reopened_egress(run, &settings, &pool, borrowed.as_ref(), position, context).await;
            let (signing, reopen_failed) =
                match reopened_signing(run, &settings, borrowed.is_some(), context).await {
                    Ok(signing) => (signing, reopen_failed),
                    Err(error) => (None, reopen_failed.or(Some(error))),
                };
            let session = session_id(run.id(), position);
            let (seat, attributes) = match &borrowed {
                Some(cdp_url) => (pool.borrowed_seat(cdp_url.clone()), BORROWED),
                None => (
                    pool.seat(Placement::Shared, egress(proxy.as_ref(), relay.as_ref())),
                    FRESH,
                ),
            };
            let held = Session {
                run: run.clone(),
                position,
                seat,
                settings,
                life: SessionLife::new(session, attributes, SessionState::Open),
                proxy,
                egress_marked: OnceCell::new(),
                relay,
                reopen_failed,
                held_frames: borrowed
                    .as_ref()
                    .map(|_| session_pages::HeldFrames::default()),
                injections: Injections::default(),
            };
            reopened.push((held, borrowed, signing));
        }
        let mut state = self.state.lock().await;
        if state.runs.contains_key(run.id()) {
            return Ok(());
        }
        for (open, borrowed, signing) in reopened {
            let session = session_id(run.id(), open.position);
            self.open_in_interception(&session, &open, borrowed.as_ref(), signing);
            state.sessions.insert(session, Arc::new(open));
        }
        state.runs.insert(run.id().clone(), runs);
        drop(state);
        self.follow_losses(run, losses);
        self.leave_with(run);
        self.end_pages_past_their_cap(run);
        Ok(())
    }

    /// Takes the Run's Sessions and pages out once it stops, warning the
    /// Run of each Session that outlived its proxy's sticky lifetime.
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
            let ended: Vec<(&SessionId, Arc<Session>)> = gone
                .iter()
                .filter_map(|session| Some((session, state.sessions.remove(session)?)))
                .collect();
            state.crashed.retain(|_, (run, _)| run != &run_id);
            drop(state);
            // Each Session ends with its Run: one that outlived its proxy's
            // sticky lifetime is named in the Run report, which is written
            // once the Run's tasks, this one among them, have ended (ADR
            // 0021's #882 note).
            for (session, open) in &ended {
                if let Some(proxy) = &open.proxy {
                    proxy.warn_if_outlived(&open.run, session).await;
                }
            }
            drop(ended);
            for session in &gone {
                let pages = pages.remove(session).unwrap_or_default();
                sessions.events.close(&pages);
                sessions.dialogs.close(&pages);
                sessions.interception.close_session(session, &pages);
                sessions.hiding.close(session).await;
            }
        });
    }

    /// Puts the Session `session`, `held`, under the interception; one no
    /// egress relay is under, on a Borrowed browser at `borrowed` or
    /// launched with none, by interception alone, its requests' hosts
    /// resolved through the Run's `network.resolve`, and a name the engine
    /// cannot resolve judged by where it would be resolved (ADR 0056;
    /// #2435).
    fn open_in_interception(
        &self,
        session: &SessionId,
        held: &Session,
        borrowed: Option<&CdpUrl>,
        signing: Option<Arc<RequestSigner>>,
    ) {
        self.interception.open_session(session);
        if let Some(signer) = signing {
            self.interception.sign_with(session, signer);
        }
        let held_by = held_browser(borrowed, held.relay.is_some(), held.proxy.is_some());
        if let Some(browser) = held_by {
            self.interception.hold_by_interception_only(
                session,
                Lookups::new(
                    resolve_of(held.run.settings().map(RunSettings::resolved)),
                    browser,
                ),
            );
        }
    }

    /// Records the Session at `position` of `run`, with its own `settings`,
    /// the identity seed it draws from the Run's `Entropy` and the
    /// `cdp_url` of the Borrowed browser it is on, `borrowed`, if it is, in
    /// its run state.
    async fn record(
        &self,
        run: &Run,
        position: u32,
        is_default: bool,
        settings: &Values,
        borrowed: Option<&CdpUrl>,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let source = match borrowed {
            Some(cdp_url) => {
                let source = json!({
                    "url": cdp_url.url,
                    "layer": cdp_url.origin.layer,
                    "acting_for_model": cdp_url.origin.acting_for_model,
                })
                .to_string();
                let sealed = run
                    .seal(&SOURCE, &position.to_string(), source.as_bytes())
                    .await
                    .map_err(|error| no_source(run, error, context))?;
                Some(sealed)
            }
            None => None,
        };
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
                    .prepare_cached(
                        "INSERT INTO page_sessions \
                         (position, is_default, closed, settings, identity_seed, source) \
                         VALUES (?1, ?2, 0, ?3, ?4, ?5)",
                    )?
                    .execute(rusqlite::params![
                        position, is_default, settings, seed, source
                    ])
                    .map(drop)
            })
            .await
            .map_err(|error| unavailable(run, &error, context))
    }
}

/// Reads a Session's own `settings` (ADRs 0083 and 0092): the Clean-up
/// rules' keys, which its pages take over the Run's (ADR 0047), its Page
/// check, `session.page_check` (ADR 0092), and the
/// `proxy.*` keys, its one proxy (ADR 0021; SA-22), which a Client acting
/// for a model may not set; and `setting_locked` for one the Run's layers
/// lock. Any other key is refused with `setting_not_applicable`:
/// `session.source`, which the call's own `source` sets, first; any other,
/// since no other setting takes effect on one Session apart from the Run's
/// yet, so none is ignored.
fn own_settings(
    settings: &Map<String, Value>,
    run: &Run,
    context: &CallContext,
) -> Result<Values, CallError> {
    if settings.is_empty() {
        return Ok(Values::default());
    }
    let mut values = Values::read(SettingsLayer::Run, settings, context.identity().product)
        .map_err(|error| settings_error(&error, context))?
        .from_origin(RunOrigin::Settings);
    if context.surface().acts_for_model() {
        values = values.from_model();
    }
    let refused = if values.get(SESSION_SOURCE.name).is_some() {
        Some((
            SESSION_SOURCE.name,
            "pass it as `run.new_session`'s own `source`",
        ))
    } else {
        values
            .keys()
            .find(|key| !applies_to(key, SettingScope::Session))
            .map(|key| {
                (
                    key,
                    set_outside_code(key)
                        .unwrap_or("set it on the Run, whose Sessions all take it"),
                )
            })
    };
    if let Some((key, fix)) = refused {
        return Err(settings_error(
            &SettingsError::NotApplicable {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: fix.to_owned(),
            },
            context,
        ));
    }
    not_locked(values, run, "Session", context)
}

/// `values`, a scope's own settings, unless the Run's layers lock one of
/// them: then `setting_locked`, its fix naming `scope`.
fn not_locked(
    values: Values,
    run: &Run,
    scope: &str,
    context: &CallContext,
) -> Result<Values, CallError> {
    let locked = values.keys().find_map(|key| {
        let effective = run.settings()?.resolved().effective(setting(key)?)?;
        effective.locked_by.map(|layer| SettingLimit {
            key: key.to_owned(),
            value: effective.value.clone(),
            layer,
            origin: None,
        })
    });
    match locked {
        Some(limit) => Err(settings_error(
            &SettingsError::Locked {
                key: limit.key.clone(),
                layer: SettingsLayer::Run,
                fix: format!(
                    "leave `{}` out of the {scope}'s settings: the `{}` layer locks it for the Run",
                    limit.key,
                    limit.layer.as_str()
                ),
                limit,
            },
            context,
        )),
        None => Ok(values),
    }
}

/// Reads a page's own `settings` (ADR 0083): `content.extractor`, the
/// main-content step its formats read with, which it takes over its
/// Session's and its Run's; `setting_locked` when the Run's layers lock
/// it. Any other key is refused with `setting_not_applicable`, since no
/// other setting takes effect on one page apart from its Session's yet,
/// so none is ignored.
fn page_settings(
    settings: &Map<String, Value>,
    run: &Run,
    context: &CallContext,
) -> Result<Values, CallError> {
    if settings.is_empty() {
        return Ok(Values::default());
    }
    let mut values = Values::read(SettingsLayer::Run, settings, context.identity().product)
        .map_err(|error| settings_error(&error, context))?
        .from_origin(RunOrigin::Settings);
    if context.surface().acts_for_model() {
        values = values.from_model();
    }
    if let Some(key) = values
        .keys()
        .find(|key| !applies_to(key, SettingScope::Page))
    {
        let fix = set_outside_code(key).unwrap_or(if setting_scopes::fixed_for_the_session(key) {
            "a Session keeps its Access mode, Identity profile and proxy for its life: open a new Session with it"
        } else {
            "set it on the page's Session or its Run, whose pages all take it"
        });
        return Err(settings_error(
            &SettingsError::NotApplicable {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: fix.to_owned(),
            },
            context,
        ));
    }
    not_locked(values, run, "page", context)
}

/// Whether the setting `key` names is one `scope`'s own `settings` set
/// (ADR 0083).
fn applies_to(key: &str, scope: SettingScope) -> bool {
    setting(key).is_some_and(|setting| setting_scopes::of(setting).contains(&scope))
}

/// Where the setting `key` names is set when a program's own code sets it
/// at no scope (ADR 0083), as `proxy.providers`, which only configuration
/// sets; `None` when some scope's own `settings` set it.
fn set_outside_code(key: &str) -> Option<&'static str> {
    let setting = setting(key).filter(|setting| setting_scopes::of(setting).is_empty())?;
    Some(match setting.authority {
        Authority::OperatorPolicy | Authority::OperatorPolicyAlone => {
            "set it in an Operator policy"
        }
        Authority::ProjectFileAlone => {
            "set it in the project file, which `connect()` and the CLI read before they connect"
        }
        Authority::Fixed => "it keeps its built-in value, which no layer changes",
        _ => "set it in configuration or an Operator policy, which alone set it",
    })
}

/// The proxy a Session of `run` opens with: the one its own settings `own`
/// name, beneath the Run's, within the Run's locks and each key's
/// authority, or the Run's.
async fn open_proxy(
    run: &Run,
    own: &Values,
    context: &CallContext,
    position: u32,
) -> Result<Option<Arc<SessionProxy>>, CallError> {
    let Some(settings) = run.settings() else {
        return match own.keys().next() {
            Some(key) => Err(settings_error(
                &SettingsError::NotApplicable {
                    key: key.to_owned(),
                    layer: SettingsLayer::Run,
                    fix: "start the Run with its settings".to_owned(),
                },
                context,
            )),
            None => Ok(None),
        };
    };
    let resolved = if own.is_empty() {
        settings.resolved().clone()
    } else {
        settings
            .resolved()
            .beneath(own)
            .map_err(|error| settings_error(&error, context))?
    };
    // A Proxy provider's price must be in the currency of each currency
    // Limit on `proxy_bytes` that holds the Run (ADR 0101).
    let currencies = if resolved.get::<Option<&str>>(&PROXY_PROVIDER).is_none() {
        Vec::new()
    } else {
        run.usage()
            .await
            .map_err(|error| unavailable(run, &error, context))?
            .limits
            .into_iter()
            .filter(|limit| limit.usage_kind == PROXY_BYTES)
            .filter_map(|limit| limit.currency)
            .collect()
    };
    let session = session_id(run.id(), position);
    let opening = Opening {
        session: &session,
        ordinal: u64::from(position),
        currencies: &currencies,
    };
    Ok(SessionProxy::open(&resolved, run, &opening)
        .await?
        .map(Arc::new))
}

/// The failure a call answers when the data key of `run` neither seals nor
/// opens a Session's source: the key-encryption key's own refusal, or the
/// Run's state as unavailable.
fn no_source(run: &Run, error: RunKeyError, context: &CallContext) -> CallError {
    match error {
        RunKeyError::Secret(error) => goodfetch_engine::run::secret_error(&error),
        error => unavailable(run, &error, context),
    }
}

/// The `cdp_url` of the Session at `position` of `run`, opened from the
/// source its run state holds `sealed`, to reopen it on its Borrowed
/// browser.
async fn opened_source(
    sealed: &Sealed,
    position: u32,
    run: &Run,
    context: &CallContext,
) -> Result<CdpUrl, CallError> {
    let source = run
        .open(&SOURCE, &position.to_string(), sealed)
        .await
        .map_err(|error| no_source(run, error, context))?;
    recorded_source(&source, run, context)
}

/// The `cdp_url` `recorded`, as the run state of `run` holds it once
/// opened, of a Session on a Borrowed browser reopened from it.
fn recorded_source(recorded: &[u8], run: &Run, context: &CallContext) -> Result<CdpUrl, CallError> {
    #[derive(serde::Deserialize)]
    struct Recorded {
        url: String,
        layer: SettingsLayer,
        acting_for_model: bool,
    }
    let recorded: Recorded =
        serde_json::from_slice(recorded).map_err(|error| unavailable(run, &error, context))?;
    Ok(CdpUrl {
        url: recorded.url,
        origin: CdpUrlOrigin {
            layer: recorded.layer,
            acting_for_model: recorded.acting_for_model,
        },
    })
}

/// A Session as the run state records it: its position, whether it is the
/// default Session, whether it closed, its own settings and, for one on a
/// Borrowed browser, its sealed source.
type RecordedSession = (u32, bool, bool, String, Option<Sealed>);

/// The Sessions the run state of `run` records, in position order.
///
/// # Errors
///
/// `unavailable` when the run state cannot be read.
async fn recorded_sessions(
    run: &Run,
    context: &CallContext,
) -> Result<Vec<RecordedSession>, CallError> {
    run.state()
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT position, is_default, closed, settings, source FROM page_sessions ORDER BY position",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<Sealed>>(4)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
        .map_err(|error| unavailable(run, &error, context))
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
        .map(|values| values.from_origin(RunOrigin::Settings))
        .map_err(|error| unavailable(run, &error, context))
}

/// The Session source a Session of `run` opens from: `own`, the call's own
/// `source` when it gave one (`null` for a fresh Session), else the Run's
/// `session.source`; with who gave it: the layer, and whether a Client
/// acting for a model gave it or makes the call that opens it (O8 of the
/// deep review). None for a fresh Session.
#[expect(
    clippy::option_option,
    reason = "a setting's three states (ADR 0083): left out inherits, `null` clears, a value sets"
)]
fn named_source(
    run: &Run,
    own: Option<Option<SessionSource>>,
    context: &CallContext,
) -> Option<NamedSource> {
    let opener_for_model = context.surface().acts_for_model();
    match own {
        Some(own) => own.map(|source| NamedSource {
            source,
            layer: SettingsLayer::Run,
            acting_for_model: opener_for_model,
        }),
        None => run
            .settings()
            .and_then(|settings| settings.resolved().effective(&SESSION_SOURCE))
            .and_then(|effective| {
                serde_json::from_value::<SessionSource>(effective.value.clone())
                    .ok()
                    .map(|source| NamedSource {
                        source,
                        layer: effective.layer,
                        acting_for_model: effective.for_model || opener_for_model,
                    })
            }),
    }
}

/// A Session source, with the layer that named it and whether a Client
/// acting for a model gave it or opens the Session.
struct NamedSource {
    source: SessionSource,
    layer: SettingsLayer,
    acting_for_model: bool,
}

/// The Access mode a Session of `run` opens in, as its `own` settings,
/// over the Run's, ask for from the source `named` (ADR 0021): the
/// [`access_mode`](crate::access_mode) module's refusal when it does not
/// open. A `stealth` Session does not open until the tickets that build its
/// Identity profile land.
fn access_mode_opens(
    own: &Values,
    run: &Run,
    named: Option<&NamedSource>,
    context: &CallContext,
) -> Result<SessionIdentityPlan, CallError> {
    let source = named.map(|named| &named.source);
    let settings = SessionSettings::new(own, run.settings().map(RunSettings::resolved));
    access_mode::resolve(&settings, source, BrowserFamily::of(source))
        .and_then(SessionIdentityPlan::built)
        .map_err(|refusal| refusal.call_error(context))
}

/// The signer of a Session of `run` that opens `identified` with the
/// signing `plan`, none for one that does not sign: its key resolved now,
/// once for the Session, from its Secret reference, so a rotated key is
/// the next Session's, and its nonces drawn from the Run's `Entropy` (ADR
/// 0045; #1551).
///
/// # Errors
///
/// The Secret's error when its reference gives no value, and
/// `setting_invalid` naming the setting, never its value, when the key
/// cannot sign.
async fn open_signing(
    run: &Run,
    plan: Option<&SigningPlan>,
    context: &CallContext,
) -> Result<Option<Arc<RequestSigner>>, CallError> {
    let Some(plan) = plan else {
        return Ok(None);
    };
    let purpose = Purpose {
        placeholder: IDENTIFIED_SIGNING_KEY.name,
        domain: None,
        domains: &[],
        layer: Some(plan.layer),
        run_id: Some(run.id()),
    };
    let key = run
        .secrets()
        .resolve(&plan.key, &purpose)
        .await
        .map_err(|error| secret_error(&error))?;
    let signer =
        RequestSigner::new(&key, &plan.agent, run.fork_entropy().await).map_err(|error| {
            let setting = match error {
                KeyError::BadAgent => &IDENTIFIED_SIGNATURE_AGENT,
                KeyError::NotJwk
                | KeyError::NotEd25519
                | KeyError::BadPrivateKey
                | KeyError::MismatchedPublicKey => &IDENTIFIED_SIGNING_KEY,
            };
            settings_error(
                &SettingsError::Invalid {
                    key: setting.name.to_owned(),
                    layer: Some(plan.layer),
                    reason: error.to_string(),
                },
                context,
            )
        })?;
    Ok(Some(Arc::new(signer)))
}

/// The Borrowed browser a Session opens on from the source `named`: none
/// for a fresh Session, the browser at a `cdp_url` (ADRs 0056 and 0115),
/// whose address is judged when it opens, by who gave it. Any other kind
/// raises `capability_unavailable` naming it, until the ticket of that kind
/// builds its reader, and so does a `cdp_url` with `headers`, whose Secret
/// references the `cdp_url` Session resolves (#743).
fn opening_source(named: Option<NamedSource>) -> Result<Option<CdpUrl>, CallError> {
    let Some(NamedSource {
        source,
        layer,
        acting_for_model,
    }) = named
    else {
        return Ok(None);
    };
    match source {
        SessionSource::CdpUrl {
            url, headers: None, ..
        } => Ok(Some(CdpUrl {
            url,
            origin: CdpUrlOrigin {
                layer,
                acting_for_model,
            },
        })),
        source => {
            let kind = source.kind();
            let what = if matches!(source, SessionSource::CdpUrl { .. }) {
                "a `cdp_url` Session source with `headers`".to_owned()
            } else {
                format!("a `{kind}` Session source")
            };
            Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "This engine opens no Session from {what}, which `session.source` names in {}: open a fresh Session with `source: null`, or leave `session.source` unset.",
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

/// Refuses a proxy under a Borrowed browser: no proxy can be put under a
/// browser that is already running (ADR 0056).
fn no_proxy_under(
    borrowed: Option<&CdpUrl>,
    proxy: Option<&SessionProxy>,
) -> Result<(), CallError> {
    let (Some(cdp_url), Some(_)) = (borrowed, proxy) else {
        return Ok(());
    };
    Err(CallError::new(
        &CAPABILITY_UNAVAILABLE,
        format!(
            "A Session on the Borrowed browser at {} takes no `proxy.*`: no proxy can be put under a browser that is already running. Leave the `proxy.*` keys unset for it.",
            cdp_url.address()
        ),
        &CapabilityUnavailableDetails {
            key: SESSION_SOURCE.name.to_owned(),
            layer: cdp_url.origin.layer,
            kind: "cdp_url".to_owned(),
            fix: "leave the `proxy.*` keys unset".to_owned(),
        },
    ))
}

/// Records in its Run's Provenance, once, how the network of the Session
/// `session` is held: through its egress relay, at the relay's loopback
/// address, which checks the address each connection uses; or, on a
/// Borrowed browser or with no relay, by interception alone, the browser
/// resolving each request's host itself, so an answer that changes after
/// the engine's check (DNS rebinding) reaches the browser (ADR 0056).
///
/// # Cancel safety
///
/// Dropping the future may lose the entry.
async fn mark_egress(open: &Session, session: &SessionId) {
    open.egress_marked
        .get_or_init(|| async {
            let entry = match &open.relay {
                Some(relay) => {
                    let mut entry = json!({
                        "action": EGRESS_RELAY,
                        "session_id": session,
                        "egress": EGRESS_RELAY,
                        "address": relay.endpoint().address.to_string(),
                    });
                    if let Some(local) = relay.local_addresses() {
                        entry["local_addresses"] = json!({
                            "ipv4": local.ipv4.map(|address| address.to_string()),
                            "ipv6": local.ipv6.map(|address| address.to_string()),
                        });
                    }
                    entry
                }
                None => json!({
                    "action": INTERCEPTION_ONLY,
                    "session_id": session,
                    "egress": INTERCEPTION_ONLY,
                    "gap": "dns_rebinding",
                }),
            };
            trace::provenance(&open.run, entry).await;
        })
        .await;
}

/// The proxy and egress relay of the Session at `position` of `run`
/// reopened from run state with its own `settings`, and why it cannot open
/// a page when either does not open, its relay terminating as `pool`'s
/// authority lets it. It keeps the proxy it opened with,
/// so its address never changes under its cookies (SA-22); one on a
/// Borrowed browser, `borrowed`, has neither (ADR 0056). One whose proxy
/// or relay does not open raises why from each page it is asked for.
async fn reopened_egress(
    run: &Run,
    settings: &Values,
    pool: &Pool,
    borrowed: Option<&CdpUrl>,
    position: u32,
    context: &CallContext,
) -> (
    Option<Arc<SessionProxy>>,
    Option<Arc<EgressRelay>>,
    Option<CallError>,
) {
    if borrowed.is_some() {
        return (None, None, None);
    }
    let proxy = match open_proxy(run, settings, context, position).await {
        Ok(proxy) => proxy,
        Err(error) => return (None, None, Some(error)),
    };
    match open_relay(
        run,
        proxy.as_ref(),
        false,
        pool,
        rotation_ordinal(position),
        context,
    )
    .await
    {
        Ok(relay) => (proxy, relay, None),
        Err(error) => (proxy, None, Some(error)),
    }
}

/// The signer of a Session of `run` reopened from run state with its own
/// `settings`, as [`open_signing`] makes it when a Session opens: its key
/// resolved again, once for the reopened Session. One on a Borrowed
/// browser, when `borrowed`, never signs (ADR 0021).
///
/// # Errors
///
/// As [`access_mode_opens`] and [`open_signing`] say: the Session then
/// raises it from each page it is asked for.
async fn reopened_signing(
    run: &Run,
    settings: &Values,
    borrowed: bool,
    context: &CallContext,
) -> Result<Option<Arc<RequestSigner>>, CallError> {
    if borrowed {
        return Ok(None);
    }
    let identity = access_mode_opens(settings, run, None, context)?;
    open_signing(run, identity.signing.as_ref(), context).await
}

/// The browser interception alone holds for a Session: a Borrowed one at
/// `borrowed`, or one launched with no egress relay, `relayed` false,
/// behind a proxy when `proxied`; none when a relay holds the Session,
/// which checks each connection's address itself (ADRs 0017 and 0056;
/// #2435).
fn held_browser(borrowed: Option<&CdpUrl>, relayed: bool, proxied: bool) -> Option<HeldBrowser> {
    match borrowed {
        Some(cdp_url) => Some(HeldBrowser::Borrowed(cdp_url.clone())),
        None if relayed => None,
        None => Some(HeldBrowser::Launched { proxied }),
    }
}

/// What a Session's connections leave through: `proxy` and `relay`.
fn egress(proxy: Option<&Arc<SessionProxy>>, relay: Option<&Arc<EgressRelay>>) -> SessionEgress {
    SessionEgress {
        proxy: proxy.cloned(),
        relay: relay.cloned(),
    }
}

/// The egress relay of a Session of `run`, bound to the addresses of the
/// `address_ordinal`-th position of the Run's local-address rotation, whose
/// upstream proxy is `proxy`, while `security.raw_cdp_egress_proxy` is on, the
/// Run's `pool` has the relay's certificate authority, for its certificate
/// exceptions, or the Run sets `egress.local_addresses`, its connections
/// then bound to the Session's addresses, none of which the switch ever
/// removes (ADRs 0042 and 0044): none on a Borrowed browser, whose network
/// the engine does not hold (ADR 0056), with the switch off and neither,
/// or behind an `https://` proxy, which the relay does not chain through;
/// such a Session is held by interception alone.
///
/// # Errors
///
/// `local_address_unavailable` when the Session's local addresses give
/// none now, or it has them behind an `https://` proxy; the relay's bind
/// failing.
async fn open_relay(
    run: &Run,
    proxy: Option<&Arc<SessionProxy>>,
    borrowed: bool,
    pool: &Pool,
    address_ordinal: u64,
    context: &CallContext,
) -> Result<Option<Arc<EgressRelay>>, CallError> {
    if borrowed {
        return Ok(None);
    }
    let local = match pool.local(address_ordinal).await {
        Ok(local) => local,
        Err(LocalFailure::Unavailable(unavailable)) => return Err(unavailable.into()),
        Err(LocalFailure::Unreadable(error)) => return Err(unavailable(run, &error, context)),
    };
    let on = run
        .settings()
        .map(RunSettings::resolved)
        .is_none_or(egress_relay::relayed);
    let tls = pool.relay_tls().await;
    if local.is_none() && !on && tls.authority.is_none() {
        return Ok(None);
    }
    let chain = match proxy {
        None => None,
        Some(proxy) => match proxy.chain() {
            Some(chain) => Some(chain),
            None if local.is_some() => {
                return Err(local_address_refused(
                    run,
                    LocalAddressFailure::ProxyNotRelayed,
                ));
            }
            None => return Ok(None),
        },
    };
    EgressRelay::open(run, chain, tls, local)
        .await
        .map(|relay| Some(Arc::new(relay)))
        .map_err(|error| unavailable(run, &error, context))
}

/// The Session source kind of a Session on the Borrowed browser at
/// `borrowed`, else on a browser the engine launches.
fn source_of(borrowed: Option<&CdpUrl>) -> Source {
    match borrowed {
        Some(_) => Source::Borrowed,
        None => Source::Launched,
    }
}

/// Refuses `egress.local_addresses` for a Session of `run` from `source`
/// whose browser the engine does not launch on this machine, a Borrowed
/// browser, a third-party CDP browser or a Browser host's, with
/// `local_address_unavailable`: an address of this machine is never claimed
/// for another (ADRs 0044 and 0056).
fn no_local_address_on(run: &Run, source: Source) -> Result<(), CallError> {
    if source.launched_here() || local_addresses_of(run).is_none() {
        return Ok(());
    }
    Err(local_address_refused(
        run,
        LocalAddressFailure::NotLaunchedHere,
    ))
}

/// `local_address_unavailable` for `run`'s `egress.local_addresses`, for
/// `failure`.
fn local_address_refused(run: &Run, failure: LocalAddressFailure) -> CallError {
    Unavailable {
        entry: local_addresses_of(run).unwrap_or_default(),
        failure,
    }
    .into()
}

/// `run`'s `egress.local_addresses` as its value writes it, when it sets
/// one: an interface or a prefix as it is, a list as JSON.
fn local_addresses_of(run: &Run) -> Option<String> {
    let value = run.settings()?.resolved().value(&EGRESS_LOCAL_ADDRESSES);
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        value => Some(value.to_string()),
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

/// The next position of `run`, taken in the table `state` before anything
/// is drawn for it, so a Session opening at the same moment takes the one
/// after: its proxy, its sticky session and its local addresses are drawn
/// for its own (#882, #891), as its Run's resume draws them again. A
/// failed open leaves its position unused. Answers it with the Run's
/// browser pool; `run_not_found` once the Run stopped.
fn taken(state: &mut State, run: &Run) -> Result<(u32, Pool), CallError> {
    let runs = reached(state, run)?;
    runs.last += 1;
    Ok((runs.last, runs.pool.clone()))
}

/// The position of the Run's local-address rotation that the program's
/// Session, or the Run's default one, at `position` binds: they take the
/// rotation's positions in their order, the first the first (ADR 0044's
/// #638 note).
fn rotation_ordinal(position: u32) -> u64 {
    u64::from(position.saturating_sub(1))
}

/// The Sessions the table holds of `run`, which [`Sessions::reach`]
/// reached; `run_not_found` once it stopped since.
fn reached<'state>(
    state: &'state mut State,
    run: &Run,
) -> Result<&'state mut RunSessions, CallError> {
    state.runs.get_mut(run.id()).ok_or_else(|| {
        CallError::new(
            &RUN_NOT_FOUND,
            format!("The Run `{}` stopped while the call ran.", run.id()),
            &RunDetails {
                run_id: run.id().clone(),
                path: None,
            },
        )
    })
}

/// Whether `session` names a Session of a Run that is no longer live but
/// whose committed state records it: the Session ended with its Run.
async fn ended_with_its_run(session: &SessionId, context: &CallContext) -> bool {
    let Some((run_id, position)) = parse_session_id(session) else {
        return false;
    };
    if context.runs().live(run_id.clone()).await.is_ok() {
        return false;
    }
    let Ok(mut reader) = context.runs().reader(run_id).await else {
        return false;
    };
    reader
        .read(move |connection| {
            connection
                .prepare_cached("SELECT count(*) FROM page_sessions WHERE position = ?1")?
                .query_row([position], |row| row.get::<_, i64>(0))
        })
        .await
        .is_ok_and(|recorded| recorded > 0)
}

/// Whether the Run `run_id`'s committed state says it ended `finished`.
async fn finished(run_id: &RunId, context: &CallContext) -> bool {
    let Ok(mut reader) = context.runs().reader(run_id.clone()).await else {
        return false;
    };
    reader
        .read(run_row::lifecycle)
        .await
        .is_ok_and(|lifecycle| lifecycle == Lifecycle::Finished)
}

/// The Run's `timeout.wait`, an app-state wait's deadline.
fn wait_deadline(run: &Run) -> Deadline {
    Deadline::of_resolved(run.settings().map(RunSettings::resolved), &TIMEOUT_WAIT)
}

/// The Run's `timeout.action`, with where it was set.
fn action_timeout(run: &Run) -> (SettingLimit, Duration) {
    let action = Deadline::of_resolved(run.settings().map(RunSettings::resolved), &TIMEOUT_ACTION);
    (action.limit().clone(), action.duration())
}
