//! The driver (spec #577): a Session's pages driven over CDP. The
//! Session-level driver of ADR 0129 owns what every browser behind a
//! Session shares (its browser context, its pages, the commands they send,
//! each with its timeout and its record in the Run trace) and asks its
//! browser for one primitive, `connect()`, which yields the browser's CDP
//! connection: a browser the engine launched fills it here, a Browser host
//! later. The driver never enables `Runtime`: `page_events` does, on a page
//! whose `page.on` call listens to `console` or `page_error`, and nothing
//! turns it off again. The guard's script lives in an isolated world no
//! page script reaches (ADR 0019). A page's navigations run on its owner
//! (`page_navigation`), which this driver serves with the navigation
//! tracking (`navigation`), the call's deadline and its permit waits; the
//! back/forward cache is off at launch, so going back or forward always
//! loads its document again (ADR 0121). Every page reports itself focused,
//! so one behind another tab of its Session keeps its timers (ADR 0085's
//! question), and a headless browser's pages show the identity it shows
//! with a window (`identity`). Each browser attaches the engine to its
//! service workers before they run (`service_workers`; ADR 0121); a page's
//! owner attaches it to the page's out-of-process frames and dedicated
//! workers (`page_children`; ADR 0014). Each browser the
//! engine launches is graded against the shipped release table before a
//! Session uses it (`version_check`; ADR 0035). A page carries what Foxhound sets on it
//! (`page_value`): its lease check is asked before each command it sends,
//! and its waits for its navigation gate's permits count against none of
//! its deadlines (`permit_waits`).
//!
//! Size: one page's driver, whose commands, navigations and turns all read
//! the page's one connection and state; its navigation tracking, worlds,
//! targets, service workers, endings, permit waits and how
//! a response the browser reports is read (`cdp_response`) are in their own
//! modules.

mod carry;
pub(crate) mod cdp_response;
mod ending;
mod gestures;
mod identity;

pub(crate) mod navigation;
mod page_reader;
mod permit_waits;
mod popups;
#[cfg(test)]
pub(crate) mod replay;
mod service_workers;
mod targets;
pub(crate) mod world;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::{browser, emulation, network, page, target};
use goodfetch_cdp::{
    Command, Connection, Failure, SessionId as TargetSession, Subscription, SubscriptionEnd,
};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::Pins;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::home::Home;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::runs::{Run, identity};
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::tasks::{self, TaskOwner, TaskStarter};
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::BrowserExit;
use goodfetch_protocol::settings::{
    TIMEOUT_ACTION, TIMEOUT_CLOSE, TIMEOUT_LAUNCH, TIMEOUT_NAVIGATION,
};
use tokio::sync::{Mutex, OnceCell, watch};
use tokio_util::sync::CancellationToken;

pub(crate) use self::carry::{Carried, Origins, Watcher};
pub(crate) use self::ending::{ENDING_KNOWN_WITHIN, Lost};
pub(crate) use self::navigation::{Arrived, browser_id, hop_id};
use self::page_reader::PageReader;
use self::permit_waits::PermitWaits;
pub(crate) use self::popups::{OnPopup, PopupHandler};
pub(crate) use self::service_workers::{OnWorker, ServiceWorkers};
pub(crate) use self::targets::pointer_click;
use crate::borrowed::CdpUrl;
use crate::egress_relay::EgressRelay;
use crate::egress_relay::authority::Authority;
use crate::errors::{Call, call_error};
use crate::launch::{Launcher, launch};
use crate::page_value::{self, PageValue};
use crate::proxy::{ProxyPermit, SessionProxy};
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
    /// `timeout.launch`: the bound of each command of bringing the browser
    /// up, its identity read included, and of that read's page load.
    launch: Deadline,
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
            launch: Deadline::of_resolved(settings, &TIMEOUT_LAUNCH),
            navigation: Deadline::of_resolved(settings, &TIMEOUT_NAVIGATION),
            action: Deadline::of_resolved(settings, &TIMEOUT_ACTION),
            close: Deadline::of_resolved(settings, &TIMEOUT_CLOSE),
        })
    }

    /// The deadline the commands of `call` wait under.
    fn of_call(&self, call: &Call) -> &Deadline {
        match call {
            Call::Launch { .. } => &self.launch,
            Call::OpenPage { .. } | Call::Document | Call::Navigation { .. } | Call::History => {
                &self.navigation
            }
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
    /// Cancelled once the reader of its own events, its workers' and new
    /// pages' attachments, fell 4,096 events behind: what it attached
    /// meanwhile may wait for the engine forever, so its pool takes it out
    /// of service (spec #2256, #3261).
    root_fell_behind: CancellationToken,
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
    /// Its Run's Counted work, where its commands and its pages' reads
    /// count (ADR 0140's C2, C3 and C7).
    work: CountedWork,
    /// The row each command it sends counts on as well: a guarded action's
    /// attempt, by its kind (ADR 0140's C3), or a guard's check; none on
    /// every other handle.
    counting: Option<Row>,
}

tokio::task_local! {
    /// Set while a page opens: each command sent meanwhile, awaited in
    /// turn, counts on `C7.opening_commands` (ADR 0140). Work the opening
    /// spawns, which it does not await, counts nothing there.
    static OPENING: ();
}

/// Runs `opening`, a page's opening, from its target's creation to its last
/// set-up, counting each command it sends on `C7.opening_commands` and
/// each task it starts on `C7.page_tasks`, on `work`, its Run's.
/// The opening is boxed, once a page: its two scopes would otherwise grow
/// every future that awaits it.
pub(crate) async fn counted_as_opening<F: Future>(work: &CountedWork, opening: F) -> F::Output {
    let opening = Box::pin(OPENING.scope((), opening));
    tasks::counting_starts(work.clone(), Row::PageTasks, opening).await
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
            exit: watch::channel(None).1,
            executable: "played".to_owned(),
            identity: None,
            process: None,
            closing: CancellationToken::new(),
            launched: false,
            service_workers: Arc::default(),
            root_fell_behind: CancellationToken::new(),
            popups: Arc::default(),
            hidden: Arc::default(),
            lease: None,
            counting: None,
            deadlines: Arc::new(Deadlines {
                launch: Deadline::declared(&TIMEOUT_LAUNCH),
                navigation: Deadline::declared(&TIMEOUT_NAVIGATION),
                action: Deadline::declared(&TIMEOUT_ACTION),
                close: Deadline::declared(&TIMEOUT_CLOSE),
            }),
            work: CountedWork::default(),
        }
    }
}

#[cfg(test)]
impl PageDriver {
    /// The driver of the page whose session is [`replay::SESSION`] and main
    /// frame [`replay::FRAME`], in a browser no Run holds, over
    /// `connection`, as a test plays one in process: its reader is read
    /// once the test runs [`PageDriver::reading`].
    pub(crate) fn played(connection: Connection) -> Self {
        let browser = Browser::played(connection);
        let session = TargetSession(replay::SESSION.to_owned());
        Self {
            page_reader: PageReader::new(&browser.connection, &session),
            browser,
            session,
            egress: SessionEgress::default(),
            context: "context".to_owned(),
            frame: replay::FRAME.to_owned(),
            started: Arc::new(watch::Sender::new(0)),
            navigating: Arc::new(watch::Sender::new(0)),
            spent: Duration::ZERO,
            value: PageValue::empty(),
            waits: None,
            reader: None,
        }
    }
}

impl Browser {
    /// Sends `params` on `session` within the deadline of `call`; a failure
    /// raises the code of `call`.
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

    /// Sends `params` on `session`, answering the failure itself, for a caller that acts on some failures: the
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

    /// Sends `params` on `session`, waiting for its answer at most `limit`,
    /// answering the failure itself.
    async fn try_send_within<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
        limit: Duration,
    ) -> Result<C::Returns, Failure> {
        self.leased::<C>()?;
        self.counted();
        self.connection.send(session, params, limit).await
    }

    /// Sends `commands` on `session` as one burst
    /// ([`Connection::burst`](goodfetch_cdp::Connection::burst)) within the
    /// deadline of `call`; a failure raises the code of `call`.
    async fn burst<C: Command>(
        &self,
        session: Option<&TargetSession>,
        commands: &[C],
        call: &Call,
    ) -> Result<(), CallError> {
        let limit = self.deadlines.command_limit(call);
        match self.try_burst_within(session, commands, limit).await {
            Ok(()) => Ok(()),
            Err(failure) => Err(self.failed(call, &failure).await),
        }
    }

    /// Sends `commands` on `session` as one burst, waiting for its answers
    /// at most `limit`, answering the failure itself. The lease is checked
    /// once, as the burst goes out whole or not at all.
    async fn try_burst_within<C: Command>(
        &self,
        session: Option<&TargetSession>,
        commands: &[C],
        limit: Duration,
    ) -> Result<(), Failure> {
        self.leased::<C>()?;
        for _ in commands {
            self.counted();
        }
        self.connection.burst(session, commands, limit).await
    }

    /// Withholds the command `C` once the page's work item's lease ended.
    fn leased<C: Command>(&self) -> Result<(), Failure> {
        match &self.lease {
            Some(lease) if lease.check().is_err() => Err(Failure::Withheld { method: C::METHOD }),
            _ => Ok(()),
        }
    }

    /// Sends `params` on `session` whatever the lease, as the engine closes
    /// a page of its own.
    async fn send_unleased<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
    ) -> Result<C::Returns, Failure> {
        self.counted();
        self.connection
            .send(session, params, self.deadlines.navigation.duration())
            .await
    }

    /// Counts one command it sends on the row it counts its commands on,
    /// and on C7's while a page opens (ADR 0140).
    fn counted(&self) {
        if let Some(row) = self.counting {
            self.work.count(row);
        }
        if OPENING.try_with(|()| ()).is_ok() {
            self.work.count(Row::PageOpeningCommands);
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
    /// The browser's owner, beside its Run's Sessions (ADR 0146): it holds
    /// the engine's connection, the readers of the browser's own events and
    /// every task started for the browser, so its stop ends them all.
    owner: TaskOwner,
    /// Closed and empty: the engine sees no exit of a browser it did not
    /// launch, so a call that lost its connection waits for none to name
    /// the loss (fix queue 104: each such call waited `timeout.close`).
    exit: watch::Receiver<Option<BrowserExit>>,
    connected: OnceCell<Browser>,
}

impl BorrowedBrowser {
    pub(crate) fn new(run: Run, cdp_url: CdpUrl) -> Self {
        let owner = run.owner().child();
        Self {
            run,
            cdp_url,
            owner,
            exit: watch::channel(None).1,
            connected: OnceCell::new(),
        }
    }

    /// The starting side of the browser's owner, whose tasks end with it.
    pub(crate) fn tasks(&self) -> &TaskStarter {
        &self.owner
    }

    /// Ends the engine's connection to the browser, which stays open, and
    /// answers once every task its owner started ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting: the connection still ends.
    pub(crate) async fn stop(&self) {
        self.owner.stop().await;
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
                let tasks = identity(&self.run).starting_on(&self.owner);
                let connection =
                    crate::borrowed::connect((&self.run, &tasks), &self.cdp_url).await?;
                let address = self.cdp_url.address();
                let browser = Browser {
                    connection,
                    exit: self.exit.clone(),
                    executable: address,
                    identity: None,
                    process: None,
                    closing: self.owner.child_token(),
                    launched: false,
                    service_workers: Arc::default(),
                    root_fell_behind: CancellationToken::new(),
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
                service_workers::start(&browser, &tasks).await?;
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
    launcher: Launcher,
    home: Home,
    pins: Pins,
    number: u64,
    /// The browser's owner, beside its Run's Sessions (ADR 0146): it
    /// supervises the browser's process, reads its own events and holds
    /// every task started for it, so its stop closes the browser and ends
    /// them all.
    owner: TaskOwner,
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
        (run, launcher): (Run, Launcher),
        home: Home,
        pins: Pins,
        number: u64,
        authority: Option<Arc<Authority>>,
    ) -> Self {
        let owner = run.owner().child();
        Self {
            run,
            launcher,
            home,
            pins,
            number,
            owner,
            launched: OnceCell::new(),
            released: Mutex::new(None),
            authority,
        }
    }

    /// Closes the browser, if it was launched: asked to close, then killed
    /// if it does not; answers once it is gone and every task its owner
    /// started ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting: the browser still closes.
    pub(crate) async fn close(&self) {
        self.owner.stop().await;
    }

    /// The starting side of the browser's owner, whose tasks end with it.
    pub(crate) fn tasks(&self) -> &TaskStarter {
        &self.owner
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

    /// Waits until the reader of the browser's own events, once it came
    /// up, fell behind; never for one that did not come up.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: it only reads.
    pub(crate) async fn root_fell_behind(&self) {
        match self.launched.get() {
            Some(browser) => browser.root_fell_behind.cancelled().await,
            None => std::future::pending().await,
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
                let tasks = identity(&self.run).starting_on(&self.owner);
                let attempt = self.owner.child_token();
                let launched = launch(
                    (&self.run, &tasks),
                    (&self.launcher, &self.home, &self.pins),
                    (
                        self.number,
                        self.authority.as_deref().map(Authority::spki_sha256),
                    ),
                    attempt.clone(),
                )
                .await?;
                *self.released.lock().await = Some(launched.released.clone());
                let coming_up = Attempt(Some(attempt.clone().drop_guard()));
                let mut browser = Browser {
                    connection: launched.connection,
                    exit: launched.exit,
                    executable: launched.executable.display().to_string(),
                    identity: None,
                    process: launched.process,
                    closing: attempt,
                    launched: true,
                    service_workers: Arc::default(),
                    root_fell_behind: CancellationToken::new(),
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
                crate::version_check::check(&self.run, &version.product).await;
                service_workers::start(&browser, &tasks).await?;
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
                    .map(|created| {
                        browser.work.count(Row::ContextsCreated);
                        created.browser_context_id
                    })
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
        let work = browser.work.clone();
        counted_as_opening(&work, self.new_page_in(browser, visibility)).await
    }

    /// Opens a page in the Session, of `visibility`, in its browser
    /// `browser`, once it came up.
    async fn new_page_in(
        &self,
        browser: Browser,
        visibility: Visibility,
    ) -> Result<PageDriver, CallError> {
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
            page_reader: PageReader::new(&browser.connection, &session),
            browser,
            session,
            egress: self.egress.clone(),
            context,
            frame,
            started: Arc::new(watch::Sender::new(0)),
            navigating: Arc::new(watch::Sender::new(0)),
            spent: Duration::ZERO,
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

/// One page's target, driven over its CDP session.
#[derive(Clone)]
pub(crate) struct PageDriver {
    browser: Browser,
    session: TargetSession,
    /// Its page's one reader, which its parts subscribe to, those of its
    /// frames' drivers included.
    page_reader: PageReader,
    /// Its Session's proxy and egress relay.
    egress: SessionEgress,
    /// Its Session's browser context.
    context: browser::BrowserContextID,
    /// The page's main frame.
    frame: page::FrameId,
    /// How many navigations the engine started on the page, by which one
    /// the browser cancelled is known to be superseded.
    started: Arc<watch::Sender<u64>>,
    /// How many navigation calls wait on the page now: a request the
    /// navigation gate refuses raises from them, or else is recorded.
    navigating: Arc<watch::Sender<u32>>,
    /// What the call driving it spent of its own deadline before its work
    /// began ([`Self::having_spent`]).
    spent: Duration,
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
pub(crate) struct NavigationCall<'page>(&'page watch::Sender<u32>);

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

    /// This page's driver for a call that spent `spent` of its own deadline
    /// before its work began, waiting for the page's `Runtime` (ADR 0019),
    /// which its navigation deadlines count.
    pub(crate) fn having_spent(&self, spent: Duration) -> Cow<'_, Self> {
        if spent.is_zero() {
            return Cow::Borrowed(self);
        }
        let mut page = self.clone();
        page.spent = spent;
        Cow::Owned(page)
    }

    /// The page's action deadline, its Run's `timeout.action`.
    pub(crate) fn action_deadline(&self) -> &Deadline {
        &self.browser.deadlines.action
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
    /// `setting_invalid` for a `timeout` of 0, naming `timeout.navigation`
    /// at the run layer from the call's argument.
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
    pub(crate) async fn within<F: Future>(
        &self,
        deadline: Option<&Deadline>,
        waited_for: WaitedFor,
        future: F,
    ) -> Result<F::Output, CallError> {
        let Some(deadline) = deadline else {
            return Ok(future.await);
        };
        let limit = deadline.duration();
        let left = limit.saturating_sub(self.spent);
        let done = match &self.waits {
            Some(waits) => waits.within(left, future).await.ok(),
            None => tokio::time::timeout(left, future).await.ok(),
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

    /// The events of the page's target, from when this answers, which its
    /// page's reader hands on.
    pub(crate) fn events(&self) -> Subscription {
        self.page_reader.events_on(&self.session)
    }

    /// Its page's reader's task, which hands each event on to the page's
    /// parts and, given `origins`, watches the origins its documents show:
    /// spawned once, where the page is taken in. A later call's ends at
    /// once.
    pub(crate) fn reading(&self, origins: Option<Watcher>) -> impl Future<Output = ()> + use<> {
        self.page_reader.reading(origins)
    }

    /// Sends `params` on the page's target as one of the commands that open the page: a failure raises
    /// `browser_launch_failed` or `page_crashed`.
    pub(crate) async fn opening_command<C: Command>(
        &self,
        params: &C,
    ) -> Result<C::Returns, CallError> {
        self.browser
            .send(Some(&self.session), params, &self.browser.opening())
            .await
    }

    /// Sends `commands` on the page's target as one burst, which goes out
    /// whole or not at all, within the deadline of `call`, recorded in the
    /// Run trace; a failure raises the code of `call`: the CDP module's
    /// primitives' commands.
    pub(crate) async fn burst<C: Command>(
        &self,
        commands: &[C],
        call: &Call,
    ) -> Result<(), CallError> {
        self.browser
            .burst(Some(&self.session), commands, call)
            .await
    }

    /// Sends `params` on the page's target within the deadline of `call`,
    /// recorded in the Run trace; a failure raises the code of `call`.
    pub(crate) async fn command_for<C: Command>(
        &self,
        params: &C,
        call: &Call,
    ) -> Result<C::Returns, CallError> {
        self.browser.send(Some(&self.session), params, call).await
    }

    /// Sends `params` on the page's target, recorded in the Run trace,
    /// answering the failure itself: the interception's commands, whose
    /// request may be gone.
    pub(crate) async fn command<C: Command>(&self, params: &C) -> Result<C::Returns, Failure> {
        self.browser.try_send(Some(&self.session), params).await
    }

    /// The Error code `failure` raises from `call`, naming how the browser
    /// ended when it was lost.
    pub(crate) async fn raised(&self, call: &Call, failure: &Failure) -> CallError {
        self.browser.failed(call, failure).await
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
        &self.browser.work
    }

    /// Marks the start of a navigation the engine sends on the page, and
    /// answers whether the engine started another after it since: a
    /// navigation the browser cancelled for that one is superseded.
    pub(crate) fn navigation_started(&self) -> impl Fn() -> bool + Send + Sync + 'static {
        let mut generation = 0;
        self.started.send_modify(|started| {
            *started += 1;
            generation = *started;
        });
        let started = Arc::clone(&self.started);
        move || *started.borrow() > generation
    }

    /// A navigation call waiting on the page from now until it is dropped:
    /// a request the navigation gate refuses meanwhile raises from it.
    pub(crate) fn navigation_waiting(&self) -> NavigationCall<'_> {
        NavigationCall::start(&self.navigating)
    }

    /// Runs `command`, a command the page's target answers, and answers
    /// once every event the browser sent before that answer is handed on.
    pub(crate) async fn after_events<T>(&self, command: impl Future<Output = T>) -> T {
        self.page_reader.after_events(command).await
    }

    /// The driver of the page's child target on `session`: its commands go
    /// there, and it shares everything else with the page's, so its page's
    /// main frame is never one of its own frames.
    pub(crate) fn on_child(&self, session: TargetSession) -> Self {
        Self {
            session,
            ..self.clone()
        }
    }
}

/// Whether `events`, which ended, ended because the browser detached its
/// target, rather than with the connection: the commands in flight on that
/// target failed with it before it ended, so their answers say what ended
/// the call.
pub(crate) fn detached(events: &mut Subscription) -> bool {
    events.ended() == Some(SubscriptionEnd::Detached)
}
