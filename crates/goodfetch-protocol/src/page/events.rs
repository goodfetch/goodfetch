//! A page's events and the bodies of its responses (ADRs 0081, 0085, 0118
//! and 0121): `page.on` listens to one of a page's events under Playwright's
//! name, spelled in `snake_case` words on the wire (ADR 0099), as a call that
//! stays in flight until the Client cancels it, as a `Subscription`'s close
//! does, or the page goes. While it listens, the engine sends that event of
//! the page on the call itself, in `$/events` batches naming it, under the
//! credits its Client grants it (ADR 0024): only to the Connection that
//! armed it, never on the engine's shared event stream, and lossless, or
//! the call raises `page_events_lost`. The page's console and uncaught
//! exceptions are read only while one listens to them. A `Request` is one
//! hop of a redirect chain, named by its page and the engine's ID for that
//! hop, which `response.body` takes: the body is read on demand, kept
//! already for a response `page.wait_for_request_finished` answered, and
//! otherwise read from the browser, which may have dropped it
//! (`body_unavailable`, as for a redirect's response, which has none); a
//! body that broke off raises `response_truncated`, never returned as a
//! whole one. A service worker's own requests are never a page's events;
//! the Run trace records them (ADR 0121).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::route::UrlMatcher;
use super::{ARRIVED_IN, BodyHandle, PAGE_DOWNLOAD, PageId, PageResponse};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation, OperationClass, OperationKind,
    Reach, Stability, schema_of,
};
use crate::jsonrpc::ErrorData;

/// A page's event a program listens to, by its words (ADR 0099): each
/// SDK joins them as Playwright spells the event, such as `pageerror`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PageEventName {
    /// `page.console`.
    Console,
    /// `page.page_error`.
    PageError,
    /// `page.request`.
    Request,
    /// `page.request_finished`.
    RequestFinished,
    /// `page.request_failed`.
    RequestFailed,
    /// `page.response`.
    Response,
    /// `page.web_socket`, with each socket's `web_socket.*` events.
    WebSocket,
    /// `page.load`.
    Load,
    /// `page.dom_content_loaded`.
    DomContentLoaded,
    /// `page.crash`.
    Crash,
    /// `page.close`.
    Close,
    /// `page.popup`.
    Popup,
}

/// What `page.on` takes: the page and the event listened to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageOnParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The event.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: PageEventName,
}

/// What `page.on` answers when its page goes, closed or with its Run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageOnEnded {}

/// Where a console message was written: its script's URL and position, as
/// the browser counts them from 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleLocation {
    /// The script's URL; empty for a script the page evaluated.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The line, from 0.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub line_number: u64,
    /// The column, from 0.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub column_number: u64,
}

/// What `page.console` reports: a message the page wrote to its console.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleMessage {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// Its type as Playwright names it, such as `log`, `warning` or
    /// `error`.
    #[serde(rename = "type")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// Its text: each argument, a primitive as its value and any other as
    /// the browser describes it, joined by spaces.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// Where it was written.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub location: ConsoleLocation,
}

/// What `page.page_error` reports, `WebError`: an exception the page did
/// not catch (ADR 0085), never a `PageError`, ADR 0079's group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebError {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// Its name, such as `TypeError`; `Error` for a thrown value that is no
    /// error.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its message.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// Its stack as the browser renders it, when it has one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stack: Option<String>,
}

/// Why a request failed, as Playwright's `request.failure()` gives it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestFailure {
    /// The browser's network error, such as `net::ERR_FAILED` for a request
    /// a route aborted or `net::ERR_BLOCKED_BY_CLIENT` for one the engine
    /// blocked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error_text: String,
    /// The Error code the engine failed it with, with its details, when the
    /// engine did, such as `egress_refused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: Option<ErrorData>,
}

/// A request of a page, ADR 0085's `Request`: one per hop of a redirect
/// chain, each hop under its own `request_id`, as Playwright's
/// `redirectedFrom()` and `redirectedTo()` tell them apart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageRequest {
    /// The page that sent it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The engine's ID for this hop, which `response.body` takes: the
    /// browser's ID for the request at its first hop, and that ID with
    /// `#` and the hop's number after each redirect.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its method, such as `GET`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// Its headers as the browser sent them, their names in lowercase, as
    /// Playwright gives them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: BTreeMap<String, String>,
    /// Its body, in Base64 (RFC 4648, with padding), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub post_data: Option<String>,
    /// What it loads, as Playwright's `resourceType()` names it: the
    /// browser's name in lowercase, such as `document`, `fetch` or `xhr`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resource_type: String,
    /// Whether it loads the document of one of the page's frames, its main
    /// frame's or an iframe's, as Playwright's `isNavigationRequest()`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub is_navigation_request: bool,
    /// The URLs of the hops a redirect led from before it, the first first;
    /// empty when no redirect led to it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub redirect_chain: Vec<String>,
    /// The `request_id` of the hop whose redirect led to it, Playwright's
    /// `redirectedFrom()`; none at a request's first hop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub redirected_from: Option<String>,
    /// Why it failed, for `page.request_failed`; none otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: Option<RequestFailure>,
}

/// What `page.response` reports, and `page.wait_for_request_finished`
/// answers: a request and the response it got.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageExchange {
    /// The request.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request: PageRequest,
    /// Its response, whose body `response.body` reads.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response: PageResponse,
}

/// What `page.web_socket` reports: a WebSocket the page opened, whose
/// frames and close follow as `web_socket.*` events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageWebSocket {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The browser's ID for the socket, which its events name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub web_socket_id: String,
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
}

/// What `web_socket.frame_sent` and `web_socket.frame_received` report: one
/// message frame of a page's WebSocket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebSocketFrame {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The socket.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub web_socket_id: String,
    /// Its payload: its text, or for a binary frame its bytes in Base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: String,
    /// Whether it is a binary frame.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub binary: bool,
}

/// What `web_socket.close` reports: a page's WebSocket closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebSocketClosed {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The socket.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub web_socket_id: String,
}

/// What `page.load`, `page.dom_content_loaded`, `page.crash` and
/// `page.close` report: the page whose main frame's document fired the
/// event, or that crashed or closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageFired {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
}

/// What `response.body` takes: the page, the request whose response's
/// body it reads, and how long it waits for that request to end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResponseBodyParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The request's hop, by the engine's ID for it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// How long the read may wait for its request to end and its body to
    /// be read, in milliseconds, at least 1: absent for `timeout.wait`,
    /// `null` for none, though an enclosing timeout still applies (ADR
    /// 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timeout: Option<Option<u64>>,
}

/// What `page.wait_for_request_finished` takes: the page, and the URLs of
/// the requests it waits for; any request when none is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitForRequestFinishedParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URLs it waits for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<UrlMatcher>,
    /// How long the wait may take, in milliseconds, at least 1: absent for
    /// `timeout.wait`, `null` for none, though an enclosing timeout still
    /// applies (ADR 0080).
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timeout: Option<Option<u64>>,
}

/// What `body_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BodyUnavailableDetails {
    /// The response's URL; empty for a request the page never sent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The request.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// What keeps a body readable: obtaining the response through
    /// `page.wait_for_request_finished`, in the caller's spelling.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A response body the browser no longer holds (ADR 0085).
pub const BODY_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "body_unavailable",
    description: "The browser no longer holds the response's body, as after the page navigated away or once its buffer evicted it, or the response has none, as a redirect's: the engine keeps a body only for a response obtained through `page.wait_for_request_finished`, `route.fetch` or `page.request`, which the fix names. Where Playwright and Puppeteer say \"Request content was evicted from inspector cache\". A retry finds the body gone all the same.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A page whose events the engine fell too far behind and stopped reading.
pub const PAGE_EVENTS_LOST: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_events_lost",
    description: "Some of a page's events are lost. Either the engine fell too far behind the browser's events of a page that is still open and stopped reading them: the page's `page.on` listeners, waits and `response.body` reads raise this rather than answer from missing events, and the Run trace records it, while the page's other calls still work; open another page to follow its events. Or one `page.on` call's Client took too few of its events, granting too few credits, so 1,024 waited unsent: that call raises this once it sent them; listen again and grant credits as events are read.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Declares one of a page's lossless events.
const fn lossless(
    name: &'static str,
    description: &'static str,
    payload: crate::catalog::SchemaOf,
) -> Event {
    Event {
        name,
        description,
        class: EventClass::Lossless,
        payload,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }
}

/// Playwright's `console`.
pub const PAGE_CONSOLE: Event = lossless(
    "page.console",
    "The page wrote a message to its console, `ConsoleMessage`: its type, text and location. Sent on each `page.on` call that listens to `console`, the only time the engine reads the page's console.",
    schema_of::<ConsoleMessage>,
);

/// Playwright's `pageerror`.
pub const PAGE_PAGE_ERROR: Event = lossless(
    "page.page_error",
    "An exception the page did not catch, `WebError`: its name, message and stack. Sent on each `page.on` call that listens to `page_error`.",
    schema_of::<WebError>,
);

/// Playwright's `request`.
pub const PAGE_REQUEST: Event = lossless(
    "page.request",
    "The page sent a request, `Request`, once per hop of a redirect chain: its URL, method, headers, body, resource type, navigation flag and the hops before it. Sent on each `page.on` call that listens to `request`; a service worker's own requests are never a page's.",
    schema_of::<PageRequest>,
);

/// Playwright's `requestfinished`.
pub const PAGE_REQUEST_FINISHED: Event = lossless(
    "page.request_finished",
    "A request of the page got its whole response, its body included. Sent on each `page.on` call that listens to `request_finished`.",
    schema_of::<PageRequest>,
);

/// Playwright's `requestfailed` (ADRs 0081 and 0085).
pub const PAGE_REQUEST_FAILED: Event = lossless(
    "page.request_failed",
    "A request of the page failed, `Request` with its `failure`, as Playwright reports one: the browser's network error, such as `net::ERR_FAILED` for one a route aborted or `net::ERR_NAME_NOT_RESOLVED`, and the Error code when the engine blocked it, such as `egress_refused`, so a refusal is never a timeout. A response whose body broke off fails too. Sent on each `page.on` call that listens to `request_failed`; the Run trace records each request the engine refused that no call waited on.",
    schema_of::<PageRequest>,
);

/// Playwright's `response`.
pub const PAGE_RESPONSE: Event = lossless(
    "page.response",
    "A request of the page got its response's head: the request and its response, whose body `response.body` reads, and which says whether a site's service worker supplied it. Sent on each `page.on` call that listens to `response`.",
    schema_of::<PageExchange>,
);

/// Playwright's `websocket`.
pub const PAGE_WEB_SOCKET: Event = lossless(
    "page.web_socket",
    "The page opened a WebSocket, whose frames and close follow as `web_socket.frame_sent`, `web_socket.frame_received` and `web_socket.close`. Sent, with those, on each `page.on` call that listens to `web_socket`.",
    schema_of::<PageWebSocket>,
);

/// Playwright's `framesent`.
pub const WEB_SOCKET_FRAME_SENT: Event = lossless(
    "web_socket.frame_sent",
    "A page's WebSocket sent a message frame: its text, or a binary frame's bytes in Base64.",
    schema_of::<WebSocketFrame>,
);

/// Playwright's `framereceived`.
pub const WEB_SOCKET_FRAME_RECEIVED: Event = lossless(
    "web_socket.frame_received",
    "A page's WebSocket received a message frame: its text, or a binary frame's bytes in Base64.",
    schema_of::<WebSocketFrame>,
);

/// Playwright's WebSocket `close`.
pub const WEB_SOCKET_CLOSE: Event = lossless(
    "web_socket.close",
    "A page's WebSocket closed.",
    schema_of::<WebSocketClosed>,
);

/// Playwright's `load`.
pub const PAGE_LOAD: Event = lossless(
    "page.load",
    "The document of the page's main frame fired `load`. Sent on each `page.on` call that listens to `load`.",
    schema_of::<PageFired>,
);

/// Playwright's `domcontentloaded`.
pub const PAGE_DOM_CONTENT_LOADED: Event = lossless(
    "page.dom_content_loaded",
    "The document of the page's main frame fired `DOMContentLoaded`. Sent on each `page.on` call that listens to `dom_content_loaded`.",
    schema_of::<PageFired>,
);

/// Playwright's `crash`.
pub const PAGE_CRASH: Event = lossless(
    "page.crash",
    "The page crashed: its renderer, or its browser's process, which exited abnormally. Its calls in flight raised `page_crashed`, and so does each later call. Sent on each `page.on` call that listens to `crash`.",
    schema_of::<PageFired>,
);

/// Playwright's `close`.
pub const PAGE_CLOSE: Event = lossless(
    "page.close",
    "The page closed: the engine closed it, or its window was closed in the browser outside every Client. Sent on each `page.on` call that listens to `close`, just before that call answers.",
    schema_of::<PageFired>,
);

/// `page.on`.
pub const PAGE_ON: Operation = Operation {
    name: "page.on",
    description: "Listens to one of the page's events, armed when the call arrives: the call stays in flight, and the engine sends that event of the page on the call itself, in `$/events` batches naming it, never more than the credits granted it with `$/credit`, its positions counting the call's own events from 1. Only the Connection that armed it gets them, never the engine's shared event stream. Lossless: past 1,024 events unsent, as when its Client grants too few credits, the call raises `page_events_lost` once it sent those. Cancelling the call, as closing its `Subscription` does, removes the listener; it answers when the page closes or its Run ends, once it sent the events it holds. The page's console and uncaught exceptions, its out-of-process frames' included, are read only while a call listens to `console` or `page_error`.",
    class: OperationClass::Read,
    kind: OperationKind::Event,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageOnParams>,
    result: schema_of::<PageOnEnded>,
    events: &[
        &PAGE_CONSOLE,
        &PAGE_PAGE_ERROR,
        &PAGE_REQUEST,
        &PAGE_REQUEST_FINISHED,
        &PAGE_REQUEST_FAILED,
        &PAGE_RESPONSE,
        &PAGE_WEB_SOCKET,
        &WEB_SOCKET_FRAME_SENT,
        &WEB_SOCKET_FRAME_RECEIVED,
        &WEB_SOCKET_CLOSE,
        &PAGE_LOAD,
        &PAGE_DOM_CONTENT_LOADED,
        &PAGE_DOWNLOAD,
        &PAGE_CRASH,
        &PAGE_CLOSE,
        &super::session_pages::PAGE_POPUP,
    ],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `response.body`.
pub const RESPONSE_BODY: Operation = Operation {
    name: "response.body",
    description: "Reads the body of the response a page's request got, once the request finished, as a page body's handle: kept already for a response `page.wait_for_request_finished` answered, and otherwise read from the browser, on the session of the frame that loaded it. A body the browser no longer holds, or a redirect's, which has none, raises `body_unavailable`, whose fix names `page.wait_for_request_finished`; one that broke off raises `response_truncated`, never returned as whole. Past its `timeout`, `timeout.wait` when the call gives none, a request that has not ended fails it with `deadline_exceeded`; a page that ends under it raises what ended it, as a call on the page does.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ResponseBodyParams>,
    result: schema_of::<BodyHandle>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.wait_for_request_finished`.
pub const PAGE_WAIT_FOR_REQUEST_FINISHED: Operation = Operation {
    name: "page.wait_for_request_finished",
    description: "Waits, armed when the call arrives, until a request of the page whose URL matches has finished, a hop that a redirect ended included, and answers it with its response, whose body the engine reads and keeps, so `response.body` never fails for it but for a redirect's, which has none. A request that fails is not waited for. Past its `timeout`, `timeout.wait` when the call gives none, it fails with `deadline_exceeded`.",
    class: OperationClass::Read,
    kind: OperationKind::Waiter,
    reach: Reach::AllSurfaces,
    params: schema_of::<WaitForRequestFinishedParams>,
    result: schema_of::<PageExchange>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
