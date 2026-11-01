//! The event stream every Product carries (ADRs 0024, 0031 and 0081):
//! `events.subscribe` opens a subscription, a call that stays in flight
//! until the Client cancels it or its Connection ends. The engine sends its
//! events in `$/events` batches, never more than the credits the Client
//! granted the call with `$/credit`. Every event carries a position that
//! only goes up within its engine instance, which `hello` names; a
//! subscription that names the position it last read, with its engine
//! instance, resumes after it, and one the engine no longer holds, or
//! another engine instance gave, starts with an `events.resync` event
//! giving the current state.
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
use crate::connection::EngineInstance;
use crate::governor::SYSTEM_INFO;
use crate::jsonrpc::RequestId;
use crate::needs_you::{NEEDS_YOU_CHANGED, NeedsYouItem};
use crate::run::{RUN_AXES_CHANGED, RunAxes};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The notification that grants a streaming call credits.
pub const CREDIT: &str = "$/credit";

/// The notification that carries a batch of a subscription's events.
pub const EVENTS: &str = "$/events";

/// The position of an event among every event of one engine, which only
/// goes up; for a `page.on` call's own events, among that call's, from 1.
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
            "description": "An event's position among every event of one engine, which only goes up; for a `page.on` call's own events, among that call's, from 1.",
        })
    }
}

/// The last event a Client read: its position, and the engine instance
/// that gave it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LastRead {
    /// The engine instance, as its `hello` named it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub engine_instance: EngineInstance,
    /// The event's position.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub position: Position,
}

/// What `$/credit` grants: more events, or chunks, for a streaming call;
/// sent by the engine, more seeds a `run.add_seeds` call takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreditParams {
    /// The streaming call's id.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// How many more events, or chunks, the Client takes; sent by the
    /// engine, how many more seeds the call takes.
    #[schemars(extend("arrived_in" = "0.1", "minimum" = 1))]
    pub credits: u32,
}

/// What `events.subscribe` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventsSubscribeParams {
    /// The last event the Client read, `{engine_instance, position}`: the
    /// subscription resumes after it, or starts with `events.resync` when
    /// the engine no longer holds it or another engine instance gave it, as
    /// after a Daemon restart. Absent, it starts with the next event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<LastRead>,
    /// Whether it also takes `engine.system_info`, the Resource governor's
    /// periodic readings; false by default, so a subscription that reads
    /// only what changes sees no periodic event.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub system_info: bool,
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
    /// The id of the streaming call whose events it carries: a
    /// subscription's `events.subscribe` call, or a `page.on` call, whose
    /// page's events go only to it.
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
    /// The axes of each live Run the engine knows. A Run it leaves out
    /// ended, and `run.list` gives its axes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runs: Vec<RunAxes>,
    /// Every item of "Needs you", by `since`, then by ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub needs_you: Vec<NeedsYouItem>,
}

/// The current state, sent first to a subscription whose position the
/// engine no longer holds (ADR 0031).
pub const RESYNC: Event = Event {
    name: "events.resync",
    description: "The engine no longer holds the position the subscription resumes after: the events since are gone, and this gives the current state, every live Run's axes and every item of \"Needs you\", from which the stream continues.",
    class: EventClass::Lossless,
    payload: schema_of::<Resync>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `events.subscribe`.
pub const EVENTS_SUBSCRIBE: Operation = Operation {
    name: "events.subscribe",
    description: "Subscribes to the engine's events. The call stays in flight, sending its events in `$/events` batches, never more than the credits granted with `$/credit`, until it is cancelled. No producer waits for a subscription: a lossless event is never dropped unseen, and a subscription whose queue is full falls behind, dropping its queued lossless events, and next sends `events.resync`, then the stream. A telemetry event is coalesced with the unsent one of the same kind and scope, and counts it as `dropped`. Given `after`, the last event read, it resumes after it, or starts with `events.resync` when the engine no longer holds it or another engine instance gave it, as after a Daemon restart. Given `system_info`, it also takes the Resource governor's periodic `engine.system_info`.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::AllSurfaces,
    params: schema_of::<EventsSubscribeParams>,
    result: schema_of::<SubscriptionEnded>,
    events: &[&RESYNC, &RUN_AXES_CHANGED, &NEEDS_YOU_CHANGED, &SYSTEM_INFO],
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
