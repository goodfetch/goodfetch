//! Lending a page to a third-party CDP library (`page.lend()`, ADRs 0017,
//! 0067, 0089 and 0099): opens a scoped WebSocket CDP endpoint that presents
//! a browser holding only the lent page's Session, checked by `Host`,
//! `Origin` and a per-attach header token, and closed with
//! `cdp_endpoint.close`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId, SessionId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, Reach, Stability,
    schema_of,
};

/// A lending endpoint opened on a page via `page.lend`.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct CdpEndpointId(pub String);

/// What `page.lend` takes: the page to lend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageLendParams {
    /// The page to lend.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
}

/// What `page.lend` answers: the lending endpoint's handle ID, WebSocket URL,
/// and authentication headers carrying its per-attach token (ADR 0089).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpEndpointOpened {
    /// The opened lending endpoint.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_endpoint_id: CdpEndpointId,
    /// The WebSocket CDP address the library connects to (loopback by default).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The HTTP headers carrying the per-attach token that the library must
    /// send on its WebSocket handshake.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: BTreeMap<String, String>,
}

/// What `cdp_endpoint.close` takes: the lending endpoint to close.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpEndpointCloseParams {
    /// The lending endpoint.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cdp_endpoint_id: CdpEndpointId,
}

/// What `cdp_endpoint.close` answers once the lending endpoint is closed and
/// the Session's state is re-applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpEndpointClosed {}

/// What `session_reserved` details: the Session that already has an active
/// lending attach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionReservedDetails {
    /// The Session that is already reserved by an open attach.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
}

/// A Session is already reserved for another attach (ADR 0089).
pub const SESSION_RESERVED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "session_reserved",
    description: "The Session already has an open lending attach (`page.lend()`): a second lend on the same Session is refused until the active attach ends so one attach's end never restores state under another (ADR 0089).",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `page.lend`.
pub const PAGE_LEND: Operation = Operation {
    name: "page.lend",
    description: "Opens a scoped WebSocket CDP endpoint for a third-party automation library (Playwright, Puppeteer, Browser Use) showing only the lent page's Session (ADR 0089). Checks `Host`, `Origin` and a per-attach header token, marks the Session `Lent` (refusing a second lend on the Session with `session_reserved`), marks program results unguarded, stops Secret typing while attached, and re-applies Session state when the attach ends. Raises `capability_unavailable` on Firefox, WebKit, or a `stealth` Session without `stealth.allow_runtime`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<PageLendParams>,
    result: schema_of::<CdpEndpointOpened>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `cdp_endpoint.close`.
pub const CDP_ENDPOINT_CLOSE: Operation = Operation {
    name: "cdp_endpoint.close",
    description: "Ends a Lent page's attach, revokes its token, closes pages the library created, re-applies the Session's state, and returns the Session to `Open`; closing an already-closed endpoint succeeds (ADR 0089).",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<CdpEndpointCloseParams>,
    result: schema_of::<CdpEndpointClosed>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
