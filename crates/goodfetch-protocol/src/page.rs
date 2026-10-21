//! A Run's pages (ADRs 0085, 0092 and 0099): `run.new_page` opens a page in
//! the Run's default Session, which the first call opens and every later
//! one shares; `page.goto`, `page.reload`, `page.go_back` and
//! `page.go_forward` navigate a page and answer the document's `Response`;
//! `locator.click`, `locator.fill` and `locator.press` act on the one
//! element a locator matches, through the guard (ADR 0019); and the Error
//! codes a page's browser, its navigations and its locators fail with.
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
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::jsonrpc::RequestId;
use crate::run::RunId;
use crate::settings::SettingLimit;

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
}

/// What a navigation answers: the document's response, or none for a
/// navigation within the same document, to a URL no server answers, such as
/// `about:blank`, or back or forward from the last entry of the page's
/// history. A 404 or a 500 is a response, a value.
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
    description: "Navigates the page to a URL and waits for its `load` event, answering the document's response: a 404 or a 500 is a response, a value. A navigation that reaches no document raises `navigation_failed`, and one another navigation supersedes before it committed `navigation_interrupted`.",
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

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        RUN_NEW_PAGE,
        PAGE_GOTO,
        PAGE_RELOAD,
        PAGE_GO_BACK,
        PAGE_GO_FORWARD,
        PAGE_BODY,
        LOCATOR_CLICK,
        LOCATOR_FILL,
        LOCATOR_PRESS,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        BODY_NOT_FOUND,
        NAVIGATION_FAILED,
        NAVIGATION_INTERRUPTED,
        PAGE_CRASHED,
        BROWSER_LAUNCH_FAILED,
        TARGET_NOT_UNIQUE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
