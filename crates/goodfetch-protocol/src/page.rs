//! A Run's pages (ADRs 0085, 0092 and 0099): `run.new_page` opens a page in
//! the Run's default Session, which the first call opens and every later
//! one shares, and `page.goto` takes a page to a URL and answers the
//! document's `Response`; and the Error codes a page's browser and its
//! navigations fail with.
//!
//! Page bodies as handles (ADRs 0024 and 0031): a body is a file in its
//! Run's folder, named by the digest of its bytes. A Client on the same
//! machine reads the file at the handle's path; any other reads it through
//! `page.body`, which sends it in `$/chunk` notifications, one per credit
//! the Client grants the call with `$/credit`, and answers once the last
//! chunk is sent.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
};
use crate::jsonrpc::RequestId;
use crate::run::RunId;

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

/// What `page.goto` takes: the page, and the URL it goes to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GotoParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
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
}

/// What `page.goto` answers: the document's response, or none for a
/// navigation within the same document or to a URL no server answers, such
/// as `about:blank`. A 404 or a 500 is a response, a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Navigated {
    /// The document's response.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub response: Option<PageResponse>,
}

/// What `navigation_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigationFailedDetails {
    /// The URL the navigation went to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The browser's network error, such as `net::ERR_NAME_NOT_RESOLVED`,
    /// when the browser named one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: Option<String>,
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
    description: "Navigates the page to a URL and waits for its `load` event, answering the document's response: a 404 or a 500 is a response, a value. A navigation that reaches no document raises `navigation_failed`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GotoParams>,
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

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_NEW_PAGE, PAGE_GOTO, PAGE_BODY],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        BODY_NOT_FOUND,
        NAVIGATION_FAILED,
        PAGE_CRASHED,
        BROWSER_LAUNCH_FAILED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[BROWSER_HEADLESS],
    hook_stages: &[],
    shared_type_values: &[],
};
