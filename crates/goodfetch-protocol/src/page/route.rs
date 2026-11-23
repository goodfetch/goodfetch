//! Routes (ADR 0085, with ADRs 0081 and 0104): `page.route` and
//! `session.route` answer a page's requests in the program, under the rules
//! of Selenium's `WebDriver BiDi` design note with Playwright's names. Each is a
//! subscription, a call that stays in flight until its Client cancels it or
//! its handler fails, armed when its request arrives. For each request its
//! URL pattern matches, the engine sends the Connection the handler request
//! `route.handle`, naming the request by a `route_id`; the handler answers
//! the request with one of `route.continue`, `route.fulfill`, `route.abort`
//! or `route.fallback`, or `route.fetch` then `route.fulfill`, and replies to
//! `route.handle` once it ends. Page routes come before Session routes, later
//! routes first; the first answer wins; a handler that ends without one falls
//! back, and past the last route the request goes on to the network. A
//! handler that fails (its reply is an error) aborts its request when no
//! answer was given yet, its route is removed, its subscription ends with
//! `page_handler_failed`, and the call waiting on that request, or else the
//! next call on its page, raises it. A second answer, or one after the
//! handler ended, raises `route_already_handled`, so a half-changed request
//! is never sent. Bodies are in Base64 (RFC 4648, with padding).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId, SessionId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, HandlerRequest, Operation, OperationClass, OperationKind,
    Reach, Stability, schema_of,
};
use crate::jsonrpc::RequestId;

/// A URL pattern the engine matches (ADR 0081): a glob as Playwright reads
/// one, or a regular expression in the `regex` crate's syntax. A predicate
/// runs in the program, which routes every URL and falls back for the
/// requests it does not match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UrlMatcher {
    /// A glob, such as `**/*.png`.
    Glob {
        /// The glob.
        #[schemars(extend("arrived_in" = "0.1"))]
        pattern: String,
    },
    /// A regular expression, matched anywhere in the URL.
    Regex {
        /// The expression's source.
        #[schemars(extend("arrived_in" = "0.1"))]
        pattern: String,
    },
}

/// What `page.route` takes: the page, and the URLs whose requests its
/// handler answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageRouteParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URLs it answers.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: UrlMatcher,
}

/// What `session.route` takes: the Session, and the URLs whose requests its
/// handler answers, on every page of the Session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionRouteParams {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// The URLs it answers.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: UrlMatcher,
}

/// What a route's subscription answers when its page or Session closes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteEnded {}

/// One intercepted request a route's handler answers, which `route.*`
/// names.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct RouteId(pub String);

/// A request as a route sees it: as the page sent it, with the overrides of
/// the routes that fell back before. The engine's own additions, such as
/// the Session's cookies, come after every route, so none is here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteRequest {
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its method, such as `GET`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// Its headers, as the page sent them, their names in lowercase, as
    /// Playwright gives them; as a route before replaced them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: BTreeMap<String, String>,
    /// What the page loads with it, as Playwright's `resourceType()` names
    /// it: the browser's name in lowercase, such as `document`, `script`
    /// or `xhr`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resource_type: String,
    /// Whether it loads the document of one of the page's frames, its main
    /// frame's or an iframe's, as Playwright's `isNavigationRequest()`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub is_navigation_request: bool,
    /// Its body, in Base64 (RFC 4648, with padding), when it has one: as
    /// the page sent it, or as a route before replaced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub post_data: Option<String>,
}

/// What `route.handle` gives the handler: the request, named by the ID its
/// answer passes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteHandleParams {
    /// The id of the `page.route` or `session.route` call whose handler is
    /// asked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// The intercepted request, which an answer names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub route_id: RouteId,
    /// The page that sent the request; none for a request a site's service
    /// worker sent itself, which belongs to no page and reaches only
    /// Session routes (ADR 0121).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: Option<PageId>,
    /// The request.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request: RouteRequest,
}

/// What the handler replies to `route.handle` once it ended; a failure
/// reply says it failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteHandled {}

/// What `route.continue`, `route.fallback` and `route.fetch` take: the
/// request, and what to send instead of what it carries. Each override
/// replaces its part whole; a header value may hold a Secret placeholder
/// (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteOverridesParams {
    /// The request.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub route_id: RouteId,
    /// Another URL, judged again by the domain policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// Another method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: Option<String>,
    /// Other headers, in place of every header the request carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: Option<BTreeMap<String, String>>,
    /// Another body, in Base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub post_data: Option<String>,
}

/// What `route.fulfill` takes: the request, and the response the program
/// answers it with; nothing reaches the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteFulfillParams {
    /// The request.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub route_id: RouteId,
    /// Its status; 200 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: Option<u16>,
    /// Its headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: Option<BTreeMap<String, String>>,
    /// Its `Content-Type`, set over `headers`' own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content_type: Option<String>,
    /// Its body, in Base64; empty when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: Option<String>,
}

/// The network error an aborted request reports to its page, under
/// Playwright's names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AbortReason {
    /// `aborted`.
    Aborted,
    /// `accessdenied`.
    AccessDenied,
    /// `addressunreachable`.
    AddressUnreachable,
    /// `blockedbyclient`.
    BlockedByClient,
    /// `blockedbyresponse`.
    BlockedByResponse,
    /// `connectionaborted`.
    ConnectionAborted,
    /// `connectionclosed`.
    ConnectionClosed,
    /// `connectionfailed`.
    ConnectionFailed,
    /// `connectionrefused`.
    ConnectionRefused,
    /// `connectionreset`.
    ConnectionReset,
    /// `internetdisconnected`.
    InternetDisconnected,
    /// `namenotresolved`.
    NameNotResolved,
    /// `timedout`.
    TimedOut,
    /// `failed`.
    #[default]
    Failed,
}

/// What `route.abort` takes: the request, and the error its page sees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteAbortParams {
    /// The request.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub route_id: RouteId,
    /// The network error; `failed` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error_code: Option<AbortReason>,
}

/// What an answer to a route answers once the engine acted on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteAnswered {}

/// What `route.fetch` answers: the response the request got, its body
/// kept, for the handler to pass to `route.fulfill`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteFetched {
    /// Its HTTP status.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// Its status text.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status_text: String,
    /// Its headers, their names lower-cased.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: BTreeMap<String, String>,
    /// Its body, in Base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: String,
}

/// The handlers a program gives a page, which `page_handler_failed` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageHandler {
    /// A route's handler.
    Route,
    /// A dialog handler.
    Dialog,
}

/// What `page_handler_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageHandlerFailedDetails {
    /// The handler that failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub handler: PageHandler,
    /// The URL of the request it was answering, or of the document whose
    /// dialog it was answering.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// What its failure said.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: String,
}

/// What `route_already_handled` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteAlreadyHandledDetails {
    /// The request answered already.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub route_id: RouteId,
}

/// A handler the program gave a page failed (ADRs 0081 and 0085).
pub const PAGE_HANDLER_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_handler_failed",
    description: "A handler the program gave a page failed, such as a route's handler that threw: its request was aborted unless it was answered already, never sent half-changed, and its route was removed; a dialog handler's dialog got its default and the handler was removed. Raised by the call that waited on that request, such as the `goto` whose document it was, or by the call the dialog blocked, or else by the next call on its page. The details name the handler, the URL of the request or of the dialog's document, and what the failure said.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A route's request answered twice (ADR 0085).
pub const ROUTE_ALREADY_HANDLED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "route_already_handled",
    description: "The route's request was answered already, by an earlier `continue`, `fulfill`, `abort` or `fallback`, or its handler ended: the first answer stands and this one changed nothing.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `page.route`.
pub const PAGE_ROUTE: Operation = Operation {
    name: "page.route",
    description: "Routes the page's requests whose URL matches to the program, armed when the call arrives: the call stays in flight, and for each such request the engine sends `route.handle`, which the handler answers with `route.continue`, `route.fulfill`, `route.abort`, `route.fallback`, or `route.fetch` then `route.fulfill`. Page routes come before Session routes, later routes first, and the first answer wins; a handler that ends without one falls back, and past the last route the request goes on to the network, so an unanswered route never stalls a request. What a route sends on still passes the domain policy. Cancelling the call removes the route; a handler that fails ends it with `page_handler_failed`. A request a site's service worker answers from its own cache never reaches the route. When a site's service worker answers a request the route must act on, set `browser.service_workers` to `block` (ADR 0121).",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<PageRouteParams>,
    result: schema_of::<RouteEnded>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.route`.
pub const SESSION_ROUTE: Operation = Operation {
    name: "session.route",
    description: "Routes the requests of every page of the Session whose URL matches to the program, as `page.route` does, after the page's own routes. When a site's service worker answers a request the route must act on, set `browser.service_workers` to `block` (ADR 0121).",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<SessionRouteParams>,
    result: schema_of::<RouteEnded>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `route.continue`.
pub const ROUTE_CONTINUE: Operation = Operation {
    name: "route.continue",
    description: "Answers a route's request by sending it on, with any overrides, past every other route: the domain policy judges what it sends.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RouteOverridesParams>,
    result: schema_of::<RouteAnswered>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `route.fallback`.
pub const ROUTE_FALLBACK: Operation = Operation {
    name: "route.fallback",
    description: "Answers a route's request by passing it, with any overrides, to the route registered before, from the page's first route to the Session's last; past the last route it goes on to the network.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RouteOverridesParams>,
    result: schema_of::<RouteAnswered>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `route.fulfill`.
pub const ROUTE_FULFILL: Operation = Operation {
    name: "route.fulfill",
    description: "Answers a route's request with a response of the program's own, or the one `route.fetch` returned with its changes; nothing reaches the network.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RouteFulfillParams>,
    result: schema_of::<RouteAnswered>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `route.abort`.
pub const ROUTE_ABORT: Operation = Operation {
    name: "route.abort",
    description: "Answers a route's request by failing it with a network error its page sees, `failed` unless named; nothing reaches the network.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RouteAbortParams>,
    result: schema_of::<RouteAnswered>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `route.fetch`.
pub const ROUTE_FETCH: Operation = Operation {
    name: "route.fetch",
    description: "Sends a route's request on, with any overrides, past every other route, as `route.continue` does, and answers the response it got, its body kept; the handler then answers the request with `route.fulfill`, or, ending without one, the page gets that response.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<RouteOverridesParams>,
    result: schema_of::<RouteFetched>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `route.handle`.
pub const ROUTE_HANDLE: HandlerRequest = HandlerRequest {
    name: "route.handle",
    description: "Asks a route's handler to answer one request its URL pattern matched, named by its `route_id`, with `route.continue`, `route.fulfill`, `route.abort`, `route.fallback`, or `route.fetch` then `route.fulfill`. The handler replies once it ends: a request it did not answer falls back. A failure reply aborts the request unless it was answered, removes the route and raises `page_handler_failed`.",
    params: schema_of::<RouteHandleParams>,
    result: schema_of::<RouteHandled>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
