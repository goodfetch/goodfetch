//! A page's raw CDP session (`page.newCDPSession()`, ADRs 0040, 0085, 0087,
//! 0092 and 0099): relayed over the program's own connection, reaching only
//! its page's target, refusing commands that attach or close targets or close
//! a Borrowed browser with `cdp_command_refused`, marking results unguarded,
//! recording the attach in the Audit log, stopping secret typing, holding the
//! page for area 3's pool, and bound to a handler's lease check.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ARRIVED_IN, PageId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Event, EventClass, Operation, OperationClass, OperationKind,
    Reach, SharedType, SharedTypeValue, Stability, schema_of,
};

/// A raw CDP session opened on a page via `page.new_cdp_session`.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct CdpSessionId(pub String);

/// What `page.new_cdp_session` takes: the page to attach to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageNewCdpSessionParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
}

/// What `page.new_cdp_session` answers: the opened CDP session's ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpSessionOpened {
    /// The opened CDP session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_session_id: CdpSessionId,
}

/// What `cdp_session.send` takes: the CDP session, the raw CDP method name,
/// and its optional JSON parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpSessionSendParams {
    /// The CDP session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_session_id: CdpSessionId,
    /// The CDP method, such as `Emulation.setCPUThrottlingRate` or
    /// `Runtime.evaluate`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The command's JSON parameters; omitted or `null` sends an empty object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub params: Option<Value>,
}

/// What `cdp_session.send` answers: the browser's raw JSON result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpSessionAnswer {
    /// The browser's JSON result.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub result: Value,
}

/// What `cdp_session.close` takes: the CDP session to close.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpSessionCloseParams {
    /// The CDP session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_session_id: CdpSessionId,
}

/// What `cdp_session.close` answers once the CDP session is closed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpSessionClosed {}

/// What `cdp_session.on` takes: the CDP session and the CDP event method name
/// to listen to (such as `Network.requestWillBeSent`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpSessionOnParams {
    /// The CDP session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_session_id: CdpSessionId,
    /// The CDP event method name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: String,
}

/// What `cdp_session.on` answers when the CDP session or its page ends.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpSessionOnEnded {}

/// What `cdp_session.event` carries: one raw CDP event from the page's target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpSessionEvent {
    /// The CDP session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_session_id: CdpSessionId,
    /// The CDP event method name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The event's raw JSON parameters.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub params: Value,
}

/// What `cdp_command_refused` details: the refused CDP method and why the
/// engine refused it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpCommandRefusedDetails {
    /// The refused CDP method name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// Why the command was refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
}

/// A raw CDP command refused by the engine (ADRs 0085 and 0089).
pub const CDP_COMMAND_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "cdp_command_refused",
    description: "The engine refused a raw CDP command sent through a CDP session or a Lent page: commands that attach or close other targets, close the target, or on a Borrowed browser close or crash the browser are refused. The details name the `method` and `reason`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Audit log kind recorded when a raw CDP session attaches to a page (ADR 0085).
pub const CDP_SESSION_ATTACHED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "cdp_session_attached",
    description: "A raw CDP session attached to a page through `page.new_cdp_session` (ADR 0085).",
    arrived_in: ARRIVED_IN,
};

/// `cdp_session.event`.
pub const CDP_SESSION_EVENT: Event = Event {
    name: "cdp_session.event",
    description: "A raw CDP event emitted by the page's target on an open CDP session.",
    class: EventClass::Lossless,
    payload: schema_of::<CdpSessionEvent>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.new_cdp_session`.
pub const PAGE_NEW_CDP_SESSION: Operation = Operation {
    name: "page.new_cdp_session",
    description: "Opens a raw CDP session on the page over the program's own connection, reaching only its page's target (ADR 0085). Marks results unguarded, records the attach in the Audit log, stops Secret typing while open, holds the page for the browser pool, and binds to a handler's lease check. Raises `capability_unavailable` on Firefox and WebKit.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<PageNewCdpSessionParams>,
    result: schema_of::<CdpSessionOpened>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `cdp_session.send`.
pub const CDP_SESSION_SEND: Operation = Operation {
    name: "cdp_session.send",
    description: "Sends a raw CDP command on the CDP session's page target without queuing behind a Collie call (ADR 0085). Refuses commands that attach or close other targets, close the target, or close a Borrowed browser with `cdp_command_refused`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CdpSessionSendParams>,
    result: schema_of::<CdpSessionAnswer>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `cdp_session.close`.
pub const CDP_SESSION_CLOSE: Operation = Operation {
    name: "cdp_session.close",
    description: "Closes the raw CDP session, releasing its page hold and external-reader mark; closing an already-closed CDP session succeeds.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CdpSessionCloseParams>,
    result: schema_of::<CdpSessionClosed>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `cdp_session.on`.
pub const CDP_SESSION_ON: Operation = Operation {
    name: "cdp_session.on",
    description: "Subscribes to a raw CDP event on the CDP session's page target until cancelled or the session, page or lease ends.",
    class: OperationClass::Read,
    kind: OperationKind::Event,
    reach: Reach::SdkOnly,
    params: schema_of::<CdpSessionOnParams>,
    result: schema_of::<CdpSessionOnEnded>,
    events: &[&CDP_SESSION_EVENT],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
