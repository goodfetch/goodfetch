//! A Session's pages (ADR 0085): there is no active page, and each one is a
//! Page of its Session, whether the program opened it or a page opened it
//! as a popup. A popup is a page of its opener's Session, announced by its
//! opener's `page.popup` and its Session's `session.page`, both published
//! once the popup is under the Session's interception and before its first
//! request leaves, so a listener armed before the action that opens it
//! never misses it; `session.pages` lists the Session's open pages, the
//! popups among them, whenever a listener came too late. `page.close`
//! closes a page, a popup as any other, without running its
//! `beforeunload` handlers unless asked.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId, PageOpened, SessionId};
use crate::catalog::{
    Event, EventClass, Operation, OperationClass, OperationKind, Reach, Stability, schema_of,
};

/// What `page.popup` reports: the page that opened a popup, and the
/// popup, a page of the same Session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PagePopup {
    /// The opener.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The popup.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub popup: PageOpened,
}

/// A Session's event a program listens to.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventName {
    /// `session.page`.
    Page,
}

/// What `session.on` takes: the Session and the event listened to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionOnParams {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// The event.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: SessionEventName,
}

/// What `session.on` answers when its Session goes, closed or with its
/// Run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionOnEnded {}

/// What `session.pages` answers: the Session's open pages, in the order
/// they opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionPages {
    /// The pages.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pages: Vec<PageId>,
}

/// What `page.close` takes: the page, and whether its `beforeunload`
/// handlers run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageCloseParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// Runs the page's `beforeunload` handlers first, whose dialog goes to
    /// the page's dialog handlers; false when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_before_unload: Option<bool>,
}

/// What `page.close` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageClosed {}

/// Playwright's `popup`.
pub const PAGE_POPUP: Event = Event {
    name: "page.popup",
    description: "The page opened a popup, `window.open()` or a link opening another window: a page of the same Session, already under its routes and its interception, published before the popup's first request leaves. Published while `page.on` listens to `popup`.",
    class: EventClass::Lossless,
    payload: schema_of::<PagePopup>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The Session's `page`.
pub const SESSION_PAGE: Event = Event {
    name: "session.page",
    description: "A page opened in the Session: one `session.new_page` or `run.new_page` opened, or a popup one of its pages opened. Published while `session.on` listens to `page`.",
    class: EventClass::Lossless,
    payload: schema_of::<PageOpened>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.on`.
pub const SESSION_ON: Operation = Operation {
    name: "session.on",
    description: "Listens to one of the Session's events, armed when the call arrives: the call stays in flight, and the engine publishes that event of the Session on its event stream, lossless, which `events.subscribe` reads under its credits. Cancelling the call removes the listener; it answers when the Session closes or its Run ends.",
    class: OperationClass::Read,
    kind: OperationKind::Event,
    reach: Reach::AllSurfaces,
    params: schema_of::<SessionOnParams>,
    result: schema_of::<SessionOnEnded>,
    events: &[&SESSION_PAGE],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.pages`.
pub const SESSION_PAGES: Operation = Operation {
    name: "session.pages",
    description: "Lists the Session's open pages, in the order they opened: those the program opened and the popups its pages opened. Once the Session is closed, it raises `closed`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<super::session::SessionParams>,
    result: schema_of::<SessionPages>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.close`.
pub const PAGE_CLOSE: Operation = Operation {
    name: "page.close",
    description: "Closes the page, a popup as any other: its calls in flight raise `call_cancelled` naming the program, it emits `close`, and a later call on it raises `closed`, while closing it again changes nothing. Its `beforeunload` handlers do not run unless `run_before_unload` is set: the page then closes once they let it, a dialog they raise going to its dialog handlers, and the call answers without waiting for that.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PageCloseParams>,
    result: schema_of::<PageClosed>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
