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
//! marked Chromium only (ADR 0146's note of 2026-11-11). A primitive is
//! anything of the module the rest of the crate calls that sends the
//! browser a command or reads its events: [`CdpPage`]'s, and the driver's
//! own. What sends nothing takes no row: [`CdpPage::of`],
//! [`CdpPage::session`] and [`CdpPage::frame`] name a page, its target's
//! session or one of its frames, and [`CdpPage::counting`],
//! [`CdpPage::work`], [`CdpPage::main_frame`] and [`CdpPage::raised`] read
//! what the driver holds. The Playwright column names what its driver's
//! protocol offers a client. The list, one row a primitive:
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
//! | [`CdpPage::followed_events`] and [`CdpPage::frame_events`]: what a page's events follow of the page's own events, or of one of its out-of-process frames' requests and console, each read once | `Network.requestWillBeSent`, `responseReceived`, `dataReceived`, `loadingFinished` and `loadingFailed`, `Network.webSocketCreated`, `webSocketFrameSent`, `webSocketFrameReceived` and `webSocketClosed`, `Page.domContentEventFired`, `loadEventFired`, `frameNavigated`, `navigatedWithinDocument` and `lifecycleEvent`, `Runtime.consoleAPICalled` and `exceptionThrown` | `network.beforeRequestSent`, `responseStarted`, `responseCompleted` and `fetchError`, `browsingContext.navigationCommitted`, `fragmentNavigated`, `domContentLoaded` and `load`, `log.entryAdded`; a WebSocket's frames and a loader's lifecycle are Chromium only | `request`, `response`, `requestFinished`, `requestFailed`, `webSocket` and its `frameSent`, `frameReceived` and `close`, a frame's `navigated` and `loadstate`, `console` and `pageError` |
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
//! | [`CdpPage::serialized_document`]: the page's document as the DOM holds it, serialized once with every shadow root, and its URL and base URL | `DOM.getDocument` and `DOM.getOuterHTML` with `includeShadowDOM` | `script.callFunction` reading `getHTML` with the `shadowRoots` that a node's serialization with `includeShadowTree: "all"` reaches, closed ones included | `Frame.content` with `includeShadow`, which reads `element.shadowRoot` and so misses closed roots: closed shadow roots are Chromium only |
//! | [`CdpPage::document_url`]: the URL of the main frame's document, or the one it could not reach | `Page.getFrameTree` | `browsingContext.getTree` | a frame's `url`; the URL behind the browser's error page is Chromium only |
//! | [`CdpPage::insert_text`]: text inserted where the focused element's selection is, as an input method commits it | `Input.insertText` | `input.performActions` with a `key` source for each character | `keyboardInsertText` |
//! | [`CdpPage::history_entry`]: the URL of the entry some steps away in the page's history, and the command that goes there | `Page.getNavigationHistory` | none: Chromium only, as `browsingContext.traverseHistory` takes the step itself | none: Chromium only, as `goBack` and `goForward` take the step themselves |
//! | [`CdpPage::navigation_events`] and [`CdpPage::navigate`]: a navigation to a URL, a reload or a step in the history, followed until it ends or reaches the state it waits for, a renderer that crashed under it told apart | `Page.navigate`, `Page.reload` or `Page.navigateToHistoryEntry`, then `Page.frameStartedNavigating`, `frameRequestedNavigation`, `frameNavigated`, `navigatedWithinDocument`, `frameStoppedLoading`, `lifecycleEvent` and `downloadWillBegin`, `Network.requestWillBeSent`, `responseReceived`, `dataReceived`, `loadingFinished` and `loadingFailed`, `Inspector.targetCrashed`, and `Page.getFrameTree` to tell a crashed renderer | `browsingContext.navigate`, `reload` or `traverseHistory` with `wait`, and the `network` and `browsingContext` events | `goto`, `reload`, `goBack` and `goForward` with `waitUntil` |
//! | [`CdpPage::document_encoding`]: the encoding a world's document was decoded with | `Runtime.evaluate` in the world | `script.evaluate` in a `sandbox` realm | `Frame.evaluateExpression` in the `utility` world |
//! | [`CdpPage::child_events`], [`CdpPage::attach_children_opening`] and [`CdpPage::attach_children`]: the page's out-of-process frames and dedicated workers, and each frame's own, attached before they run and told when they go | `Target.setAutoAttach` with `waitForDebuggerOnStart` and a filter, `Target.attachedToTarget` and `detachedFromTarget` | `browsingContext.contextCreated` and `contextDestroyed`, `script.realmCreated` and `realmDestroyed`; a child held before it runs is Chromium only | a frame's `attached` and `detached`, and `worker`; a child held before it runs is Chromium only |
//! | [`CdpPage::let_run`]: a child held before it runs let run | `Runtime.runIfWaitingForDebugger` | none: Chromium only | none: Chromium only |
//! | [`CdpPage::memory_held`]: what the page holds in memory, its JavaScript heap, its embedder's and its buffers | `Runtime.getHeapUsage` | none: Chromium only | none: Chromium only |
//! | [`driver::SessionDriver::open`]: the Session's browser context, made in its browser once the engine connected, or a Borrowed browser's default one; on connecting, the browser holds each page and worker that opens before it runs, until the engine lets it run, and a headless build's window identity is read in a page in the background | `Browser.getVersion`, a launched headless build's page: `Target.createTarget`, `Target.attachToTarget`, `Page.enable`, `Page.setLifecycleEventsEnabled`, `Page.navigate`, `Runtime.evaluate` and `Target.closeTarget`, then `Target.setAutoAttach` on the browser with `waitForDebuggerOnStart` and a filter of pages and workers; then `Target.getBrowserContexts` or `Target.createBrowserContext` | `session.new`, and the `default` user context or `browser.createUserContext`; a target held before it runs and the window identity are Chromium only | `BrowserType.launch`, or `connectOverCDP` and its `defaultContext`, and `Browser.newContext`; a target held before it runs is the driver's own |
//! | [`driver::SessionDriver::close`]: the Session's browser context closed, its hidden pages first, then the context disposed; on a Borrowed browser, only the pages the Session opened close, and its default context stays | `Target.closeTarget` for each hidden page, or each page the Session opened, and `Target.disposeBrowserContext` | `browsingContext.close` for each page the Session opened, and `browser.removeUserContext` | `Page.close` for each page the Session opened, and `BrowserContext.close` |
//! | [`driver::SessionDriver::new_page`]: a page of the Session's context, shown or hidden, prepared before it runs: focused, with a headless build's window identity, its frames' lifecycle and its network reported, and the guard's world's marker held; its target closed if preparing it fails | `Target.createTarget` with `hidden` and `Target.attachToTarget`, then `Emulation.setFocusEmulationEnabled`, `Emulation.setUserAgentOverride` with the client hints the build gives, `Page.enable`, `Page.setLifecycleEventsEnabled`, `Network.enable` and `Page.addScriptToEvaluateOnNewDocument` with a `worldName`; `Target.closeTarget` if preparing fails | `browsingContext.create` with a `userContext`; a hidden page, focus emulation and client hints are Chromium only, `emulation.setUserAgentOverride` setting the string alone | `BrowserContext.newPage`, the driver emulating focus itself and the context's `userAgent` taking client hints the driver derives from the string; a hidden page is Chromium only |
//! | [`driver::PageDriver::reading`]: the page's reader, which hands each event of the page's targets on to the page's parts, each read once, and, for a Session that carries its storage, keeps the origin of each document its frames show | every event of the page's target's session and of its children's, and `Page.frameNavigated`'s `securityOrigin` | the events of the page's context that `session.subscribe` reports, and `browsingContext.navigationCommitted`'s `url` | the page's and its frames' events, and a frame's `navigated` URL |
//! | [`driver::PageDriver::call_in`]: a function called with an argument in a world of the page's target or of one of its frames' targets, its value answered, never through `Runtime.enable` | `Runtime.callFunctionOn` with an `executionContextId` | `script.callFunction` in a `sandbox` realm | `Frame.evaluateExpression` in the `utility` world; a world named by the engine is Chromium only |
//! | [`driver::PageDriver::close`] and [`driver::PageDriver::close_running_before_unload`]: the page closed, or asked to close as a person closing it does, its `beforeunload` handlers run first | `Target.closeTarget`, or `Page.close` | `browsingContext.close`, with `promptUnload` set to true | `Page.close`, or `Page.runBeforeUnload` |
//! | [`driver::PageDriver::lost`]: how the page's target ended without the engine closing it: its renderer crashed, its target was destroyed, or its browser went | `Inspector.targetCrashed` and `Inspector.detached` on the page's session, and the connection's end | `browsingContext.contextDestroyed` and the session's end; a renderer's crash told apart is Chromium only | the page's `crash` and `close`, and the browser's `close` |
//! | [`driver::LocalBrowser::close`], [`driver::BorrowedBrowser::stop`] and [`driver::Behind::stop`]: a launched browser closed, then killed if it does not close, or the engine's connection to a Borrowed one ended, the browser staying open | `Browser.close`, or the connection's end | `browser.close`, or `session.end` | `Browser.close`, which on a `connectOverCDP` browser ends the connection alone |
//! | [`driver::LocalBrowser::ended`], [`driver::BorrowedBrowser::ended`] and [`driver::LocalBrowser::root_fell_behind`]: how a browser ended once its connection did: it quit, it crashed, or the engine closed it; and the reader of a launched browser's own events fell behind, its events read again from then | the connection's end and a launched browser's exit status, and the browser's own events, on no session | the session's end | the browser's `close` |
//! | [`driver::SessionDriver::capture`] and [`driver::SessionDriver::restore`]: the Session's cookies and each origin's `localStorage`, read from a browser being recycled and written into its replacement, each origin's in a page that commits an empty document of that origin the engine answers itself | `Storage.getCookies` and `Storage.setCookies` with the context's ID, then for each origin `Target.createTarget`, `Target.attachToTarget`, `Fetch.enable`, `Page.enable`, `Page.navigate`, `Fetch.fulfillRequest`, `Page.createIsolatedWorld`, `Runtime.callFunctionOn` and `Target.closeTarget` | `storage.getCookies` and `storage.setCookie` with the user context's partition, then for each origin `browsingContext.create`, `network.addIntercept`, `browsingContext.navigate`, `network.provideResponse`, `script.callFunction` in a `sandbox` and `browsingContext.close` | `BrowserContext.storageState` and `setStorageState` |
//! | [`driver::PageDriver::follow_popups`]: each page the context's pages open, which the browser holds before it runs ([`driver::SessionDriver::open`]'s row), prepared as a new page is and handed to its Session, then let run, or closed before it runs when the Session does not take it in; any other page is let run and let go of | `Target.attachedToTarget` with `waitingForDebugger`, the commands that prepare a page in [`driver::SessionDriver::new_page`]'s row, `Runtime.runIfWaitingForDebugger`, and `Target.closeTarget`, or `Target.detachFromTarget` for any other page | `browsingContext.contextCreated` with its `originalOpener`; a popup held before it runs is Chromium only, a global intercept and its user context's preload scripts covering its first request instead | the context's `page` event, for a page whose opener is set, the context's routes covering its first request |
//! | [`driver::PageDriver::service_workers`] and [`driver::PageDriver::shared_workers`]: the service workers and shared workers of the context's sites, which the browser holds before they run ([`driver::SessionDriver::open`]'s row), handed to the Session's interception and let run, or, for service workers the context blocks, their scripts' fetch failed | for a context that blocks service workers, `Fetch.enable` on the browser, then for each worker's script `Fetch.requestPaused`, `Target.getTargetInfo`, and `Fetch.continueRequest` or `Fetch.failRequest`; for each worker, `Target.attachedToTarget` with `waitingForDebugger`, then `Network.setUserAgentOverride` on a headless build and `Runtime.runIfWaitingForDebugger`, or `Target.closeTarget` | `script.realmCreated` with a `service-worker` or `shared-worker` realm, once it runs; a worker held before it runs and its requests intercepted are Chromium only, the `network` module defining no service worker's fields yet | `serviceWorker`, whose requests the context's routes cover, and the context's `serviceWorkers` set to `"block"`, which replaces `register` in an init script; a shared worker is Chromium only, as the driver detaches from it |
//! | [`driver::PageDriver::watch_gesture`] and [`driver::PageDriver::settle_gesture`]: a locator action's wait for the navigation of the main frame its input requested, until its document committed and the browser finished it | the navigation events of [`CdpPage::navigate`]'s row, read from before the input, and `Runtime.getIsolateId` as a round trip into the renderer once the navigation ended | `browsingContext.navigationStarted`, `navigationCommitted` and `load` of the context the input reached; a round trip into the renderer is Chromium only | none: the driver's input actions wait for the navigations they start themselves |
//! | [`driver::cdp_response::whole_body`]: a paused response's whole body, after any content coding | `Fetch.getResponseBody` | none: Chromium only, as `network.getData` reads a body once it is collected, not while its response is held | none: Chromium only, as Playwright pauses no response |

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

/// The browser's ID of a frame.
pub(crate) type FrameId = String;

/// The browser's ID of a world in a document.
pub(crate) type ExecutionContextId = i64;

/// The browser's ID of a page's target, which is its main frame's ID too:
/// a popup names its opener by it. Held and compared, never read.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TargetId(goodfetch_cdp::generated::target::TargetID);

/// The key events that press one key or combination, from the first key
/// down to the last key up, as [`keys::events`] spells them: sent whole by
/// [`CdpPage::press_keys`], never read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KeyPress(Vec<goodfetch_cdp::generated::input::DispatchKeyEventParams>);

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

    /// Sends `press`, from its first key down to its last key up, to the
    /// focused element, input the page receives as trusted: one burst, as
    /// [`pointer_click`](Self::pointer_click)'s.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends all of the burst or none of it, so it
    /// leaves no key down.
    pub(crate) async fn press_keys(&self, press: &KeyPress, call: &Call) -> Result<(), CallError> {
        self.page.burst(&press.0, call).await
    }
}
