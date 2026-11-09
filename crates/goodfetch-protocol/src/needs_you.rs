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
use crate::settings::SettingsLayer;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The kind of a filesystem at the `low` or `critical` storage level, with
/// its folder (ADR 0122).
pub const STORAGE_LEVEL: &str = "storage_level";

/// The kind of a Run stopped by storage, which a start with its Run ID
/// resumes once space is freed (ADR 0122).
pub const RUN_STOPPED_BY_STORAGE: &str = "run_stopped_by_storage";

/// The kind of a Run stopped by a Limit that still holds it, which a
/// person raises (ADRs 0031 and 0041): raising it resumes a Recipe's Run in
/// the Daemon, and leaves an SDK script's Run waiting for its script.
pub const RUN_STOPPED_BY_LIMIT: &str = "run_stopped_by_limit";

/// The kind of a domain whose consent wall a Run met and left, which a
/// person may add to `consent.domains` for the next runs (ADRs 0048 and
/// 0052).
pub const CONSENT_WALL: &str = "consent_wall";

/// The kind of a domain on which a Run's refetch without the site's stored
/// data stopped, which a person may remove from the layer of
/// `wall.refetch_domains` that lists it or deny (ADR 0052).
pub const WALL_REFETCH_STOPPED: &str = "wall_refetch_stopped";

/// The kind of a pinned host whose certificate's key changed, which the
/// person whose store holds the pin accepts or forgets (ADR 0042).
pub const CHANGED_CERTIFICATE: &str = "certificate_changed";

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
    /// `limit`, `amount`, `needs` and, for a Recipe's Run, `recipe`; for
    /// `consent_wall`, `domain`; for `wall_refetch_stopped`, `domain`,
    /// `cause`, the `layer` and `file` that list it and `only_star`; for
    /// `certificate_changed`, `host`, `person`, `pinned_spki_sha256` and
    /// `presented_spki_sha256`.
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

/// What `needs_you.allow_consent_domain` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeedsYouAllowConsentDomainParams {
    /// The registrable domain whose consent walls the next runs accept, as
    /// a `consent_wall` item names it, or `*` for every domain.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
}

/// What `needs_you.allow_consent_domain` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsentDomainAllowed {
    /// The domain, as `consent.domains` now lists it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// Whether it was added: false when user configuration listed it
    /// already, which then changed nothing and recorded nothing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: bool,
    /// The layer written: user configuration.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// What the person takes on, in plain words, as the Console showed it
    /// beside the answer and the Audit log records it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub warning: String,
}

/// What `needs_you.remove_refetch_domain` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeedsYouRemoveRefetchDomainParams {
    /// The registrable domain, as a `wall_refetch_stopped` item names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// When only `*` covers the domain in its layer, the registrable
    /// domains that replace `*` there, the domain itself not among them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub instead_of_star: Option<Vec<String>>,
}

/// What `needs_you.remove_refetch_domain` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefetchDomainRemoved {
    /// The domain.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// Whether its layer changed: false when neither user configuration
    /// nor the Operator policy lists it, which then changed nothing and
    /// recorded nothing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed: bool,
    /// The layer that listed it, and now does not; none when no layer was
    /// changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: Option<SettingsLayer>,
    /// That layer's `wall.refetch_domains` now.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<String>,
}

/// What `needs_you.deny_wall_domain` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeedsYouDenyWallDomainParams {
    /// The registrable domain to deny.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// Why, such as the site's objection; required.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
}

/// What `needs_you.deny_wall_domain` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WallDomainDenied {
    /// The domain, as `wall.denied_domains` now lists it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// Whether it was added: false when the Operator policy denied it
    /// already, which then changed nothing and recorded nothing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: bool,
    /// The reason, as the Audit log records it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
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

/// `needs_you.allow_consent_domain`.
pub const NEEDS_YOU_ALLOW_CONSENT_DOMAIN: Operation = Operation {
    name: "needs_you.allow_consent_domain",
    description: "Adds a domain whose consent wall a Run met, or `*` for every domain, to `consent.domains` in user configuration, so the next runs accept its consent walls instead of raising `blocked` with `consent_required`, and records the answer in the Audit log with the warning of what the person takes on; answers whether it added it. One call, no confirmation asked. A domain that file lists already adds nothing; an Operator policy that locks `consent.domains` refuses it with `setting_locked`. A person's answer: offered only in the Console and the interactive CLI prompt. The `consent_wall` items it covers leave the list with `needs_you.changed`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<NeedsYouAllowConsentDomainParams>,
    result: schema_of::<ConsentDomainAllowed>,
    events: &[&NEEDS_YOU_CHANGED],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `needs_you.remove_refetch_domain`.
pub const NEEDS_YOU_REMOVE_REFETCH_DOMAIN: Operation = Operation {
    name: "needs_you.remove_refetch_domain",
    description: "Removes a domain on which a Run's refetch stopped from the layer of `wall.refetch_domains` that lists it, user configuration or else the Operator policy, every other entry and byte of the file kept, and records the answer in the Audit log; answers whether it removed it. When only `*` covers the domain there, `instead_of_star` gives the registrable domains that replace `*`, and without it the call is refused with `invalid_params`. A domain only project configuration lists is changed in that file, never here. A managed Operator policy, or one that locks the key against user configuration, refuses it with `setting_locked`. A person's answer: offered only in the Console and the interactive CLI prompt. The `wall_refetch_stopped` item leaves the list with `needs_you.changed`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<NeedsYouRemoveRefetchDomainParams>,
    result: schema_of::<RefetchDomainRemoved>,
    events: &[&NEEDS_YOU_CHANGED],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `needs_you.deny_wall_domain`.
pub const NEEDS_YOU_DENY_WALL_DOMAIN: Operation = Operation {
    name: "needs_you.deny_wall_domain",
    description: "Adds a registrable domain, with its required reason, to `wall.denied_domains` in the Operator policy, creating the file when there is none, so no run reads past its Walls or refetches its pages whatever another layer allows (ADR 0052), and records the denial and its reason in the Audit log; answers whether it added it. One call, no confirmation asked. A managed Operator policy refuses it with `setting_locked`. A person's answer: offered only in the Console and the interactive CLI prompt. The `wall_refetch_stopped` item of the domain leaves the list with `needs_you.changed`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<NeedsYouDenyWallDomainParams>,
    result: schema_of::<WallDomainDenied>,
    events: &[&NEEDS_YOU_CHANGED],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A person removed a domain from `wall.refetch_domains`.
pub const REFETCH_DOMAIN_REMOVED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "refetch_domain_removed",
    description: "A person removed a domain on which a Run's refetch stopped from the layer of `wall.refetch_domains` that listed it, or replaced `*` there with a list (ADR 0052); `details` give the `domain` and the layer's `domains` after.",
    arrived_in: ARRIVED_IN,
};

/// A person denied a domain the Wall allowance.
pub const WALL_DOMAIN_DENIED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "wall_domain_denied",
    description: "A person added a domain to `wall.denied_domains` in the Operator policy, so no run reads past its Walls or refetches its pages (ADR 0052); `details` give the `domain` and the `reason`.",
    arrived_in: ARRIVED_IN,
};

/// A person added a domain, or `*`, to `consent.domains`.
pub const CONSENT_DOMAIN_ALLOWED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "consent_domain_allowed",
    description: "A person added a domain, or `*`, to `consent.domains` in user configuration, so later runs accept its consent walls (ADR 0048), with the warning of what they take on that they were shown (ADR 0052).",
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        NEEDS_YOU_LIST,
        NEEDS_YOU_DISMISS,
        NEEDS_YOU_ALLOW_CONSENT_DOMAIN,
        NEEDS_YOU_REMOVE_REFETCH_DOMAIN,
        NEEDS_YOU_DENY_WALL_DOMAIN,
    ],
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
            description: "A Run stopped by a Limit that still holds it (ADR 0041): the Limit's period has not started again since the stop, and it was not set again since then with something left. `details` give the `limit` as `limit_reached` names it, the `amount` it allows now, the Product the Run `needs` and, for a Recipe's Run, its `recipe`. Its actions offer `usage.raise`, a person's answer, and for a Recipe's Run `run.resume`, which resumes it in the Daemon once the Limit is raised; an SDK script's Run then waits for its script, which resumes it by its Run ID (ADR 0031). It leaves the list once a person raises the Limit or its period starts again, once the Run runs again, or when a person dismisses it.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::NeedsYouKind,
            name: CONSENT_WALL,
            description: "A domain whose consent wall a Run met and left at `consent_required` (ADR 0048), offered for `consent.domains` with `needs_you.allow_consent_domain`, a domain in `wall.denied_domains` too (ADR 0052); `details` give the `domain`, and `run` the last Run that met it. It leaves the list once `consent.domains` in the Operator policy or user configuration covers the domain, or when a person dismisses it.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::NeedsYouKind,
            name: WALL_REFETCH_STOPPED,
            description: "A domain on which a Run's refetch without the site's stored data stopped, at a Block cause, a 401 or 403 status or a login page (ADR 0052); `details` give the `domain`, the `cause`, and the `layer` of `wall.refetch_domains` that lists it with its `file`, and whether `only_star` covers it there; `run` is the last Run whose refetch stopped there. Its actions offer `needs_you.remove_refetch_domain`, except for a domain project configuration lists, which is changed in its file, and `needs_you.deny_wall_domain`. It leaves the list once no layer lists the domain, once `wall.denied_domains` denies it, or when a person dismisses it.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::NeedsYouKind,
            name: CHANGED_CERTIFICATE,
            description: "A pinned host whose certificate presented another public key, stopped with `certificate_changed` (ADR 0042); `details` give the `host`, the `person` whose known-certificates store holds the pin, and both keys' SHA-256 as `pinned_spki_sha256` and `presented_spki_sha256`; `run` is the Run that met it. Its actions offer `certs.accept` and `certs.forget`, which that person alone answers. It leaves the list once they answer.",
            arrived_in: ARRIVED_IN,
        },
        CONSENT_DOMAIN_ALLOWED,
        REFETCH_DOMAIN_REMOVED,
        WALL_DOMAIN_DENIED,
    ],
};
