//! The page crate's boundary with the browser's protocol (ADR 0146): a
//! concrete type, [`CdpPage`], whose narrow primitives a Page's logic
//! calls, in ADR 0129's shape, with no trait until a second implementation
//! is built. It is the one module of the page crate that names the CDP
//! crate, as the module check holds (`[crates]` in the crate's
//! `modules.toml`): the browser's launch and the borrowed browser's
//! connection, the driver, the CDP command log, what a CDP failure raises,
//! the keys a keyboard sends and the Live view's screencast are its own. A primitive sends through
//! the page's driver, whose lease check, counted rows, Run trace and
//! deadlines it keeps.
//!
//! Each primitive is shaped so that it fits `WebDriver BiDi` and Playwright's
//! driver wherever either has a counterpart, and one that neither has is
//! marked Chromium only (ADR 0146's note of 2026-11-11). The list, one row
//! a primitive:
//!
//! | Primitive | CDP | `WebDriver BiDi` | Playwright's driver |
//! |---|---|---|---|
//! | [`CdpPage::pointer_click`]: a left click at a point of the page's viewport, as trusted input | `Input.dispatchMouseEvent` ×3: `mouseMoved`, `mousePressed` and `mouseReleased`, written as one burst | one `input.performActions` with a `pointer` source: `pointerMove`, `pointerDown` and `pointerUp` | `mouseClick` |
//! | [`CdpPage::press_keys`]: a key or a combination pressed on the focused element, as trusted input | one `Input.dispatchKeyEvent` for each key down and key up, written as one burst | one `input.performActions` with a `key` source: a `keyDown` and a `keyUp` for each key | `keyboardDown` and `keyboardUp`, or `keyboardPress` for one key |
//! | [`CdpPage::paused_events`]: what the interception follows of the page's events, each read once | `Fetch.requestPaused`, `Fetch.authRequired`, `Network.loadingFailed`, `Network.webSocketCreated`, `Page.loadEventFired`, `Page.frameStartedNavigating`, and `Page.frameScheduledNavigation` or `Page.frameRequestedNavigation` | `network.beforeRequestSent` and `network.responseStarted` while blocked, `network.authRequired`, `network.fetchError`, `browsingContext.load` and `browsingContext.navigationStarted`; a scheduled navigation and a WebSocket's opening are Chromium only | `route`, `requestFailed`, `webSocket`, and a frame's `navigated` and `loadstate`; a scheduled navigation is Chromium only |
//! | [`CdpPage::pause_requests_opening`] and [`CdpPage::pause_requests`]: every request paused before it leaves, a document's response too, and the challenges the page answers | `Fetch.enable` with a request-stage pattern, a response-stage one for documents, and `handleAuthRequests` | `network.addIntercept` at `beforeRequestSent`, `responseStarted` and `authRequired` | `setNetworkInterceptionPatterns`; a paused response is Chromium only |
//! | [`CdpPage::follow_web_sockets`]: the WebSockets the page opens reported | `Network.enable` | `session.subscribe` to the `network` module | none: the driver reports a page's WebSockets itself |
//! | [`CdpPage::continue_request`]: a paused request sent on, each given part replacing its own | `Fetch.continueRequest` | `network.continueRequest` | `Route.continue` |
//! | [`CdpPage::fulfill`]: a paused request answered with the engine's response | `Fetch.fulfillRequest` | `network.provideResponse` | `Route.fulfill` |
//! | [`CdpPage::fail`]: a paused request failed with a network error | `Fetch.failRequest` | `network.failRequest` | `Route.abort` |
//! | [`CdpPage::continue_with_auth`]: a challenge answered by default, with credentials, or cancelled | `Fetch.continueWithAuth` | `network.continueWithAuth` | none: the driver answers with the context's `httpCredentials` |
//! | [`CdpPage::continue_response`]: a paused response sent on as it came | `Fetch.continueResponse` | `network.continueResponse` | none: Chromium only, as Playwright pauses no response |
//! | [`CdpPage::continue_response_with`]: a paused response sent on with its status and the headers given in place of its own | `Fetch.continueResponse` with `responseCode` and `responseHeaders` | `network.continueResponse` with `statusCode` and `headers` | none: Chromium only, as Playwright pauses no response |
//! | [`CdpPage::isolate_opening`]: a page that sends one request alone: its scripts never run, no service worker answers it, every request pauses before it leaves, and the challenges the page answers | `Emulation.setScriptExecutionDisabled`, `Network.setBypassServiceWorker`, and `Fetch.enable` with a request-stage pattern and `handleAuthRequests` | `emulation.setScriptingEnabled` set to false, and `network.addIntercept` at `beforeRequestSent` and `authRequired`; bypassing service workers is Chromium only | a context made with `javaScriptEnabled: false` and `serviceWorkers: "block"`, and `setNetworkInterceptionPatterns` |
//! | [`CdpPage::load_document`]: a load of a URL as the main frame's document started, the browser's refusal of it answered | `Page.navigate` | `browsingContext.navigate` with `wait: "none"` | `goto` with `waitUntil: "commit"` |
//! | [`CdpPage::hop_events`]: what the browser-fetch tier follows of its page's events, each read once | `Fetch.requestPaused`, `Fetch.authRequired`, `Network.responseReceived`, `loadingFinished` and `loadingFailed` | `network.beforeRequestSent` and `network.responseStarted` while blocked, `network.authRequired`, `responseCompleted` and `fetchError` | `route`, `response`, `requestFinished` and `requestFailed`; a paused response is Chromium only |
//! | [`CdpPage::take_body_stream`], [`CdpPage::read_body_stream`] and [`CdpPage::close_body_stream`]: a paused response's body read in chunks from the browser's stream of it, after any content coding, and the stream closed | `Fetch.takeResponseBodyAsStream`, `IO.read` and `IO.close` | none: Chromium only, as `network.getData` reads a body whole | none: Chromium only, as `Route.fetch` reads a body whole |
//! | [`CdpPage::cookie_names`]: the names of the cookies a request to a URL would carry | `Network.getCookies` | `storage.getCookies` | `BrowserContext.cookies` |
//! | [`CdpPage::followed_events`] and [`CdpPage::frame_events`]: what a page's events follow of the page's own events, or of one of its out-of-process frames' requests and console, each read once | `Network.requestWillBeSent`, `responseReceived`, `dataReceived`, `loadingFinished` and `loadingFailed`, `Network.webSocketCreated`, `webSocketFrameSent`, `webSocketFrameReceived` and `webSocketClosed`, `Page.domContentEventFired`, `loadEventFired`, `frameNavigated`, `navigatedWithinDocument` and `lifecycleEvent`, `Runtime.consoleAPICalled` and `exceptionThrown` | `network.beforeRequestSent`, `responseStarted`, `responseCompleted` and `fetchError`, `browsingContext.navigationCommitted`, `fragmentNavigated`, `domContentLoaded` and `load`, `log.entryAdded`; a WebSocket's frames and a loader's lifecycle are Chromium only | `request`, `response`, `requestFinished`, `requestFailed`, `webSocket`, a frame's `navigated` and `loadstate`, `console` and `pageError` |
//! | [`CdpPage::follow_console_opening`], [`CdpPage::follow_frame_console_opening`] and [`CdpPage::follow_frame_console`]: the console calls and uncaught exceptions of the page, or of one of its out-of-process frames, reported | `Runtime.enable` | `session.subscribe` to the `log` module | none: the driver reports a page's console itself |
//! | [`CdpPage::follow_frame_network`]: an out-of-process frame's requests reported | `Network.enable` on the frame's target | `session.subscribe` to the `network` module, which covers the page's frames | none: the driver reports a frame's requests itself |
//! | [`CdpPage::response_body`]: the body of a response the browser received, once it is | `Network.getResponseBody` | `network.getData` with a data collector added before the response | `Response.body` |
//! | [`CdpPage::dialog_events`]: the JavaScript dialogs the page opens and closes, each read once | `Page.javascriptDialogOpening` and `Page.javascriptDialogClosed` | `browsingContext.userPromptOpened` and `browsingContext.userPromptClosed` | `dialog`; a dialog the page closes itself is Chromium only |
//! | [`CdpPage::answer_dialog`]: the page's open dialog accepted, with a prompt's text, or dismissed | `Page.handleJavaScriptDialog` | `browsingContext.handleUserPrompt` | `Dialog.accept` and `Dialog.dismiss` |
//! | [`CdpPage::add_init_script`] and [`CdpPage::remove_init_script`]: a script run in the main world of each document, before its own, or no more | `Page.addScriptToEvaluateOnNewDocument` and `Page.removeScriptToEvaluateOnNewDocument`, on the page's target and on each out-of-process frame's | `script.addPreloadScript` and `script.removePreloadScript` | `addInitScript`, and `dispose` on the `Disposable` it answers |
//! | [`CdpPage::consent_events`]: what the Consent prompt runner follows of one of the page's targets, each read once | `Target.detachedFromTarget`, `Page.frameNavigated`, `navigatedWithinDocument` and `frameDetached`, and `Runtime.bindingCalled` | `browsingContext.contextDestroyed`, `navigationCommitted` and `fragmentNavigated`, and `script.message` on a channel | a frame's `navigated` and `detached`, and a binding's call; a call from an isolated world is Chromium only |
//! | [`CdpPage::add_world_script_opening`]: a script run in a named isolated world of each document, before the page's own, and of those the target holds | `Page.addScriptToEvaluateOnNewDocument` with a `worldName` and `runImmediately` | `script.addPreloadScript` with a `sandbox` | none: Chromium only, as the driver runs no client's script in an isolated world |
//! | [`CdpPage::follow_frames_opening`]: an out-of-process frame's target reports its frames' navigations | `Page.enable` | `session.subscribe` to the `browsingContext` module, which covers the page's frames | none: the driver reports a frame's navigations itself |
//! | [`CdpPage::isolated_world`] and [`CdpPage::bind`]: a named isolated world in a frame's document, and a function there whose calls the engine reads | `Page.createIsolatedWorld`, and `Runtime.addBinding` named by the world's name | `script.addPreloadScript` with a `sandbox` and a `channel` argument | none: Chromium only, as the driver's `exposeBinding` binds in the main world |
//! | [`CdpPage::frame_owner_origin`]: where a frame's owner element starts in its parent's viewport, scrolled into view | `DOM.getFrameOwner`, `DOM.scrollIntoViewIfNeeded` and `DOM.getBoxModel` | `script.callFunction` on the frame's owner element, reading its bounding box | `Frame.frameElement`, then `ElementHandle.scrollIntoViewIfNeeded` and `boundingBox` |
//! | [`CdpPage::true_in_main_world`]: a snippet run on the main world's `document` of a frame whose guard world is known | `Runtime.evaluate` in the guard's world, `DOM.describeNode`, `DOM.resolveNode`, `Runtime.callFunctionOn` and `Runtime.releaseObject` | `script.callFunction` in the frame's window realm | `Frame.evaluate` |
//! | [`CdpPage::pointer_press`] and [`CdpPage::pointer_move`]: a left click, or a move, at a point of the page's viewport, as trusted input, with no deadline | `Input.dispatchMouseEvent`: ×3 written as one burst, or one `mouseMoved` | `input.performActions` with a `pointer` source | `mouseClick` and `mouseMove` |
//! | [`CdpPage::hiding_events`]: what live hiding follows of the page's events, each read once | `Page.frameNavigated` and `Page.lifecycleEvent` | `browsingContext.navigationCommitted`, `domContentLoaded` and `load` | a frame's `navigated` and `loadstate` |
//! | [`CdpPage::take_styles_opening`] and [`CdpPage::take_styles`]: the page's target, or an out-of-process frame's, takes style sheets | `DOM.enable` and `CSS.enable` | none: nothing is enabled first | none: nothing is enabled first |
//! | [`CdpPage::create_style_sheet`] and [`CdpPage::set_style_sheet`]: a style sheet no element of a document owns, which `document.styleSheets` never lists, and its text | `CSS.createStyleSheet` and `CSS.setStyleSheetText` | none: Chromium only | none: Chromium only |
//! | [`CdpPage::guard_world`] and [`CdpPage::call_in_world`]: the guard's isolated world in a frame's document, made or found by its name, and a function called there with an argument, its promise awaited and its value answered by value | `Page.createIsolatedWorld` and `Runtime.callFunctionOn` with an `executionContextId` | `script.callFunction` in a `sandbox` realm | `Frame.evaluateExpression` in the `utility` world; a world named by the engine is Chromium only |
//! | [`CdpPage::page_function_world`] and [`CdpPage::evaluate`]: a page function evaluated in the main frame's main world, or in the page functions' isolated world, its promise awaited and its value answered by value | `Page.createIsolatedWorld` and `Runtime.evaluate` | `script.evaluate` in the window realm or a `sandbox` | `Frame.evaluateExpression` in the `main` or `utility` world |
//! | [`CdpPage::serialized_document`]: the page's document as the DOM holds it, serialized once with every shadow root, and its URL and base URL | `DOM.getDocument` and `DOM.getOuterHTML` with `includeShadowDOM` | `script.callFunction` reading `getHTML` with `serializableShadowRoots` | `Frame.content`; closed shadow roots are Chromium only |
//! | [`CdpPage::document_url`]: the URL of the main frame's document, or the one it could not reach | `Page.getFrameTree` | `browsingContext.getTree` | a frame's `url`; the URL behind the browser's error page is Chromium only |
//! | [`CdpPage::insert_text`]: text inserted where the focused element's selection is, as an input method commits it | `Input.insertText` | `input.performActions` with a `key` source for each character | `keyboardInsertText` |
//! | [`CdpPage::history_entry`]: the URL of the entry some steps away in the page's history, and the command that goes there | `Page.getNavigationHistory` | none: Chromium only, as `browsingContext.traverseHistory` takes the step itself | none: Chromium only, as `goBack` and `goForward` take the step themselves |
//! | [`CdpPage::navigation_events`] and [`CdpPage::navigate`]: a navigation to a URL, a reload or a step in the history, followed until it ends or reaches the state it waits for, a renderer that crashed under it told apart | `Page.navigate`, `Page.reload` or `Page.navigateToHistoryEntry`, then `Page.frameStartedNavigating`, `frameRequestedNavigation`, `frameNavigated`, `navigatedWithinDocument`, `frameStoppedLoading`, `lifecycleEvent` and `downloadWillBegin`, `Network.requestWillBeSent`, `responseReceived`, `dataReceived`, `loadingFinished` and `loadingFailed`, `Inspector.targetCrashed`, and `Page.getFrameTree` to tell a crashed renderer | `browsingContext.navigate`, `reload` or `traverseHistory` with `wait`, and the `network` and `browsingContext` events | `goto`, `reload`, `goBack` and `goForward` with `waitUntil` |
//! | [`CdpPage::document_encoding`]: the encoding a world's document was decoded with | `Runtime.evaluate` in the world | `script.evaluate` in a `sandbox` realm | `Frame.evaluateExpression` in the `utility` world |
//! | [`CdpPage::child_events`], [`CdpPage::attach_children_opening`] and [`CdpPage::attach_children`]: the page's out-of-process frames and dedicated workers, and each frame's own, attached before they run and told when they go | `Target.setAutoAttach` with `waitForDebuggerOnStart` and a filter, `Target.attachedToTarget` and `detachedFromTarget` | `browsingContext.contextCreated` and `contextDestroyed`, `script.realmCreated` and `realmDestroyed`; a child held before it runs is Chromium only | a frame's `attached` and `detached`, and `worker`; a child held before it runs is Chromium only |
//! | [`CdpPage::let_run`]: a child held before it runs let run | `Runtime.runIfWaitingForDebugger` | none: Chromium only | none: Chromium only |
//! | [`CdpPage::memory_held`]: what the page holds in memory, its JavaScript heap, its embedder's and its buffers | `Runtime.getHeapUsage` | none: Chromium only | none: Chromium only |

pub(crate) mod borrowed;
mod children;
mod command_log;
mod consent;
mod dialogs;
pub(crate) mod driver;
mod failures;
mod followed;
mod functions;
mod hiding;
mod hop;
mod init_scripts;
pub(crate) mod keys;
pub(crate) mod launch;
pub(crate) mod live_view;
mod memory;
mod navigation;
mod paused;
mod reads;
mod requests;
mod runtime_values;

use goodfetch_cdp::Command as _;
use goodfetch_engine::CallError;

pub(crate) use goodfetch_cdp::SessionId as TargetSession;
/// One key event a keyboard sends, as [`keys::events`] spells a key.
pub(crate) use goodfetch_cdp::generated::input::DispatchKeyEventParams as KeyEvent;
/// The browser's ID of a target: a page, a frame or a worker.
pub(crate) use goodfetch_cdp::generated::target::TargetID as TargetId;

/// The command that opens a page's target, which an ended lease withholds
/// before it leaves (ADR 0087).
pub(crate) const CREATE_TARGET: &str = goodfetch_cdp::generated::target::CreateTargetParams::METHOD;

pub(crate) use self::children::{AttachedChild, ChildEvent, ChildEvents, ChildKind};
pub(crate) use self::consent::{ConsentEvent, ConsentEvents};
pub(crate) use self::dialogs::{DialogEvent, DialogEvents, OpenedDialog};
use self::driver::{PageDriver, pointer_click};
pub(crate) use self::followed::{
    FollowedEvent, FollowedEvents, FrameNavigated, RequestSent, SocketFrame,
};
pub(crate) use self::hiding::{HidingEvent, HidingEvents};
pub(crate) use self::hop::{HopEvent, HopEvents};
pub(crate) use self::init_scripts::InitScriptId;
pub(crate) use self::navigation::Going;
pub(crate) use self::paused::{
    AuthAnswer, AuthChallenge, InterceptionEvent, InterceptionEvents, LoadingFailed, NetworkId,
    NotTaken, PausedRequest,
};
pub(crate) use self::reads::SerializedDocument;
pub(crate) use self::requests::{ContinueRequest, Fulfill, Header, PausedId, RequestHeaders};
use crate::errors::Call;

/// One page of a browser as the page's logic sees the protocol: the
/// primitives it calls on that page.
#[derive(Clone)]
pub(crate) struct CdpPage {
    /// The page's driver, as the call that sends counts and checks its
    /// commands.
    page: PageDriver,
}

impl CdpPage {
    /// The page `page` drives, its commands counted, leased and traced as
    /// `page`'s are.
    pub(crate) fn of(page: &PageDriver) -> Self {
        Self { page: page.clone() }
    }

    /// Clicks at (`x`, `y`) in the page's viewport with the pointer's left
    /// button: the pointer moves there, presses and releases, input the
    /// page receives as trusted. One burst, which CDP sends as three events,
    /// none waiting for the answer to another, all within `call`'s deadline
    /// and raising its codes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it, so it
    /// leaves no button down.
    pub(crate) async fn pointer_click(&self, x: f64, y: f64, call: &Call) -> Result<(), CallError> {
        self.page.burst(&pointer_click(x, y), call).await
    }

    /// Sends `events`, one key press's from the first key down to the last
    /// key up, to the focused element, input the page receives as trusted:
    /// one burst, as [`pointer_click`](Self::pointer_click)'s.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it, so it
    /// leaves no key down.
    pub(crate) async fn press_keys(
        &self,
        events: &[KeyEvent],
        call: &Call,
    ) -> Result<(), CallError> {
        self.page.burst(events, call).await
    }
}
