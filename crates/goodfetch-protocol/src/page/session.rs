//! A Run's Sessions (ADRs 0021, 0078 and 0092): `run.new_session` opens a
//! Session of its own, its own browser context with its own cookies and
//! storage, sharing the Run's browser process; `session.new_page` opens a
//! page in it; and `session.close` closes its pages and its context and
//! gives its pages' admissions back to the Resource governor, never closing
//! a Borrowed browser. A call on a closed Session raises `closed`, and a
//! second `session.close` changes nothing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageOpened, SessionId};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};
use crate::run::RunId;

/// What `run.new_session` takes: the Run whose Session it opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewSessionParams {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
}

/// What `run.new_session` answers: the Session it opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionOpened {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
}

/// What `session.new_page` and `session.close` take: the Session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionParams {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
}

/// What `session.close` answers once the Session is closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionClosed {}

/// `run.new_session`.
pub const RUN_NEW_SESSION: Operation = Operation {
    name: "run.new_session",
    description: "Opens a Session of the Run's own: its own browser context, with its own cookies, storage, identity and proxy, sharing the Run's browser process, so two Sessions of a Run never share a login. The Run's first Session launches its browser, so a missing browser build raises its code from this call.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<NewSessionParams>,
    result: schema_of::<SessionOpened>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.new_page`.
pub const SESSION_NEW_PAGE: Operation = Operation {
    name: "session.new_page",
    description: "Opens a page in the Session, with its cookies and storage. Once the Session is closed, it raises `closed`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SessionParams>,
    result: schema_of::<PageOpened>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.close`.
pub const SESSION_CLOSE: Operation = Operation {
    name: "session.close",
    description: "Closes the Session: its pages and its browser context close, and its pages' admissions go back to the Resource governor; a Borrowed browser is never closed. A later call on the Session or one of its pages raises `closed`, and closing it again changes nothing.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SessionParams>,
    result: schema_of::<SessionClosed>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
