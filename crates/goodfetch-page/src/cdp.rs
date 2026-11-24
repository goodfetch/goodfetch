//! The page crate's boundary with the browser's protocol (ADR 0146): a
//! concrete type, [`CdpPage`], whose narrow primitives a Page's logic
//! calls, in ADR 0129's shape, with no trait until a second implementation
//! is built. Spec #2256 moves the page crate's calls behind it ticket by
//! ticket, until this is the one module that names the CDP crate; until
//! then a primitive sends through the page's driver, whose lease check,
//! counted rows, Run trace and deadlines it keeps.
//!
//! Each primitive is shaped so that it fits `WebDriver BiDi` and Playwright's
//! driver wherever either has a counterpart, and one that neither has is
//! marked Chromium only (ADR 0146's note of 2026-11-11). The list, one row
//! a primitive:
//!
//! | Primitive | CDP | `WebDriver BiDi` | Playwright's driver |
//! |---|---|---|---|
//! | [`CdpPage::pointer_click`]: a left click at a point of the page's viewport, as trusted input | `Input.dispatchMouseEvent` ×3: `mouseMoved`, `mousePressed` and `mouseReleased` | one `input.performActions` with a `pointer` source: `pointerMove`, `pointerDown` and `pointerUp` | `mouseClick` |
//! | [`CdpPage::paused_events`]: what the interception follows of the page's events, each read once | `Fetch.requestPaused`, `Fetch.authRequired`, `Network.loadingFailed`, `Network.webSocketCreated`, `Page.loadEventFired`, `Page.frameStartedNavigating`, and `Page.frameScheduledNavigation` or `Page.frameRequestedNavigation` | `network.beforeRequestSent` and `network.responseStarted` while blocked, `network.authRequired`, `network.fetchError`, `browsingContext.load` and `browsingContext.navigationStarted`; a scheduled navigation and a WebSocket's opening are Chromium only | `route`, `requestFailed`, `webSocket`, and a frame's `navigated` and `loadstate`; a scheduled navigation is Chromium only |
//! | [`CdpPage::pause_requests_opening`] and [`CdpPage::pause_requests`]: every request paused before it leaves, a document's response too, and the challenges the page answers | `Fetch.enable` with a request-stage pattern, a response-stage one for documents, and `handleAuthRequests` | `network.addIntercept` at `beforeRequestSent`, `responseStarted` and `authRequired` | `setNetworkInterceptionPatterns`; a paused response is Chromium only |
//! | [`CdpPage::follow_web_sockets`]: the WebSockets the page opens reported | `Network.enable` | `session.subscribe` to the `network` module | none: the driver reports a page's WebSockets itself |
//! | [`CdpPage::continue_request`]: a paused request sent on, each given part replacing its own | `Fetch.continueRequest` | `network.continueRequest` | `Route.continue` |
//! | [`CdpPage::fulfill`]: a paused request answered with the engine's response | `Fetch.fulfillRequest` | `network.provideResponse` | `Route.fulfill` |
//! | [`CdpPage::fail`]: a paused request failed with a network error | `Fetch.failRequest` | `network.failRequest` | `Route.abort` |
//! | [`CdpPage::continue_with_auth`]: a challenge answered by default, with credentials, or cancelled | `Fetch.continueWithAuth` | `network.continueWithAuth` | none: the driver answers with the context's `httpCredentials` |
//! | [`CdpPage::continue_response`]: a paused response sent on as it came | `Fetch.continueResponse` | `network.continueResponse` | none: Chromium only, as Playwright pauses no response |
//! | [`CdpPage::cookie_names`]: the names of the cookies a request to a URL would carry | `Network.getCookies` | `storage.getCookies` | `BrowserContext.cookies` |

mod paused;
mod requests;

use goodfetch_engine::CallError;

pub(crate) use goodfetch_cdp::NetError;

pub(crate) use self::paused::{
    AuthAnswer, AuthChallenge, InterceptionEvent, InterceptionEvents, LoadingFailed, NetworkId,
    PausedRequest, is_navigation, post_data, resource_type,
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
    /// page receives as trusted. One primitive for the whole burst, which
    /// CDP sends as three events, each within `call`'s deadline and raising
    /// its codes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends none of the burst's later events; an event
    /// already written is the browser's.
    pub(crate) async fn pointer_click(&self, x: f64, y: f64, call: &Call) -> Result<(), CallError> {
        for params in pointer_click(x, y) {
            self.page.send(&params, call).await?;
        }
        Ok(())
    }
}
