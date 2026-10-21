//! The event stream every Product carries (ADRs 0024, 0031 and 0081):
//! `events.subscribe` opens a subscription, a call that stays in flight
//! until the Client cancels it or its Connection ends. The engine sends its
//! events in `$/events` batches, never more than the credits the Client
//! granted the call with `$/credit`. Every event carries a position, per
//! engine, that only goes up; a subscription that names the position it
//! last read resumes after it, and one the engine no longer holds starts
//! with an `events.resync` event giving the current state.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, Event, EventClass, Operation, OperationClass, OperationKind, ProtocolVersion, Reach,
    Stability, schema_of,
};
use crate::jsonrpc::RequestId;
use crate::run::{RUN_AXES_CHANGED, RunAxes};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The notification that grants a streaming call credits.
pub const CREDIT: &str = "$/credit";

/// The notification that carries a batch of a subscription's events.
pub const EVENTS: &str = "$/events";

/// The position of an event among every event of one engine, which only
/// goes up.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Position(pub u64);

impl JsonSchema for Position {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Position".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "integer",
            "minimum": 0,
            "description": "An event's position among every event of one engine, which only goes up.",
        })
    }
}

/// What `$/credit` grants: more events, or chunks, for a streaming call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreditParams {
    /// The streaming call's id.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// How many more events, or chunks, the Client takes.
    #[schemars(extend("arrived_in" = "0.1", "minimum" = 1))]
    pub credits: u32,
}

/// What `events.subscribe` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventsSubscribeParams {
    /// The position of the last event the Client read: the subscription
    /// resumes after it, or starts with `events.resync` when the engine no
    /// longer holds it. Absent, it starts with the next event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<Position>,
}

/// What `events.subscribe` answers when its subscription ends without the
/// Client cancelling it: the engine stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubscriptionEnded {}

/// One event of a batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    /// Its position.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub position: Position,
    /// The catalog's name of the event.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: String,
    /// Its payload, of the type the catalog gives the event.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: Value,
    /// For a telemetry event, how many of the same kind and scope it
    /// replaced unsent.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dropped: u64,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// What `$/events` carries: a batch of one subscription's events, in the
/// order of their positions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventBatch {
    /// The id of the subscription's `events.subscribe` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// Its events.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub events: Vec<EventEnvelope>,
}

/// What `events.resync` gives: the current state, from which the stream
/// continues.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Resync {
    /// The axes of each Run the engine knows, live or ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runs: Vec<RunAxes>,
}

/// The current state, sent first to a subscription whose position the
/// engine no longer holds (ADR 0031).
pub const RESYNC: Event = Event {
    name: "events.resync",
    description: "The engine no longer holds the position the subscription resumes after: the events since are gone, and this gives the current state, from which the stream continues.",
    class: EventClass::Lossless,
    payload: schema_of::<Resync>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `events.subscribe`.
pub const EVENTS_SUBSCRIBE: Operation = Operation {
    name: "events.subscribe",
    description: "Subscribes to the engine's events. The call stays in flight, sending its events in `$/events` batches, never more than the credits granted with `$/credit`, until it is cancelled. A lossless event is never dropped: while the subscription's queue is full, its producer waits. A telemetry event is coalesced with the unsent one of the same kind and scope, and counts it as `dropped`. Given `after`, it resumes after that position, or starts with `events.resync` when the engine no longer holds it.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::AllSurfaces,
    params: schema_of::<EventsSubscribeParams>,
    result: schema_of::<SubscriptionEnded>,
    events: &[&RESYNC, &RUN_AXES_CHANGED],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[EVENTS_SUBSCRIBE],
    events: &[RESYNC],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
