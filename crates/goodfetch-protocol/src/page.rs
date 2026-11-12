//! A Run's pages (ADRs 0085, 0092 and 0099): `run.new_page` opens a page in
//! the Run's default Session, which the first call opens and every later
//! one shares; `page.goto`, `page.reload`, `page.go_back` and
//! `page.go_forward` navigate a page and answer the document's `Response`,
//! or none for a file the browser downloads, which `page.download` reports;
//! `locator.click`, `locator.fill` and `locator.press` act on the one
//! element a locator matches, through the guard (ADR 0019); and the Error
//! codes a page's browser, its navigations, its locators and the URL
//! patterns its calls pass fail with. Its `route` module holds the page's
//! and the Session's routes, its `session` module a Run's own Sessions,
//! its `events` module the page's events and its responses' bodies, its
//! `request` module `page.request`, the request API on a page's Session,
//! its `proxy` module a Session's proxy, its `dialog` module a page's
//! JavaScript dialogs, its `tabs` module a Session's pages and popups
//! and `page.close`, its `functions` module the page functions,
//! `page.evaluate` and `page.add_init_script`, and its `waits` module the
//! app-state waits, each armed when its call arrives.
//!
//! Page bodies as handles (ADRs 0024 and 0031): a body is a file in its
//! Run's folder, named by the digest of its bytes. A Client on the same
//! machine reads the file at the handle's path; any other reads it through
//! `page.body`, which sends it in `$/chunk` notifications, one per credit
//! the Client grants the call with `$/credit`, and answers once the last
//! chunk is sent.
//!
//! Size: one catalog module, the contract of a Run's pages: their
//! operations, Error codes and `browser.*` settings, kept whole so each
//! entry sits beside the others it is listed with.

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::block::BlockCause;
use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation,
    OperationClass, OperationKind, ProtocolVersion, Reach, Setting, Stability, ValueKind,
    schema_of,
};
use crate::jsonrpc::{ErrorData, RequestId};
use crate::run::RunId;
use crate::settings::SettingLimit;

pub mod dialog;
pub mod events;
pub mod functions;
pub mod proxy;
pub mod request;
pub mod route;
pub mod session;
pub mod session_pages;
pub mod waits;

use self::dialog::{
    DIALOG_ACCEPT, DIALOG_ALREADY_HANDLED, DIALOG_DISMISS, DIALOG_HANDLE, PAGE_ON_DIALOG,
};
use self::events::{
    BODY_UNAVAILABLE, PAGE_CLOSE, PAGE_CONSOLE, PAGE_CRASH, PAGE_DOM_CONTENT_LOADED,
    PAGE_EVENTS_LOST, PAGE_LOAD, PAGE_ON, PAGE_PAGE_ERROR, PAGE_REQUEST, PAGE_REQUEST_FAILED,
    PAGE_REQUEST_FINISHED, PAGE_RESPONSE, PAGE_WAIT_FOR_REQUEST_FINISHED, PAGE_WEB_SOCKET,
    RESPONSE_BODY, WEB_SOCKET_CLOSE, WEB_SOCKET_FRAME_RECEIVED, WEB_SOCKET_FRAME_SENT,
};
use self::functions::{
    LOCATOR_EVALUATE_ALL, PAGE_ADD_INIT_SCRIPT, PAGE_EVALUATE, PAGE_FUNCTION_FAILED,
    PAGE_VALUE_INVALID,
};
use self::route::{
    PAGE_HANDLER_FAILED, PAGE_ROUTE, ROUTE_ABORT, ROUTE_ALREADY_HANDLED, ROUTE_CONTINUE,
    ROUTE_FALLBACK, ROUTE_FETCH, ROUTE_FULFILL, ROUTE_HANDLE, SESSION_ROUTE,
};
use self::session::{RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE};
use self::session_pages::{PAGE_POPUP, SESSION_ON, SESSION_PAGE, SESSION_PAGES};
use self::waits::{
    LOCATOR_WAIT_FOR, LOCATOR_WAIT_FOR_COUNT, LOCATOR_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_FUNCTION,
    PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_REQUEST, PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A field that is present, `null` included, as `Some`; one left out stays
/// `None` through `#[serde(default)]`.
#[expect(
    clippy::option_option,
    reason = "three states (ADRs 0080 and 0083): left out inherits, `null` clears, a value sets"
)]
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

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
    input: None,
    bound: None,
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

/// What `run.new_page` takes: the Run whose default Session opens the page,
/// and the page's own settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewPageParams {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The page's own settings, nested by key (ADR 0083), which it takes
    /// over its Session's and its Run's: `content.extractor`, the
    /// main-content step its formats read with. Any other key is refused
    /// with `setting_not_applicable`, as only its Session or its Run sets
    /// it.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

/// What `run.new_page` and `session.new_page` answer: the page, and the
/// Session it belongs to, which `page.session()` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageOpened {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// Its Session: the Run's default Session for `run.new_page`.
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
    /// How long the navigation may take, in milliseconds, at least 1:
    /// absent for `timeout.navigation`, `null` for none, though an
    /// enclosing timeout still applies (ADR 0080). Its waits for permits
    /// count against it no more than against the setting.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timeout: Option<Option<u64>>,
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
    /// The engine's ID for its request's hop, which `response.body` takes:
    /// its request's `request_id`.
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
    /// The URL the navigation went to; none when the page's history, which
    /// names it for a reload or a step back or forward, did not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
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

/// What judged a page: the Session's own Page check, or the built-in
/// detector of a Session that declares none (ADR 0006).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JudgedBy {
    /// The Session's `session.page_check`.
    PageCheck,
    /// The built-in detector, whose pages are weakly checked.
    BuiltIn,
}

/// What `page_check_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageCheckFailedDetails {
    /// The URL of the document the navigation reached.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its HTTP status.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// Whether the document is marked `noindex`, by a `noindex` or `none`
    /// directive in its `X-Robots-Tag` header or a robots `meta` element:
    /// a crawl takes such a page for a decoy (ADR 0035).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub noindex: bool,
}

/// What `logged_out` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoggedOutDetails {
    /// The URL of the login page the navigation reached.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its HTTP status.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// What judged it a login page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub judged_by: JudgedBy,
}

/// What `blocked` details (ADRs 0092 and 0118).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BlockedDetails {
    /// The Block cause, kept as received by an SDK that does not know it
    /// (ADR 0079).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cause: BlockCause,
    /// The URL of the document that met it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its HTTP status; `null` for a document never fetched, as a crawl
    /// request's page reached through a suspected trap link, a decoy a
    /// browser tier never opens (ADR 0035).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: Option<u16>,
    /// The time the site's `Retry-After` asks for, in RFC 3339, read in
    /// either of its forms; `null` when it asks for none. The engine never
    /// waits on it for a program's own navigation.
    #[schemars(with = "Option<String>", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub retry_after: Option<jiff::Timestamp>,
}

/// The count of redirects a fetch follows at most, and what set it (ADR
/// 0118).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedirectLimit {
    /// The browser's own count, 20 requests in a navigation's chain, its
    /// first included, which a navigation cannot change: it follows 19
    /// redirects (ADR 0118).
    Browser {
        /// The requests the chain may hold, its first included: 20, one
        /// more than the redirects followed, which `hops` lists.
        #[schemars(extend("arrived_in" = "0.1"))]
        count: u32,
    },
    /// A setting, such as `fetch.redirect.max`, at its resolved value.
    Setting {
        /// The setting's key.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
        /// The redirects it allows, the chain's first request left out.
        #[schemars(extend("arrived_in" = "0.1"))]
        count: u32,
    },
    /// A call's option, such as `page.request`'s `maxRedirects`.
    Option {
        /// The option's name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
        /// The redirects it allows, the chain's first request left out.
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

/// Which of a site's preferences refused a navigation (ADR 0086, as ADR
/// 0093 amends it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SitePreference {
    /// The site's robots.txt disallows the URL.
    RobotsTxt,
    /// The site's robots.txt `Crawl-delay` asks for a wait longer than the
    /// crawl allows.
    CrawlDelay,
    /// A use signal the site sends (aipref's `Content-Usage` header,
    /// `TDMRep`'s headers, RSL's `Link`) refuses the crawl's Purpose.
    UseSignals,
}

/// What `site_preferences_refused` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SitePreferencesRefusedDetails {
    /// The URL of the navigation or request it refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The preference that refused it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: SitePreference,
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

/// A page that is neither its Session's expected page nor its login page
/// (ADRs 0006 and 0079).
pub const PAGE_CHECK_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_check_failed",
    description: "The document the navigation reached is neither the expected page nor the login page of its Session's Page check, `session.page_check`, whatever its HTTP status: a page the program did not mean to reach, such as an error page or a layout the site changed. The page stays on that document. The details name its URL and status.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A page judged a login page: the Session's login is lost (ADRs 0006 and
/// 0092).
pub const LOGGED_OUT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "logged_out",
    description: "The document the navigation reached is a login page, even one answered with a 200: the Session's login is lost, judged by its Page check's `login_page`, or by the built-in detector when it declares none. The page stays on the login page, the Session open with its cookies. The details name the URL, the status and what judged it.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Block cause the engine cannot get past on a Session a program or an
/// Agent opened (ADRs 0088, 0092 and 0118).
pub const BLOCKED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "blocked",
    description: "The site refused the document a navigation reached with a Block cause the engine cannot get past, such as a rate limit, an IP block or a challenge: raised by the call that waited for that navigation, or else by the page's next call. The Session stays open with its cookies, never replaced, and nothing waits on the site's `Retry-After`: the program decides whether to wait, open another Session or pause for a person. The details name the cause, the URL, the status and `retry_after`, the time the site asked for or `null`.",
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

/// A navigation or request a site's preferences refused, at the page's
/// navigation gate (ADRs 0086, 0087 and 0093).
pub const SITE_PREFERENCES_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "site_preferences_refused",
    description: "The site's preferences refused the navigation or the request before it left, or refused the document once its response arrived, leaving the page on a blank document: its robots.txt disallows the URL (`robots_txt`), its `Crawl-delay` asks for more than the crawl allows (`crawl_delay`), or a use signal it sends refuses the crawl's Purpose (`use_signals`). Only a crawl handler's page asks a site's preferences; a page a program opens never raises it. The details name the URL and the reason.",
    group: ErrorGroup::Refused,
    retryable: false,
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
    description: "Clicks the element the locator matches once it is unique, visible, enabled, stable and receiving the click (the guard, ADR 0019); a click that lands on another element is cancelled and sent again, until `timeout.action` passes. A click that starts a navigation of the page answers once that navigation's document committed or it ended without one, within `timeout.navigation` (ADR 0085).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ClickParams>,
    result: schema_of::<Acted>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.fill`.
pub const LOCATOR_FILL: Operation = Operation {
    name: "locator.fill",
    description: "Replaces the text of the field the locator matches, once it is unique, visible, enabled and editable, with text the page receives as typed input, until `timeout.action` passes. An input whose value a person picks rather than types (a date, a time, a month, a week, a local date and time, a colour or a range) has its value set, firing `input` and `change`, and a value it does not take fails with `invalid_params`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<FillParams>,
    result: schema_of::<Acted>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `locator.press`.
pub const LOCATOR_PRESS: Operation = Operation {
    name: "locator.press",
    description: "Focuses the element the locator matches, once it is unique, visible and enabled, and presses a key or a combination on it as a keyboard does, until `timeout.action` passes. A press that starts a navigation of the page answers once that navigation's document committed or it ended without one, within `timeout.navigation` (ADR 0085).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PressParams>,
    result: schema_of::<Acted>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
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
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.goto`.
pub const PAGE_GOTO: Operation = Operation {
    name: "page.goto",
    description: "Navigates the page to a URL and waits for its document's `load` event, or the state `wait_until` names, answering the document's response: a 404 or a 500 is a response, a value. A file the browser downloads, such as a PDF, answers no response and emits `page.download`. A navigation that reaches no document raises `navigation_failed`, one another navigation supersedes before it committed `navigation_interrupted`, a redirect loop `too_many_redirects`, a document whose body broke off `response_truncated`, unless the navigation answered at `commit`, and one past its `timeout`, or `timeout.navigation` without one, `deadline_exceeded`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GotoParams>,
    result: schema_of::<Navigated>,
    events: &[&PAGE_DOWNLOAD],
    cancellable: true,
    input: None,
    bound: None,
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
    input: None,
    bound: None,
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
    input: None,
    bound: None,
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
    input: None,
    bound: None,
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
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn headless() -> serde_json::Value {
    serde_json::Value::Bool(true)
}

/// The resource kinds `browser.block_resources` takes, a closed list (ADR
/// 0117).
pub const BLOCKABLE_RESOURCES: &[&str] = &["image", "media", "font", "stylesheet"];

/// `browser.block_resources` (ADRs 0047 and 0117; spec #577).
pub const BROWSER_BLOCK_RESOURCES: Setting = Setting {
    name: "browser.block_resources",
    description: "The resource kinds the Session's pages never load, failed as `BlockedByClient` by its interception before every navigation: any of `image`, `media`, `font` and `stylesheet`; none by default. The never-block list still loads a challenge's own. In `standard` and `stealth` the Run warns once per Session that the site can see the missing loads.",
    kind: ValueKind::List(&ValueKind::Choice(BLOCKABLE_RESOURCES)),
    default: no_resources,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn no_resources() -> serde_json::Value {
    serde_json::Value::Array(Vec::new())
}

/// The values `browser.service_workers` takes, Playwright's (ADR 0121).
pub const SERVICE_WORKER_MODES: &[&str] = &["allow", "block"];

/// `browser.service_workers` (ADRs 0083 and 0121; spec #577).
pub const BROWSER_SERVICE_WORKERS: Setting = Setting {
    name: "browser.service_workers",
    description: "Whether a site's service workers run in the Session's pages: `allow` (the default on Chromium) runs them under the Session's interception, each worker's own requests passing the domain policy, the Filter lists and the Session's routes, never a page's routes; `block` makes `navigator.serviceWorker.register()` reject as on a failed fetch of the worker's script, `register` staying the browser's own function, so that every request of a page reaches its routes.",
    kind: ValueKind::Choice(SERVICE_WORKER_MODES),
    default: allow,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn allow() -> serde_json::Value {
    serde_json::Value::String("allow".to_owned())
}

/// The values of `content.extractor`: the built-in main-content steps
/// (ADR 0007).
pub const CONTENT_EXTRACTORS: &[&str] = &["dom_smoothie", "trafilatura"];

/// `content.extractor` (ADRs 0007, 0083 and 0104; spec #580).
pub const CONTENT_EXTRACTOR: Setting = Setting {
    name: "content.extractor",
    description: "The main-content step `page.formats` reads a page's main content with, before its Markdown, its main HTML and its text: `dom_smoothie` (the default), Readability's algorithm, or `trafilatura`, the rs-trafilatura port. A step that finds no text in a document holding text outside its navigation landmarks answers that text. Set at any scope down to a page: a Run, a Session's or a page's own `settings`.",
    kind: ValueKind::Choice(CONTENT_EXTRACTORS),
    default: dom_smoothie,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn dom_smoothie() -> serde_json::Value {
    serde_json::Value::String("dom_smoothie".to_owned())
}

/// The values `extraction.removal_share` takes (ADR 0138).
pub const REMOVAL_SHARES: &[&str] = &["estimate", "exact"];

/// `extraction.removal_share` (ADR 0138): how a `page.formats` call
/// measures ADR 0047's share of its main content the Clean-up rules took.
pub const EXTRACTION_REMOVAL_SHARE: Setting = Setting {
    name: "extraction.removal_share",
    description: "How a `page.formats` call that read its main content measures the share of that text the Clean-up rules removed, which its Provenance entry records, flagged above a fifth: `estimate` (the default), the text removed over that text plus the main content's text left, an upper bound of the exact share that runs no second main-content step; or `exact`, the main content's text of the page as served against the cleaned page's, at the cost of one more main-content step. The entry names the measure it holds.",
    kind: ValueKind::Choice(REMOVAL_SHARES),
    default: estimate,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn estimate() -> serde_json::Value {
    serde_json::Value::String("estimate".to_owned())
}

/// The name the Run trace's request log records a service worker's own
/// request under, beside its pages' events (ADR 0121): no page sent it, so
/// no page's event reports it.
pub const SERVICE_WORKER_REQUEST: &str = "service_worker.request";

/// What the Run trace records of a request a site's service worker sent
/// itself, which the Session's interception sent on or refused (ADR 0121).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceWorkerRequest {
    /// The Session whose interception held it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// Its URL, as the last route sent it on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its method, such as `GET`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// What the worker loads with it, as Playwright's `resourceType()`
    /// names it, such as `fetch`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resource_type: String,
    /// Why the interception refused it, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: Option<events::RequestFailure>,
}

/// The values `live_view.hiding` takes (ADR 0047).
pub const LIVE_HIDING_MODES: &[&str] = &["auto", "off", "always"];

/// `live_view.hiding` (ADR 0047's `live_hiding`, by ADR 0083's name;
/// spec #577).
pub const LIVE_VIEW_HIDING: Setting = Setting {
    name: "live_view.hiding",
    description: "Whether a Session's pages hide, on the live page, the elements the Filter lists' cosmetic rules match: `auto` (the default) and `always` hide them through a style sheet the site cannot see on Chromium, in every frame and every new document; `off` leaves the live page as served. When the guard finds an action's target hidden, it lifts the rule for the Session and acts. Extracted content has the elements removed whatever this says.",
    kind: ValueKind::Choice(LIVE_HIDING_MODES),
    default: auto,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn auto() -> serde_json::Value {
    serde_json::Value::String("auto".to_owned())
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
    /// The whole body, after the Clean-up rules.
    Full,
    /// The main content without its link-dense blocks (more than half
    /// their text in links), its image-only blocks and the navigation
    /// lists the document repeats: the "fit" of `Crawl4AI`.
    Fit,
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

/// Which of the page's HTML an `html` format holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HtmlKind {
    /// The document as served, or as the DOM stands on a browser tier,
    /// before the Clean-up rules: what `content()` returns there.
    Raw,
    /// The document after the Clean-up rules' cosmetic rules.
    Cleaned,
    /// The main content the extractor finds, the default.
    #[default]
    Main,
}

/// The options of the `html` format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HtmlOptions {
    /// Which of the page's HTML it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: HtmlKind,
}

/// The `html` format of a request: `true`, or its options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum HtmlFormat {
    /// With its default options.
    Asked(Asked),
    /// With these options.
    Options(HtmlOptions),
}

impl HtmlFormat {
    /// Which of the page's HTML it holds.
    pub fn kind(self) -> HtmlKind {
        match self {
            Self::Asked(Asked) => HtmlKind::default(),
            Self::Options(options) => options.kind,
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
    /// The page's HTML: its main content by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub html: Option<HtmlFormat>,
    /// The text of the page's main content, a paragraph per block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: Option<Asked>,
    /// The page's links, each with its URL resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub links: Option<Asked>,
    /// The page's images, each with its URL resolved and its `alt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub images: Option<Asked>,
    /// The page's video and audio sources, each with its URL resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub media: Option<Asked>,
    /// The page's tables, as rows of cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tables: Option<Asked>,
    /// The page's metadata: its title, description, canonical URL,
    /// language, Open Graph and Twitter tags, JSON-LD blocks and microdata
    /// items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub metadata: Option<Asked>,
    /// The e-mail addresses, telephone numbers and social profiles the
    /// page shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub contacts: Option<Asked>,
}

impl FormatsRequest {
    /// Whether it names a format read from the page's main content: the
    /// Markdown of the main content or "fit", the main content's HTML, or
    /// `text`. Only then does its result say whether the page has main
    /// content, so a call that asks for none costs no main-content step.
    pub fn reads_main_content(&self) -> bool {
        self.markdown.is_some_and(|format| {
            matches!(
                format.content(),
                MarkdownContent::Main | MarkdownContent::Fit
            )
        }) || self
            .html
            .is_some_and(|format| format.kind() == HtmlKind::Main)
            || self.text.is_some()
    }
}

/// A link of the page: an `<a href>` or `<area href>`, in document order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// Its `href` resolved against the document's base URL by the WHATWG
    /// URL Standard; `null` when it does not parse.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// Its `href` as the document writes it, its character references
    /// decoded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub href: String,
    /// Its text, white space collapsed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The tokens of its `rel`, as written; empty when it has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rel: Vec<String>,
}

/// An image of the page: an `<img src>`, in document order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Image {
    /// Its `src` resolved against the document's base URL; `null` when it
    /// does not parse.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// Its `src` as the document writes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub src: String,
    /// Its `alt`: empty for an image marked decorative, `null` when it has
    /// none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub alt: Option<String>,
}

/// Whether a media source plays video or audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// A `<video>`'s, its own `src` or one of its `<source>`s.
    Video,
    /// An `<audio>`'s, its own `src` or one of its `<source>`s.
    Audio,
}

/// A media source of the page: a `<video src>`, an `<audio src>` or a
/// `<source src>` of either, in document order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Media {
    /// Its `src` resolved against the document's base URL; `null` when it
    /// does not parse.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// Its `src` as the document writes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub src: String,
    /// Whether it plays video or audio.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: MediaKind,
    /// Its `type`, the MIME type a `<source>` declares; `null` when it has
    /// none.
    #[serde(rename = "type")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub media_type: Option<String>,
}

/// A table of the page, in document order, a table inside another listed
/// on its own: each row its cells' text, white space collapsed, a row's
/// header kept as its first cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Its `<caption>`'s text; `null` when it has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub caption: Option<String>,
    /// Its header rows: its `<thead>`'s, else the rows it begins with whose
    /// cells are all `<th>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub header_rows: Vec<Vec<String>>,
    /// Its other rows, its `<tfoot>`'s last.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body_rows: Vec<Vec<String>>,
}

/// One `<meta>` tag of a vocabulary, Open Graph's or Twitter's, in
/// document order: a property a page writes more than once, such as
/// `og:image`, is listed once per tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetaTag {
    /// Its `property`, else its `name`, as written, such as `og:title`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub property: String,
    /// Its `content`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content: String,
}

/// A microdata item (the HTML Standard's microdata), as its JSON
/// conversion writes it, with every field given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MicrodataItem {
    /// Its `itemtype` tokens; empty when it has none.
    #[serde(rename = "type")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub types: Vec<String>,
    /// Its `itemid`, resolved against the document's base URL; `null` when
    /// it has none or it does not parse.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: Option<String>,
    /// Its properties, by name, each name's values in tree order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub properties: BTreeMap<String, Vec<MicrodataValue>>,
}

/// The value of a microdata property: text, or an item of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MicrodataValue {
    /// Text: the element's content, value or resolved URL, as the HTML
    /// Standard reads each element.
    Text(String),
    /// An item, the element having `itemscope`.
    Item(MicrodataItem),
}

/// The page's metadata, each value as the document writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    /// Its first `<title>`'s text, white space collapsed; `null` when it
    /// has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub title: Option<String>,
    /// Its `<meta name="description">`'s content; `null` when it has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub description: Option<String>,
    /// Its `<link rel="canonical">`'s `href` resolved against the base URL;
    /// `null` when it has none or it does not parse.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub canonical: Option<String>,
    /// Its root element's `lang`; `null` when it has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lang: Option<String>,
    /// Its Open Graph tags: every `<meta property>` not named `twitter:`,
    /// and every `<meta name>` named `og:`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub open_graph: Vec<MetaTag>,
    /// Its Twitter tags, those named `twitter:`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub twitter: Vec<MetaTag>,
    /// Each `<script type="application/ld+json">` block that parses, as
    /// JSON, in document order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub json_ld: Vec<Value>,
    /// Its top-level microdata items, in tree order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub microdata: Vec<MicrodataItem>,
    /// Its `<meta name>` tags other than Open Graph's and Twitter's, by name
    /// in lower case, the first of a name kept.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub meta: BTreeMap<String, String>,
}

/// An e-mail address the page shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Email {
    /// The address.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
    /// The text it was read from: its `mailto:` link's text, else the
    /// address as the text writes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
}

/// A telephone number the page shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Phone {
    /// The number as written: its `tel:` link's number, else the text's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
    /// The text it was read from: its `tel:` link's text, else the number
    /// as the text writes it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The number in E.164, such as `+33123456789`, when it carries its
    /// country code or the document's language tag carries a region, and
    /// it is a valid number there; `null` otherwise, as no country is
    /// guessed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub e164: Option<String>,
}

/// A social network whose profiles the page links to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SocialNetwork {
    /// X, formerly Twitter.
    X,
    /// `LinkedIn`.
    Linkedin,
    /// Facebook.
    Facebook,
    /// Instagram.
    Instagram,
    /// `YouTube`.
    Youtube,
    /// `TikTok`.
    Tiktok,
    /// `GitHub`.
    Github,
    /// A Mastodon instance's profile, linked with `rel="me"`.
    Mastodon,
}

/// A social profile the page links to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SocialProfile {
    /// The profile's URL, resolved against the document's base URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
    /// Its link's text, white space collapsed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// Its network.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network: SocialNetwork,
}

/// The contacts the page shows, each value once, where it first appears.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contacts {
    /// Its e-mail addresses, from `mailto:` links and from text matching
    /// the HTML Standard's valid e-mail address syntax.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub emails: Vec<Email>,
    /// Its telephone numbers, from `tel:` links and from text.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub phones: Vec<Phone>,
    /// Its social profiles, from links to the extractor's list of networks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub social: Vec<SocialProfile>,
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
/// none it did not, with the document's URL, its base URL and, when a
/// format it asked for is read from the main content, whether the document
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
    /// Whether it has main content, when the request asked for a format
    /// read from it ([`FormatsRequest::reads_main_content`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "MainContent", extend("arrived_in" = "0.1"))]
    pub main_content: Option<MainContent>,
    /// Its content as Markdown, when the request asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub markdown: Option<String>,
    /// Its HTML of the kind the request named, when it asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub html: Option<String>,
    /// Its main content's text, when the request asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub text: Option<String>,
    /// Its links, when the request asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<Link>", extend("arrived_in" = "0.1"))]
    pub links: Option<Vec<Link>>,
    /// Its images, when the request asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<Image>", extend("arrived_in" = "0.1"))]
    pub images: Option<Vec<Image>>,
    /// Its media sources, when the request asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<Media>", extend("arrived_in" = "0.1"))]
    pub media: Option<Vec<Media>>,
    /// Its tables, when the request asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<Table>", extend("arrived_in" = "0.1"))]
    pub tables: Option<Vec<Table>>,
    /// Its metadata, when the request asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Metadata", extend("arrived_in" = "0.1"))]
    pub metadata: Option<Metadata>,
    /// Its contacts, when the request asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Contacts", extend("arrived_in" = "0.1"))]
    pub contacts: Option<Contacts>,
}

/// The JSON Schema of what `page.formats` answers to `request`: the
/// result's envelope, `main_content` when the request reads the main
/// content, and each format it asks for, all required, and no other
/// property, so a code-free crawl writes its Dataset's schema from
/// the request alone (ADR 0103). Derived from [`PageFormats`]' own schema
/// as the engine writes it, so a field that may be `null`, such as a
/// link's URL, is still required.
pub fn page_formats_schema(request: &FormatsRequest) -> Value {
    let written = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<PageFormats>();
    let mut schema = serde_json::to_value(written).expect("invariant: a JSON Schema is JSON");
    let Ok(Value::Object(mut asked)) = serde_json::to_value(request) else {
        unreachable!("invariant: a formats request serializes as an object");
    };
    if request.reads_main_content() {
        asked.insert("main_content".to_owned(), Value::Bool(true));
    }
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
    description: "Reads the formats a request names from the page's current document, each read from one serialization of it, and answers only those, with the document's URL, its base URL and, when a format it names is read from the main content (`markdown` of the main content or fit, `html` of the main content, `text`), whether the document has main content. A document with no main content answers those formats empty and `main_content: \"none\"`, a value. It needs neither layout nor pixels, so every tier serves it.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<FormatsParams>,
    result: schema_of::<PageFormats>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        RUN_NEW_PAGE,
        RUN_NEW_SESSION,
        SESSION_NEW_PAGE,
        SESSION_CLOSE,
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
        request::PAGE_REQUEST,
        PAGE_ON_DIALOG,
        DIALOG_ACCEPT,
        DIALOG_DISMISS,
        SESSION_ON,
        SESSION_PAGES,
        session_pages::PAGE_CLOSE,
        PAGE_EVALUATE,
        LOCATOR_EVALUATE_ALL,
        PAGE_ADD_INIT_SCRIPT,
        PAGE_WAIT_FOR_URL,
        PAGE_WAIT_FOR_REQUEST,
        PAGE_WAIT_FOR_RESPONSE,
        PAGE_WAIT_FOR_LOAD_STATE,
        PAGE_WAIT_FOR_FUNCTION,
        LOCATOR_WAIT_FOR_FUNCTION,
        LOCATOR_WAIT_FOR,
        LOCATOR_WAIT_FOR_COUNT,
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
        PAGE_CRASH,
        PAGE_CLOSE,
        PAGE_POPUP,
        SESSION_PAGE,
    ],
    handler_requests: &[ROUTE_HANDLE, DIALOG_HANDLE],
    error_codes: &[
        BODY_NOT_FOUND,
        BODY_UNAVAILABLE,
        PAGE_EVENTS_LOST,
        NAVIGATION_FAILED,
        NAVIGATION_INTERRUPTED,
        PAGE_CHECK_FAILED,
        LOGGED_OUT,
        BLOCKED,
        TOO_MANY_REDIRECTS,
        RESPONSE_TRUNCATED,
        SITE_PREFERENCES_REFUSED,
        PAGE_CRASHED,
        BROWSER_LAUNCH_FAILED,
        BROWSER_HOST_OUTDATED,
        TARGET_NOT_UNIQUE,
        PATTERN_UNSUPPORTED,
        PAGE_HANDLER_FAILED,
        ROUTE_ALREADY_HANDLED,
        DIALOG_ALREADY_HANDLED,
        PAGE_FUNCTION_FAILED,
        PAGE_VALUE_INVALID,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        BROWSER_HEADLESS,
        BROWSER_BLOCK_RESOURCES,
        BROWSER_SERVICE_WORKERS,
        LIVE_VIEW_HIDING,
        CONTENT_EXTRACTOR,
        EXTRACTION_REMOVAL_SHARE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
