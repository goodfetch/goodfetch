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
//! The Run's state keeps each Session it opened, whether it is closed, and
//! (for a fresh Session) its cookies and `localStorage` sealed under the
//! Run's data key, so a start resuming the Run under its ID, in this engine
//! or another, finds its Sessions under the same IDs: the first call that
//! reaches the resumed Run reopens them from its run state, each restoring
//! its kept cookies and `localStorage` into its browser context on its next
//! page (ADRs 0092 and 0102). A Session's ID names its Run, so a call naming
//! a Session of a Run this engine has not reached yet reaches it. When the
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
//! Size: one table, each live Run's Sessions with their opening, closing
//! and reopening from run state, each of which reads and changes that table
//! under its one lock; the index of the Sessions and their pages, from ID
//! to handle, and the pages each Session holds itself (ADR 0146), a page's
//! end and the tables of states are in their own modules.

mod ends;
mod index;
mod life;
mod lifecycle;
mod memory;
pub(crate) mod robots_txt;
mod session_pages;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::egress::CdpUrlOrigin;
use goodfetch_engine::governor::{PageHold, Permit};
use goodfetch_engine::lease::LeaseCheck;
use goodfetch_engine::local_addresses::Unavailable;
use goodfetch_engine::request_signing::{KeyError, RequestSigner};
use goodfetch_engine::response_cache::Caching;
use goodfetch_engine::run::{secret_error, settings_error};
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_key::Sealed;
use goodfetch_engine::run_state::{RunStateError, RunStateTables, SecretColumn, run_row};
use goodfetch_engine::runs::{self, RunError, RunKeyError};
use goodfetch_engine::secrets::Purpose;
use goodfetch_engine::settings::{
    RunSettings, SettingsError, Values, check_session_source, get_or_default, setting,
};
use goodfetch_engine::tasks::{TaskOwner, TaskStarter};
use goodfetch_engine::{CALLER, CallContext, CallError, RunIndex, RunOfCall};
use goodfetch_extractor::BuiltIn;
use goodfetch_identity_profile::IdentitySeed;
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::{Authority, SettingScope};
use goodfetch_protocol::connection::{CLOSED, CancelledBy, ClosedValue};
use goodfetch_protocol::egress::{EGRESS_LOCAL_ADDRESSES, LocalAddressFailure};
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::access_mode::{IDENTIFIED_SIGNATURE_AGENT, IDENTIFIED_SIGNING_KEY};
use goodfetch_protocol::page::proxy::PROXY_PROVIDER;
use goodfetch_protocol::page::session::{NewSessionParams, SessionOpened};
use goodfetch_protocol::page::{
    BROWSER_HEADLESS, CONTENT_EXTRACTOR, CONTENT_EXTRACTORS, PageId, PageOpened, SessionId,
};
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

use self::index::{HeldPages, Index};
use self::life::{Ending, PageLife, SessionLife, session_closed};
pub(crate) use self::lifecycle::PageEvent;
use self::lifecycle::{
    Attributes, Owner, PageEffect, SessionEffect, SessionEvent, SessionState, Source,
};
use crate::access_mode::{
    self, BrowserFamily, Refusal, SessionIdentityPlan, SessionSettings, SigningPlan,
};
use crate::browser_run::BrowserRun;
use crate::cdp::CREATE_TARGET;
use crate::cdp::borrowed::{CdpUrl, resolve_of};
use crate::cdp::driver::{Carried, PageDriver, SessionEgress, Visibility, counted_as_opening};
use crate::cdp::launch::{Launcher, Window};
use crate::cdp::live_view::WatchedTabs;
use crate::consent::Consent;
use crate::consent::gpc::GpcPlan;
use crate::dialogs::Dialogs;
use crate::document_request::{DocumentRequest, Unread};
use crate::egress_relay::{self, EgressRelay};
use crate::emulation::{self, SessionEmulation};
pub(crate) use crate::errors::page_closed;
use crate::errors::{cancelled, invalid_transition};
use crate::fingerprint_method::FingerprintPlan;
use crate::hiding::SessionHiding;
use crate::human_input;
use crate::interception::{HeldBrowser, Interception, Lookups, RunPowers, SessionInterception};
use crate::page_events;
use crate::page_functions::Injections;
use crate::page_owner::{PageOwner, PageOwners};
use crate::page_value::{HandlerPages, HeldPlace, PageValue, PooledSession};
use crate::pool::{BrowserPlacement, InBrowser, LocalFailure, Placement, Pool, Seat};
use crate::proxy::{self, Opening, SessionProxy};
use crate::recovery::Checking;
use crate::trace;

/// What the page crate adds to every run file: each Session the Run opened,
/// by its position among them, whether it is the Run's default Session,
/// whether it is closed, its own settings, as a JSON object by key, its
/// identity seed, its 64 bits as SQLite's signed integer, for one on a
/// Borrowed browser, its `cdp_url` and who gave it (`source`, null for a
/// fresh Session), sealed under the Run's data key ([`SOURCE`]), and, for
/// a fresh Session, its kept cookies and `localStorage` (`state`, null when
/// empty or closed), sealed under the Run's data key ([`SESSION_STATE`]).
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "page-sessions",
    statements: &[
        "CREATE TABLE IF NOT EXISTS page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL, identity_seed INTEGER NOT NULL, source BLOB, state BLOB) STRICT",
    ],
};

/// A Session's `source` is secret: a `cdp_url` names the one browser it
/// reaches and may carry its provider's access token.
const SOURCE: SecretColumn = SecretColumn {
    table: "page_sessions",
    column: "source",
};

/// A Session's `state` is secret: its cookies and `localStorage` may hold
/// login tokens and session identifiers (ADRs 0092, 0102).
const SESSION_STATE: SecretColumn = SecretColumn {
    table: "page_sessions",
    column: "state",
};

/// What separates a Session's Run ID from its position in its ID; a
/// position is digits alone, so the last one splits the ID.
const SESSION_MARK: &str = ".session-";

/// The engine's Sessions and their pages, which every call of every
/// Connection shares. Clones share them.
#[derive(Clone)]
pub(crate) struct Sessions {
    /// Each live Run's table of its Sessions, and the pages that ended.
    state: Arc<Mutex<State>>,
    /// The open Sessions and their pages, by ID, to their Session's handle.
    index: Index,
    pins: Vec<PinnedArtifact>,
    /// How each Run's browsers start.
    launcher: Launcher,
    interception: Interception,
    /// The Consent prompt runner every page starts.
    consent: Consent,
    /// The open dialogs' slots, which each page's reader of its dialogs
    /// opens.
    dialogs: Dialogs,
    /// What each page carries for the Product that hands pages to
    /// handlers: its value and its response observer (ADR 0087).
    handler_pages: Arc<dyn HandlerPages>,
    /// Each open page's owner, by its ID.
    page_owners: PageOwners,
}

/// What the engine holds of its live Runs' Sessions besides its index of
/// them: under its lock, a Session goes into the index while its Run is
/// there (#1763), and a page that ends goes into `crashed` before it
/// leaves its Session.
#[derive(Default)]
struct State {
    runs: BTreeMap<RunId, RunSessions>,
    /// The crashed pages a close released, and those the Resource governor
    /// ended past their memory cap, by their Run, whose later calls raise
    /// `page_crashed` or `page_memory_exceeded` again until their Run stops.
    /// They outlive their Session, so they are their Run's, never their
    /// Session's (ADR 0146).
    crashed: BTreeMap<PageId, (RunId, PageLife)>,
    /// How many Page IDs the engine gave, to the pages it opened and to
    /// the requests its pages serve next (ADR 0146), in one order.
    page_ids_given: u64,
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
    /// Cursory's human-like input shapes loaded once per Run (ADR 0035, 0140).
    human_input: Arc<OnceCell<human_input::Plan>>,
}

/// One Session this engine holds: opening, open, or crashed.
struct Session {
    /// Its owner, a child of its Run's, under which its pages' owners run
    /// (ADR 0146).
    owner: TaskOwner,
    /// Its pages, which it holds until it ends.
    pages: HeldPages,
    run: Arc<BrowserRun>,
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
    /// Domain-bound HTTP authentication credentials loaded once when the
    /// Session opens (ADR 0092, ticket #733).
    http_credentials: Option<Arc<crate::cdp::driver::HttpCredentials>>,
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
    /// How its pages mask their browser's fingerprint, when it is
    /// `stealth` (ADR 0035).
    fingerprint: Option<FingerprintPlan>,
    /// How its pages shape the engine's own gestures when `stealth.human_input`
    /// is on (ADR 0035).
    human_input: Option<human_input::Plan>,
    /// The live hiding its pages share: the clicks' lifted rules and its
    /// pages under live hiding.
    hiding: Arc<SessionHiding>,
    /// Its side of the interception: its routes, its pages' pending
    /// navigations, its lookups and its signer.
    interception: Arc<SessionInterception>,
    /// How many calls listen to its `page` event.
    page_listeners: session_pages::PageListeners,
    /// The engine's Run index, where its pages' consent walls are recorded
    /// for "Needs you".
    run_index: RunIndex,
    /// Whether this Session was opened on a user's Profile directory in place
    /// (ADR 0056; ticket #737): `session.save_identity` refuses it with
    /// `session_not_savable`.
    is_profile_directory: bool,
    /// The browser family this Session runs on (Chromium, Firefox, WebKit).
    browser_family: BrowserFamily,
    /// The cookies and `localStorage` last persisted to `page_sessions.state`,
    /// serialized per Session so concurrent navigations cannot overwrite newer
    /// state with older state.
    saved_state: Arc<Mutex<SavedState>>,
}

/// The `localStorage` accumulated across origins a Session has visited and
/// the [`Carried`] snapshot last written to run state.
#[derive(Default)]
struct SavedState {
    storage: BTreeMap<String, Vec<(String, String)>>,
    last: Carried,
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
    pub(crate) run: Arc<BrowserRun>,
    /// Its Session.
    pub(crate) session: SessionId,
    /// Its own settings, which it takes over its Session's and its Run's.
    pub(crate) settings: Values,
    pub(crate) driver: PageDriver,
    /// Its owner (ADR 0146), which a click's input passes through.
    pub(crate) owner: PageOwner,
    /// Where it is, through its table: each call on it runs through it.
    pub(crate) life: PageLife,
    /// Its Run's `timeout.action` and where it was set.
    pub(crate) action_timeout: (SettingLimit, Duration),
    /// Its Run's clock.
    pub(crate) clock: Clock,
    /// How its Session's pages are judged: its Page check, when it declares
    /// one, and whether it may hold a login.
    pub(crate) page_check: Arc<Checking>,
    /// Its Session's marks of main-world injections in Provenance.
    pub(crate) injections: Injections,
    /// Its human-like input state when its Session is `stealth` with
    /// `stealth.human_input` on (ADR 0035).
    pub(crate) human_input: Option<human_input::PageHumanInput>,
    /// The browser family this page's Session runs on.
    pub(crate) browser_family: BrowserFamily,
    /// Its admission of the governor's `browser_page` kind, given back once
    /// the page leaves and no call holds it.
    _admission: Arc<Permit>,
    /// Its place among its browser's open pages, left at the same moment.
    in_browser: Arc<InBrowser>,
}

impl OpenPage {
    /// Its lease, when a handler's page: the attempt whose commands it
    /// sends for the request it serves now (ADR 0106's window).
    pub(crate) fn lease(&self) -> Option<Arc<dyn LeaseCheck>> {
        self.driver.value().lease().cloned()
    }

    /// Holds the page's browser from being recycled or shed until the
    /// returned hold drops (ADR 0040): used by Human pauses, Live view, and
    /// raw CDP attach.
    pub(crate) fn hold(&self, by: PageHold) -> crate::pool::Hold {
        self.in_browser.hold(by)
    }

    /// Marks sent each body of the page's attempt, or of its Run outside
    /// any attempt, that nothing was sent for yet, before `command`, which
    /// may change the page or a site, goes out (ADR 0106).
    ///
    /// # Errors
    ///
    /// `run_state_unavailable` when the mark could not be written:
    /// `command` must not go out.
    pub(crate) async fn sending(&self, command: &str) -> Result<(), CallError> {
        self.run
            .send_window
            .sending(self.lease().as_deref(), command)
            .await
    }
}

// The Run a raw call names by a page or a Session, whose trace records it
// (ADR 0099).
impl RunOfCall for Sessions {
    fn page_run<'a>(
        &'a self,
        page_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<RunId>> + Send + 'a>> {
        Box::pin(async move {
            let page_id = PageId(page_id.to_owned());
            if let Some(page) = self.index.page(&page_id) {
                return Some(page.run.identity.id().clone());
            }
            self.state
                .lock()
                .await
                .crashed
                .get(&page_id)
                .map(|(run_id, _)| run_id.clone())
        })
    }

    fn session_run(&self, session_id: &str) -> Option<RunId> {
        parse_session_id(&SessionId(session_id.to_owned())).map(|(run_id, _)| run_id)
    }
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

/// The identity and the powers of `run` its pages' events take (ADR 0145).
pub(crate) fn events_run(run: &BrowserRun) -> page_events::RunPowers {
    page_events::RunPowers {
        identity: run.identity.clone(),
        files: run.files.clone(),
        secrets: run.keys.secrets().clone(),
        trace: run.trace.clone(),
    }
}

/// The identity and the powers of `run` its pages' interception takes
/// (ADR 0145).
pub(crate) fn interception_run(run: &BrowserRun) -> RunPowers {
    RunPowers::new(
        run.identity.clone(),
        run.state.clone(),
        run.trace.clone(),
        run.warnings.clone(),
        run.keys.secrets().clone(),
        run.governor.clone(),
    )
}

/// The identity and the powers of `run` a Session's egress relay takes
/// (ADR 0145), its tasks starting on `tasks`, the Session's owner's
/// (ADR 0146).
fn relay_run(run: &BrowserRun, tasks: &TaskStarter) -> egress_relay::RunPowers {
    egress_relay::RunPowers {
        identity: run.identity.starting_on(tasks),
        trace: run.trace.clone(),
        refusals: run.engine.refusals(),
    }
}

/// The identity and the powers of `run` a Session's proxy takes (ADR
/// 0145), its tasks starting on `tasks`, the Session's owner's (ADR 0146).
fn proxy_run(run: &BrowserRun, tasks: &TaskStarter) -> proxy::RunPowers {
    proxy::RunPowers {
        identity: run.identity.starting_on(tasks),
        secrets: run.keys.secrets().clone(),
        politeness: run.politeness.clone(),
        charging: run.charging.clone(),
        refusals: run.engine.refusals(),
    }
}

/// The failure of a read or write of `run`'s state.
pub(crate) fn unavailable(
    run: &BrowserRun,
    error: &impl std::fmt::Display,
    context: &CallContext,
) -> CallError {
    goodfetch_engine::run::run_error(
        RunError::Unavailable {
            run_id: run.identity.id().clone(),
            folder: run.identity.folder().to_owned(),
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
            .map_err(|ended| lease.withheld(ended, CREATE_TARGET)),
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
/// included, so no tab stays open in the Session's browser. It is a child
/// of its Session's owner, which reads its events and runs its close when
/// it is dropped: nothing of it runs on the Run.
pub(crate) struct EnginePage {
    driver: PageDriver,
    /// Its place in its browser, until it closes.
    in_browser: Option<InBrowser>,
    /// Its Session's owner, which its close runs on when it is dropped.
    owner: TaskStarter,
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
            self.owner.spawn(async move {
                driver.close().await;
                drop(in_browser);
            });
        }
    }
}

impl Sessions {
    /// The engine's Sessions, whose browsers `launcher` starts from `pins`,
    /// under `interception`, their pages' open dialogs in `dialogs`, each page's
    /// value and response observer from `handler_pages`.
    pub(crate) fn new(
        (pins, launcher): (Vec<PinnedArtifact>, Launcher),
        interception: Interception,
        dialogs: Dialogs,
        handler_pages: Arc<dyn HandlerPages>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            index: Index::default(),
            pins,
            launcher,
            interception,
            consent: Consent::default(),
            dialogs,
            handler_pages,
            page_owners: PageOwners::default(),
        }
    }

    /// Its open Sessions' tabs, as the Live view watches them.
    pub(crate) fn watched(&self) -> WatchedTabs {
        self.index.watched()
    }

    /// Each open page's owner, by its ID, which a call that acts as it
    /// arrives reads.
    pub(crate) fn page_owners(&self) -> &PageOwners {
        &self.page_owners
    }

    /// The request a Session's browser sends for a document, read from
    /// the build it would launch, pinned or installed, for `run`.
    pub(crate) async fn document_request(
        &self,
        run: &RunIdentity,
    ) -> Result<Option<DocumentRequest>, Unread> {
        DocumentRequest::of_browser(&self.pins, run).await
    }

    /// Opens a Session of the live Run `params` names, from its own source,
    /// else the Run's, under its own settings: its browser context is made
    /// now, launching the Run's browser when no Session has.
    #[expect(
        clippy::too_many_lines,
        reason = "opens a Session across settings, access mode, emulation, egress, proxy, relay, and pool seating"
    )]
    pub(crate) async fn new_session(
        &self,
        params: NewSessionParams,
        context: &CallContext,
    ) -> Result<SessionOpened, CallError> {
        let run = live(&params.run_id, context).await?;
        let settings = own_settings(&params.settings, &run, context)?;
        let own_source = params.source.is_some();
        let named = named_source(&run, params.source, context);
        // A Session's own source meets the rule `session.source` meets in
        // every layer (ADR 0056), in the layer `named_source` gives it; the
        // Run's met it when the Run started.
        if own_source && let Some(named) = &named {
            check_session_source(&named.source, Some(named.layer))
                .map_err(|error| settings_error(&error, context))?;
        }
        let settings = extractor_installed(settings, &run, context).await?;
        let browser_family = BrowserFamily::of(named.as_ref().map(|n| &n.source));
        let (identity, emulation) =
            session_plans(&settings, &run.identity, named.as_ref(), context)?;
        let seed = IdentitySeed(run.entropy.fork().await.next_u64());
        let fingerprint = fingerprint_of(&run, &settings, &identity, seed, context).await?;
        let (opened, gpc) = opening_source_and_gpc(&settings, &run, named, context).await?;
        let borrowed = opened.borrowed();
        let carried = opened.carried();
        no_local_address_on(&run, source_of(borrowed))?;
        let signing = open_signing(&run, identity.signing.as_ref(), context).await?;
        self.reach(&run, context).await?;
        let (position, pool, human_cell) = taken(&mut *self.state.lock().await, &run)?;
        let human_input = human_input_of(&human_cell, &run, &identity, context).await;
        let owner = run.identity.owner().child();
        let proxy = open_proxy((&run, &owner), &settings, context, position).await?;
        no_proxy_under(borrowed, proxy.as_deref())?;
        let relay = open_relay(
            (&run, &owner),
            proxy.as_ref(),
            borrowed.is_some(),
            &pool,
            rotation_ordinal(position),
            context,
        )
        .await?;
        let http_credentials = http_credentials_of(&run, &settings);
        let attributes = if borrowed.is_some() { BORROWED } else { FRESH };
        // It goes into the table under the lock, while its Run is there:
        // once the Run stops, its clean-up has taken the Run out, and a
        // Session put in after it would hold the Run forever (#1763).
        let (session, life, open) = {
            let mut state = self.state.lock().await;
            let runs = reached(&mut state, &run.identity)?;
            let seat = open_seat(
                &runs.pool,
                borrowed,
                carried,
                placement(&run, fingerprint.as_ref(), gpc, opened.profile()),
                egress(proxy.as_ref(), relay.as_ref(), http_credentials.as_ref()),
                emulation,
                TaskStarter::clone(&owner),
            );
            let session = session_id(run.identity.id(), position);
            let life = SessionLife::new(session.clone(), attributes, SessionState::Opening);
            let interception = session_interception(
                (&session, &run),
                borrowed,
                (relay.is_some(), proxy.is_some()),
                signing,
                gpc.header_only(),
                identity.mode == crate::access_mode::AccessMode::Identified,
                settings.clone(),
            );
            let saved_state = carried.map_or_else(Arc::default, |carried| {
                Arc::new(Mutex::new(SavedState {
                    storage: carried.storage_map(),
                    last: carried.clone(),
                }))
            });
            let open = Arc::new(Session {
                owner,
                pages: HeldPages::default(),
                run: Arc::clone(&run),
                position,
                seat,
                settings: settings.clone(),
                life: life.clone(),
                proxy,
                egress_marked: OnceCell::new(),
                relay,
                http_credentials,
                reopen_failed: None,
                held_frames: borrowed.map(|_| session_pages::HeldFrames::default()),
                injections: Injections::default(),
                fingerprint,
                human_input,
                hiding: Arc::default(),
                interception,
                page_listeners: session_pages::PageListeners::default(),
                run_index: context.engine().run_index().clone(),
                is_profile_directory: opened.profile().is_some(),
                browser_family,
                saved_state,
            });
            self.index.opened(&session, Arc::clone(&open));
            self.watch_session(session.clone(), &open);
            (session, life, open)
        };
        self.seat_and_record(
            (&run, &open),
            (&session, &life),
            (SessionRow::opened(position, seed), &settings, &opened, gpc),
            context,
        )
        .await
    }

    /// Opens the browser context of `open`, transitions its lifecycle to
    /// `Open`, writes its run-state record, and emits its opening marks and
    /// warnings.
    async fn seat_and_record(
        &self,
        (run, open): (&Arc<BrowserRun>, &Arc<Session>),
        (session, life): (&SessionId, &SessionLife),
        (row, settings, opened, gpc): (SessionRow, &Values, &OpenedSource, GpcPlan),
        context: &CallContext,
    ) -> Result<SessionOpened, CallError> {
        if let Err(error) = open.seat.open().await {
            life.open_failed(error.clone())?;
            self.index.ended(session, open);
            open.interception.close();
            open.owner.stop().await;
            return Err(error);
        }
        match life.apply(SessionEvent::SourceReady, None)? {
            SessionEffect::Opened => {}
            // A `close()` while it opened abandoned the open (S3).
            _ => return Err(cancelled(CancelledBy::Program, ClosedValue::Session)),
        }
        self.record(run, row, settings, opened, context).await?;
        mark_opened_session(run, open, session).await;
        warn_gpc(run, session, gpc).await;
        Ok(SessionOpened {
            session_id: session.clone(),
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
        let _ = page_settings(settings, &run, context)?;
        let named = named_source(&run, None, context);
        let (identity, emulation) =
            session_plans(&Values::default(), &run.identity, named.as_ref(), context)?;
        let opened = opening_source(&run, named, context).await?;
        self.reach(&run, context).await?;
        let default = Arc::clone(&reached(&mut *self.state.lock().await, &run.identity)?.default);
        let position = *default
            .get_or_try_init(|| {
                self.open_held(
                    &run,
                    &opened,
                    (&identity, &emulation),
                    Opener::Default,
                    context,
                )
            })
            .await?;
        if reached(&mut *self.state.lock().await, &run.identity)?
            .closed
            .contains(&position)
        {
            return Err(CallError::new(
                &CLOSED,
                format!(
                    "the default Session of the Run `{}` is closed: open a Session of its own with `run.new_session`",
                    run.identity.id()
                ),
                &json!({ "session_id": session_id(run.identity.id(), position) }),
            ));
        }
        let session = session_id(run.identity.id(), position);
        Box::pin(self.new_page(&session, settings, &HeldPlace::default(), context)).await
    }

    /// Opens the default Session of `run`, or a Session its crawl's Session
    /// pool holds, as `opener` says, on the Borrowed browser at `borrowed`
    /// if there is one, with the `identity` it opens with: signing as it
    /// plans when it is `identified`, masking its fingerprint when it is
    /// `stealth`; and answers its position: its signing key is resolved,
    /// and its proxy and the default Session's record in run state are
    /// opened and written, outside the table's lock.
    #[expect(
        clippy::too_many_lines,
        reason = "opens a held default or pool Session across identity, egress, proxy, relay, and pool seating"
    )]
    async fn open_held(
        &self,
        run: &Arc<BrowserRun>,
        source: &OpenedSource,
        (identity, emulation): (&SessionIdentityPlan, &SessionEmulation),
        opener: Opener,
        context: &CallContext,
    ) -> Result<u32, CallError> {
        let borrowed = source.borrowed();
        let carried = source.carried();
        no_local_address_on(run, source_of(borrowed))?;
        let signing = open_signing(run, identity.signing.as_ref(), context).await?;
        let seed = IdentitySeed(run.entropy.fork().await.next_u64());
        let fingerprint = fingerprint_of(run, &Values::default(), identity, seed, context).await?;
        let gpc = GpcPlan::of(
            &Values::default(),
            run.identity.settings().map(RunSettings::resolved),
            None,
            borrowed.is_some(),
        );
        let (position, pool, human_cell) = taken(&mut *self.state.lock().await, run)?;
        let human_input = human_input_of(&human_cell, run, identity, context).await;
        let owner = run.identity.owner().child();
        let proxy = open_proxy((run, &owner), &Values::default(), context, position).await?;
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
            (run, &owner),
            proxy.as_ref(),
            borrowed.is_some(),
            &pool,
            address_ordinal,
            context,
        )
        .await?;
        let http_credentials = http_credentials_of(run, &Values::default());
        let seat = open_seat(
            &pool,
            borrowed,
            carried,
            placement(run, fingerprint.as_ref(), gpc, source.profile()),
            egress(proxy.as_ref(), relay.as_ref(), http_credentials.as_ref()),
            emulation.clone(),
            TaskStarter::clone(&owner),
        );
        if let Opener::Default = opener {
            self.record(
                run,
                SessionRow::the_default(position, seed),
                &Values::default(),
                source,
                context,
            )
            .await?;
        }
        let attributes = match (opener, borrowed.is_some()) {
            (Opener::Default, false) => FRESH,
            (Opener::Default, true) => BORROWED,
            (Opener::Pool { .. }, false) => POOL,
            (Opener::Pool { .. }, true) => POOL_BORROWED,
        };
        let session = session_id(run.identity.id(), position);
        // The Run may have stopped while its record was written: a Session
        // goes into the table only while its Run is there, under the lock
        // its clean-up takes it out under (#1763).
        let interception = session_interception(
            (&session, run),
            borrowed,
            (relay.is_some(), proxy.is_some()),
            signing,
            gpc.header_only(),
            identity.mode == crate::access_mode::AccessMode::Identified,
            Values::default(),
        );
        let held = Arc::new(Session {
            owner,
            pages: HeldPages::default(),
            run: Arc::clone(run),
            position,
            seat,
            settings: Values::default(),
            life: SessionLife::new(session.clone(), attributes, SessionState::Open),
            proxy,
            egress_marked: OnceCell::new(),
            relay,
            http_credentials,
            reopen_failed: None,
            held_frames: borrowed.map(|_| session_pages::HeldFrames::default()),
            injections: Injections::default(),
            fingerprint,
            human_input,
            hiding: Arc::default(),
            interception,
            page_listeners: session_pages::PageListeners::default(),
            run_index: context.engine().run_index().clone(),
            is_profile_directory: source.profile().is_some(),
            browser_family: BrowserFamily::Chromium,
            saved_state: Arc::default(),
        });
        {
            let mut state = self.state.lock().await;
            reached(&mut state, &run.identity)?;
            self.index.opened(&session, Arc::clone(&held));
            self.watch_session(session.clone(), &held);
        }
        if let Some(fingerprint) = &held.fingerprint {
            fingerprint.chosen(run, &session).await;
        }
        warn_gpc(run, &session, gpc).await;
        if let Some(human_input) = &held.human_input {
            human_input.chosen(run, &session).await;
        }
        Ok(position)
    }

    /// Opens a page in the Session `session`, once the governor admits one
    /// more browser page, or in the place `held` if it holds one; `closed`
    /// when the Session is closed.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(session_id = %session.0, page_id = tracing::field::Empty))]
    pub(crate) async fn new_page(
        &self,
        session: &SessionId,
        settings: &Map<String, Value>,
        held: &HeldPlace,
        context: &CallContext,
    ) -> Result<PageOpened, CallError> {
        let open = self.session(session, context).await?;
        if let Some(failed) = &open.reopen_failed {
            return Err(failed.clone());
        }
        let settings = page_settings(settings, &open.run, context)?;
        let settings = extractor_installed(settings, &open.run, context).await?;
        open.life.call().await?;
        let run = Arc::clone(&open.run);
        if let Some(proxy) = &open.proxy {
            proxy.warn_if_outlived(&run.warnings, session).await;
        }
        let value = self.handler_pages.value(run.identity.id(), session);
        lease_holds(&value)?;
        let admission = match held.take() {
            Some(held) => held,
            None => run.governor.share().admit(WorkKind::BrowserPage).await,
        };
        // A Session closed while the page waited, or whose browser went
        // under it, raises what its table says now (#697, S8, S9).
        let (driver, in_browser) = match open.seat.new_page(Visibility::Visible).await {
            Ok(opened) => opened,
            Err(error) => return Err(open.life.settled(error).await),
        };
        let driver = driver
            .with_value(value, run.identity.clock())
            .with_reader(self.handler_pages.reader(run.identity.id(), session));
        let open_session = Arc::clone(&open);
        // Its set-up is the rest of its opening (ADR 0140's C7).
        let page = counted_as_opening(
            run.identity.metrics().counted_work(),
            self.adopt(session_pages::Adopting {
                observer: self.handler_pages.observer(run.identity.id(), session),
                settings,
                open,
                driver,
                in_browser,
                admission,
                events: context.events().clone(),
            }),
        )
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
    /// Saves a Session's login as a named Saved identity (`session.save_identity`,
    /// ADR 0092): refuses a Profile directory used in place with
    /// `session_not_savable` naming `state_file` (ticket #737).
    pub(crate) fn save_identity(
        &self,
        session_id: &SessionId,
        _name: &str,
        _context: &CallContext,
    ) -> Result<goodfetch_protocol::page::session::IdentitySaved, CallError> {
        let open = self
            .index
            .session(session_id)
            .ok_or_else(|| self.closed_session(session_id))?;
        if open.is_profile_directory {
            return Err(CallError::new(
                &goodfetch_protocol::page::session::SESSION_NOT_SAVABLE,
                "A Session opened on a Profile directory in place cannot be saved directly as a Saved identity; export or import via `state_file` instead (ADR 0092).",
                &goodfetch_protocol::page::session::SessionNotSavableDetails {
                    source: "profile_directory".to_owned(),
                    alternative: "state_file".to_owned(),
                },
            ));
        }
        Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            "Saved identities are not built yet; use `state_file` for a run-only import.",
            &CapabilityUnavailableDetails {
                key: "session.save_identity".to_owned(),
                layer: SettingsLayer::Run,
                kind: "saved_identity".to_owned(),
                fix: "`state_file`".to_owned(),
            },
        ))
    }

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
        Box::pin(self.new_page(&session, &Map::new(), &pooled.place, context)).await
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
        let (identity, emulation) =
            session_plans(&Values::default(), &run.identity, named.as_ref(), context)?;
        let source = opening_source(&run, named, context).await?;
        self.reach(&run, context).await?;
        let cell = Arc::clone(
            reached(&mut *self.state.lock().await, &run.identity)?
                .pooled
                .entry(pooled.id.clone())
                .or_default(),
        );
        let opener = Opener::Pool {
            address_ordinal: pooled.address_ordinal,
        };
        let position = *cell
            .get_or_try_init(|| {
                self.open_held(&run, &source, (&identity, &emulation), opener, context)
            })
            .await?;
        Ok(session_id(run.identity.id(), position))
    }

    /// What a call on the Session `session`, which the index no longer
    /// names, raises: the `engine_panicked` of the panic that closed it,
    /// until its Run ends, or else `closed`.
    fn closed_session(&self, session: &SessionId) -> CallError {
        self.index
            .panic_of(session)
            .unwrap_or_else(|| session_closed(session))
    }

    /// The Run of the open Session `session` and its own settings, which
    /// its pages take over the Run's; `closed` once it closed.
    pub(crate) fn own_settings(
        &self,
        session: &SessionId,
    ) -> Result<(Arc<BrowserRun>, Values), CallError> {
        self.index
            .session(session)
            .map(|open| (Arc::clone(&open.run), open.settings.clone()))
            .ok_or_else(|| self.closed_session(session))
    }

    /// The side of the interception the open Session `session` holds.
    ///
    /// # Errors
    ///
    /// `closed` when the Session is closed, or its Run ended.
    pub(crate) fn interception_of(
        &self,
        session: &SessionId,
    ) -> Result<Arc<SessionInterception>, CallError> {
        self.index
            .session(session)
            .map(|open| Arc::clone(&open.interception))
            .ok_or_else(|| self.closed_session(session))
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
        let Some(open) = self.index.session(&session) else {
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
        self.end_session_with(open, session, effect, &Ending::Closed(ClosedValue::Session))
            .await?
            .map_err(|error| unavailable(&open.run, &error, context))
    }

    /// Ends the Session `open`, whose table answered `effect` to its
    /// `Close`, its `Retire` or a panic of one of its owner's tasks: its
    /// pages end by `ending`, then its browser context closes. Answers
    /// whether its run state recorded its end.
    async fn end_session_with(
        &self,
        open: &Arc<Session>,
        session: &SessionId,
        effect: SessionEffect,
        ending: &Ending,
    ) -> Result<Result<(), RunStateError>, CallError> {
        if let Some(proxy) = &open.proxy {
            proxy.warn_if_outlived(&open.run.warnings, session).await;
        }
        let crashed = match effect {
            // Already closing or closed: `close()` changes nothing (C).
            SessionEffect::Nothing => return Ok(Ok(())),
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
        for (_, page) in open.pages.all() {
            match page.life.end(ending)? {
                PageEffect::Close => closing.push(page),
                effect => self.page_ended(&open.run, &page.life, effect).await,
            }
        }
        open.seat.close().await;
        for page in &closing {
            // The context's disposal destroyed each target: the close the
            // engine sent is confirmed (P29).
            let effect = page.life.end(&Ending::Destroyed)?;
            self.page_ended(&open.run, &page.life, effect).await;
        }
        if crashed {
            open.owner.stop().await;
            forget_cached(open, session).await;
            return Ok(Ok(()));
        }
        open.life.apply(SessionEvent::ChildrenClosed, None)?;
        let ended = self.session_ended(open, session).await;
        // Its owner stops last, waiting for every task it and its pages
        // started (ADR 0146).
        open.owner.stop().await;
        forget_cached(open, session).await;
        Ok(ended)
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
        {
            let mut state = self.state.lock().await;
            if let Some(runs) = state.runs.get_mut(open.run.identity.id()) {
                runs.closed.insert(open.position);
            }
            self.index.ended(session, open);
        }
        // Its pages go once the index no longer names it: a page kept
        // meanwhile is among them.
        self.page_owners.close(&open.pages.end());
        open.interception.close();
        if open.life.pooled() {
            return Ok(());
        }
        let position = open.position;
        open.run
            .state
            .write(move |connection| {
                connection
                    .prepare_cached(
                        "UPDATE page_sessions SET closed = 1, state = NULL WHERE position = ?1",
                    )?
                    .execute([position])
                    .map(drop)
            })
            .await
    }

    /// Captures the cookies and current page's `localStorage` of `page`'s
    /// Session and keeps them sealed in `page_sessions.state` when they
    /// changed (ADRs 0092, 0102). A no-op for closed, pooled or Borrowed
    /// Sessions; errors are logged and never fail the caller's operation.
    pub(crate) async fn persist_page_state(&self, page: &OpenPage) {
        let Some(open) = self.index.session(&page.session) else {
            return;
        };
        if open.life.pooled() || open.held_frames.is_some() || !open.life.serves() {
            return;
        }
        let mut saved = open.saved_state.lock().await;
        if !open.life.serves() {
            return;
        }
        let carried = match page.driver.capture_page_state(&mut saved.storage).await {
            Ok(carried) => carried,
            Err(error) => {
                tracing::debug!(%error, "could not capture Session cookies and storage");
                return;
            }
        };
        if carried == saved.last {
            return;
        }
        let sealed = if carried.is_empty() {
            None
        } else {
            let bytes = match carried.to_bytes() {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::debug!(%error, "could not encode Session state");
                    return;
                }
            };
            match open
                .run
                .keys
                .seal(&SESSION_STATE, &open.position.to_string(), &bytes)
                .await
            {
                Ok(sealed) => Some(sealed),
                Err(error) => {
                    tracing::debug!(%error, "could not seal Session state");
                    return;
                }
            }
        };
        let position = open.position;
        let written = open
            .run
            .state
            .write(move |connection| {
                connection
                    .prepare_cached(
                        "UPDATE page_sessions SET state = ?1 WHERE position = ?2 AND closed = 0",
                    )?
                    .execute(rusqlite::params![sealed, position])
                    .map(drop)
            })
            .await;
        match written {
            Ok(()) => saved.last = carried,
            Err(error) => tracing::debug!(%error, "could not write Session state"),
        }
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
        let open = self
            .index
            .session(session)
            .ok_or_else(|| self.closed_session(session))?;
        let races = match work {
            EngineWork::ArrivingDocument => true,
            EngineWork::Request => {
                open.interception.navigation_pending()
                    || open.pages.find(|page| page.life.navigating()).is_some()
            }
        };
        if let Some(failed) = &open.reopen_failed {
            return Err(failed.clone());
        }
        open.life.call().await?;
        lease_holds(value)?;
        let (driver, in_browser) = open.seat.new_engine_page(races).await?;
        Ok(EnginePage {
            driver: driver.with_value(value.clone(), open.run.identity.clock()),
            in_browser: Some(in_browser),
            owner: TaskStarter::clone(&open.owner),
        })
    }

    /// Whether the open Session `session` serves the engine's own page for
    /// a page carrying `value` now, with no wait: it is open and serves
    /// calls, and `value`'s lease holds. A fetch from its engine's page that
    /// failed while it no longer did failed in the engine, not at the site.
    pub(crate) fn serves(&self, session: &SessionId, value: &PageValue) -> bool {
        let serving = self
            .index
            .session(session)
            .is_some_and(|open| open.reopen_failed.is_none() && open.life.serves());
        serving && lease_holds(value).is_ok()
    }

    /// The page `page_id` this engine holds; `page_crashed` again for a
    /// crashed one a close released, else `closed`.
    pub(crate) async fn page(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        if let Some(page) = self.index.page(page_id) {
            return Ok(page);
        }
        // A page that ends goes into the ended pages before it leaves its
        // Session, so one no longer held is found there.
        Err(self
            .state
            .lock()
            .await
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
        if let Some(open) = self.index.session(session) {
            return Ok(open);
        }
        let Some((run_id, _)) = parse_session_id(session) else {
            return Err(session_closed(session));
        };
        let Ok(run) = context.runs().live(run_id).await else {
            return Err(session_closed(session));
        };
        let run = BrowserRun::of(&run, context.engine());
        self.reach(&run, context).await?;
        self.index
            .session(session)
            .ok_or_else(|| self.closed_session(session))
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
    async fn reach(&self, run: &Arc<BrowserRun>, context: &CallContext) -> Result<(), CallError> {
        if self.state.lock().await.runs.contains_key(run.identity.id()) {
            return Ok(());
        }
        let recorded = recorded_sessions(run, context).await?;
        let (pool, losses) = self.new_pool(run, context);
        let mut runs = RunSessions {
            pool: pool.clone(),
            last: 0,
            default: Arc::new(OnceCell::new()),
            closed: BTreeSet::new(),
            pooled: BTreeMap::new(),
            human_input: Arc::new(OnceCell::new()),
        };
        let mut reopened = Vec::new();
        for entry in recorded {
            runs.last = runs.last.max(entry.position);
            if entry.is_default {
                runs.default = Arc::new(OnceCell::new_with(Some(entry.position)));
            }
            if entry.closed {
                runs.closed.insert(entry.position);
                continue;
            }
            let settings = recorded_settings(&entry.settings, run, context)?;
            reopened.push(
                reopened_session(
                    (run, &pool, &runs.human_input),
                    entry.position,
                    entry.seed,
                    settings,
                    entry.source,
                    entry.state,
                    context,
                )
                .await?,
            );
        }
        let mut state = self.state.lock().await;
        if state.runs.contains_key(run.identity.id()) {
            return Ok(());
        }
        for open in reopened {
            let session = session_id(run.identity.id(), open.position);
            let open = Arc::new(open);
            self.index.opened(&session, Arc::clone(&open));
            self.watch_session(session, &open);
        }
        state.runs.insert(run.identity.id().clone(), runs);
        drop(state);
        self.follow_losses(&run.identity, losses);
        self.leave_with(run);
        self.end_pages_past_their_cap(run);
        Ok(())
    }

    /// A browser pool of its own for `run`, whose browsers the engine's
    /// launcher starts from its pins, and the losses of its browsers.
    fn new_pool(
        &self,
        run: &Arc<BrowserRun>,
        context: &CallContext,
    ) -> (Pool, tokio::sync::mpsc::Receiver<crate::pool::BrowserLoss>) {
        Pool::new(
            Arc::clone(run),
            self.launcher.clone(),
            context.engine().home().clone(),
            Pins {
                pinned_by: PinnedBy {
                    product: context.identity().product,
                    product_version: context.identity().product_version.clone(),
                },
                artifacts: self.pins.clone(),
            },
        )
    }

    /// Takes the Run's Sessions and pages out once it stops, warning the
    /// Run of each Session that outlived its proxy's sticky lifetime, then
    /// stops its browsers' owners.
    fn leave_with(&self, run: &Arc<BrowserRun>) {
        let sessions = self.clone();
        let stopping = run.identity.owner().child_token();
        let run_id = run.identity.id().clone();
        run.identity.spawn(async move {
            stopping.cancelled().await;
            let (runs, ended) = {
                let mut state = sessions.state.lock().await;
                let runs = state.runs.remove(&run_id);
                state.crashed.retain(|_, (run, _)| run != &run_id);
                (runs, sessions.index.run_ended(&run_id))
            };
            // Each Session ends with its Run: one that outlived its proxy's
            // sticky lifetime is named in the Run report, which is written
            // once the Run's tasks, this one among them, have ended (ADR
            // 0021's #882 note).
            for (session, open) in &ended {
                if let Some(proxy) = &open.proxy {
                    proxy.warn_if_outlived(&open.run.warnings, session).await;
                }
            }
            // Their pages go once the index no longer names them: a page
            // kept meanwhile is among them.
            for (_, open) in &ended {
                sessions.page_owners.close(&open.pages.end());
                open.interception.close();
            }
            // Their owners, cancelled with the Run's, are waited for here,
            // once their pages' parts have gone: a stop of the Run waits for
            // this task, never for a child owner's.
            for (_, open) in &ended {
                open.owner.stop().await;
            }
            drop(ended);
            // The browsers' owners, beside the Sessions' (ADR 0146), stop
            // last: each launched browser is gone, each Borrowed one's
            // connection ended, before the Run report is written.
            if let Some(runs) = runs {
                runs.pool.stop().await;
            }
        });
    }

    /// Records the Session `row` of `run`, with its own `settings` and the
    /// `cdp_url` of the Borrowed browser it is on, `borrowed`, if it is, in
    /// its run state.
    async fn record(
        &self,
        run: &Arc<BrowserRun>,
        SessionRow {
            position,
            is_default,
            seed,
        }: SessionRow,
        settings: &Values,
        opened: &OpenedSource,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let source = match opened.borrowed() {
            Some(cdp_url) => {
                let source = json!({
                    "url": cdp_url.url,
                    "layer": cdp_url.origin.layer,
                    "acting_for_model": cdp_url.origin.acting_for_model,
                })
                .to_string();
                let sealed = run
                    .keys
                    .seal(&SOURCE, &position.to_string(), source.as_bytes())
                    .await
                    .map_err(|error| no_source(run, error, context))?;
                Some(sealed)
            }
            None => None,
        };
        let sealed_state = match opened.carried().filter(|carried| !carried.is_empty()) {
            Some(carried) => {
                let bytes = carried.to_bytes().map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("could not encode imported Session state: {error}"),
                    )
                })?;
                let sealed = run
                    .keys
                    .seal(&SESSION_STATE, &position.to_string(), &bytes)
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
        let seed = i64::from_le_bytes(seed.0.to_le_bytes());
        run.state
            .write(move |connection| {
                connection
                    .prepare_cached(
                        "INSERT INTO page_sessions \
                         (position, is_default, closed, settings, identity_seed, source, state) \
                         VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6)",
                    )?
                    .execute(rusqlite::params![
                        position,
                        is_default,
                        settings,
                        seed,
                        source,
                        sealed_state
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
    run: &Arc<BrowserRun>,
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
    run: &Arc<BrowserRun>,
    scope: &str,
    context: &CallContext,
) -> Result<Values, CallError> {
    let locked = values.keys().find_map(|key| {
        let effective = run
            .identity
            .settings()?
            .resolved()
            .effective(setting(key)?)?;
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
    run: &Arc<BrowserRun>,
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

/// `values`, a Session's or a page's own settings, unless their
/// `content.extractor` names no built-in and no installed Plugin of the
/// Extractor kind: then `setting_invalid` (spec #596). Nothing starts
/// here: the Plugin starts as a page first reads with it.
async fn extractor_installed(
    values: Values,
    run: &Arc<BrowserRun>,
    context: &CallContext,
) -> Result<Values, CallError> {
    let Some(name) = values
        .get(CONTENT_EXTRACTOR.name)
        .and_then(Value::as_str)
        .filter(|name| BuiltIn::named(name).is_none())
        .map(str::to_owned)
    else {
        return Ok(values);
    };
    match run.plugins.check_extractor(&name).await {
        Ok(()) => Ok(values),
        Err(error) => Err(settings_error(
            &SettingsError::Invalid {
                key: CONTENT_EXTRACTOR.name.to_owned(),
                layer: Some(SettingsLayer::Run),
                reason: format!(
                    "{error}: name a built-in, {}, or an installed Extractor Plugin",
                    CONTENT_EXTRACTORS.join(", ")
                ),
            },
            context,
        )),
    }
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
/// authority, or the Run's. Its tasks are `tasks`', the Session's owner's.
async fn open_proxy(
    (run, tasks): (&Arc<BrowserRun>, &TaskStarter),
    own: &Values,
    context: &CallContext,
    position: u32,
) -> Result<Option<Arc<SessionProxy>>, CallError> {
    let Some(settings) = run.identity.settings() else {
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
        runs::usage(&run.charging, &run.state)
            .await
            .map_err(|error| unavailable(run, &error, context))?
            .limits
            .into_iter()
            .filter(|limit| limit.usage_kind == PROXY_BYTES)
            .filter_map(|limit| limit.currency)
            .collect()
    };
    let session = session_id(run.identity.id(), position);
    let opening = Opening {
        session: &session,
        ordinal: u64::from(position),
        currencies: &currencies,
    };
    Ok(
        SessionProxy::open(&resolved, &proxy_run(run, tasks), &opening)
            .await?
            .map(Arc::new),
    )
}

/// The failure a call answers when the data key of `run` neither seals nor
/// opens a Session's source: the key-encryption key's own refusal, or the
/// Run's state as unavailable.
fn no_source(run: &Arc<BrowserRun>, error: RunKeyError, context: &CallContext) -> CallError {
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
    run: &Arc<BrowserRun>,
    context: &CallContext,
) -> Result<CdpUrl, CallError> {
    let source = run
        .keys
        .open(&SOURCE, &position.to_string(), sealed)
        .await
        .map_err(|error| no_source(run, error, context))?;
    recorded_source(&source, run, context)
}

/// The `cdp_url` `recorded`, as the run state of `run` holds it once
/// opened, of a Session on a Borrowed browser reopened from it.
fn recorded_source(
    recorded: &[u8],
    run: &Arc<BrowserRun>,
    context: &CallContext,
) -> Result<CdpUrl, CallError> {
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

/// The kept cookies and `localStorage` of the Session at `position` of `run`,
/// opened from its sealed `state` in run state.
async fn opened_state(
    sealed: Option<Sealed>,
    position: u32,
    run: &Arc<BrowserRun>,
    context: &CallContext,
) -> Result<Carried, CallError> {
    let Some(sealed) = sealed else {
        return Ok(Carried::default());
    };
    let bytes = run
        .keys
        .open(&SESSION_STATE, &position.to_string(), &sealed)
        .await
        .map_err(|error| no_source(run, error, context))?;
    Carried::from_bytes(&bytes).map_err(|error| unavailable(run, &error, context))
}

/// What the run state records of a Session as it opens: its position,
/// whether it is the default Session, and the seed its Identity profile is
/// drawn from.
#[derive(Clone, Copy, Debug)]
struct SessionRow {
    position: u32,
    is_default: bool,
    seed: IdentitySeed,
}

impl SessionRow {
    /// A Session a client opens, at `position`.
    const fn opened(position: u32, seed: IdentitySeed) -> Self {
        Self {
            position,
            is_default: false,
            seed,
        }
    }

    /// The Run's default Session, at `position`.
    const fn the_default(position: u32, seed: IdentitySeed) -> Self {
        Self {
            position,
            is_default: true,
            seed,
        }
    }
}

/// A Session as the run state records it: its position, whether it is the
/// default Session, whether it closed, its own settings, its identity seed,
/// for one on a Borrowed browser its sealed source, and for a fresh Session
/// its sealed cookies and `localStorage`.
struct RecordedSession {
    position: u32,
    is_default: bool,
    closed: bool,
    settings: String,
    seed: IdentitySeed,
    source: Option<Sealed>,
    state: Option<Sealed>,
}

/// The Sessions the run state of `run` records, in position order.
///
/// # Errors
///
/// `unavailable` when the run state cannot be read.
async fn recorded_sessions(
    run: &Arc<BrowserRun>,
    context: &CallContext,
) -> Result<Vec<RecordedSession>, CallError> {
    run.state
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT position, is_default, closed, settings, identity_seed, source, state FROM page_sessions ORDER BY position",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(RecordedSession {
                    position: row.get(0)?,
                    is_default: row.get(1)?,
                    closed: row.get(2)?,
                    settings: row.get(3)?,
                    seed: IdentitySeed(u64::from_le_bytes(row.get::<_, i64>(4)?.to_le_bytes())),
                    source: row.get(5)?,
                    state: row.get(6)?,
                })
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
    run: &Arc<BrowserRun>,
    context: &CallContext,
) -> Result<Values, CallError> {
    let nested: Map<String, Value> =
        serde_json::from_str(recorded).map_err(|error| unavailable(run, &error, context))?;
    Values::read(SettingsLayer::Run, &nested, context.identity().product)
        .map(|values| values.from_origin(RunOrigin::Settings))
        .map_err(|error| unavailable(run, &error, context))
}

/// The open Session at `position` that the run state of `run` records,
/// reopened on `pool` with its own `settings` and the Identity profile its
/// `seed` draws, on the Borrowed browser its sealed `source` names if it
/// has one, or with its sealed `state` (cookies and `localStorage`) carried
/// into its next browser context, under an owner of its own, a child of the
/// Run's. A proxy or signing key it cannot reopen is kept as its
/// `reopen_failed`, which each page it is asked for raises.
#[expect(
    clippy::too_many_lines,
    reason = "reopens a recorded Session across source, state import, egress, identity, and pool seating"
)]
async fn reopened_session(
    (run, pool, human_cell): (&Arc<BrowserRun>, &Pool, &OnceCell<human_input::Plan>),
    position: u32,
    seed: IdentitySeed,
    settings: Values,
    source: Option<Sealed>,
    state: Option<Sealed>,
    context: &CallContext,
) -> Result<Session, CallError> {
    let borrowed = match source {
        Some(sealed) => Some(opened_source(&sealed, position, run, context).await?),
        None => None,
    };
    let carried = if borrowed.is_some() {
        Carried::default()
    } else {
        opened_state(state, position, run, context).await?
    };
    let owner = run.identity.owner().child();
    let (proxy, relay, reopen_failed) = reopened_egress(
        (run, &owner),
        &settings,
        pool,
        borrowed.as_ref(),
        position,
        context,
    )
    .await;
    let (reopened, reopen_failed) = match reopened_identity(
        (run, human_cell),
        &settings,
        borrowed.is_some(),
        seed,
        context,
    )
    .await
    {
        Ok(identity) => (identity, reopen_failed),
        Err(error) => (ReopenedIdentity::default(), reopen_failed.or(Some(error))),
    };
    let ReopenedIdentity {
        signing,
        fingerprint,
        human_input,
        emulation,
        identified,
    } = reopened;
    let session = session_id(run.identity.id(), position);
    let gpc = GpcPlan::of(
        &settings,
        run.identity.settings().map(RunSettings::resolved),
        None,
        borrowed.is_some(),
    );
    let saved_state = Arc::new(Mutex::new(SavedState {
        storage: carried.storage_map(),
        last: carried.clone(),
    }));
    let http_credentials = http_credentials_of(run, &settings);
    let (seat, attributes) = match &borrowed {
        Some(cdp_url) => (
            pool.borrowed_seat(
                cdp_url.clone(),
                egress(proxy.as_ref(), relay.as_ref(), http_credentials.as_ref()),
                TaskStarter::clone(&owner),
            ),
            BORROWED,
        ),
        None => (
            pool.seat_with_carried(
                placement(run, fingerprint.as_ref(), gpc, None),
                egress(proxy.as_ref(), relay.as_ref(), http_credentials.as_ref()),
                emulation,
                carried,
                TaskStarter::clone(&owner),
            ),
            FRESH,
        ),
    };
    let interception = session_interception(
        (&session, run),
        borrowed.as_ref(),
        (relay.is_some(), proxy.is_some()),
        signing,
        gpc.header_only(),
        identified,
        settings.clone(),
    );
    warn_gpc(run, &session, gpc).await;
    Ok(Session {
        owner,
        pages: HeldPages::default(),
        run: Arc::clone(run),
        position,
        seat,
        settings,
        life: SessionLife::new(session, attributes, SessionState::Open),
        proxy,
        egress_marked: OnceCell::new(),
        relay,
        http_credentials,
        reopen_failed,
        held_frames: borrowed
            .as_ref()
            .map(|_| session_pages::HeldFrames::default()),
        injections: Injections::default(),
        fingerprint,
        human_input,
        hiding: Arc::default(),
        interception,
        page_listeners: session_pages::PageListeners::default(),
        run_index: context.engine().run_index().clone(),
        is_profile_directory: false,
        browser_family: BrowserFamily::Chromium,
        saved_state,
    })
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
    run: &Arc<BrowserRun>,
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
            .identity
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

/// The Access mode a Session of the Run `identity` names opens in, as its
/// `own` settings, over the Run's, ask for from the source `named` (ADR
/// 0021): the [`access_mode`](crate::access_mode) module's refusal when it
/// does not open or asks for an effect this engine does not build yet
/// (ADR 0083). Its `stealth` identity is drawn, and checked against its
/// browser family's population, by [`fingerprint_of`].
fn access_mode_opens(
    own: &Values,
    identity: &RunIdentity,
    named: Option<&NamedSource>,
    context: &CallContext,
) -> Result<SessionIdentityPlan, CallError> {
    let source = named.map(|named| &named.source);
    let settings = SessionSettings::new(own, identity.settings().map(RunSettings::resolved));
    access_mode::resolve(&settings, source, BrowserFamily::of(source))
        .and_then(|plan| plan.built(&settings))
        .map_err(|refusal| refusal.call_error(context))
}

/// The Access mode and device emulation plans a Session of the Run
/// `identity` opens with, from `own` over the Run's settings and `named`
/// (ADR 0083; spec #578, ticket #3289).
fn session_plans(
    own: &Values,
    identity: &RunIdentity,
    named: Option<&NamedSource>,
    context: &CallContext,
) -> Result<(SessionIdentityPlan, SessionEmulation), CallError> {
    let access = access_mode_opens(own, identity, named, context)?;
    let emulation = emulation_opens(own, identity, named, context)?;
    Ok((access, emulation))
}

/// The device emulation and network throttling a Session of the Run
/// `identity` opens with, from `own` over the Run's settings and `named`
/// (ADR 0083; spec #578, ticket #3289).
fn emulation_opens(
    own: &Values,
    identity: &RunIdentity,
    named: Option<&NamedSource>,
    context: &CallContext,
) -> Result<SessionEmulation, CallError> {
    let source = named.map(|named| &named.source);
    let source_layer = named.map(|named| named.layer);
    let family = BrowserFamily::of(source);
    let settings = SessionSettings::new(own, identity.settings().map(RunSettings::resolved));
    emulation::resolve(&settings, source, source_layer, family)
        .map_err(|refusal| refusal.call_error(context))
}

/// The Fingerprint method a Session of `run`, opening as `plan` says with
/// its `own` settings, masks its pages with, if it does: with the Identity
/// profile `seed` draws from its narrowing of its browser family's
/// population on the machine's operating system (#913), none when it
/// generates none. The population is read on one of the Run's blocking
/// threads.
///
/// # Errors
///
/// `identity_outside_population` naming the `stealth.identity.*` key, in
/// the layer that set it, whose values the population draws no browser
/// from.
async fn fingerprint_of(
    run: &Arc<BrowserRun>,
    own: &Values,
    plan: &SessionIdentityPlan,
    seed: IdentitySeed,
    context: &CallContext,
) -> Result<Option<FingerprintPlan>, CallError> {
    let identity = match plan.narrowing() {
        Some(narrowing) => run
            .identity
            .owner()
            .blocking(move || narrowing.identity(seed))
            .await
            .map_err(|error| {
                let settings =
                    SessionSettings::new(own, run.identity.settings().map(RunSettings::resolved));
                Refusal::outside_population(&settings, &error).call_error(context)
            })?,
        None => None,
    };
    Ok(FingerprintPlan::of(plan, identity))
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
    run: &Arc<BrowserRun>,
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
        run_id: Some(run.identity.id()),
    };
    let key = run
        .keys
        .secrets()
        .resolve(&plan.key, &purpose)
        .await
        .map_err(|error| secret_error(&error))?;
    let signer =
        RequestSigner::new(&key, &plan.agent, run.entropy.fork().await).map_err(|error| {
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
/// What a Session's source resolved to when the Session opened (ADR 0056).
enum OpenedSource {
    Fresh,
    Borrowed(CdpUrl),
    Carried(Carried),
    ProfileDirectory(crate::profile_directory::ProfileDirectory),
}

impl OpenedSource {
    fn borrowed(&self) -> Option<&CdpUrl> {
        match self {
            Self::Borrowed(cdp_url) => Some(cdp_url),
            Self::Fresh | Self::Carried(_) | Self::ProfileDirectory(_) => None,
        }
    }

    fn carried(&self) -> Option<&Carried> {
        match self {
            Self::Carried(carried) => Some(carried),
            Self::Fresh | Self::Borrowed(_) | Self::ProfileDirectory(_) => None,
        }
    }

    fn profile(&self) -> Option<&crate::profile_directory::ProfileDirectory> {
        match self {
            Self::ProfileDirectory(profile) => Some(profile),
            Self::Fresh | Self::Borrowed(_) | Self::Carried(_) => None,
        }
    }
}

async fn opening_source(
    run: &Arc<BrowserRun>,
    named: Option<NamedSource>,
    context: &CallContext,
) -> Result<OpenedSource, CallError> {
    let Some(NamedSource {
        source,
        layer,
        acting_for_model,
    }) = named
    else {
        return Ok(OpenedSource::Fresh);
    };
    match source {
        SessionSource::CdpUrl {
            url, headers: None, ..
        } => Ok(OpenedSource::Borrowed(CdpUrl {
            url,
            origin: CdpUrlOrigin {
                layer,
                acting_for_model,
            },
        })),
        SessionSource::StateFile {
            path,
            format,
            key,
            domains,
        } => {
            let carried =
                import_state_file(run, (path, format, key, domains), layer, context).await?;
            Ok(OpenedSource::Carried(carried))
        }
        SessionSource::CookieHeader { header, domain } => {
            let imported =
                crate::state_import::parse_cookie_header(&header, &domain).map_err(|error| {
                    CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
                })?;
            Ok(OpenedSource::Carried(Carried::from_imported(imported)))
        }
        SessionSource::ProfileDirectory {
            path,
            browser,
            chromium_profile,
            ..
        } => {
            let profile = run
                .identity
                .owner()
                .blocking(move || {
                    crate::profile_directory::inspect(
                        std::path::Path::new(&path),
                        browser,
                        chromium_profile.as_deref(),
                        Some(135),
                        None,
                        layer,
                    )
                })
                .await?;
            Ok(OpenedSource::ProfileDirectory(profile))
        }
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

/// Reads a `state_file` on the Run's blocking pool, enforcing the 32 MiB cap.
#[expect(
    clippy::disallowed_methods,
    reason = "called exclusively inside `run.identity.owner().blocking(...)`"
)]
fn read_state_file_blocking(path: &str) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path)
        .map_err(|error| format!("could not read state_file `{path}`: {error}"))?;
    if meta.len() > crate::state_import::MAX_STATE_FILE_BYTES as u64 {
        return Err(format!("state_file `{path}` exceeds the 32 MiB limit"));
    }
    std::fs::read(path).map_err(|error| format!("could not read state_file `{path}`: {error}"))
}

/// Reads and imports a `state_file` Session source into [`Carried`] without
/// keeping any copy of the source file on disk (ADR 0056; ticket #735).
async fn import_state_file(
    run: &Arc<BrowserRun>,
    (path, format, key, domains): (
        String,
        Option<goodfetch_protocol::settings::session_source::StateFileFormat>,
        Option<String>,
        Option<Vec<String>>,
    ),
    layer: SettingsLayer,
    context: &CallContext,
) -> Result<Carried, CallError> {
    use goodfetch_engine::audit::Origin;
    use goodfetch_engine::session_bundle::IMPORT_LIMITS;
    use goodfetch_engine::session_transfer::{TransferError, import as import_bundle};
    use goodfetch_protocol::audit::Actor;
    use goodfetch_protocol::secret_reference::SecretReference;
    use goodfetch_protocol::settings::session_source::StateFileFormat;

    let bytes = run
        .identity
        .owner()
        .blocking(move || read_state_file_blocking(&path))
        .await
        .map_err(|message| CallError::protocol(ProtocolFailure::InvalidParams, message))?;

    let detected = format.unwrap_or_else(|| crate::state_import::detect_format(&bytes));
    let imported = match detected {
        StateFileFormat::StorageState => {
            crate::state_import::parse_storage_state(&bytes).map_err(|error| {
                CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
            })?
        }
        StateFileFormat::CookiesTxt => {
            crate::state_import::parse_cookies_txt(&bytes).map_err(|error| {
                CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
            })?
        }
        StateFileFormat::CdpCookies => {
            crate::state_import::parse_cdp_cookies(&bytes).map_err(|error| {
                CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
            })?
        }
        StateFileFormat::SessionExport => {
            let Some(key_str) = key else {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "a `session_export` state_file requires `key` naming the age identity or passphrase Secret reference",
                ));
            };
            let reference = SecretReference::parse(&key_str).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("state_file `key` is not a valid Secret reference: {error}"),
                )
            })?;
            let origin = Origin {
                actor: Actor::ClientSurface {
                    surface: context.surface(),
                },
                surface: Some(context.surface()),
                layer: Some(layer),
                run_id: Some(run.identity.id().clone()),
            };
            let export = import_bundle(
                bytes,
                &reference,
                IMPORT_LIMITS,
                &origin,
                run.keys.secrets(),
            )
            .await
            .map_err(|error| match error {
                TransferError::Secret(secret) => secret_error(&secret),
                TransferError::Open(open) => open.to_call_error(),
                TransferError::Unrecorded { source, .. } => CallError::protocol(
                    ProtocolFailure::InternalError,
                    format!("could not record session_imported in the Audit log: {source}"),
                ),
            })?;
            crate::state_import::from_session_export(export)
        }
    };
    let filtered = crate::state_import::filter_domains(imported, domains.as_deref().unwrap_or(&[]));
    Ok(Carried::from_imported(filtered))
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
            trace::provenance(&open.run.trace, entry).await;
        })
        .await;
}

/// The proxy and egress relay of the Session at `position` of `run`
/// reopened from run state with its own `settings`, and why it cannot open
/// a page when either does not open, its relay terminating as `pool`'s
/// authority lets it. It keeps the proxy it opened with,
/// so its address never changes under its cookies (SA-22); one on a
/// Borrowed browser, `borrowed`, has neither (ADR 0056). One whose proxy
/// or relay does not open raises why from each page it is asked for. Its
/// relay's tasks are `tasks`', the Session's owner's.
async fn reopened_egress(
    (run, tasks): (&Arc<BrowserRun>, &TaskStarter),
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
    let proxy = match open_proxy((run, tasks), settings, context, position).await {
        Ok(proxy) => proxy,
        Err(error) => return (None, None, Some(error)),
    };
    match open_relay(
        (run, tasks),
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

/// Records egress, Fingerprint method and human-like input choices for an
/// opened Session.
async fn mark_opened_session(run: &BrowserRun, open: &Session, session: &SessionId) {
    mark_egress(open, session).await;
    if let Some(fingerprint) = &open.fingerprint {
        fingerprint.chosen(run, session).await;
    }
    if let Some(human_input) = &open.human_input {
        human_input.chosen(run, session).await;
    }
}

/// The human-like input plan of a `stealth` Session whose
/// `stealth.human_input` is on, loaded once per Run from the Browser cache
/// or falling back to the ghost-cursor port (ADRs 0035, 0140).
async fn human_input_of(
    cell: &OnceCell<human_input::Plan>,
    run: &Arc<BrowserRun>,
    plan: &SessionIdentityPlan,
    context: &CallContext,
) -> Option<human_input::Plan> {
    if !plan
        .profile
        .as_ref()
        .is_some_and(|profile| profile.human_input)
    {
        return None;
    }
    let loaded = cell
        .get_or_init(|| async {
            let pinned_by = PinnedBy {
                product: context.identity().product,
                product_version: context.identity().product_version.clone(),
            };
            human_input::Plan::open(context.engine().home(), &run.identity, &pinned_by).await
        })
        .await;
    Some(loaded.clone())
}

/// The signer, Fingerprint method and human-like input plan of a Session of
/// `run` reopened from run state with its own `settings`, as a Session opens
/// with them: its key resolved again, once for the reopened Session, as
/// [`open_signing`] makes it, and its Identity profile drawn again from
/// its recorded `seed`, the one it opened with. One on a Borrowed browser,
/// when `borrowed`, neither signs nor masks (ADR 0021).
///
/// # Errors
///
/// As [`session_plans`], [`open_signing`] and [`fingerprint_of`] say:
/// the Session then raises it from each page it is asked for.
#[derive(Default)]
struct ReopenedIdentity {
    signing: Option<Arc<RequestSigner>>,
    fingerprint: Option<FingerprintPlan>,
    human_input: Option<human_input::Plan>,
    emulation: SessionEmulation,
    identified: bool,
}

async fn reopened_identity(
    (run, human_cell): (&Arc<BrowserRun>, &OnceCell<human_input::Plan>),
    settings: &Values,
    borrowed: bool,
    seed: IdentitySeed,
    context: &CallContext,
) -> Result<ReopenedIdentity, CallError> {
    if borrowed {
        return Ok(ReopenedIdentity {
            emulation: SessionEmulation::borrowed(),
            ..ReopenedIdentity::default()
        });
    }
    let (identity, emulation) = session_plans(settings, &run.identity, None, context)?;
    let identified = identity.mode == crate::access_mode::AccessMode::Identified;
    let signing = open_signing(run, identity.signing.as_ref(), context).await?;
    let fingerprint = fingerprint_of(run, settings, &identity, seed, context).await?;
    let human_input = human_input_of(human_cell, run, &identity, context).await;
    Ok(ReopenedIdentity {
        signing,
        fingerprint,
        human_input,
        emulation,
        identified,
    })
}

/// Takes a seat in `pool`: a Borrowed seat when `borrowed` names a browser,
/// else a pooled seat under `placement`, `egress`, and `emulation`.
fn open_seat(
    pool: &Pool,
    borrowed: Option<&CdpUrl>,
    carried: Option<&Carried>,
    placement: (BrowserPlacement, Window),
    egress: SessionEgress,
    emulation: SessionEmulation,
    tasks: TaskStarter,
) -> Seat {
    match borrowed {
        Some(cdp_url) => pool.borrowed_seat(cdp_url.clone(), egress, tasks),
        None => match carried.filter(|carried| !carried.is_empty()) {
            Some(carried) => {
                pool.seat_with_carried(placement, egress, emulation, carried.clone(), tasks)
            }
            None => pool.seat(placement, egress, emulation, tasks),
        },
    }
}

/// Where a Session of `run` whose pages mask their fingerprint as `fingerprint`
/// says, and whose Global Privacy Control plan is `gpc`, is placed in its
/// Run's browser pool (ADRs 0048 and 0129), and the window its browser shows
/// them in: a dedicated GPC or profile browser is never shared with another Session.
fn placement(
    run: &BrowserRun,
    fingerprint: Option<&FingerprintPlan>,
    gpc: GpcPlan,
    profile: Option<&crate::profile_directory::ProfileDirectory>,
) -> (BrowserPlacement, Window) {
    let window = match fingerprint {
        Some(fingerprint) => fingerprint.window(),
        None => Window::of(get_or_default(
            run.identity.settings().map(RunSettings::resolved),
            &BROWSER_HEADLESS,
        )),
    };
    let browser_placement = if let Some(profile) = profile {
        BrowserPlacement::OwnProcess {
            gpc: gpc.own_browser_gpc(),
            profile: Some(profile.clone()),
        }
    } else if gpc.own_browser_gpc() {
        BrowserPlacement::OwnProcess {
            gpc: true,
            profile: None,
        }
    } else if fingerprint.is_some_and(|plan| plan.placement() == Placement::OwnProcess) {
        BrowserPlacement::OwnProcess {
            gpc: false,
            profile: None,
        }
    } else {
        BrowserPlacement::Shared
    };
    (browser_placement, window)
}

/// Resolves `named` into a Borrowed browser URL (if any) and the Session's
/// Global Privacy Control plan (ADR 0048).
async fn opening_source_and_gpc(
    settings: &Values,
    run: &Arc<BrowserRun>,
    named: Option<NamedSource>,
    context: &CallContext,
) -> Result<(OpenedSource, GpcPlan), CallError> {
    let source = named.as_ref().map(|named| named.source.clone());
    let opened = opening_source(run, named, context).await?;
    let gpc = GpcPlan::of(
        settings,
        run.identity.settings().map(RunSettings::resolved),
        source.as_ref(),
        opened.borrowed().is_some(),
    );
    Ok((opened, gpc))
}

/// Records the `gpc_forced` warning in `run`'s report when `gpc` degrades
/// consistency or runs header-only on a Borrowed browser (ADR 0048).
async fn warn_gpc(run: &BrowserRun, session: &SessionId, gpc: GpcPlan) {
    let Some(warning) = gpc.warning() else {
        return;
    };
    if let Err(error) = run
        .warnings
        .warn_once(warning.to_run_warning(session))
        .await
    {
        tracing::warn!(%error, "the warning for `consent.gpc: on` was not kept");
    }
}

/// The side of the interception of the Session `session` of `run`, which
/// the Session holds: one no egress relay is under, `relayed` false, on a
/// Borrowed browser at `borrowed` or launched with none, behind a proxy
/// when `proxied`, is held by interception alone, its requests' hosts
/// resolved through the Run's `network.resolve`, and a name the engine
/// cannot resolve judged by where it would be resolved (ADR 0056; #2435);
/// its requests are signed by `signing` when it is `identified`, and carry
/// `Sec-GPC: 1` when `send_gpc_header` is set (ADR 0048).
fn session_interception(
    (session, run): (&SessionId, &Arc<BrowserRun>),
    borrowed: Option<&CdpUrl>,
    (relayed, proxied): (bool, bool),
    signing: Option<Arc<RequestSigner>>,
    send_gpc_header: bool,
    identified: bool,
    own_settings: Values,
) -> Arc<SessionInterception> {
    let lookups = held_browser(borrowed, relayed, proxied).map(|browser| {
        Lookups::new(
            resolve_of(run.identity.settings().map(RunSettings::resolved)),
            browser,
        )
    });
    // Its requests are answered from its Run's sources, as the HTTP tier's
    // are, so an entry either stored serves the other where the key
    // matches (ADR 0046).
    let caching = run.identity.settings().and_then(|settings| {
        Caching::of(
            run.engine.response_source(settings.resolved()),
            CALLER,
            run.identity.id().clone(),
            run.identity.clock(),
        )
        .map(|caching| caching.of_session(session))
    });
    Arc::new(
        SessionInterception::new(
            session.clone(),
            lookups,
            signing,
            send_gpc_header,
            identified,
            own_settings,
        )
        .with_caching(caching),
    )
}

/// Forgets what `open`, the Session `session` that ended, kept in its own
/// partition of its Run's cache, once no task of its own stores more
/// (ADR 0046).
async fn forget_cached(open: &Session, session: &SessionId) {
    if let Some(caching) = open.interception.caching() {
        caching.forget(session).await;
    }
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

/// Loads `session.http_credentials` once when a Session opens, from the
/// Session's own settings beneath its Run's (ADR 0092, ticket #733).
fn http_credentials_of(
    run: &Arc<BrowserRun>,
    own: &Values,
) -> Option<Arc<crate::cdp::driver::HttpCredentials>> {
    use goodfetch_protocol::secret_reference::SecretReference;
    use goodfetch_protocol::settings::SESSION_HTTP_CREDENTIALS;
    use goodfetch_protocol::settings::http_credentials::HttpCredential;
    let resolved = run.identity.settings().map(|settings| {
        settings
            .resolved()
            .beneath(own)
            .expect("invariant: the Session's own settings opened beneath its Run's")
    })?;
    let effective = resolved.effective(&SESSION_HTTP_CREDENTIALS)?;
    let raw: Vec<HttpCredential> = serde_json::from_value(effective.value.clone()).ok()?;
    let entries: Vec<crate::cdp::driver::BoundHttpCredential> = raw
        .into_iter()
        .filter_map(|entry| {
            let password = SecretReference::parse(&entry.password).ok()?;
            Some(crate::cdp::driver::BoundHttpCredential {
                username: entry.username,
                password,
                domains: entry.domains,
            })
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(Arc::new(crate::cdp::driver::HttpCredentials {
        run_id: run.identity.id().clone(),
        secrets: run.keys.secrets().clone(),
        entries,
        layer: Some(effective.layer),
    }))
}

/// What a Session's connections leave through: `proxy`, `relay` and
/// `http_credentials`.
fn egress(
    proxy: Option<&Arc<SessionProxy>>,
    relay: Option<&Arc<EgressRelay>>,
    http_credentials: Option<&Arc<crate::cdp::driver::HttpCredentials>>,
) -> SessionEgress {
    SessionEgress {
        proxy: proxy.cloned(),
        relay: relay.cloned(),
        http_credentials: http_credentials.cloned(),
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
/// such a Session is held by interception alone. Its tasks are `tasks`',
/// the Session's owner's.
///
/// # Errors
///
/// `local_address_unavailable` when the Session's local addresses give
/// none now, or it has them behind an `https://` proxy; the relay's bind
/// failing.
async fn open_relay(
    (run, tasks): (&Arc<BrowserRun>, &TaskStarter),
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
        .identity
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
    EgressRelay::open(relay_run(run, tasks), chain, tls, local)
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
fn no_local_address_on(run: &Arc<BrowserRun>, source: Source) -> Result<(), CallError> {
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
fn local_address_refused(run: &Arc<BrowserRun>, failure: LocalAddressFailure) -> CallError {
    Unavailable {
        entry: local_addresses_of(run).unwrap_or_default(),
        failure,
    }
    .into()
}

/// `run`'s `egress.local_addresses` as its value writes it, when it sets
/// one: an interface or a prefix as it is, a list as JSON.
fn local_addresses_of(run: &Arc<BrowserRun>) -> Option<String> {
    let value = run
        .identity
        .settings()?
        .resolved()
        .value(&EGRESS_LOCAL_ADDRESSES);
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        value => Some(value.to_string()),
    }
}

/// The live Run `run_id`; `run_ended` for one that ended `finished`, which
/// takes no new work (ADR 0102), and `run_not_found` for any other.
async fn live(run_id: &RunId, context: &CallContext) -> Result<Arc<BrowserRun>, CallError> {
    let error = match context.runs().live(run_id.clone()).await {
        Ok(run) => return Ok(BrowserRun::of(&run, context.engine())),
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
fn taken(
    state: &mut State,
    run: &Arc<BrowserRun>,
) -> Result<(u32, Pool, Arc<OnceCell<human_input::Plan>>), CallError> {
    let runs = reached(state, &run.identity)?;
    runs.last += 1;
    Ok((runs.last, runs.pool.clone(), Arc::clone(&runs.human_input)))
}

/// The position of the Run's local-address rotation that the program's
/// Session, or the Run's default one, at `position` binds: they take the
/// rotation's positions in their order, the first the first (ADR 0044's
/// #638 note).
fn rotation_ordinal(position: u32) -> u64 {
    u64::from(position.saturating_sub(1))
}

/// The Sessions the table holds of the Run `identity` names, which
/// [`Sessions::reach`] reached; `run_not_found` once it stopped since.
fn reached<'state>(
    state: &'state mut State,
    identity: &RunIdentity,
) -> Result<&'state mut RunSessions, CallError> {
    state.runs.get_mut(identity.id()).ok_or_else(|| {
        CallError::new(
            &RUN_NOT_FOUND,
            format!("The Run `{}` stopped while the call ran.", identity.id()),
            &RunDetails {
                run_id: identity.id().clone(),
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

/// The `timeout.wait` of the Run `identity` names, an app-state wait's
/// deadline.
fn wait_deadline(identity: &RunIdentity) -> Deadline {
    Deadline::of_resolved(
        identity.settings().map(RunSettings::resolved),
        &TIMEOUT_WAIT,
    )
}

/// The `timeout.action` of the Run `identity` names, with where it was
/// set.
fn action_timeout(identity: &RunIdentity) -> (SettingLimit, Duration) {
    let action = Deadline::of_resolved(
        identity.settings().map(RunSettings::resolved),
        &TIMEOUT_ACTION,
    );
    (action.limit().clone(), action.duration())
}
