//! Borrowing the program's own Playwright or Puppeteer browser over the SDK's
//! JSON-RPC connection (`run.borrow()`, ADRs 0056, 0078, 0089 and 0099):
//! carries raw CDP frames in both directions under area 3's credits with no
//! listening port, maps each borrowed browser context to one Session of the
//! Run, never closes the borrowed browser, refuses Playwright's Firefox and
//! WebKit with `borrow_requires_cdp`, and ends on `$/cancel` or when the
//! crawler retires its browser.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId, SessionId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, Reach, Stability,
    StreamInput, schema_of,
};
use crate::jsonrpc::RequestId;
use crate::run::RunId;

/// The client -> engine notification carrying one raw CDP JSON frame from the
/// borrowed browser (or `end: true` when the crawler retires its browser).
pub const BORROW_FRAME: &str = "$/borrow_frame";

/// The engine -> client notification carrying one raw CDP JSON command to send
/// to the borrowed browser, governed by the client's `$/credit` grants.
pub const BORROW_COMMAND: &str = "$/borrow_command";

/// The engine -> client notification announcing the borrowed Session (and
/// optional Page) once the relay has attached to the borrowed browser.
pub const BORROW_READY: &str = "$/borrow_ready";

/// What `run.borrow` takes: the Run to borrow into, the browser family, and
/// optionally a specific page target and browser context ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunBorrowParams {
    /// The Run whose Session (and optional Page) the borrowed browser serves.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The browser engine kind the client is borrowing (`chromium`, `firefox`,
    /// `webkit`). Defaults to `chromium`; Firefox and WebKit are refused with
    /// `borrow_requires_cdp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser: Option<String>,
    /// Optional CDP target ID of an existing page to borrow immediately
    /// (`run.borrow(page)`). Omitting it borrows the browser/context alone
    /// (`run.borrow(browser)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target_id: Option<String>,
    /// Optional CDP `BrowserContextID` of the borrowed page or context.
    /// Multiple borrows in the same Run with the same `browser_context_id`
    /// share one Session of the Run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_context_id: Option<String>,
}

/// One client -> engine `$/borrow_frame` notification carrying raw CDP JSON
/// text from the borrowed browser (never parsed into a `Value` twice).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BorrowFrameFromClient {
    /// The `run.borrow` call's request ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// One raw CDP JSON frame (response or event) from the borrowed browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub frame: Option<String>,
    /// Set to `true` when the crawler retires or closes the borrowed browser.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub end: bool,
}

/// One engine -> client `$/borrow_command` notification carrying raw CDP JSON
/// text for the client to forward to its borrowed browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BorrowCommandToClient {
    /// The `run.borrow` call's request ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// One raw CDP JSON command from the engine.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub frame: String,
}

/// The engine -> client `$/borrow_ready` notification announcing the Session
/// (and optional Page) created or joined for this borrow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BorrowReady {
    /// The `run.borrow` call's request ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// The Session of the Run mapped to the borrowed browser context.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// The Collie Page attached to `target_id`, when `target_id` was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: Option<PageId>,
    /// The borrowed browser context ID mapped to `session_id`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_context_id: String,
}

/// What `run.borrow` answers once the borrow ends (by `$/cancel`, browser
/// retirement, or Run end).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BorrowEnded {
    /// Why the borrow ended (`cancelled`, `browser_closed`, `run_ended`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
}

/// What `borrow_requires_cdp` details: the non-CDP browser family refused and
/// the suggested fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BorrowRequiresCdpDetails {
    /// The browser family refused (for example `firefox` or `webkit`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser: String,
    /// What would work instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// `borrow_requires_cdp` (ADR 0056, ADR 0089).
pub const BORROW_REQUIRES_CDP: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "borrow_requires_cdp",
    description: "`run.borrow()` requires a Chromium-family browser speaking CDP: Playwright's Firefox and WebKit expose no CDP and cannot be borrowed (ADRs 0056 and 0089).",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `run.borrow`.
pub const RUN_BORROW: Operation = Operation {
    name: "run.borrow",
    description: "Relays the program's own Chromium-family browser (Playwright or Puppeteer) over the SDK's JSON-RPC connection with no port, under credits (ADRs 0056, 0078 and 0089). Each borrowed browser context maps to one Session of the Run, never closed by Collie; results are unguarded, Secrets raise `secret_input_refused`, `session.save_identity` raises `session_not_savable`, and Playwright's Firefox or WebKit raises `borrow_requires_cdp`. `$/cancel` or retiring the browser ends the borrow.",
    class: OperationClass::Effect,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<RunBorrowParams>,
    result: schema_of::<BorrowEnded>,
    events: &[],
    cancellable: true,
    input: Some(StreamInput {
        notification: BORROW_FRAME,
        field: "frame",
    }),
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
