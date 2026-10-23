//! A Run's pages (ADRs 0085, 0092 and 0099): `run.new_page` opens a page in
//! the Run's default Session, which the first call opens and every later
//! one shares; `page.goto`, `page.reload`, `page.go_back` and
//! `page.go_forward` navigate a page and answer the document's `Response`,
//! or none for a file the browser downloads, which `page.download` reports;
//! `locator.click`, `locator.fill` and `locator.press` act on the one
//! element a locator matches, through the guard (ADR 0019); and the Error
//! codes a page's browser, its navigations, its locators and the URL
//! patterns its calls pass fail with. Its `route` module holds the page's
//! and the Session's routes, its `events` module the page's events and its
//! responses' bodies.
//!
//! Page bodies as handles (ADRs 0024 and 0031): a body is a file in its
//! Run's folder, named by the digest of its bytes. A Client on the same
//! machine reads the file at the handle's path; any other reads it through
//! `page.body`, which sends it in `$/chunk` notifications, one per credit
//! the Client grants the call with `$/credit`, and answers once the last
//! chunk is sent.
//!
//! Size: the page API's catalog module, whose `ENTRIES` lists every entry of
//! its submodules, kept whole so a page's operations, events and Error
//! codes are declared in one list (ADR 0099).

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation,
    OperationClass, OperationKind, ProtocolVersion, Reach, Setting, Stability, ValueKind,
    schema_of,
};
use crate::jsonrpc::{ErrorData, RequestId};
use crate::run::RunId;
use crate::settings::SettingLimit;

pub mod events;
pub mod route;

use self::events::{
    BODY_UNAVAILABLE, PAGE_CONSOLE, PAGE_DOM_CONTENT_LOADED, PAGE_LOAD, PAGE_ON, PAGE_PAGE_ERROR,
    PAGE_REQUEST, PAGE_REQUEST_FAILED, PAGE_REQUEST_FINISHED, PAGE_RESPONSE,
    PAGE_WAIT_FOR_REQUEST_FINISHED, PAGE_WEB_SOCKET, RESPONSE_BODY, WEB_SOCKET_CLOSE,
    WEB_SOCKET_FRAME_RECEIVED, WEB_SOCKET_FRAME_SENT,
};
use self::route::{
    PAGE_HANDLER_FAILED, PAGE_ROUTE, ROUTE_ABORT, ROUTE_ALREADY_HANDLED, ROUTE_CONTINUE,
    ROUTE_FALLBACK, ROUTE_FETCH, ROUTE_FULFILL, ROUTE_HANDLE, SESSION_ROUTE,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The notification that carries one chunk of a body `page.body` reads.
pub const CHUNK: &str = "$/chunk";

/// The most bytes one chunk carries before its Base64 encoding.
pub const CHUNK_BYTES: usize = 64 * 1024;

/// The characters of a body's ID: the lowercase hex SHA-256 of its bytes.
pub const BODY_ID_PATTERN: &str = "^[0-9a-f]{64}$";

/// A page body as a handle: its ID, the file that holds it and its size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BodyHandle {
    /// The Run whose folder holds it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Its ID, the lowercase hex SHA-256 of its bytes.
    #[schemars(extend("arrived_in" = "0.1", "pattern" = BODY_ID_PATTERN))]
    pub body: String,
    /// The file that holds it, which a Client on the same machine reads.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Its size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
}

/// What `page.body` takes: the body to read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageBodyParams {
    /// The Run whose folder holds it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Its ID.
    #[schemars(extend("arrived_in" = "0.1", "pattern" = BODY_ID_PATTERN))]
    pub body: String,
}

/// What `page.body` answers once it sent the last chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageBodyRead {
    /// The body's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
}

/// What `$/chunk` carries: the next bytes of the body a call reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    /// The id of the `page.body` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// Where its bytes start in the body.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub offset: u64,
    /// Its bytes, in Base64 (RFC 4648, with padding).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: String,
}

/// What `body_not_found` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BodyNotFoundDetails {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The body's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: String,
}

/// A body no Run's folder holds.
pub const BODY_NOT_FOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "body_not_found",
    description: "The Run's folder holds no body of this ID: the Run or the body is unknown, or the ID is not a body's.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `page.body`.
pub const PAGE_BODY: Operation = Operation {
    name: "page.body",
    description: "Reads a page body by its handle, for a Client that cannot read the handle's file: sends the body in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, then answers its size.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageBodyParams>,
    result: schema_of::<PageBodyRead>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A page of a Run, named by the engine when it opens it.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct PageId(pub String);

/// A Session of a Run, named by the engine when it opens it.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SessionId(pub String);

/// What `run.new_page` takes: the Run whose default Session opens the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewPageParams {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What `run.new_page` answers: the page, and the Session it belongs to,
/// which `page.session()` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageOpened {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// Its Session: the Run's default Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
}

/// What `page.goto` takes: the page, the URL it goes to, and when it
/// answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GotoParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// When the navigation answers; `load` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wait_until: Option<WaitUntil>,
}

/// When a navigation answers, by the state its document reached
/// (Playwright's `waitUntil`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WaitUntil {
    /// Once the browser committed the document, its response received: a
    /// body cut after that is left to the body's read.
    Commit,
    /// Once the document fired `DOMContentLoaded`.
    DomContentLoaded,
    /// Once the document fired `load`.
    #[default]
    Load,
}

/// What `page.reload`, `page.go_back` and `page.go_forward` take: the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
}

/// The response to a request of a page, such as a navigation's document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageResponse {
    /// Its final URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its HTTP status.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// Its status text.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status_text: String,
    /// Its headers, their names lower-cased, as the browser received them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: BTreeMap<String, String>,
    /// For a navigation's document, the encoding the browser decoded it
    /// with, as `document.characterSet` names it, such as `Shift_JIS` (ADR
    /// 0119); none for another response, and for a navigation that answered
    /// at `commit`, before its document was decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub encoding: Option<String>,
    /// The browser's ID for its request, which `response.body` takes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: String,
    /// Whether a site's service worker supplied it, from its own cache or
    /// through its own fetch (ADR 0121).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub from_service_worker: bool,
}

/// What a navigation answers: the document's response, or none for a
/// navigation within the same document, to a URL no server answers, such as
/// `about:blank`, to a file the browser downloads instead of showing, which
/// `page.download` reports, or back or forward from the last entry of the
/// page's history. A 404 or a 500 is a response, a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Navigated {
    /// The document's response.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response: Option<PageResponse>,
}

/// What `navigation_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigationFailedDetails {
    /// The URL the navigation went to; empty when the page's history, which
    /// names it for a reload or a step back or forward, did not read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The browser's network error, such as `net::ERR_NAME_NOT_RESOLVED`,
    /// when the browser named one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: Option<String>,
}

/// What `navigation_interrupted` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigationInterruptedDetails {
    /// The URL of the navigation that was superseded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
}

/// The count of redirects a fetch follows at most, and what set it (ADR
/// 0118).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedirectLimit {
    /// The browser's own count, 20 requests in a navigation's chain, its
    /// first included, which a navigation cannot change.
    Browser {
        /// The count.
        #[schemars(extend("arrived_in" = "0.1"))]
        count: u32,
    },
    /// A setting, such as `fetch.redirect.max`, at its resolved value.
    Setting {
        /// The setting's key.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
        /// The count.
        #[schemars(extend("arrived_in" = "0.1"))]
        count: u32,
    },
    /// A call's option, such as `page.request`'s `maxRedirects`.
    Option {
        /// The option's name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
        /// The count.
        #[schemars(extend("arrived_in" = "0.1"))]
        count: u32,
    },
}

/// What `too_many_redirects` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TooManyRedirectsDetails {
    /// The first URL of the chain.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The URL each redirect followed led to, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hops: Vec<String>,
    /// The count it went past, and what set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: RedirectLimit,
}

/// What `response_truncated` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResponseTruncatedDetails {
    /// The URL of the response that was cut.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The bytes of its body that arrived, as the browser decoded them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub received: u64,
    /// The bytes its `Content-Length` declared, as sent; none when it
    /// declared no length, as a chunked body does not.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expected: Option<u64>,
}

/// What `page.download` reports: a navigation of a page that the browser
/// turned into a download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageDownload {
    /// The page whose navigation it was.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The file's URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The file name the browser suggests for it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub suggested_filename: String,
}

/// What the Run trace records of a request the engine's own HTTP client
/// sent for the Run that failed while no call waited for it, such as one the
/// egress policy refused; a page's failed request is a `page.request_failed`
/// event's `PageRequest`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestFailed {
    /// The request's URL, as the run gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its method, such as `GET`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// Why it failed: the Error code a call waiting for it would have
    /// raised, with its details, such as `egress_refused`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: ErrorData,
}

/// How a browser process ended (ADR 0113).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserExit {
    /// Its exit status, when it exited.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: Option<i32>,
    /// The signal that ended it, when one did.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signal: Option<i32>,
}

/// What `page_crashed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageCrashedDetails {
    /// How the browser process ended, when it is what was lost; none for a
    /// renderer's crash alone.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_exit: Option<BrowserExit>,
}

/// What `browser_launch_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLaunchFailedDetails {
    /// The browser's executable.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub executable: String,
    /// How it ended, when it exited before it was ready.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exit: Option<BrowserExit>,
}

/// A navigation that reached no document (ADR 0085).
pub const NAVIGATION_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "navigation_failed",
    description: "The navigation reached no document: the browser's network error, such as `net::ERR_NAME_NOT_RESOLVED`, is in the details, or the browser failed it for a reason it did not name.",
    group: ErrorGroup::Page,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A navigation another superseded before it committed (ADR 0085).
pub const NAVIGATION_INTERRUPTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "navigation_interrupted",
    description: "Another navigation of the page started before this one committed, so this one reached no document: the page went where the other took it.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A navigation past the count of redirects it may follow (ADR 0118).
pub const TOO_MANY_REDIRECTS: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "too_many_redirects",
    description: "The navigation followed as many redirects as it may and was redirected again, as a redirect loop is: the browser's count is 20 requests in a navigation's chain, its first included, so Chromium follows 19 redirects. The details name the first URL, the hops followed and the count. A retry meets the same chain.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A response whose body broke off before its end (ADR 0118).
pub const RESPONSE_TRUNCATED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "response_truncated",
    description: "The response's body broke off before its end: shorter than its `Content-Length`, a chunked body without its last chunk, or a stream the server or the connection ended. A navigation waiting for `domcontentloaded` or `load` raises it for its main document; one that answered at `commit` leaves it to the body's read. The details name the URL and the bytes received and expected.",
    group: ErrorGroup::Page,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A page whose renderer or browser was lost (ADRs 0085 and 0113).
pub const PAGE_CRASHED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_crashed",
    description: "The page's renderer crashed, or its browser process was lost while the call ran; `browser_exit` says how the browser ended when it is what was lost. The page is gone: open another.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An installed browser that did not start or open the page (#259).
pub const BROWSER_LAUNCH_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "browser_launch_failed",
    description: "The browser's build is installed, but it did not start or did not open the page: it exited, or its connection refused or fell silent. The details name its executable and how it ended.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Browser host older than the engine that reaches it can drive (ADRs
/// 0035 and 0096).
pub const BROWSER_HOST_OUTDATED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "browser_host_outdated",
    description: "The Browser host the engine reached is of a version this engine cannot drive. Upgrade the Browser host, which either Product's executable runs as its `browser-host` command.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: Some(12),
    arrived_in: ARRIVED_IN,
};

/// A locator (ADR 0085): what an action looks for on a page, resolved
/// again at each use, never a handle on an element. This tracer's locators
/// are CSS selectors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    /// The page it looks on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// A CSS selector, which also matches inside open shadow roots.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub selector: String,
}

/// What `locator.click` takes: the locator whose element it clicks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClickParams {
    /// The locator.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
}

/// What `locator.fill` takes: the locator whose field it fills, and the
/// text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillParams {
    /// The locator.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// The text the field holds afterwards; an empty text clears it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
}

/// What `locator.press` takes: the locator whose element gets the key, and
/// the key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PressParams {
    /// The locator.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: Locator,
    /// A key or a combination, such as `Enter`, `a` or `Shift+Tab`, by
    /// Playwright's key names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
}

/// What a locator action answers once its input landed on its target:
/// nothing, since no operation returns a handle on an element (ADR 0085).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Acted {}

/// One element an ambiguous locator matched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TargetMatch {
    /// The element as its opening tag and the start of its text, such as
    /// `<button id="target">Target</button>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub element: String,
    /// A locator that reaches it alone, in the caller's language (ADR
    /// 0079), such as `page.locator('#target').nth(1)`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locator: String,
}

/// What `target_not_unique` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TargetNotUniqueDetails {
    /// The locator, in the caller's language.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: String,
    /// Every element it matched at the last observation.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub matches: Vec<TargetMatch>,
    /// The timeout's setting, its value in milliseconds and the layer that
    /// set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
    /// How long the action waited, in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub elapsed: u64,
}

/// A locator still matching several elements when its action's deadline
/// passed (ADRs 0080 and 0085).
pub const TARGET_NOT_UNIQUE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "target_not_unique",
    description: "The locator still matched several elements when the action's timeout passed, so the action did nothing. The details list every match, each with a locator that reaches it alone, and `limit` names the timeout's setting; a longer timeout would not fix the locator.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A construct of a URL pattern the engine's matcher cannot read (ADR 0081).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatternConstruct {
    /// A regular expression's look-ahead or look-behind, such as `(?=x)`.
    LookAround,
    /// A regular expression's back-reference, such as `\1`.
    Backreference,
    /// A glob's `{` inside another `{…}` group, which Playwright refuses
    /// too.
    NestedBraces,
    /// A glob's `{` with no `}`, or `}` with no `{`, which Playwright
    /// refuses too.
    UnmatchedBrace,
    /// Any other construct outside the `regex` crate's syntax, or a
    /// regular expression too large to compile; the message names it.
    Other,
}

/// What `pattern_unsupported` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PatternUnsupportedDetails {
    /// The pattern as the call passed it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: String,
    /// The construct the engine cannot read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub construct: PatternConstruct,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A URL pattern outside the syntax of the engine's matcher (ADR 0081).
pub const PATTERN_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "pattern_unsupported",
    description: "A URL pattern the engine cannot match: a regular expression with look-around, a back-reference or another construct outside the `regex` crate's syntax, or a glob with nested or unmatched braces. The details name the construct; match the URL with a predicate instead, which runs in the program.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A navigation the browser turned into a download (ADR 0119).
pub const PAGE_DOWNLOAD: Event = Event {
    name: "page.download",
    description: "A navigation of the page reached a file the browser downloads instead of showing, such as a PDF: the navigation answers no response and the page keeps its document. Names the page, the file's URL and the name the browser suggests.",
    class: EventClass::Lossless,
    payload: schema_of::<PageDownload>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.click`.
pub const LOCATOR_CLICK: Operation = Operation {
    name: "locator.click",
    description: "Clicks the element the locator matches once it is unique, visible, enabled, stable and receiving the click (the guard, ADR 0019); a click that lands on another element is cancelled and sent again, until `timeout.action` passes.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ClickParams>,
    result: schema_of::<Acted>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.fill`.
pub const LOCATOR_FILL: Operation = Operation {
    name: "locator.fill",
    description: "Replaces the text of the field the locator matches, once it is unique, visible, enabled and editable, with text the page receives as typed input, until `timeout.action` passes.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<FillParams>,
    result: schema_of::<Acted>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.press`.
pub const LOCATOR_PRESS: Operation = Operation {
    name: "locator.press",
    description: "Focuses the element the locator matches, once it is unique, visible and enabled, and presses a key or a combination on it as a keyboard does, until `timeout.action` passes.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PressParams>,
    result: schema_of::<Acted>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.new_page`.
pub const RUN_NEW_PAGE: Operation = Operation {
    name: "run.new_page",
    description: "Opens a page in the Run's default Session, which the first call opens with the Run's settings and every later call shares; `page.session()` reaches it. The first page launches the Run's browser, so a missing browser build raises its code from this call.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<NewPageParams>,
    result: schema_of::<PageOpened>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.goto`.
pub const PAGE_GOTO: Operation = Operation {
    name: "page.goto",
    description: "Navigates the page to a URL and waits for its document's `load` event, or the state `wait_until` names, answering the document's response: a 404 or a 500 is a response, a value. A file the browser downloads, such as a PDF, answers no response and emits `page.download`. A navigation that reaches no document raises `navigation_failed`, one another navigation supersedes before it committed `navigation_interrupted`, a redirect loop `too_many_redirects`, and a document whose body broke off `response_truncated`, unless the navigation answered at `commit`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GotoParams>,
    result: schema_of::<Navigated>,
    events: &[&PAGE_DOWNLOAD],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.reload`.
pub const PAGE_RELOAD: Operation = Operation {
    name: "page.reload",
    description: "Loads the page's document again and waits for its `load` event, answering the document's response: a 404 or a 500 is a response, a value. A navigation that reaches no document raises `navigation_failed`, and one another navigation supersedes before it committed `navigation_interrupted`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageParams>,
    result: schema_of::<Navigated>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.go_back`.
pub const PAGE_GO_BACK: Operation = Operation {
    name: "page.go_back",
    description: "Goes back one entry in the page's history and waits for its `load` event, answering the document's response, or none when there is no entry to go back to. The back/forward cache is off, so the document always loads again (ADR 0121).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageParams>,
    result: schema_of::<Navigated>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.go_forward`.
pub const PAGE_GO_FORWARD: Operation = Operation {
    name: "page.go_forward",
    description: "Goes forward one entry in the page's history and waits for its `load` event, answering the document's response, or none when there is no entry to go forward to. The back/forward cache is off, so the document always loads again (ADR 0121).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageParams>,
    result: schema_of::<Navigated>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
/// `browser.headless` (ADRs 0083 and 0092; spec #577).
pub const BROWSER_HEADLESS: Setting = Setting {
    name: "browser.headless",
    description: "Whether the Session's Chromium runs with no window: `true` by default. Headless or not, the site sees one browser's identity, with no `HeadlessChrome` token in its user agent.",
    kind: ValueKind::Boolean,
    default: headless,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

fn headless() -> serde_json::Value {
    serde_json::Value::Bool(true)
}

/// A format asked for as `true`: with its default options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asked;

impl Serialize for Asked {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for Asked {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(
                "a format is asked for with `true` or its options; leave out a format not wanted",
            ))
        }
    }
}

impl JsonSchema for Asked {
    fn schema_name() -> Cow<'static, str> {
        "Asked".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The format, with its default options.",
            "const": true,
        })
    }
}

/// How much of the page a Markdown format holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownContent {
    /// The main content the extractor finds, the default.
    #[default]
    Main,
}

/// The options of the `markdown` format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownOptions {
    /// How much of the page it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content: MarkdownContent,
}

/// The `markdown` format of a request: `true`, or its options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MarkdownFormat {
    /// With its default options.
    Asked(Asked),
    /// With these options.
    Options(MarkdownOptions),
}

impl MarkdownFormat {
    /// How much of the page it holds.
    pub fn content(self) -> MarkdownContent {
        match self {
            Self::Asked(Asked) => MarkdownContent::default(),
            Self::Options(options) => options.content,
        }
    }
}

/// The formats a `page.formats` call asks for, each `true` or its options:
/// one value, which a Record can carry and a Dataset's schema can be
/// generated from (ADR 0103). A format left out is left out of the result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatsRequest {
    /// The page's content as Markdown, with GFM's tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub markdown: Option<MarkdownFormat>,
}

/// What `page.formats` takes: the page, and the formats it asks for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatsParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The formats.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub formats: FormatsRequest,
}

/// Whether the extractor found the document's main content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MainContent {
    /// It found it.
    Found,
    /// The document has none: the formats read from it are empty, a value
    /// and never the whole page in its place.
    None,
}

/// What `page.formats` answers: the formats the request asked for, and
/// none it did not, with the document's URL, its base URL and whether it
/// has main content. Text is as the document holds it, never repaired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageFormats {
    /// The document's URL, after redirects.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The URL its relative URLs resolve against: its first `<base href>`,
    /// else its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub base_url: String,
    /// Whether it has main content.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub main_content: MainContent,
    /// Its content as Markdown, when the request asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub markdown: Option<String>,
}

/// The JSON Schema of what `page.formats` answers to `request`: the
/// result's envelope and each format it asks for, all required, and no
/// other property, so a code-free crawl writes its Dataset's schema from
/// the request alone (ADR 0103). Derived from [`PageFormats`]' own schema.
pub fn page_formats_schema(request: &FormatsRequest) -> Value {
    let mut schema =
        serde_json::to_value(schema_for!(PageFormats)).expect("invariant: a JSON Schema is JSON");
    let Ok(Value::Object(asked)) = serde_json::to_value(request) else {
        unreachable!("invariant: a formats request serializes as an object");
    };
    let mut required: Vec<Value> = schema["required"].as_array().cloned().unwrap_or_default();
    let envelope: Vec<String> = required
        .iter()
        .filter_map(|name| name.as_str().map(ToOwned::to_owned))
        .collect();
    if let Some(properties) = schema["properties"].as_object_mut() {
        properties.retain(|name, _| envelope.contains(name) || asked.contains_key(name));
    }
    required.extend(asked.keys().map(|name| Value::String(name.clone())));
    schema["required"] = Value::Array(required);
    schema["additionalProperties"] = Value::Bool(false);
    schema
}

/// `page.formats`.
pub const PAGE_FORMATS: Operation = Operation {
    name: "page.formats",
    description: "Reads the formats a request names from the page's current document, each read from one serialization of it, and answers only those, with the document's URL, its base URL and whether it has main content. A document with no main content answers empty formats and `main_content: \"none\"`, a value. It needs neither layout nor pixels, so every tier serves it.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<FormatsParams>,
    result: schema_of::<PageFormats>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        RUN_NEW_PAGE,
        PAGE_GOTO,
        PAGE_RELOAD,
        PAGE_GO_BACK,
        PAGE_GO_FORWARD,
        PAGE_FORMATS,
        PAGE_BODY,
        LOCATOR_CLICK,
        LOCATOR_FILL,
        LOCATOR_PRESS,
        PAGE_ROUTE,
        SESSION_ROUTE,
        ROUTE_CONTINUE,
        ROUTE_FALLBACK,
        ROUTE_FULFILL,
        ROUTE_ABORT,
        ROUTE_FETCH,
        PAGE_ON,
        RESPONSE_BODY,
        PAGE_WAIT_FOR_REQUEST_FINISHED,
    ],
    events: &[
        PAGE_DOWNLOAD,
        PAGE_CONSOLE,
        PAGE_PAGE_ERROR,
        PAGE_REQUEST,
        PAGE_REQUEST_FINISHED,
        PAGE_REQUEST_FAILED,
        PAGE_RESPONSE,
        PAGE_WEB_SOCKET,
        WEB_SOCKET_FRAME_SENT,
        WEB_SOCKET_FRAME_RECEIVED,
        WEB_SOCKET_CLOSE,
        PAGE_LOAD,
        PAGE_DOM_CONTENT_LOADED,
    ],
    handler_requests: &[ROUTE_HANDLE],
    error_codes: &[
        BODY_NOT_FOUND,
        BODY_UNAVAILABLE,
        NAVIGATION_FAILED,
        NAVIGATION_INTERRUPTED,
        TOO_MANY_REDIRECTS,
        RESPONSE_TRUNCATED,
        PAGE_CRASHED,
        BROWSER_LAUNCH_FAILED,
        BROWSER_HOST_OUTDATED,
        TARGET_NOT_UNIQUE,
        PATTERN_UNSUPPORTED,
        PAGE_HANDLER_FAILED,
        ROUTE_ALREADY_HANDLED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[BROWSER_HEADLESS],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
