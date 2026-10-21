//! The driver (spec #577): a Session's pages driven over CDP. The
//! Session-level driver of ADR 0129 owns what every browser behind a
//! Session shares (its browser context, its pages, the commands they send,
//! each with its timeout and its record in the Run trace) and asks its
//! browser for one primitive, `connect()`, which yields the browser's CDP
//! connection: a browser the engine launched fills it here, a Browser host
//! later. `Runtime` is never enabled, and the guard's script lives in an
//! isolated world no page script reaches (ADR 0019).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::{browser, input, network, page, runtime, target};
use goodfetch_cdp::{Command, Connection, Failure, Refusal, SessionId as TargetSession};
use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::Pins;
use goodfetch_engine::home::Home;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{BrowserExit, PageResponse};
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use tokio::sync::{Mutex, MutexGuard, OnceCell, watch};

use crate::errors::{Call, call_error, network_error, renderer_crashed};
use crate::launch::launch;
use crate::trace::CommandLog;

/// How long one CDP command may go unanswered.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a lost browser's exit status is waited for, to name it.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// The isolated world of the guard and every engine script (ADR 0019).
pub(crate) const GUARD_WORLD: &str = "goodfetch-guard";

/// What the guard's world holds before the guard does: a marker, which no
/// page script can read, as the Proving ground's CDP probe checks.
const GUARD_MARKER: &str = "globalThis.__goodfetch_guard__ = true;";

/// The event of a target whose renderer crashed.
const CRASHED: &str = "Inspector.targetCrashed";

/// What a navigation lost before its `load` event.
enum Lost {
    /// The page's renderer crashed.
    Renderer,
    /// The browser's connection ended.
    Browser,
}

/// A browser's CDP connection as the driver uses it.
#[derive(Clone)]
pub(crate) struct Browser {
    connection: Connection,
    log: CommandLog,
    exit: watch::Receiver<Option<BrowserExit>>,
    executable: String,
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
                let browser = Browser {
                    connection: launched.connection,
                    log,
                    exit: launched.exit,
                    executable: launched.executable.display().to_string(),
                };
                // The first answer says the browser is up.
                browser
                    .send(None, &browser::GetVersionParams {}, &browser.opening())
                    .await?;
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
            // A page target's main frame bears the target's ID.
            frame: created.target_id,
            navigation_timeout,
            turn: Arc::new(Mutex::new(None)),
        })
    }
}

/// One page's target, driven over its CDP session.
#[derive(Clone)]
pub(crate) struct PageDriver {
    browser: Browser,
    session: TargetSession,
    frame: page::FrameId,
    navigation_timeout: Duration,
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
        let context = if let Some(context) = *self.world {
            context
        } else {
            let made = self
                .send(&page::CreateIsolatedWorldParams {
                    frame_id: self.page.frame.clone(),
                    world_name: Some(GUARD_WORLD.to_owned()),
                    grant_univeral_access: None,
                    content_security_policy: None,
                })
                .await?;
            *self.world = Some(made.execution_context_id);
            made.execution_context_id
        };
        let called = self
            .send(&runtime::CallFunctionOnParams {
                function_declaration: function.to_owned(),
                object_id: None,
                arguments: Some(vec![runtime::CallArgument {
                    value: Some(argument),
                    unserializable_value: None,
                    object_id: None,
                }]),
                silent: Some(true),
                return_by_value: Some(true),
                generate_preview: None,
                user_gesture: None,
                await_promise: Some(true),
                execution_context_id: Some(context),
                object_group: None,
                throw_on_side_effect: None,
                unique_context_id: None,
                serialization_options: None,
            })
            .await?;
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

impl PageDriver {
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

    /// Goes to `url` and waits for its `load` event, answering the
    /// document's response, or none for a navigation within the document or
    /// to a URL no server answers.
    ///
    /// A navigation is a gesture: it waits for the page's turn and holds it
    /// until its `load` event.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the navigation to go on in the browser.
    pub(crate) async fn goto(&self, url: &str) -> Result<Option<PageResponse>, CallError> {
        let mut world = self.turn.lock().await;
        // The guard's world belongs to the document the navigation leaves.
        *world = None;
        let call = Call::Navigation {
            url: url.to_owned(),
        };
        let mut events = self.browser.connection.subscribe(Some(&self.session)).await;
        let navigated = self
            .browser
            .send(
                Some(&self.session),
                &page::NavigateParams {
                    url: url.to_owned(),
                    referrer: None,
                    transition_type: None,
                    frame_id: None,
                    referrer_policy: None,
                },
                &call,
            )
            .await?;
        if let Some(error) = navigated.error_text {
            return Err(network_error(url, &error));
        }
        let Some(loader) = navigated.loader_id else {
            return Ok(None);
        };
        let waited = tokio::time::timeout(self.navigation_timeout, async {
            let mut response = None;
            while let Some(event) = events.next().await {
                if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
                    if received.loader_id == loader
                        && received.frame_id.as_ref() == Some(&navigated.frame_id)
                        && received.r#type == network::ResourceType::Document
                    {
                        response = Some(response_of(&received.response));
                    }
                } else if let Some(lifecycle) = event.read::<page::LifecycleEventEvent>() {
                    if lifecycle.loader_id == loader && lifecycle.name == "load" {
                        return Ok(response);
                    }
                } else if event.method == CRASHED {
                    return Err(Lost::Renderer);
                }
            }
            Err(Lost::Browser)
        })
        .await;
        match waited {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(Lost::Renderer)) => Err(renderer_crashed()),
            Ok(Err(Lost::Browser)) => Err(self.browser.failed(&call, &Failure::Disconnected).await),
            Err(_) => Err(network_error(
                url,
                &format!("no load event within `{}`", TIMEOUT_NAVIGATION.name),
            )),
        }
    }
}

/// A CDP response as the protocol's, its header names lower-cased.
fn response_of(response: &network::Response) -> PageResponse {
    let headers: BTreeMap<String, String> = response
        .headers
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            (name.to_ascii_lowercase(), value)
        })
        .collect();
    PageResponse {
        url: response.url.clone(),
        status: u16::try_from(response.status).unwrap_or_default(),
        status_text: response.status_text.clone(),
        headers,
    }
}
