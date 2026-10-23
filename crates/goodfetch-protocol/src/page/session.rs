//! A Run's Sessions (ADRs 0021, 0078 and 0092): `run.new_session` opens a
//! Session of its own, its own browser context with its own cookies and
//! storage, sharing the Run's browser process; `session.new_page` opens a
//! page in it; and `session.close` closes its pages and its context and
//! gives its pages' admissions back to the Resource governor, never closing
//! a Borrowed browser. A call on a closed Session raises `closed`, and a
//! second `session.close` changes nothing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{ARRIVED_IN, PageOpened, SessionId};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};
use crate::run::RunId;
use crate::settings::session_source::SessionSource;

/// What `run.new_session` takes: the Run whose Session it opens, the
/// Session's own source and its own settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewSessionParams {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// `session.source` for this Session alone: left out, the Run's or the
    /// connection's applies; `null`, the Session opens fresh (ADRs 0083
    /// and 0092).
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: Option<Option<SessionSource>>,
    /// The Session's own settings, nested by key: `filter_lists.ads`,
    /// `.trackers`, `.annoyances`, `.first_party_scripts` and
    /// `browser.block_resources` choose its pages' Clean-up rules over the
    /// Run's. `session.source` is refused here with `setting_not_applicable`,
    /// as the call's own `source` sets it, and so is any other key, which
    /// only the Run sets.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
}

/// A field that is present, `null` included, as `Some`; one left out stays
/// `None` through `#[serde(default)]`.
#[expect(
    clippy::option_option,
    reason = "a setting's three states (ADR 0083): left out inherits, `null` clears, a value sets"
)]
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
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
    description: "Opens a Session of the Run's own: its own browser context, with its own cookies, storage, identity and proxy, sharing the Run's browser process, so two Sessions of a Run never share a login. It opens from `source`, or else from the Run's `session.source`, or fresh; `session.source` in its own `settings` is refused with `setting_not_applicable`. The Run's first Session launches its browser, so a missing browser build raises its code from this call.",
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
