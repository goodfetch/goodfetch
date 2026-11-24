//! The page crate's boundary with the browser's protocol (ADR 0146): a
//! concrete type, [`CdpPage`], whose narrow primitives a Page's logic
//! calls, in ADR 0129's shape, with no trait until a second implementation
//! is built. Spec #2256 moves the page crate's calls behind it ticket by
//! ticket, until this is the one module that names the CDP crate; the
//! interception's and a Page's events' calls are behind it already. A
//! primitive sends through the page's driver, whose lease check, counted
//! rows, Run trace and deadlines it keeps.
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
//! | [`CdpPage::cookie_names`]: the names of the cookies a request to a URL would carry | `Network.getCookies` | `storage.getCookies` | `BrowserContext.cookies` |
//! | [`CdpPage::followed_events`] and [`CdpPage::frame_events`]: what a page's events follow of the page's own events, or of one of its out-of-process frames' requests and console, each read once | `Network.requestWillBeSent`, `responseReceived`, `dataReceived`, `loadingFinished` and `loadingFailed`, `Network.webSocketCreated`, `webSocketFrameSent`, `webSocketFrameReceived` and `webSocketClosed`, `Page.domContentEventFired`, `loadEventFired`, `frameNavigated`, `navigatedWithinDocument` and `lifecycleEvent`, `Runtime.consoleAPICalled` and `exceptionThrown` | `network.beforeRequestSent`, `responseStarted`, `responseCompleted` and `fetchError`, `browsingContext.navigationCommitted`, `fragmentNavigated`, `domContentLoaded` and `load`, `log.entryAdded`; a WebSocket's frames and a loader's lifecycle are Chromium only | `request`, `response`, `requestFinished`, `requestFailed`, `webSocket`, a frame's `navigated` and `loadstate`, `console` and `pageError` |
//! | [`CdpPage::follow_console_opening`], [`CdpPage::follow_frame_console_opening`] and [`CdpPage::follow_frame_console`]: the console calls and uncaught exceptions of the page, or of one of its out-of-process frames, reported | `Runtime.enable` | `session.subscribe` to the `log` module | none: the driver reports a page's console itself |
//! | [`CdpPage::follow_frame_network`]: an out-of-process frame's requests reported | `Network.enable` on the frame's target | `session.subscribe` to the `network` module, which covers the page's frames | none: the driver reports a frame's requests itself |
//! | [`CdpPage::response_body`]: the body of a response the browser received, once it is | `Network.getResponseBody` | `network.getData` with a data collector added before the response | `Response.body` |

mod followed;
mod paused;
mod requests;
mod runtime_values;

use goodfetch_cdp::generated::input::DispatchKeyEventParams;
use goodfetch_engine::CallError;

pub(crate) use goodfetch_cdp::{NetError, SessionId as TargetSession};

pub(crate) use self::followed::{
    FollowedEvent, FollowedEvents, FrameNavigated, RequestSent, SocketFrame,
};
pub(crate) use self::paused::{
    AuthAnswer, AuthChallenge, InterceptionEvent, InterceptionEvents, LoadingFailed, NetworkId,
    PausedRequest,
};
pub(crate) use self::requests::{ContinueRequest, Fulfill, Header, PausedId, RequestHeaders};
use crate::driver::{PageDriver, pointer_click};
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
        events: &[DispatchKeyEventParams],
        call: &Call,
    ) -> Result<(), CallError> {
        self.page.burst(events, call).await
    }
}
