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

/// The kind of a domain whose consent wall a Run met and left, which a
/// person may add to `consent.domains` for the next runs (ADRs 0048 and
/// 0052).
pub const CONSENT_WALL: &str = "consent_wall";

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
    /// `run_stopped_by_storage`, `folder`; for `consent_wall`, `domain`.
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

/// What a person takes on by adding `domain`, or `*`, to `consent.domains`
/// (ADRs 0048 and 0052), in the words the Console shows beside the answer
/// and the Audit log records with it.
pub fn consent_domain_warning(domain: &str) -> String {
    if domain == "*" {
        "Accepting consent walls on every domain gives consent in your name on every site that shows one: those sites may load their trackers and set their cookies, and under the GDPR that consent is yours. An Agent also no longer asks you before accepting a consent prompt on any domain. Every later run accepts them until you remove `*` from `consent.domains`.".to_owned()
    } else {
        format!(
            "Accepting the consent walls of {domain} gives consent in your name: the site may load its trackers and set their cookies, and under the GDPR that consent is yours. Every later run accepts them until you remove {domain} from `consent.domains`."
        )
    }
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
            name: CONSENT_WALL,
            description: "A domain whose consent wall a Run met and left at `consent_required` (ADR 0048), offered for `consent.domains` with `needs_you.allow_consent_domain`, a domain in `wall.denied_domains` too (ADR 0052); `details` give the `domain`, and `run` the last Run that met it. It leaves the list once `consent.domains` in the Operator policy or user configuration covers the domain, or when a person dismisses it.",
            arrived_in: ARRIVED_IN,
        },
        CONSENT_DOMAIN_ALLOWED,
    ],
};
