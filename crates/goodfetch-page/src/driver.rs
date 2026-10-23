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

mod identity;

mod navigation;
mod world;

use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::{browser, dom, emulation, input, network, page, runtime, target};
use goodfetch_cdp::{
    Command, Connection, Failure, Refusal, SessionId as TargetSession, Subscription,
};
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::Pins;
use goodfetch_engine::home::Home;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    BrowserExit, PAGE_GO_BACK, PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageResponse, WaitUntil,
};
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use tokio::sync::{Mutex, MutexGuard, OnceCell, watch};

pub(crate) use self::navigation::Arrived;
use self::navigation::{Ended, Expected, Navigation};
use crate::errors::{
    Call, call_error, interrupted, network_error, renderer_crashed, too_many_redirects, truncated,
};
use crate::launch::launch;
use crate::proxy::{Answer, Party, SessionProxy, party, proxy_error};
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

/// The header an intermediary names its own errors in (RFC 9209), as the
/// browser gives a response's headers, lower-cased.
const PROXY_STATUS: &str = "proxy-status";

/// The expression that reads the encoding a document was decoded with.
const DOCUMENT_ENCODING: &str = "document.characterSet";

/// The command that makes, or finds by its name, the guard's isolated
/// world in `frame`'s document.
fn isolated_world(frame: page::FrameId) -> page::CreateIsolatedWorldParams {
    page::CreateIsolatedWorldParams {
        frame_id: frame,
        world_name: Some(GUARD_WORLD.to_owned()),
        grant_univeral_access: None,
        content_security_policy: None,
    }
}

/// The command that reads the document's encoding in the world whose
/// context is `context`, never through `Runtime.enable` (ADR 0019).
fn encoding_read(context: runtime::ExecutionContextId) -> runtime::EvaluateParams {
    runtime::EvaluateParams {
        expression: DOCUMENT_ENCODING.to_owned(),
        object_group: None,
        include_command_line_api: None,
        silent: Some(true),
        context_id: Some(context),
        return_by_value: Some(true),
        generate_preview: None,
        user_gesture: None,
        await_promise: None,
        throw_on_side_effect: None,
        timeout: None,
        disable_breaks: None,
        repl_mode: None,
        allow_unsafe_eval_blocked_by_csp: None,
        unique_context_id: None,
        serialization_options: None,
    }
}

/// The encoding an [`encoding_read`] answered, when it named one.
fn encoding_of(evaluated: runtime::EvaluateReturns) -> Option<String> {
    evaluated
        .result
        .value
        .and_then(|value| value.as_str().map(str::to_owned))
}

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
        match self.try_send(session, params).await {
            Ok(returned) => Ok(returned),
            Err(failure) => Err(self.failed(call, &failure).await),
        }
    }

    /// Sends `params` on `session`, recorded in the Run trace, answering
    /// the failure itself, for a caller that acts on some failures.
    async fn try_send<C: Command>(
        &self,
        session: Option<&TargetSession>,
        params: &C,
    ) -> Result<C::Returns, Failure> {
        self.log.record(C::METHOD).await;
        self.connection.send(session, params, COMMAND_TIMEOUT).await
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

/// A browser several Sessions share, as the Run's Sessions share the
/// browser the engine launched for it (ADR 0037 / SA-09).
impl<B: Connect> Connect for Arc<B> {
    async fn connect(&self) -> Result<Browser, CallError> {
        B::connect(self).await
    }
}

/// The browser the engine launches for a Run, on the first Session that
/// needs it.
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

/// The Session-level driver: one Session's browser context, its own
/// cookies and storage (ADR 0037 / SA-10) and its one proxy, set on the
/// context so no page or request has another (SA-22), in which it opens its
/// pages, made on its first use.
pub(crate) struct SessionDriver<B> {
    browser: B,
    proxy: Option<Arc<SessionProxy>>,
    context: OnceCell<browser::BrowserContextID>,
}

impl<B: Connect> SessionDriver<B> {
    /// The driver of a Session of `browser` whose requests go through
    /// `proxy`, if it has one.
    pub(crate) fn new(browser: B, proxy: Option<Arc<SessionProxy>>) -> Self {
        Self {
            browser,
            proxy,
            context: OnceCell::new(),
        }
    }

    /// Makes the Session's browser context, connecting to its browser
    /// first: a missing build or a browser that does not start raises its
    /// code here.
    pub(crate) async fn open(&self) -> Result<(), CallError> {
        let browser = self.browser.connect().await?;
        self.context_in(&browser).await.map(drop)
    }

    /// The Session's browser context in `browser`, made on first use.
    async fn context_in(&self, browser: &Browser) -> Result<browser::BrowserContextID, CallError> {
        let opening = browser.opening();
        self.context
            .get_or_try_init(|| async {
                let proxy = self.proxy.as_deref();
                browser
                    .send(
                        None,
                        &target::CreateBrowserContextParams {
                            proxy_server: proxy.map(SessionProxy::browser_server),
                            proxy_bypass_list: proxy.map(SessionProxy::browser_bypass),
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

    /// Closes the Session's browser context, and with it every page in it;
    /// the browser itself stays, for the Run's other Sessions. A browser
    /// already gone has nothing left to close.
    pub(crate) async fn close(&self) {
        let Some(context) = self.context.get() else {
            return;
        };
        let Ok(browser) = self.browser.connect().await else {
            return;
        };
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

    /// Opens a page in the Session, `navigation_timeout` its navigations'
    /// deadline.
    pub(crate) async fn new_page(
        &self,
        navigation_timeout: Duration,
    ) -> Result<PageDriver, CallError> {
        let browser = self.browser.connect().await?;
        let opening = browser.opening();
        let context = self.context_in(&browser).await?;
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
                    target_id: created.target_id.clone(),
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
            proxy: self.proxy.clone(),
            frame,
            navigation_timeout,
            started: Arc::new(watch::Sender::new(0)),
            turn: Arc::new(Mutex::new(None)),
        })
    }
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
    /// Its Session's proxy, if it has one.
    proxy: Option<Arc<SessionProxy>>,
    /// The page's main frame.
    frame: page::FrameId,
    navigation_timeout: Duration,
    /// How many navigations the engine started on the page, by which one
    /// the browser cancelled is known to be superseded.
    started: Arc<watch::Sender<u64>>,
    /// The page's input order (ADR 0085): a gesture holds it from its first
    /// command to its last, and waiters take it in arrival order. It keeps
    /// the guard's world in the current document, once one was made.
    turn: Arc<Mutex<Option<runtime::ExecutionContextId>>>,
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
        for (kind, button, buttons) in [
            (input::DispatchMouseEventType::MouseMoved, None, None),
            (
                input::DispatchMouseEventType::MousePressed,
                Some(input::MouseButton::Left),
                Some(1),
            ),
            (
                input::DispatchMouseEventType::MouseReleased,
                Some(input::MouseButton::Left),
                Some(0),
            ),
        ] {
            let params = input::DispatchMouseEventParams {
                r#type: kind,
                x,
                y,
                modifiers: None,
                timestamp: None,
                button,
                buttons,
                click_count: button.map(|_| 1),
                force: None,
                tangential_pressure: None,
                tilt_x: None,
                tilt_y: None,
                twist: None,
                delta_x: None,
                delta_y: None,
                pointer_type: None,
            };
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
        self.proxy.as_deref()
    }

    /// The events of the page's target, from when this answers.
    pub(crate) async fn events(&self) -> Subscription {
        self.browser.connection.subscribe(Some(&self.session)).await
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
    ) -> Result<Arrived, CallError> {
        let Some((url, step)) = self.step_to(destination).await? else {
            return Ok(Arrived::Document(None));
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
                        (Some(error), loader) => navigation::refused(
                            &error,
                            loader,
                            navigated.is_download == Some(true),
                            superseded(),
                        )
                        .map(Some),
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
            self.follow(command, &mut events, &call, wait_until, superseded),
        )
        .await;
        match followed {
            Ok(Ok(Ok(Arrived::Document(Some(response)))))
                if let Some(failed) = self.proxys_own(&url, &response) =>
            {
                Err(failed)
            }
            Ok(Ok(Ok(Arrived::Document(Some(response))))) if wait_until != WaitUntil::Commit => {
                Ok(Arrived::Document(Some(PageResponse {
                    encoding: self.document_encoding(&call).await?,
                    ..response
                })))
            }
            Ok(Ok(Ok(arrived))) => Ok(arrived),
            Ok(Ok(Err(Ended::Failed(error)))) => Err(self
                .proxy
                .as_ref()
                .and_then(|proxy| proxy.failure(&url, &error))
                .unwrap_or_else(|| network_error(&url, &error))),
            Ok(Ok(Err(Ended::Interrupted))) => Err(interrupted(&url)),
            Ok(Ok(Err(Ended::TooManyRedirects(chain)))) => Err(too_many_redirects(&url, chain)),
            Ok(Ok(Err(Ended::Truncated { received, expected }))) => {
                Err(truncated(&url, received, expected))
            }
            Ok(Ok(Err(Ended::Crashed))) => Err(renderer_crashed()),
            Ok(Err(failed)) => Err(failed),
            Err(_) => Err(network_error(
                &url,
                &format!("no load event within `{}`", TIMEOUT_NAVIGATION.name),
            )),
        }
    }

    /// `proxy_unavailable` when `response`, the document's response to a
    /// navigation to `url`, is the Session's proxy's own, never the site's
    /// (the proxy-or-site rule).
    fn proxys_own(&self, url: &str, response: &PageResponse) -> Option<CallError> {
        let proxy = self.proxy.as_deref()?;
        let proxy_status = response.headers.get(PROXY_STATUS).map(String::as_str);
        if party(Answer::Response { proxy_status }) != Party::Proxy {
            return None;
        }
        let error = proxy_status.and_then(proxy_error).unwrap_or_default();
        Some(proxy.unavailable(url, &error))
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
        let mut command = std::pin::pin!(command);
        let mut expected = None;
        loop {
            if let Some(expected) = &expected
                && let Some(outcome) = navigation.outcome(expected, wait_until, superseded())
            {
                return Ok(outcome);
            }
            tokio::select! {
                answered = &mut command, if expected.is_none() => match answered? {
                    Ok(Some(answered)) => expected = Some(answered),
                    Ok(None) => return Ok(Ok(Arrived::Document(None))),
                    Err(ended) => return Ok(Err(ended)),
                },
                event = events.next() => match event {
                    Some(event) => navigation.feed(&event),
                    None => return Err(self.browser.failed(call, &Failure::Disconnected).await),
                },
            }
        }
    }

    /// The page's document as the DOM holds it now, serialized once, with
    /// its URL and base URL.
    pub(crate) async fn document(&self) -> Result<SerializedDocument, CallError> {
        let call = Call::Document;
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
        let html = self
            .browser
            .send(
                Some(&self.session),
                &dom::GetOuterHTMLParams {
                    node_id: Some(root.node_id),
                    backend_node_id: None,
                    object_id: None,
                    include_shadow_dom: None,
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
