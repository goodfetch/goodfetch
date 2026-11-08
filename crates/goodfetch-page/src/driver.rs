//! The driver (spec #577): a Session's pages driven over CDP. The
//! Session-level driver of ADR 0129 owns what every browser behind a
//! Session shares (its browser context, its pages, the commands they send,
//! each with its timeout and its record in the Run trace) and asks its
//! browser for one primitive, `connect()`, which yields the browser's CDP
//! connection: a browser the engine launched fills it here, a Browser host
//! later. `Runtime` is never enabled, and the guard's script lives in an
//! isolated world no page script reaches (ADR 0019). A page's navigations
//! each wait for their document's `load` event, or the state the call
//! names, and answer its response with the encoding it was decoded with,
//! read in the guard's world (ADR 0119); the back/forward cache is off at
//! launch, so going back or forward always loads its document again (ADR
//! 0121). Every page reports itself focused, so one behind another tab of
//! its Session keeps its timers (ADR 0085's question), and a headless
//! browser's pages show the identity it shows with a window (`identity`).
//! Each browser attaches the engine to its service workers before they run
//! (`service_workers`; ADR 0121), and each page to its out-of-process
//! frames and dedicated workers (`children`; ADR 0014). A page carries what Foxhound sets on it
//! (`page_value`): its lease check is asked before each command it sends,
//! and its waits for its navigation gate's permits count against none of
//! its deadlines (`permit_waits`).
//!
//! Size: one page's driver, whose commands, navigations and turns all read
//! the page's one connection and state; its navigation tracking, worlds,
//! targets, service workers, encoding reads, endings, permit waits and how
//! a response the browser reports is read (`cdp_response`) are in their own
//! modules.

mod carry;
pub(crate) mod cdp_response;
mod children;
mod encoding;
mod ending;
mod functions;
mod gestures;
mod identity;

mod navigation;
mod permit_waits;
mod popups;
#[cfg(test)]
mod replay;
mod service_workers;
mod targets;
mod world;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::{browser, dom, emulation, input, network, page, runtime, target};
use goodfetch_cdp::{
    Command, Connection, Failure, NetError, Refusal, SessionId as TargetSession, Subscription,
    SubscriptionEnd,
};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::Pins;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::home::Home;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::tls::CertificateInvalid;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::functions::World;
use goodfetch_protocol::page::{
    BrowserExit, PAGE_GO_BACK, PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageResponse, WaitUntil,
};
use goodfetch_protocol::settings::{TIMEOUT_ACTION, TIMEOUT_CLOSE, TIMEOUT_NAVIGATION};
use tokio::sync::{Mutex, MutexGuard, OnceCell, watch};
use tokio_util::sync::CancellationToken;

pub(crate) use self::carry::{Carried, Origins};
pub(crate) use self::children::{Child, ChildKind, OnChild};
use self::encoding::{encoding_of, encoding_read, isolated_world};
pub(crate) use self::ending::{ENDING_KNOWN_WITHIN, Lost};
use self::functions::{evaluation, page_function_world};
pub(crate) use self::navigation::{Arrived, browser_id, hop_id};
use self::navigation::{Ended, Expected, Followed, Navigation};
use self::permit_waits::PermitWaits;
pub(crate) use self::popups::{OnPopup, PopupHandler};
pub(crate) use self::service_workers::{OnWorker, ServiceWorkers};
use crate::borrowed::CdpUrl;
use crate::egress_relay::EgressRelay;
use crate::egress_relay::authority::Authority;
use crate::errors::{
    self, Call, call_error, interrupted, network_error, renderer_crashed, too_many_redirects,
    truncated,
};
use crate::launch::launch;
use crate::page_value::{self, PageValue};
use crate::proxy::{PROXY_STATUS, Party, ProxyPermit, SessionProxy, party, proxy_error};
use crate::trace::CommandLog;
use identity::Identity;

/// No bound of a command's own: its call's deadline bounds it, such as a
/// navigation's, or none, as Playwright's `evaluate` has none, and then
/// the call ends when its Client cancels it, or with its page or its
/// browser.
const UNBOUNDED: Duration = Duration::MAX;

/// Whether the browser shows a page the engine opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visibility {
    /// Shown, as a program's page is.
    Visible,
    /// Shown nowhere (`Target.createTarget`'s `hidden`), as an engine page
    /// is: a hidden page has no icon to show, so Chromium never asks its
    /// document's site for `/favicon.ico`, a request that leaves past the
    /// page's interception once that stops (ticket #1608).
    Hidden,
}

/// The deadlines of a Run's settings that its browser's commands wait under
/// (ADRs 0059 and 0080): a command the browser leaves unanswered past its
/// call's raises `deadline_exceeded` naming its setting.
pub(crate) struct Deadlines {
    /// `timeout.navigation`: a navigation's, and the bound of each command
    /// of opening a page, reading its document or its history, and of the
    /// engine's own closing commands.
    navigation: Deadline,
    /// `timeout.action`: each command of a locator action's.
    action: Deadline,
    /// `timeout.close`: how long a lost browser's exit is waited for, to
    /// name it.
    close: Deadline,
}

impl Deadlines {
    /// The deadlines of `run`'s settings.
    fn of(run: &Run) -> Arc<Self> {
        let settings = run.settings().map(RunSettings::resolved);
        Arc::new(Self {
            navigation: Deadline::of_resolved(settings, &TIMEOUT_NAVIGATION),
            action: Deadline::of_resolved(settings, &TIMEOUT_ACTION),
            close: Deadline::of_resolved(settings, &TIMEOUT_CLOSE),
        })
    }

    /// The deadline the commands of `call` wait under.
    fn of_call(&self, call: &Call) -> &Deadline {
        match call {
            Call::Launch { .. }
            | Call::OpenPage { .. }
            | Call::Document
            | Call::Navigation { .. }
            | Call::History => &self.navigation,
            Call::Action { .. } => &self.action,
        }
    }

    /// How long one command of `call` may go unanswered: a navigation's
    /// commands as long as the navigation goes on, since the navigation's
    /// own deadline, which leaves out its waits for permits, bounds them;
    /// any other's its call's deadline.
    fn command_limit(&self, call: &Call) -> Duration {
        match call {
            Call::Navigation { .. } => UNBOUNDED,
            Call::Launch { .. }
            | Call::OpenPage { .. }
            | Call::Document
            | Call::History
            | Call::Action { .. } => self.of_call(call).duration(),
        }
    }
}

/// The isolated world of the guard and every engine script (ADR 0019).
pub(crate) const GUARD_WORLD: &str = "goodfetch-guard";

/// What the guard's world holds before the guard does: a marker, which no
/// page script can read, as the Proving ground's CDP probe checks.
const GUARD_MARKER: &str = "globalThis.__goodfetch_guard__ = true;";

/// A browser's CDP connection as the driver uses it.
#[derive(Clone)]
pub(crate) struct Browser {
    connection: Connection,
    log: CommandLog,
    exit: watch::Receiver<Option<BrowserExit>>,
    executable: String,
    /// The identity a headless browser's pages show; `None` with a window.
    identity: Option<Identity>,
    /// Its process's ID.
    process: Option<u32>,
    /// Cancelled once the engine closes it, so its exit is not a loss.
    closing: CancellationToken,
    /// Whether the engine launched it, so its process's exit tells a quit
    /// from a crash; a Borrowed browser's ended connection is its quit
    /// (spec #594's decision 11).
    launched: bool,
    /// Its service workers, which it attaches the engine to (ADR 0121).
    service_workers: Arc<service_workers::ServiceWorkerChoices>,
    /// What each browser context does with the popups its pages open.
    popups: Arc<popups::Popups>,
    /// Its hidden pages still open, by their browser context.
    hidden: Arc<HiddenPages>,
    /// The lease check of the page this connection drives, asked before
    /// each command it sends; none on the browser's own connection and on
    /// a page whose value has none (ADR 0087).
    lease: Option<Arc<dyn LeaseCheck>>,
    /// Its Run's deadlines, which its commands wait under.
    deadlines: Arc<Deadlines>,
    /// Where its pages' reads count their work (ADR 0140's C2).
    work: CountedWork,
    /// The row each command it sends counts on as well: a guarded action's
    /// attempt, by its kind (ADR 0140's C3), or a guard's check; none on
    /// every other handle.
    counting: Option<Row>,
}

/// A browser's hidden pages (`Target.createTarget`'s `hidden`, as an
/// engine page is) still open, by their browser context: Chromium keeps a
/// hidden page open past its context's disposal, its commands in flight
/// left unanswered, so a Session's close closes its own first. Each page
/// leaves the list as it closes.
#[derive(Default)]
struct HiddenPages(Mutex<BTreeMap<browser::BrowserContextID, BTreeSet<target::TargetID>>>);

impl HiddenPages {
    /// The page `page` opened hidden in the context `context`.
    async fn opened(&self, context: &browser::BrowserContextID, page: &target::TargetID) {
        self.0
            .lock()
            .await
            .entry(context.clone())
            .or_default()
            .insert(page.clone());
    }

    /// The page `page` of the context `context` closed, if it was hidden.
    async fn closed(&self, context: &browser::BrowserContextID, page: &target::TargetID) {
        let mut contexts = self.0.lock().await;
        if let Some(pages) = contexts.get_mut(context) {
            pages.remove(page);
            if pages.is_empty() {
                contexts.remove(context);
            }
        }
    }

    /// The hidden pages of the context `context` still open, which leave
    /// the list.
    async fn take(&self, context: &browser::BrowserContextID) -> BTreeSet<target::TargetID> {
        self.0.lock().await.remove(context).unwrap_or_default()
    }
}

#[cfg(test)]
impl Browser {
    /// A browser no Run holds, over `connection`, under the timeouts'
    /// declared defaults, as a test plays one in process.
    fn played(connection: Connection) -> Self {
        Self {
            connection,
            log: CommandLog::unwritten(),
            exit: watch::channel(None).1,
            executable: "played".to_owned(),
            identity: None,
            process: None,
            closing: CancellationToken::new(),
            launched: false,
            service_workers: Arc::default(),
            popups: Arc::default(),
            hidden: Arc::default(),
            lease: None,
            counting: None,
            deadlines: Arc::new(Deadlines {
                navigation: Deadline::declared(&TIMEOUT_NAVIGATION),
                action: Deadline::declared(&TIMEOUT_ACTION),
                close: Deadline::declared(&TIMEOUT_CLOSE),
            }),
            work: CountedWork::default(),
        }
    }
}

impl Browser {
    /// Sends `params` on `session`, recorded in the Run trace, within the
    /// deadline of `call`; a failure raises the code of `call`.
    async fn send<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
        call: &Call,
    ) -> Result<C::Returns, CallError> {
        let limit = self.deadlines.command_limit(call);
        match self.try_send_within(session, params, limit).await {
            Ok(returned) => Ok(returned),
            Err(failure) => Err(self.failed(call, &failure).await),
        }
    }

    /// Sends `params` on `session`, recorded in the Run trace, answering
    /// the failure itself, for a caller that acts on some failures: the
    /// caller's own deadline bounds it, such as a navigation's, which
    /// leaves out its waits for permits while the browser holds back its
    /// `Page.navigate`, and the connection's end ends it.
    async fn try_send<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
    ) -> Result<C::Returns, Failure> {
        self.try_send_within(session, params, UNBOUNDED).await
    }

    /// Sends `params` on `session`, recorded in the Run trace, waiting for
    /// its answer at most `limit`, answering the failure itself.
    async fn try_send_within<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
        limit: Duration,
    ) -> Result<C::Returns, Failure> {
        if let Some(lease) = &self.lease
            && lease.check().is_err()
        {
            return Err(Failure::Withheld { method: C::METHOD });
        }
        self.counted();
        self.log.record(C::METHOD).await;
        self.connection.send(session, params, limit).await
    }

    /// Sends `params` on `session` whatever the lease, as the engine closes
    /// a page of its own, recorded in the Run trace.
    async fn send_unleased<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
    ) -> Result<C::Returns, Failure> {
        self.counted();
        self.log.record(C::METHOD).await;
        self.connection
            .send(session, params, self.deadlines.navigation.duration())
            .await
    }

    /// Counts one command it sends on the row it counts its commands on.
    fn counted(&self) {
        if let Some(row) = self.counting {
            self.log.work().count(row);
        }
    }

    /// The Error code `failure` raises from `call`, naming how the browser
    /// ended when it was lost; a command the lease check withheld raises
    /// the error that check gives, the Product's own (ADR 0087).
    async fn failed(&self, call: &Call, failure: &Failure) -> CallError {
        if let Failure::Withheld { method } = failure
            && let Some(lease) = &self.lease
        {
            let ended = lease.check().err().unwrap_or(LeaseEnded::Released);
            return lease.withheld(ended, method);
        }
        let mut exit = self.exit.clone();
        let ended = if *failure == Failure::Disconnected {
            tokio::time::timeout(
                self.deadlines.close.duration(),
                exit.wait_for(Option::is_some),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|ended| ended.clone())
        } else {
            None
        };
        call_error(call, failure, ended, self.deadlines.of_call(call))
    }

    /// The call of opening a page in the browser, once it came up.
    fn opening(&self) -> Call {
        Call::OpenPage {
            executable: self.executable.clone(),
        }
    }

    /// The call of bringing the browser up, whose every failure is the
    /// launch's.
    fn launching(&self) -> Call {
        Call::Launch {
            executable: self.executable.clone(),
        }
    }
}

/// The primitive each browser behind a Session fills (ADR 0129): its CDP
/// connection, opened on the first call.
pub(crate) trait Connect: Send + Sync {
    /// The browser's connection.
    fn connect(&self) -> impl Future<Output = Result<Browser, CallError>> + Send;

    /// Whether it is a Borrowed browser, whose Session holds its default
    /// browser context and never closes it (ADR 0056).
    fn borrowed(&self) -> bool {
        false
    }
}

/// A browser several Sessions share, as the Run's Sessions share the
/// browser the engine launched for it (ADR 0037 / SA-09).
impl<B: Connect> Connect for Arc<B> {
    async fn connect(&self) -> Result<Browser, CallError> {
        B::connect(self).await
    }

    fn borrowed(&self) -> bool {
        B::borrowed(self)
    }
}

/// The browser behind a Session: one the engine launched, or a Borrowed
/// one.
pub(crate) enum Behind {
    /// A browser of the Run's browser pool.
    Launched(Arc<LocalBrowser>),
    /// A Borrowed browser, outside the pool.
    Borrowed(Arc<BorrowedBrowser>),
}

impl Connect for Behind {
    async fn connect(&self) -> Result<Browser, CallError> {
        match self {
            Self::Launched(browser) => browser.connect().await,
            Self::Borrowed(browser) => browser.connect().await,
        }
    }

    fn borrowed(&self) -> bool {
        matches!(self, Self::Borrowed(_))
    }
}

/// A Borrowed browser reached through its `cdp_url` (ADRs 0056 and 0115),
/// connected to on its Session's opening: the engine holds its own
/// connection, which ends with the Run, and never closes the browser.
pub(crate) struct BorrowedBrowser {
    run: Run,
    cdp_url: CdpUrl,
    /// The Run trace's CDP log, which every browser of the Run shares.
    log: Arc<OnceCell<CommandLog>>,
    /// Closed and empty: the engine sees no exit of a browser it did not
    /// launch, so a call that lost its connection waits for none to name
    /// the loss (fix queue 104: each such call waited `timeout.close`).
    exit: watch::Receiver<Option<BrowserExit>>,
    connected: OnceCell<Browser>,
}

impl BorrowedBrowser {
    pub(crate) fn new(run: Run, cdp_url: CdpUrl, log: Arc<OnceCell<CommandLog>>) -> Self {
        Self {
            run,
            cdp_url,
            log,
            exit: watch::channel(None).1,
            connected: OnceCell::new(),
        }
    }
}

impl BorrowedBrowser {
    /// Waits until the browser, once connected to, is lost: its connection
    /// ended, a quit (decision 11), or the engine closed it with its Run.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: it only reads.
    pub(crate) async fn ended(&self) -> Lost {
        match self.connected.get() {
            Some(browser) => browser.ended().await,
            None => Lost::EngineClosed,
        }
    }
}

impl Connect for BorrowedBrowser {
    async fn connect(&self) -> Result<Browser, CallError> {
        self.connected
            .get_or_try_init(|| async {
                let connection = crate::borrowed::connect(&self.run, &self.cdp_url).await?;
                let address = self.cdp_url.address();
                let log = self
                    .log
                    .get_or_try_init(|| async {
                        CommandLog::start(
                            self.run.owner(),
                            self.run.folder(),
                            self.run.metrics().counted_work().clone(),
                        )
                        .map_err(|error| {
                            CallError::new(
                                &goodfetch_protocol::page::BROWSER_LAUNCH_FAILED,
                                format!("the Run trace's CDP log cannot start: {error}"),
                                &goodfetch_protocol::page::BrowserLaunchFailedDetails {
                                    executable: address.clone(),
                                    exit: None,
                                },
                            )
                        })
                    })
                    .await?
                    .clone();
                let browser = Browser {
                    connection,
                    log,
                    exit: self.exit.clone(),
                    executable: address,
                    identity: None,
                    process: None,
                    closing: self.run.cancellation().child_token(),
                    launched: false,
                    service_workers: Arc::default(),
                    popups: Arc::default(),
                    hidden: Arc::default(),
                    lease: None,
                    counting: None,
                    deadlines: Deadlines::of(&self.run),
                    work: self.run.metrics().counted_work().clone(),
                };
                // The first answer says a browser speaks CDP there.
                browser
                    .send(None, &browser::GetVersionParams {}, &browser.launching())
                    .await?;
                service_workers::start(&browser, &self.run).await?;
                Ok(browser)
            })
            .await
            .cloned()
    }

    fn borrowed(&self) -> bool {
        true
    }
}

/// A browser the engine launches for a Run, numbered in the Run's browser
/// pool, on the first Session that needs it; it closes when the Run stops
/// or the pool closes it.
pub(crate) struct LocalBrowser {
    run: Run,
    home: Home,
    pins: Pins,
    number: u64,
    /// The Run trace's CDP log, which every browser of the Run shares.
    log: Arc<OnceCell<CommandLog>>,
    stopping: CancellationToken,
    launched: OnceCell<Browser>,
    /// Cancelled once the browser the last launch started is gone, its
    /// Browser data folder with it: a launch whose browser did not come up
    /// waits for it before it takes the same folder.
    released: Mutex<Option<CancellationToken>>,
    /// The egress relay's certificate authority, which it trusts.
    authority: Option<Arc<Authority>>,
}

impl LocalBrowser {
    pub(crate) fn new(
        run: Run,
        home: Home,
        pins: Pins,
        number: u64,
        log: Arc<OnceCell<CommandLog>>,
        authority: Option<Arc<Authority>>,
    ) -> Self {
        let stopping = run.cancellation().child_token();
        Self {
            run,
            home,
            pins,
            number,
            log,
            stopping,
            launched: OnceCell::new(),
            released: Mutex::new(None),
            authority,
        }
    }

    /// Closes the browser, if it was launched: asked to close, then killed
    /// if it does not.
    pub(crate) fn close(&self) {
        self.stopping.cancel();
    }

    /// Its process's ID, once launched.
    pub(crate) fn process(&self) -> Option<u32> {
        self.launched.get().and_then(|browser| browser.process)
    }

    /// Whether it came up and its connection ended since.
    pub(crate) fn is_lost(&self) -> bool {
        self.launched
            .get()
            .is_some_and(|browser| browser.connection.is_closed())
    }

    /// Waits until the browser, once it came up, is lost, and says how: it
    /// quit, it crashed, or the engine closed it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: it only reads.
    pub(crate) async fn ended(&self) -> Lost {
        match self.launched.get() {
            Some(browser) => browser.ended().await,
            None => Lost::EngineClosed,
        }
    }
}

/// A browser launched and not yet up: closed, then killed, unless it comes
/// up, whether its bring-up fails or its call gives up on it.
struct Attempt(Option<tokio_util::sync::DropGuard>);

impl Attempt {
    /// The browser came up: it stays.
    fn up(mut self) {
        if let Some(guard) = self.0.take() {
            guard.disarm();
        }
    }
}

impl Connect for LocalBrowser {
    /// Launches the browser on the first call, then answers its connection.
    /// A launch whose browser does not come up closes it before it answers,
    /// and the next launch takes its data folder once that browser is gone.
    async fn connect(&self) -> Result<Browser, CallError> {
        self.launched
            .get_or_try_init(|| async {
                let earlier = self.released.lock().await.take();
                if let Some(released) = earlier {
                    released.cancelled().await;
                }
                let attempt = self.stopping.child_token();
                let launched = launch(
                    &self.run,
                    &self.home,
                    &self.pins,
                    (
                        self.number,
                        self.authority.as_deref().map(Authority::spki_sha256),
                    ),
                    attempt.clone(),
                )
                .await?;
                *self.released.lock().await = Some(launched.released.clone());
                let coming_up = Attempt(Some(attempt.clone().drop_guard()));
                let log = self
                    .log
                    .get_or_try_init(|| async {
                        CommandLog::start(
                            self.run.owner(),
                            self.run.folder(),
                            self.run.metrics().counted_work().clone(),
                        )
                        .map_err(|error| {
                            CallError::new(
                                &goodfetch_protocol::page::BROWSER_LAUNCH_FAILED,
                                format!("the Run trace's CDP log cannot start: {error}"),
                                &goodfetch_protocol::page::BrowserLaunchFailedDetails {
                                    executable: launched.executable.display().to_string(),
                                    exit: None,
                                },
                            )
                        })
                    })
                    .await?
                    .clone();
                let mut browser = Browser {
                    connection: launched.connection,
                    log,
                    exit: launched.exit,
                    executable: launched.executable.display().to_string(),
                    identity: None,
                    process: launched.process,
                    closing: attempt,
                    launched: true,
                    service_workers: Arc::default(),
                    popups: Arc::default(),
                    hidden: Arc::default(),
                    lease: None,
                    counting: None,
                    deadlines: Deadlines::of(&self.run),
                    work: self.run.metrics().counted_work().clone(),
                };
                // The first answer says the browser is up.
                let version = browser
                    .send(None, &browser::GetVersionParams {}, &browser.launching())
                    .await?;
                if launched.headless {
                    browser.identity = Some(identity::read(&browser, &version.user_agent).await?);
                }
                service_workers::start(&browser, &self.run).await?;
                coming_up.up();
                Ok(browser)
            })
            .await
            .cloned()
    }
}

/// What a Session's connections leave through: its upstream proxy, if it
/// has one, and the egress relay its browser reaches the network through,
/// if it has one.
#[derive(Clone, Default)]
pub(crate) struct SessionEgress {
    pub(crate) proxy: Option<Arc<SessionProxy>>,
    pub(crate) relay: Option<Arc<EgressRelay>>,
}

/// The Session-level driver: one Session's browser context, its own
/// cookies and storage (ADR 0037 / SA-10) and its one proxy, set on the
/// context so no page or request has another (SA-22), in which it opens its
/// pages, made on its first use.
/// On a Borrowed browser, the Session holds the browser's default context,
/// where the login lives, and closes only the pages it opened there (ADR
/// 0056).
pub(crate) struct SessionDriver<B> {
    browser: B,
    egress: SessionEgress,
    context: OnceCell<browser::BrowserContextID>,
    /// The pages it opened in a Borrowed browser's default context, which
    /// its close closes.
    opened: Mutex<Vec<target::TargetID>>,
}

impl SessionDriver<Behind> {
    /// Whether its browser is one of the pool's whose connection ended: it
    /// quit or crashed, whether or not its pool has heard yet.
    pub(crate) fn browser_lost(&self) -> bool {
        match &self.browser {
            Behind::Launched(browser) => browser.is_lost(),
            Behind::Borrowed(_) => false,
        }
    }
}

impl<B: Connect> SessionDriver<B> {
    /// The driver of a Session of `browser` whose requests leave through
    /// `egress`.
    pub(crate) fn new(browser: B, egress: SessionEgress) -> Self {
        Self {
            browser,
            egress,
            context: OnceCell::new(),
            opened: Mutex::new(Vec::new()),
        }
    }

    /// Makes the Session's browser context, connecting to its browser
    /// first: a missing build or a browser that does not start raises its
    /// code here.
    pub(crate) async fn open(&self) -> Result<(), CallError> {
        let browser = self.browser.connect().await?;
        self.context_in(&browser).await.map(drop)
    }

    /// The Session's browser context in `browser`, made on first use; a
    /// Borrowed browser's default one.
    async fn context_in(&self, browser: &Browser) -> Result<browser::BrowserContextID, CallError> {
        let opening = browser.opening();
        self.context
            .get_or_try_init(|| async {
                if self.browser.borrowed() {
                    let contexts = browser
                        .send(None, &target::GetBrowserContextsParams {}, &opening)
                        .await?;
                    return contexts.default_browser_context_id.ok_or_else(|| {
                        CallError::new(
                            &goodfetch_protocol::page::BROWSER_LAUNCH_FAILED,
                            "the Borrowed browser names no default browser context",
                            &goodfetch_protocol::page::BrowserLaunchFailedDetails {
                                executable: browser.executable.clone(),
                                exit: None,
                            },
                        )
                    });
                }
                // Through its egress relay, every connection, loopback's
                // included; else through its proxy, if it has one.
                let (proxy_server, proxy_bypass_list) =
                    match (&self.egress.relay, &self.egress.proxy) {
                        (Some(relay), _) => (
                            Some(relay.browser_server()),
                            Some(crate::proxy::PROXIED_LOOPBACK.to_owned()),
                        ),
                        (None, Some(proxy)) => {
                            (Some(proxy.browser_server()), Some(proxy.browser_bypass()))
                        }
                        (None, None) => (None, None),
                    };
                browser
                    .send(
                        None,
                        &target::CreateBrowserContextParams {
                            proxy_server,
                            proxy_bypass_list,
                            ..target::CreateBrowserContextParams::default()
                        },
                        &opening,
                    )
                    .await
                    .map(|created| created.browser_context_id)
            })
            .await
            .cloned()
    }

    /// Closes the Session's browser context, and with it every page in it,
    /// its hidden pages closed first, which the context's disposal leaves
    /// open; the browser itself stays, for the Run's other Sessions. A
    /// browser already gone has nothing left to close. On a Borrowed
    /// browser, only the pages the Session opened close, and its default
    /// context stays.
    pub(crate) async fn close(&self) {
        let Some(context) = self.context.get() else {
            return;
        };
        let Ok(browser) = self.browser.connect().await else {
            return;
        };
        // Its service workers' choice goes with it, and what that holds.
        browser.service_workers.forget(context);
        if self.browser.borrowed() {
            for target_id in std::mem::take(&mut *self.opened.lock().await) {
                if let Err(failure) = browser
                    .try_send(None, &target::CloseTargetParams { target_id })
                    .await
                {
                    tracing::debug!(%failure, "a page of the Session was gone already");
                }
            }
            return;
        }
        // Closed, each one's target detaches, and its commands in flight
        // fail at once.
        for target_id in browser.hidden.take(context).await {
            if let Err(failure) = browser
                .try_send(None, &target::CloseTargetParams { target_id })
                .await
            {
                tracing::debug!(%failure, "a hidden page of the Session was gone already");
            }
        }
        if let Err(failure) = browser
            .try_send(
                None,
                &target::DisposeBrowserContextParams {
                    browser_context_id: context.clone(),
                },
            )
            .await
        {
            tracing::debug!(%failure, "the Session's browser context was gone already");
        }
    }

    /// Opens a page in the Session, of `visibility`, its navigations'
    /// deadline its Run's `timeout.navigation`.
    pub(crate) async fn new_page(&self, visibility: Visibility) -> Result<PageDriver, CallError> {
        let browser = self.browser.connect().await?;
        let opening = browser.opening();
        let context = self.context_in(&browser).await?;
        let created = browser
            .send(
                None,
                &target::CreateTargetParams {
                    url: "about:blank".to_owned(),
                    // A Borrowed browser's pages open in its default
                    // context, which takes no ID.
                    browser_context_id: (!self.browser.borrowed()).then(|| context.clone()),
                    left: None,
                    top: None,
                    width: None,
                    height: None,
                    window_state: None,
                    enable_begin_frame_control: None,
                    new_window: None,
                    background: None,
                    for_tab: None,
                    hidden: (visibility == Visibility::Hidden).then_some(true),
                    focus: None,
                },
                &opening,
            )
            .await?;
        // A page's main frame has its target's ID.
        let frame = created.target_id.clone();
        if self.browser.borrowed() {
            self.opened.lock().await.push(created.target_id.clone());
        } else if visibility == Visibility::Hidden {
            browser.hidden.opened(&context, &created.target_id).await;
        }
        let prepared = async {
            let session = browser.attach_own(&created.target_id, &opening).await?;
            prepare(&browser, &session).await.map(|()| session)
        };
        // A target the page never became is closed, so no tab is left
        // outside the Session's pages and the governor's count.
        let session = match prepared.await {
            Ok(session) => session,
            Err(error) => {
                browser.hidden.closed(&context, &created.target_id).await;
                if let Err(failure) = browser
                    .send_unleased(
                        None,
                        &target::CloseTargetParams {
                            target_id: created.target_id,
                        },
                    )
                    .await
                {
                    tracing::debug!(%failure, "a page that did not open was gone already");
                }
                return Err(error);
            }
        };
        Ok(PageDriver {
            browser,
            session,
            egress: self.egress.clone(),
            context,
            frame,
            started: Arc::new(watch::Sender::new(0)),
            navigating: Arc::new(watch::Sender::new(0)),
            turn: Arc::new(Mutex::new(None)),
            gesture: Arc::new(Mutex::new(())),
            value: PageValue::empty(),
            waits: None,
            reader: None,
        })
    }
}

/// Prepares the page target `browser` attached on `session` for the
/// engine, before it runs anything of its own: it reports itself focused,
/// shows its browser's identity, reports its frames' lifecycle and its
/// network, and holds the guard's world's marker.
async fn prepare(browser: &Browser, session: &TargetSession) -> Result<(), CallError> {
    let opening = browser.opening();
    browser
        .send(
            Some(session),
            &emulation::SetFocusEmulationEnabledParams { enabled: true },
            &opening,
        )
        .await?;
    if let Some(identity) = &browser.identity {
        browser
            .send(Some(session), &identity.command(), &opening)
            .await?;
    }
    browser
        .send(Some(session), &page::EnableParams::default(), &opening)
        .await?;
    browser
        .send(
            Some(session),
            &page::SetLifecycleEventsEnabledParams { enabled: true },
            &opening,
        )
        .await?;
    browser
        .send(Some(session), &network::EnableParams::default(), &opening)
        .await?;
    browser
        .send(
            Some(session),
            &page::AddScriptToEvaluateOnNewDocumentParams {
                source: GUARD_MARKER.to_owned(),
                world_name: Some(GUARD_WORLD.to_owned()),
                include_command_line_api: None,
                run_immediately: Some(true),
            },
            &opening,
        )
        .await?;
    Ok(())
}

/// A page's document as it stands, serialized once.
pub(crate) struct SerializedDocument {
    /// Its HTML.
    pub(crate) html: String,
    /// Its URL.
    pub(crate) url: String,
    /// Its base URL.
    pub(crate) base_url: String,
}

/// One page's target, driven over its CDP session.
#[derive(Clone)]
pub(crate) struct PageDriver {
    browser: Browser,
    session: TargetSession,
    /// Its Session's proxy and egress relay.
    egress: SessionEgress,
    /// Its Session's browser context.
    context: browser::BrowserContextID,
    /// The page's main frame.
    frame: page::FrameId,
    /// How many navigations the engine started on the page, by which one
    /// the browser cancelled is known to be superseded.
    started: Arc<watch::Sender<u64>>,
    /// The page's input order (ADR 0085): a gesture holds it from its first
    /// command to its last, and waiters take it in arrival order, as Tokio
    /// grants its lock (`STYLE.md`'s one exception to holding a lock across
    /// `.await`). It keeps the guard's world in the current document, once
    /// one was made.
    turn: Arc<Mutex<Option<runtime::ExecutionContextId>>>,
    /// The page's pointer, held from a check that arms the guard's listener
    /// to its disarm, so no two gestures interleave (`targets`).
    gesture: Arc<Mutex<()>>,
    /// How many navigation calls wait on the page now: a request the
    /// navigation gate refuses raises from them, or else is recorded.
    navigating: Arc<watch::Sender<u32>>,
    /// What it carries for Foxhound: its navigation gate and lease check,
    /// the latter asked before each command its `browser` sends.
    value: PageValue,
    /// Its waits for its gate's permits, when it has a gate.
    waits: Option<Arc<PermitWaits>>,
    /// The document reader Foxhound registered on it, no part of its
    /// value: none on a program's page (ADR 0119).
    reader: Option<page_value::Reader>,
}

/// A navigation call waiting on its page, counted until dropped.
struct NavigationCall<'page>(&'page watch::Sender<u32>);

impl<'page> NavigationCall<'page> {
    fn start(navigating: &'page watch::Sender<u32>) -> Self {
        navigating.send_modify(|navigating| *navigating += 1);
        Self(navigating)
    }
}

impl Drop for NavigationCall<'_> {
    fn drop(&mut self) {
        self.0
            .send_modify(|navigating| *navigating = navigating.saturating_sub(1));
    }
}

/// A gesture's turn on its page: its commands, sent while no other
/// gesture's are.
pub(crate) struct Turn<'page> {
    page: &'page PageDriver,
    world: MutexGuard<'page, Option<runtime::ExecutionContextId>>,
    call: Call,
}

/// Why a function did not answer in the guard's world.
pub(crate) enum Unanswered {
    /// The document it ran in went away, by a navigation: the world is made
    /// again in the next document.
    WorldGone,
    /// The call fails with this Error code.
    Failed(CallError),
}

impl Turn<'_> {
    /// Calls `function`, a JavaScript function's source, with `argument` in
    /// the guard's isolated world of the page's main frame, made on first
    /// use in each document, and answers its value.
    pub(crate) async fn call(
        &mut self,
        function: &str,
        argument: serde_json::Value,
    ) -> Result<serde_json::Value, Unanswered> {
        let context = self.world().await?;
        self.called(function, argument, context).await
    }

    /// Calls the guard's check `check`, a JavaScript function's source, with
    /// `argument`, as [`call`](Self::call) does, counting the command it sends
    /// and the source it carries (ADR 0140's C3). The world it may make
    /// first is its document's, made once there, so no check's own.
    pub(crate) async fn check(
        &mut self,
        check: &str,
        argument: serde_json::Value,
    ) -> Result<serde_json::Value, Unanswered> {
        let context = self.world().await?;
        let work = self.page.work();
        work.count(Row::ActionCheckCommands);
        work.count(Row::ActionScriptSends);
        self.called(check, argument, context).await
    }

    /// Calls `function` with `argument` in the guard's world, whose context
    /// is `context`, and answers its value.
    async fn called(
        &mut self,
        function: &str,
        argument: serde_json::Value,
        context: runtime::ExecutionContextId,
    ) -> Result<serde_json::Value, Unanswered> {
        let called = self.send(&world::call(function, argument, context)).await?;
        if let Some(thrown) = called.exception_details {
            return Err(Unanswered::Failed(CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the guard's script threw in the page: {}", thrown.text),
            )));
        }
        Ok(called.result.value.unwrap_or(serde_json::Value::Null))
    }

    /// Moves the pointer to (`x`, `y`) in the page's viewport and clicks
    /// there with its left button: input the page receives as trusted.
    pub(crate) async fn click(&mut self, x: f64, y: f64) -> Result<(), CallError> {
        for params in targets::pointer_click(x, y) {
            self.page
                .browser
                .send(Some(&self.page.session), &params, &self.call)
                .await?;
        }
        Ok(())
    }

    /// Sends one key event to the focused element.
    pub(crate) async fn key(
        &mut self,
        event: &input::DispatchKeyEventParams,
    ) -> Result<(), CallError> {
        self.page
            .browser
            .send(Some(&self.page.session), event, &self.call)
            .await
            .map(|_| ())
    }

    /// Inserts `text` where the focused element's selection is, as an
    /// input method commits text: one trusted `input` event.
    pub(crate) async fn insert_text(&mut self, text: &str) -> Result<(), CallError> {
        self.page
            .browser
            .send(
                Some(&self.page.session),
                &input::InsertTextParams {
                    text: text.to_owned(),
                },
                &self.call,
            )
            .await
            .map(|_| ())
    }

    /// The context of the guard's isolated world in the page's main frame,
    /// made on first use in each document.
    async fn world(&mut self) -> Result<runtime::ExecutionContextId, Unanswered> {
        if let Some(context) = *self.world {
            return Ok(context);
        }
        let made = self.send(&isolated_world(self.page.frame.clone())).await?;
        *self.world = Some(made.execution_context_id);
        Ok(made.execution_context_id)
    }

    /// The encoding the main frame's document was decoded with, read in
    /// the guard's world.
    async fn document_encoding(&mut self) -> Result<Option<String>, Unanswered> {
        let context = self.world().await?;
        let evaluated = self.send(&encoding_read(context)).await?;
        Ok(encoding_of(evaluated))
    }

    /// Sends a command of the guard's world, a lost document being
    /// [`Unanswered::WorldGone`].
    async fn send<C: Command>(&mut self, params: &C) -> Result<C::Returns, Unanswered> {
        match self
            .page
            .browser
            .try_send(Some(&self.page.session), params)
            .await
        {
            Ok(returned) => Ok(returned),
            Err(Failure::Refused {
                refusal: Refusal::ContextGone,
                ..
            }) => {
                *self.world = None;
                Err(Unanswered::WorldGone)
            }
            Err(failure) => Err(Unanswered::Failed(
                self.page.browser.failed(&self.call, &failure).await,
            )),
        }
    }
}

/// Where a navigation goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination<'a> {
    /// To a URL.
    Url(&'a str),
    /// To the page's document again.
    Reload,
    /// To the previous entry of the page's history.
    Back,
    /// To the next entry of the page's history.
    Forward,
}

impl Destination<'_> {
    /// The operation that navigates there, which a page's span records.
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::Url(_) => PAGE_GOTO.name,
            Self::Reload => PAGE_RELOAD.name,
            Self::Back => PAGE_GO_BACK.name,
            Self::Forward => PAGE_GO_FORWARD.name,
        }
    }
}

/// The command that starts a navigation.
enum Step {
    /// `Page.navigate`.
    Navigate,
    /// `Page.reload`.
    Reload,
    /// `Page.navigateToHistoryEntry` to this entry.
    Entry(i64),
}

impl PageDriver {
    /// The page's main frame.
    pub(crate) fn main_frame(&self) -> &page::FrameId {
        &self.frame
    }

    /// Its Session's proxy, if it has one.
    pub(crate) fn proxy(&self) -> Option<&SessionProxy> {
        self.egress.proxy.as_deref()
    }

    /// Its Session's egress relay, if it has one.
    pub(crate) fn relay(&self) -> Option<&EgressRelay> {
        self.egress.relay.as_deref()
    }

    /// Whether the browser asks the engine for the credentials of the proxy
    /// it connects to: its Session's egress relay's, or an HTTP or HTTPS
    /// proxy's, whose 407 then reaches the engine.
    pub(crate) fn answers_authentication(&self) -> bool {
        self.relay().is_some()
            || self
                .proxy()
                .is_some_and(SessionProxy::answers_authentication)
    }

    /// The credentials the engine answers the browser's proxy's 407 with:
    /// the egress relay's own when the Session has one, which sends the
    /// upstream proxy's itself, else its proxy's, if it has any.
    pub(crate) fn proxy_credentials(&self) -> Option<(&str, &str)> {
        if let Some(relay) = self.relay() {
            let endpoint = relay.endpoint();
            return Some((endpoint.username, endpoint.password));
        }
        self.proxy()?.credentials().map(|(username, password)| {
            (
                username,
                password.map_or("", |password| {
                    secrecy::ExposeSecret::expose_secret(password)
                }),
            )
        })
    }

    /// The page carrying `value`, its Run reading time from `clock`: its
    /// commands from now on ask its lease check first, and its navigations'
    /// deadlines leave out its waits for its gate's permits and for its
    /// Session's proxy's.
    pub(crate) fn with_value(mut self, value: PageValue, clock: Clock) -> Self {
        self.browser.lease = value.lease().cloned();
        self.waits = (value.gate().is_some() || self.egress.proxy.is_some())
            .then(|| PermitWaits::new(clock));
        self.value = value;
        self
    }

    /// What the page carries for Foxhound.
    pub(crate) fn value(&self) -> &PageValue {
        &self.value
    }

    /// The page reading its documents through `reader`, when one is
    /// registered.
    pub(crate) fn with_reader(mut self, reader: Option<page_value::Reader>) -> Self {
        self.reader = reader;
        self
    }

    /// The document reader registered on the page, if any.
    pub(crate) fn reader(&self) -> Option<&page_value::Reader> {
        self.reader.as_ref()
    }

    /// Asks the page's navigation gate to admit `navigation`, answering its
    /// permit; none from a page with no gate, which admits everything. The
    /// wait counts against no deadline of the page, and the page's lease
    /// check holds the lease through it (ADR 0093).
    ///
    /// # Errors
    ///
    /// The gate's refusal.
    pub(crate) async fn admit(
        &self,
        navigation: &page_value::Navigation,
    ) -> Result<Option<page_value::Permit>, page_value::Refusal> {
        let Some(gate) = self.value.gate() else {
            return Ok(None);
        };
        let waiting = self.waits.as_ref().map(PermitWaits::begin);
        let holding = self
            .value
            .lease()
            .map(|lease| lease.permit_wait(&navigation.url));
        let admitted = gate.admit(navigation).await;
        drop((holding, waiting));
        admitted.map(Some)
    }

    /// Tells the page's navigation gate that a request to `url` it
    /// admitted left the browser ([`page_value::NavigationGate::sending`]);
    /// nothing on a page with no gate.
    pub(crate) async fn sent(&self, url: &str) {
        if let Some(gate) = self.value.gate() {
            gate.sending(url).await;
        }
    }

    /// A Permit for one hop of a navigation through its Session's proxy,
    /// once the proxy's limits admit it (ADRs 0053 and 0088); none on a
    /// page without a proxy, and for a hop to `url` that `proxy.bypass`
    /// sends around the proxy. The wait counts against no deadline of the
    /// page, and its lease check holds the lease through it.
    pub(crate) async fn proxy_permit(&self, url: &str) -> Option<ProxyPermit> {
        let proxy = self
            .egress
            .proxy
            .as_deref()
            .filter(|proxy| !proxy.bypasses(url))?;
        let waiting = self.waits.as_ref().map(PermitWaits::begin);
        let holding = self.value.lease().map(|lease| lease.permit_wait(url));
        let permit = proxy.permit().await;
        drop((holding, waiting));
        permit
    }

    /// Whether a navigation call waits on the page now.
    pub(crate) fn navigation_waits(&self) -> bool {
        *self.navigating.borrow() > 0
    }

    /// Runs `future` within the page's navigation deadline, its Run's
    /// `timeout.navigation`, the page's waits for its gate's and its
    /// proxy's permits not counted.
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` naming `timeout.navigation` and `waited_for`
    /// once the deadline passed first; it counts as elapsed the time the
    /// deadline counted, its permit waits left out (ADRs 0080 and 0093).
    pub(crate) async fn within_navigation_deadline<F: Future>(
        &self,
        waited_for: WaitedFor,
        future: F,
    ) -> Result<F::Output, CallError> {
        Box::pin(self.within(Some(&self.browser.deadlines.navigation), waited_for, future)).await
    }

    /// The deadline of a navigation whose call's own `timeout` is
    /// `timeout`: its Run's `timeout.navigation` when absent
    /// ([`Deadline::of_call`]).
    ///
    /// # Errors
    ///
    /// `invalid_params` for a `timeout` of 0.
    #[expect(
        clippy::option_option,
        reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
    )]
    pub(crate) fn navigation_deadline(
        &self,
        timeout: Option<Option<u64>>,
    ) -> Result<Option<Deadline>, CallError> {
        Deadline::of_call(
            &self.browser.deadlines.navigation,
            &TIMEOUT_NAVIGATION,
            timeout,
        )
    }

    /// Runs `future` within `deadline`, the page's waits for its gate's and
    /// its proxy's permits not counted; with no deadline, until it ends.
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` naming the deadline's setting and `waited_for`
    /// once the deadline passed first; it counts as elapsed the time the
    /// deadline counted, its permit waits left out (ADRs 0080 and 0093).
    async fn within<F: Future>(
        &self,
        deadline: Option<&Deadline>,
        waited_for: WaitedFor,
        future: F,
    ) -> Result<F::Output, CallError> {
        let Some(deadline) = deadline else {
            return Ok(future.await);
        };
        let limit = deadline.duration();
        let done = match &self.waits {
            Some(waits) => waits.within(limit, future).await.ok(),
            None => tokio::time::timeout(limit, future).await.ok(),
        };
        done.ok_or_else(|| deadline.exceeded(waited_for, limit))
    }

    /// Closes the page's target, as the engine closes a page of its own
    /// once it is done with it; a target gone already has nothing to close.
    pub(crate) async fn close(&self) {
        if let Err(failure) = self
            .browser
            .send_unleased(
                None,
                &target::CloseTargetParams {
                    target_id: self.frame.clone(),
                },
            )
            .await
        {
            tracing::debug!(%failure, "the engine's page was gone already");
        }
        self.browser.hidden.closed(&self.context, &self.frame).await;
    }

    /// Asks the page to close as a person closing its tab does: its
    /// `beforeunload` handlers run first, and it closes once they let it.
    ///
    /// # Errors
    ///
    /// The code of `operation` when the page cannot be asked.
    pub(crate) async fn close_running_before_unload(
        &self,
        operation: &'static str,
    ) -> Result<(), CallError> {
        self.browser
            .send(
                Some(&self.session),
                &page::CloseParams {},
                &Call::Action { operation },
            )
            .await
            .map(drop)
    }

    /// The events of the page's target, from when this answers.
    pub(crate) fn events(&self) -> Subscription {
        self.browser.connection.subscribe(Some(&self.session))
    }

    /// Sends `params` on the page's target, recorded in the Run trace, as
    /// one of the commands that open the page: a failure raises
    /// `browser_launch_failed` or `page_crashed`.
    pub(crate) async fn opening_command<C: Command>(
        &self,
        params: &C,
    ) -> Result<C::Returns, CallError> {
        self.browser
            .send(Some(&self.session), params, &self.browser.opening())
            .await
    }

    /// Sends `params` on the page's target, recorded in the Run trace,
    /// answering the failure itself: the interception's commands, whose
    /// request may be gone.
    pub(crate) async fn command<C: Command>(&self, params: &C) -> Result<C::Returns, Failure> {
        self.browser.try_send(Some(&self.session), params).await
    }

    /// Calls `function`, a JavaScript function's source, with `argument` in
    /// the guard's isolated world of the frame `frame`'s document, made or
    /// found by its name, outside any gesture's turn: the reads engine
    /// scripts make of a frame, such as live hiding's (ADR 0047). A frame
    /// gone, or a function that throws, answers none.
    ///
    /// The one way an engine read reaches a frame, so it counts C2's work
    /// (ADR 0140): each command it sends, and the world it makes, which,
    /// with no registry of each frame's document yet, it makes on every read
    /// and so counts as one past its document's first (X10).
    pub(crate) async fn read_in_frame(
        &self,
        frame: &page::FrameId,
        function: &str,
        argument: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, Failure> {
        let work = &self.browser.work;
        for row in [
            Row::PageReadCommands,
            Row::PageReadDocumentWorlds,
            Row::PageReadWorldCreations,
        ] {
            work.count(row);
        }
        let made = self
            .browser
            .try_send(Some(&self.session), &isolated_world(frame.clone()))
            .await?;
        work.count(Row::PageReadCommands);
        let called = self
            .browser
            .try_send(
                Some(&self.session),
                &world::call(function, argument, made.execution_context_id),
            )
            .await?;
        if called.exception_details.is_some() {
            return Ok(None);
        }
        Ok(called.result.value)
    }

    /// Calls `function` with `argument` in the guard's isolated world of the
    /// page's main frame, outside any gesture's turn, as
    /// [`read_in_frame`](Self::read_in_frame) does: the locator waits'
    /// checks. A document gone or not there yet, or a function that throws,
    /// answers none; a failure raises the code of the operation `operation`.
    pub(crate) async fn read_in_page(
        &self,
        function: &str,
        argument: serde_json::Value,
        operation: &'static str,
    ) -> Result<Option<serde_json::Value>, CallError> {
        match self.read_in_frame(&self.frame, function, argument).await {
            Ok(value) => Ok(value),
            Err(Failure::Refused {
                refusal: Refusal::ContextGone,
                ..
            }) => Ok(None),
            Err(failure) => Err(self
                .browser
                .failed(&Call::Action { operation }, &failure)
                .await),
        }
    }

    /// Evaluates `expression`, a page function's, in the page's main frame:
    /// in its main world, or in the page functions' isolated world, made or
    /// found by its name; its promise awaited and its value answered by
    /// value, outside any gesture's turn (ADR 0085), for as long as it
    /// takes, as Playwright's `evaluate` has no timeout. A document replaced
    /// under it is tried once more in the next one; a failure raises the
    /// code of the operation `operation`.
    pub(crate) async fn evaluate(
        &self,
        expression: &str,
        world: World,
        operation: &'static str,
    ) -> Result<runtime::EvaluateReturns, CallError> {
        let call = &Call::Action { operation };
        let mut tries = 2;
        loop {
            tries -= 1;
            let context = match world {
                World::Main => None,
                World::Isolated => Some(
                    self.browser
                        .send(
                            Some(&self.session),
                            &page_function_world(self.frame.clone()),
                            call,
                        )
                        .await?
                        .execution_context_id,
                ),
            };
            match self
                .browser
                .try_send_within(
                    Some(&self.session),
                    &evaluation(expression, context),
                    UNBOUNDED,
                )
                .await
            {
                Ok(returned) => return Ok(returned),
                Err(Failure::Refused {
                    refusal: Refusal::ContextGone,
                    ..
                }) if tries > 0 => {}
                Err(failure) => return Err(self.browser.failed(call, &failure).await),
            }
        }
    }

    /// Registers `source` to run in the main world of every document the
    /// page loads from now on, its frames' included, before their own
    /// scripts, answering the script's identifier; a failure raises the
    /// code of the operation `operation`.
    pub(crate) async fn add_init_script(
        &self,
        source: String,
        operation: &'static str,
    ) -> Result<page::ScriptIdentifier, CallError> {
        let call = &Call::Action { operation };
        Ok(self
            .browser
            .send(
                Some(&self.session),
                &page::AddScriptToEvaluateOnNewDocumentParams {
                    source,
                    world_name: None,
                    include_command_line_api: None,
                    run_immediately: None,
                },
                call,
            )
            .await?
            .identifier)
    }

    /// Removes the init script `identifier`; a page gone has none left.
    pub(crate) async fn remove_init_script(&self, identifier: page::ScriptIdentifier) {
        let _ = self
            .browser
            .try_send(
                Some(&self.session),
                &page::RemoveScriptToEvaluateOnNewDocumentParams { identifier },
            )
            .await;
    }

    /// This page's driver with each command it sends counted on `row` as
    /// well, as a guarded action's attempt counts its commands by its kind
    /// and a guard's check its own (ADR 0140's C3).
    pub(crate) fn counting(&self, row: Row) -> Self {
        let mut page = self.clone();
        page.browser.counting = Some(row);
        page
    }

    /// The Counted work of the page's Run.
    pub(crate) fn work(&self) -> &CountedWork {
        self.browser.log.work()
    }

    /// Waits for the page's turn, in arrival order, for the gesture
    /// `operation`, whose failures raise its codes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the place in the order.
    pub(crate) async fn turn(&self, operation: &'static str) -> Turn<'_> {
        Turn {
            page: self,
            world: self.turn.lock().await,
            call: Call::Action { operation },
        }
    }

    /// Navigates the page to `destination` and waits until its document
    /// reaches the state `wait_until` names, answering where it arrived:
    /// the document's response, with the encoding it was decoded with once
    /// it was; none for a navigation within the document, to a URL no
    /// server answers, or to a history entry the page does not have; or the
    /// file the browser downloads instead.
    ///
    /// A navigation takes no turn among the locator actions, so that a
    /// later one supersedes it; a guard's world it leaves behind is made
    /// again in the next document when the browser no longer knows it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the navigation to go on in the browser.
    pub(crate) async fn navigate(
        &self,
        destination: Destination<'_>,
        wait_until: WaitUntil,
        deadline: Option<&Deadline>,
    ) -> Result<Arrived, CallError> {
        let Some((url, step)) = self.step_to(destination).await? else {
            return Ok(Arrived::Document(None));
        };
        let call = Call::Navigation { url: url.clone() };
        // The relay's refusals from here on are this navigation's to raise.
        let relay_mark = self.relay().map(EgressRelay::mark);
        let mut events = self.browser.connection.subscribe(Some(&self.session));
        let mut generation = 0;
        self.started.send_modify(|started| {
            *started += 1;
            generation = *started;
        });
        let superseded = || *self.started.borrow() > generation;
        let command = async {
            Ok(match step {
                Step::Navigate => {
                    let navigated = self
                        .browser
                        .send(
                            Some(&self.session),
                            &page::NavigateParams {
                                url: url.clone(),
                                referrer: None,
                                transition_type: None,
                                frame_id: None,
                                referrer_policy: None,
                            },
                            &call,
                        )
                        .await?;
                    navigation::navigated(&url, navigated, superseded())
                }
                Step::Reload => {
                    self.browser
                        .send(Some(&self.session), &page::ReloadParams::default(), &call)
                        .await?;
                    Ok(Some(Expected::FirstStarted))
                }
                Step::Entry(entry_id) => {
                    self.browser
                        .send(
                            Some(&self.session),
                            &page::NavigateToHistoryEntryParams { entry_id },
                            &call,
                        )
                        .await?;
                    Ok(Some(Expected::FirstStarted))
                }
            })
        };
        let waiting = NavigationCall::start(&self.navigating);
        let followed = self
            .within(
                deadline,
                waited_for(wait_until),
                // Boxed: the deadline that leaves out permit waits keeps it
                // in its own state across its sleeps.
                Box::pin(self.follow(command, &mut events, &call, wait_until, superseded)),
            )
            .await;
        // No later command waits on a subscription nobody reads.
        drop((waiting, events));
        let (followed, stopped) = self.as_the_site_sent(followed?);
        match followed {
            Ok(Ok(Arrived::Document(Some(response))))
                if let Some(failed) = self.answered_by_proxy(&url, &response) =>
            {
                Err(failed)
            }
            Ok(Ok(Arrived::Document(Some(response))))
                if wait_until != WaitUntil::Commit && !stopped =>
            {
                Ok(Arrived::Document(Some(PageResponse {
                    encoding: self.document_encoding(&call).await?,
                    ..response
                })))
            }
            Ok(Ok(arrived)) => Ok(arrived),
            Ok(Err(Ended::Failed(error))) => Err(self.failed(&url, &error, relay_mark).await),
            Ok(Err(Ended::Interrupted)) => Err(interrupted(&url)),
            Ok(Err(Ended::TooManyRedirects(chain))) => Err(too_many_redirects(&url, chain)),
            Ok(Err(Ended::Truncated { received, expected })) => {
                Err(truncated(&url, received, expected))
            }
            Ok(Err(Ended::Crashed)) => Err(renderer_crashed()),
            Err(failed) => Err(failed),
        }
    }

    /// `followed`, a navigation's end, its response holding the site's own
    /// headers where the page's document reader had the browser read it
    /// otherwise (ADR 0119); and whether the reader stopped that navigation
    /// at the interception, the page then holding no document whose
    /// encoding could be read.
    fn as_the_site_sent(
        &self,
        followed: Result<Result<Arrived, Ended>, CallError>,
    ) -> (Result<Result<Arrived, Ended>, CallError>, bool) {
        match followed {
            Ok(Ok(Arrived::Document(Some(mut response)))) => {
                let site = self
                    .reader
                    .as_ref()
                    .and_then(|reader| reader.site(&response.request_id));
                let stopped = site.as_ref().is_some_and(|site| site.stopped);
                if let Some(site) = site {
                    response.headers = site.headers;
                }
                (Ok(Ok(Arrived::Document(Some(response)))), stopped)
            }
            other => (other, false),
        }
    }

    /// The URL of the document the main frame holds: the one it could not
    /// reach when it shows the browser's error page.
    pub(crate) async fn document_url(&self) -> Result<String, CallError> {
        let tree = self
            .browser
            .send(
                Some(&self.session),
                &page::GetFrameTreeParams {},
                &Call::Document,
            )
            .await?;
        let frame = tree.frame_tree.frame;
        Ok(frame.unreachable_url.unwrap_or(frame.url))
    }

    /// The error of a navigation to `url` that failed with the browser's
    /// network error `error`, the Session's egress relay standing at
    /// `mark` as it began: the relay's refusal, else the proxy's own
    /// failure, else the site's, named as the browser names it when it
    /// connects itself, since behind the relay it sees only the relay's
    /// answer. A certificate the browser refused is `certificate_invalid`,
    /// named by the relay's own look at it where it can.
    async fn failed(&self, url: &str, error: &NetError, mark: Option<u64>) -> CallError {
        let relayed = self
            .relay()
            .and_then(|relay| relay.net_error(url, error, mark));
        let error = relayed.as_ref().unwrap_or(error);
        if let Some(refused) = self.refused_by_relay(mark, url) {
            return refused;
        }
        if let Some(failure) = errors::certificate_failure(error) {
            let examined = match self.relay() {
                Some(relay) => relay.examine(url).await,
                None => None,
            };
            let host = url::Url::parse(url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default();
            return examined
                .unwrap_or(CertificateInvalid { host, failure })
                .error();
        }
        self.egress
            .proxy
            .as_ref()
            .and_then(|proxy| proxy.failure(url, error))
            .unwrap_or_else(|| network_error(url, error.name()))
    }

    /// `egress_refused` when the Session's egress relay refused a
    /// connection since `mark`, as it was when the navigation to `url`
    /// began: the browser reports that refusal only as a network error.
    fn refused_by_relay(&self, mark: Option<u64>, url: &str) -> Option<CallError> {
        Some(self.relay()?.take_refusal(url, Some(mark?))?.error())
    }

    /// `proxy_unavailable` when `response`, the document's response to a
    /// navigation to `url`, is the Session's proxy's own, never the site's
    /// (the proxy-or-site rule).
    fn answered_by_proxy(&self, url: &str, response: &PageResponse) -> Option<CallError> {
        let proxy = self.egress.proxy.as_deref()?;
        let proxy_status = response.headers.get(PROXY_STATUS).map(String::as_str);
        if party(proxy.response(&response.url, proxy_status)) != Party::Proxy {
            return None;
        }
        let error = proxy_status.and_then(proxy_error).unwrap_or_default();
        Some(proxy.answered(url, &error))
    }

    /// The encoding the main frame's document was decoded with, read in
    /// the guard's world; none when the frame holds no document that names
    /// one. The world is the one the page's turn keeps when no gesture
    /// holds the turn, so that the next gesture finds it made; otherwise
    /// the browser gives the read the same world by its name.
    async fn document_encoding(&self, call: &Call) -> Result<Option<String>, CallError> {
        let Ok(world) = self.turn.try_lock() else {
            let made = self
                .browser
                .send(
                    Some(&self.session),
                    &isolated_world(self.frame.clone()),
                    call,
                )
                .await?;
            let evaluated = self
                .browser
                .send(
                    Some(&self.session),
                    &encoding_read(made.execution_context_id),
                    call,
                )
                .await?;
            return Ok(encoding_of(evaluated));
        };
        let mut turn = Turn {
            page: self,
            world,
            call: call.clone(),
        };
        // A world kept from the document before is gone: made again once.
        for _ in 0..2 {
            match turn.document_encoding().await {
                Ok(encoding) => return Ok(encoding),
                Err(Unanswered::WorldGone) => {}
                Err(Unanswered::Failed(error)) => return Err(error),
            }
        }
        Ok(None)
    }

    /// The URL `destination` names and the command that goes there; none
    /// for a history entry the page does not have.
    async fn step_to(
        &self,
        destination: Destination<'_>,
    ) -> Result<Option<(String, Step)>, CallError> {
        let offset = match destination {
            Destination::Url(url) => return Ok(Some((url.to_owned(), Step::Navigate))),
            Destination::Reload => 0,
            Destination::Back => -1,
            Destination::Forward => 1,
        };
        let history = self
            .browser
            .send(
                Some(&self.session),
                &page::GetNavigationHistoryParams {},
                &Call::History,
            )
            .await?;
        let Some(entry) = usize::try_from(history.current_index + offset)
            .ok()
            .and_then(|index| history.entries.get(index))
        else {
            return Ok(None);
        };
        let step = if offset == 0 {
            Step::Reload
        } else {
            Step::Entry(entry.id)
        };
        Ok(Some((entry.url.clone(), step)))
    }

    /// Follows the navigation `command` starts through the page's `events`,
    /// read while the command runs too, until it ends or reaches the state
    /// `wait_until` names.
    async fn follow(
        &self,
        command: impl Future<Output = Result<Result<Option<Expected>, Ended>, CallError>>,
        events: &mut Subscription,
        call: &Call,
        wait_until: WaitUntil,
        superseded: impl Fn() -> bool,
    ) -> Result<Result<Arrived, Ended>, CallError> {
        let mut navigation = Navigation::of(self.frame.clone());
        let (outcome, expected) = match navigation
            .follow(command, events, wait_until, &superseded)
            .await?
        {
            Followed::Ended { outcome, expected } => (outcome, expected),
            Followed::Disconnected => {
                return Err(self.browser.failed(call, &Failure::Disconnected).await);
            }
        };
        match outcome {
            Err(Ended::Failed(error)) if navigation::abandoned(&error) => {
                if self.renderer_crashed(events, &mut navigation, call).await? {
                    return Ok(Err(Ended::Crashed));
                }
                // Every event the browser sent before the renderer answered
                // is read now: a navigation the page started that cancelled
                // this one has started by then.
                Ok(Err(expected
                    .and_then(|expected| navigation.outcome(&expected, wait_until, superseded()))
                    .and_then(Result::err)
                    .unwrap_or(Ended::Failed(error))))
            }
            outcome => Ok(outcome),
        }
    }

    /// Whether the renderer of a navigation the browser abandoned crashed;
    /// else it is the browser's own failure of it, such as for an answer
    /// with no document, or a navigation the page started superseded it.
    /// The pinned Chromium abandons the navigation of a renderer that
    /// crashes before it reports the crash, once the crash's dump is
    /// written (~200 ms idle, longer under load), so the page's renderer is
    /// asked for its frame tree, the main frame's events read meanwhile: a
    /// live one answers, and a crashed one never does before its crash is
    /// reported. A target the browser detached meanwhile, as when the
    /// engine closes its browser, did not crash: the frame tree's answer,
    /// failed with it, ends the check, so the navigation raises its own
    /// failure, such as a Limit's that stopped the Run.
    async fn renderer_crashed(
        &self,
        events: &mut Subscription,
        navigation: &mut Navigation,
        call: &Call,
    ) -> Result<bool, CallError> {
        let alive = self
            .browser
            .try_send(Some(&self.session), &page::GetFrameTreeParams {});
        let mut alive = std::pin::pin!(alive);
        let mut reading = true;
        loop {
            if navigation.crashed() {
                return Ok(true);
            }
            tokio::select! {
                biased;
                event = events.next(), if reading => match event {
                    Some(event) => navigation.feed(&event),
                    None if detached(events) => reading = false,
                    None => return Err(self.browser.failed(call, &Failure::Disconnected).await),
                },
                answered = &mut alive => return match answered {
                    Err(Failure::Disconnected) => {
                        Err(self.browser.failed(call, &Failure::Disconnected).await)
                    }
                    Ok(_) | Err(_) => Ok(false),
                },
            }
        }
    }

    /// The page's document as the DOM holds it now, serialized once, with
    /// its URL and base URL: two commands, each counted on
    /// `C2.serialization_commands` (ADR 0140).
    pub(crate) async fn document(&self) -> Result<SerializedDocument, CallError> {
        let call = Call::Document;
        let work = &self.browser.work;
        work.count(Row::PageSerializationCommands);
        let root = self
            .browser
            .send(
                Some(&self.session),
                &dom::GetDocumentParams {
                    depth: Some(0),
                    pierce: None,
                },
                &call,
            )
            .await?
            .root;
        work.count(Row::PageSerializationCommands);
        let html = self
            .browser
            .send(
                Some(&self.session),
                &dom::GetOuterHTMLParams {
                    node_id: Some(root.node_id),
                    backend_node_id: None,
                    object_id: None,
                    // Every shadow root, open or closed, serialized as a
                    // `<template shadowrootmode>`, so that extraction-side
                    // removal walks them (ADR 0047).
                    include_shadow_dom: Some(true),
                },
                &call,
            )
            .await?
            .outer_html;
        let url = root.document_url.unwrap_or_default();
        Ok(SerializedDocument {
            html,
            base_url: root.base_url.unwrap_or_else(|| url.clone()),
            url,
        })
    }
}

/// Whether `events`, which ended, ended because the browser detached its
/// target, rather than with the connection: the commands in flight on that
/// target failed with it before it ended, so their answers say what ended
/// the call.
pub(crate) fn detached(events: &mut Subscription) -> bool {
    events.ended() == Some(SubscriptionEnd::Detached)
}

/// What a navigation that answers at `wait_until` waits for, as its
/// `deadline_exceeded` names it: the state, as the call spells it.
fn waited_for(wait_until: WaitUntil) -> WaitedFor {
    match wait_until {
        WaitUntil::Commit => WaitedFor::COMMIT,
        WaitUntil::DomContentLoaded => WaitedFor::DOM_CONTENT_LOADED,
        WaitUntil::Load => WaitedFor::LOAD,
    }
}
