//! The driver (spec #577): a Session's pages driven over CDP. The
//! Session-level driver of ADR 0129 owns what every browser behind a
//! Session shares (its browser context, its pages, the commands they send,
//! each with its timeout and its record in the Run trace) and asks its
//! browser for one primitive, `connect()`, which yields the browser's CDP
//! connection: a browser the engine launched fills it here, a Browser host
//! later. `Runtime` is never enabled, and the guard's script lives in an
//! isolated world no page script reaches (ADR 0019). A page's navigations
//! each wait for their document's `load` event and answer its response; the
//! back/forward cache is off at launch, so going back or forward always
//! loads its document again (ADR 0121). Every page reports itself focused,
//! so one behind another tab of its Session keeps its timers (ADR 0085's
//! question), and a headless browser's pages show the identity it shows
//! with a window (`identity`).

mod identity;

mod navigation;

use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::{browser, emulation, network, page, target};
use goodfetch_cdp::{Command, Connection, Failure, SessionId as TargetSession, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::Pins;
use goodfetch_engine::home::Home;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::page::{BrowserExit, PageResponse};
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use tokio::sync::{OnceCell, watch};

use self::navigation::{Ended, Expected, Navigation};
use crate::errors::{Call, call_error, interrupted, network_error, renderer_crashed};
use crate::launch::launch;
use crate::trace::CommandLog;
use identity::Identity;

/// How long one CDP command may go unanswered.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a lost browser's exit status is waited for, to name it.
const EXIT_WAIT: Duration = Duration::from_secs(2);

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
}

impl Browser {
    /// Sends `params` on `session`, recorded in the Run trace; a failure
    /// raises the code of `call`.
    async fn send<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
        call: &Call,
    ) -> Result<C::Returns, CallError> {
        self.log.record(C::METHOD).await;
        match self.connection.send(session, params, COMMAND_TIMEOUT).await {
            Ok(returned) => Ok(returned),
            Err(failure) => Err(self.failed(call, &failure).await),
        }
    }

    /// The Error code `failure` raises from `call`, naming how the browser
    /// ended when it was lost.
    async fn failed(&self, call: &Call, failure: &Failure) -> CallError {
        let mut exit = self.exit.clone();
        let ended = if *failure == Failure::Disconnected {
            tokio::time::timeout(EXIT_WAIT, exit.wait_for(Option::is_some))
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|ended| ended.clone())
        } else {
            None
        };
        call_error(call, failure, ended)
    }

    fn opening(&self) -> Call {
        Call::OpenPage {
            executable: self.executable.clone(),
        }
    }
}

/// The primitive each browser behind a Session fills (ADR 0129): its CDP
/// connection, opened on the first call.
pub(crate) trait Connect: Send + Sync {
    /// The browser's connection.
    fn connect(&self) -> impl Future<Output = Result<Browser, CallError>> + Send;
}

/// The browser the engine launches for a Run, on the first page it opens.
pub(crate) struct LocalBrowser {
    run: Run,
    home: Home,
    pins: Pins,
    launched: OnceCell<Browser>,
}

impl LocalBrowser {
    pub(crate) fn new(run: Run, home: Home, pins: Pins) -> Self {
        Self {
            run,
            home,
            pins,
            launched: OnceCell::new(),
        }
    }
}

impl Connect for LocalBrowser {
    async fn connect(&self) -> Result<Browser, CallError> {
        self.launched
            .get_or_try_init(|| async {
                let launched = launch(&self.run, &self.home, &self.pins).await?;
                let log =
                    CommandLog::start(self.run.owner(), self.run.folder()).map_err(|error| {
                        CallError::new(
                            &goodfetch_protocol::page::BROWSER_LAUNCH_FAILED,
                            format!("the Run trace's CDP log cannot start: {error}"),
                            &goodfetch_protocol::page::BrowserLaunchFailedDetails {
                                executable: launched.executable.display().to_string(),
                                exit: None,
                            },
                        )
                    })?;
                let mut browser = Browser {
                    connection: launched.connection,
                    log,
                    exit: launched.exit,
                    executable: launched.executable.display().to_string(),
                    identity: None,
                };
                // The first answer says the browser is up.
                let version = browser
                    .send(None, &browser::GetVersionParams {}, &browser.opening())
                    .await?;
                if launched.headless {
                    browser.identity = Some(identity::read(&browser, &version.user_agent).await?);
                }
                Ok(browser)
            })
            .await
            .cloned()
    }
}

/// The Session-level driver: one Session's browser context, in which it
/// opens its pages.
pub(crate) struct SessionDriver<B> {
    browser: B,
    context: OnceCell<browser::BrowserContextID>,
}

impl<B: Connect> SessionDriver<B> {
    pub(crate) fn new(browser: B) -> Self {
        Self {
            browser,
            context: OnceCell::new(),
        }
    }

    /// Opens a page in the Session, `navigation_timeout` its navigations'
    /// deadline.
    pub(crate) async fn new_page(
        &self,
        navigation_timeout: Duration,
    ) -> Result<PageDriver, CallError> {
        let browser = self.browser.connect().await?;
        let opening = browser.opening();
        let context: browser::BrowserContextID = self
            .context
            .get_or_try_init(|| async {
                browser
                    .send(
                        None,
                        &target::CreateBrowserContextParams::default(),
                        &opening,
                    )
                    .await
                    .map(|created| created.browser_context_id)
            })
            .await?
            .clone();
        let created = browser
            .send(
                None,
                &target::CreateTargetParams {
                    url: "about:blank".to_owned(),
                    browser_context_id: Some(context),
                    left: None,
                    top: None,
                    width: None,
                    height: None,
                    window_state: None,
                    enable_begin_frame_control: None,
                    new_window: None,
                    background: None,
                    for_tab: None,
                    hidden: None,
                    focus: None,
                },
                &opening,
            )
            .await?;
        // A page's main frame has its target's ID.
        let frame = created.target_id.clone();
        let attached = browser
            .send(
                None,
                &target::AttachToTargetParams {
                    target_id: created.target_id,
                    flatten: Some(true),
                },
                &opening,
            )
            .await?;
        let session = TargetSession(attached.session_id);
        browser
            .send(
                Some(&session),
                &emulation::SetFocusEmulationEnabledParams { enabled: true },
                &opening,
            )
            .await?;
        if let Some(identity) = &browser.identity {
            browser
                .send(Some(&session), &identity.command(), &opening)
                .await?;
        }
        browser
            .send(Some(&session), &page::EnableParams::default(), &opening)
            .await?;
        browser
            .send(
                Some(&session),
                &page::SetLifecycleEventsEnabledParams { enabled: true },
                &opening,
            )
            .await?;
        browser
            .send(Some(&session), &network::EnableParams::default(), &opening)
            .await?;
        browser
            .send(
                Some(&session),
                &page::AddScriptToEvaluateOnNewDocumentParams {
                    source: GUARD_MARKER.to_owned(),
                    world_name: Some(GUARD_WORLD.to_owned()),
                    include_command_line_api: None,
                    run_immediately: Some(true),
                },
                &opening,
            )
            .await?;
        Ok(PageDriver {
            browser,
            session,
            frame,
            navigation_timeout,
            started: Arc::new(watch::Sender::new(0)),
        })
    }
}

/// One page's target, driven over its CDP session.
#[derive(Clone)]
pub(crate) struct PageDriver {
    browser: Browser,
    session: TargetSession,
    /// The page's main frame.
    frame: page::FrameId,
    navigation_timeout: Duration,
    /// How many navigations the engine started on the page, by which one
    /// the browser cancelled is known to be superseded.
    started: Arc<watch::Sender<u64>>,
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
    /// Navigates the page to `destination` and waits for its document's
    /// `load` event, answering the document's response: none for a
    /// navigation within the document, to a URL no server answers, or to a
    /// history entry the page does not have.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the navigation to go on in the browser.
    pub(crate) async fn navigate(
        &self,
        destination: Destination<'_>,
    ) -> Result<Option<PageResponse>, CallError> {
        let Some((url, step)) = self.step_to(destination).await? else {
            return Ok(None);
        };
        let call = Call::Navigation { url: url.clone() };
        let mut events = self.browser.connection.subscribe(Some(&self.session)).await;
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
                    match (navigated.error_text, navigated.loader_id) {
                        (Some(error), _) => Err(navigation::refused(&error, superseded())),
                        (None, Some(loader)) => Ok(Some(Expected::Loader(loader))),
                        // Within the document: no document loads.
                        (None, None) => Ok(None),
                    }
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
        let followed = tokio::time::timeout(
            self.navigation_timeout,
            self.follow(command, &mut events, &call, superseded),
        )
        .await;
        match followed {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(Ended::Failed(error)))) => Err(network_error(&url, &error)),
            Ok(Ok(Err(Ended::Interrupted))) => Err(interrupted(&url)),
            Ok(Ok(Err(Ended::Crashed))) => Err(renderer_crashed()),
            Ok(Err(failed)) => Err(failed),
            Err(_) => Err(network_error(
                &url,
                &format!("no load event within `{}`", TIMEOUT_NAVIGATION.name),
            )),
        }
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
    /// read while the command runs too, until it ends.
    async fn follow(
        &self,
        command: impl Future<Output = Result<Result<Option<Expected>, Ended>, CallError>>,
        events: &mut Subscription,
        call: &Call,
        superseded: impl Fn() -> bool,
    ) -> Result<Result<Option<PageResponse>, Ended>, CallError> {
        let mut navigation = Navigation::of(self.frame.clone());
        let mut command = std::pin::pin!(command);
        let mut expected = None;
        loop {
            if let Some(expected) = &expected
                && let Some(outcome) = navigation.outcome(expected, superseded())
            {
                return Ok(outcome);
            }
            tokio::select! {
                answered = &mut command, if expected.is_none() => match answered? {
                    Ok(Some(answered)) => expected = Some(answered),
                    Ok(None) => return Ok(Ok(None)),
                    Err(ended) => return Ok(Err(ended)),
                },
                event = events.next() => match event {
                    Some(event) => navigation.feed(&event),
                    None => return Err(self.browser.failed(call, &Failure::Disconnected).await),
                },
            }
        }
    }
}
