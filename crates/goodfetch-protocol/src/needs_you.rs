//! "Needs you" (ADRs 0031 and 0122; spec #592): one list of everything
//! waiting for a person, across every Run in the Run index and the stores
//! beside it. `needs_you.list` reads it a page at a time, the lossless event
//! `needs_you.changed` carries each change, an item appearing, changing or
//! going, and `needs_you.dismiss` hides an item whose source allows it.
//! Each item's `kind` is a value of a type both Products share (ADR 0095):
//! Collie's storage kinds are here, and each later area, and Foxhound,
//! adds its own beside its entries.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    Entries, Event, EventClass, Operation, OperationClass, OperationKind, ProtocolVersion, Reach,
    SharedType, SharedTypeValue, Stability, schema_of,
};
use crate::run::{PointInTime, RunId};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The kind of a filesystem at the `low` or `critical` storage level, with
/// its folder (ADR 0122).
pub const STORAGE_LEVEL: &str = "storage_level";

/// The kind of a Run stopped by storage, which a start with its Run ID
/// resumes once space is freed (ADR 0122).
pub const RUN_STOPPED_BY_STORAGE: &str = "run_stopped_by_storage";

/// The kind of a Run stopped by a Limit that still has nothing left, which
/// a person raises (ADRs 0031 and 0041): raising it resumes a Recipe's Run
/// in the Daemon, and leaves an SDK script's Run waiting for its script.
pub const RUN_STOPPED_BY_LIMIT: &str = "run_stopped_by_limit";

/// How many items one `needs_you.list` page holds at most.
pub const PAGE: usize = 100;

/// One thing waiting for a person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NeedsYouItem {
    /// Its ID, which names this one occurrence for good: a Run stopped
    /// again by storage after a resume is another item.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// What it is, a value of a type both Products share, such as
    /// `storage_level`; a Client shows a kind it does not know by its
    /// summary.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// Since when it waits.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub since: PointInTime,
    /// The Run it is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run: Option<RunId>,
    /// The Schedule it is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule: Option<String>,
    /// What waits, in the engine's English, which a Client shows for a kind
    /// it has no words of its own for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub summary: String,
    /// The catalog operations a person may answer it with, such as
    /// `needs_you.dismiss`; empty when only something outside the engine,
    /// such as freeing space, resolves it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub actions: Vec<String>,
    /// Its kind's own fields, which the kind's value in the catalog names:
    /// for `storage_level`, `level` and `folder`; for
    /// `run_stopped_by_storage`, `folder`; for `run_stopped_by_limit`,
    /// `limit`, `amount`, `needs` and, for a Recipe's Run, `recipe`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub details: Map<String, Value>,
}

/// What `needs_you.list` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeedsYouListParams {
    /// Only the items of these kinds; every kind when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: Option<Vec<String>>,
    /// The `next` of the page read before: the list goes on after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<String>,
}

/// What `needs_you.list` answers: one page of the items, the longest
/// waiting first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NeedsYouList {
    /// The items, by `since`, then by ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub items: Vec<NeedsYouItem>,
    /// Where the next page starts, as the next call's `after`; absent on
    /// the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub next: Option<String>,
}

/// What `needs_you.dismiss` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeedsYouDismissParams {
    /// The item's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub item: String,
}

/// What `needs_you.dismiss` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NeedsYouDismissed {
    /// Whether it hid the item: false when the list no longer holds it, or
    /// its source does not let a person dismiss it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dismissed: bool,
}

/// Why an item left the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NeedsYouRemoval {
    /// What waited is resolved: by a person, here or elsewhere, or by its
    /// Run.
    Resolved,
    /// A person dismissed it.
    Dismissed,
}

/// What `needs_you.changed` carries: one change of the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum NeedsYouChange {
    /// An item appeared, or changed: it as it is now.
    Listed {
        /// The item.
        #[schemars(extend("arrived_in" = "0.1"))]
        item: NeedsYouItem,
    },
    /// An item left the list.
    Removed {
        /// The item's ID.
        #[schemars(extend("arrived_in" = "0.1"))]
        id: String,
        /// Why.
        #[schemars(extend("arrived_in" = "0.1"))]
        reason: NeedsYouRemoval,
    },
}

/// `needs_you.changed`.
pub const NEEDS_YOU_CHANGED: Event = Event {
    name: "needs_you.changed",
    description: "An item of \"Needs you\" appeared, changed or left the list: resolved, by a person or by its Run, or dismissed. Lossless: a subscription that falls behind next takes `events.resync`, which holds every item.",
    class: EventClass::Lossless,
    payload: schema_of::<NeedsYouChange>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `needs_you.list`.
pub const NEEDS_YOU_LIST: Operation = Operation {
    name: "needs_you.list",
    description: "Lists what waits for a person across every Run in the Run index and the stores beside it, the longest waiting first, a page at a time: `after` takes the `next` of the page before. `kinds` keeps the items of those kinds. Each change after the answer arrives as `needs_you.changed` on the event stream.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<NeedsYouListParams>,
    result: schema_of::<NeedsYouList>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `needs_you.dismiss`.
pub const NEEDS_YOU_DISMISS: Operation = Operation {
    name: "needs_you.dismiss",
    description: "Hides an item of \"Needs you\" whose source lets a person dismiss it, for every Client, and answers whether it did; an item the list no longer holds, or one only resolving it removes, stays as it is. The item leaves the list with `needs_you.changed`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<NeedsYouDismissParams>,
    result: schema_of::<NeedsYouDismissed>,
    events: &[&NEEDS_YOU_CHANGED],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[NEEDS_YOU_LIST, NEEDS_YOU_DISMISS],
    events: &[NEEDS_YOU_CHANGED],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[
        SharedTypeValue {
            shared_type: SharedType::NeedsYouKind,
            name: STORAGE_LEVEL,
            description: "A filesystem a Run writes to is at the `low` or `critical` storage level (ADR 0122); `details` give its `level` and a `folder` on it. It leaves the list at the first sample that finds the space again.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::NeedsYouKind,
            name: RUN_STOPPED_BY_STORAGE,
            description: "A Run stopped by storage, resumable once space is freed by a start with its Run ID (ADR 0122); `details` give the `folder` whose filesystem filled. It leaves the list once the Run runs again, or when a person dismisses it.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::NeedsYouKind,
            name: RUN_STOPPED_BY_LIMIT,
            description: "A Run stopped by a Limit that still has nothing left in its period (ADR 0041); `details` give the `limit` as `limit_reached` names it, the `amount` it allows now, the Product the Run `needs` and, for a Recipe's Run, its `recipe`. Its actions offer `usage.raise`, a person's answer, and for a Recipe's Run `run.resume`, which resumes it in the Daemon once the Limit is raised; an SDK script's Run then waits for its script, which resumes it by its Run ID (ADR 0031). It leaves the list once the Limit has room again (a person raised it, or its period started again), once the Run runs again, or when a person dismisses it.",
            arrived_in: ARRIVED_IN,
        },
    ],
};
