//! Limits across runs (ADR 0041): the Limits the per-user Usage ledger
//! holds on the install, a Daemon, a person, a named token and a Client
//! connection, each over a period in an explicit IANA time zone or none; the
//! Usage charged against each, read with `usage.list` and added to with
//! `usage.set`; the Limits a program gives a Run at its start, on the Run's
//! own scope (ADRs 0080 and 0102); the Error codes a start meets,
//! `limit_reached`, `limit_not_understood` and `limit_raise_refused`; and
//! the Usage kinds and Limit scopes Collie adds
//! to the types both Products share. Foxhound adds its own, such as
//! `paid_fetch`, in its `foxhound` module (ADRs 0087 and 0088). Raising a
//! Limit, `usage.raise`, is a person's answer: the catalog marks it
//! [`Reach::PersonOnly`], so it is offered only in the Kennel and the
//! interactive CLI prompt, and refused elsewhere with `person_required`.

use crate::point_in_time::PointInTime;
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, SharedType, SharedTypeValue, Stability, schema_of,
};
use crate::run::{LimitScope, LimitSetter, Period};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The scope kind of the install: every run of the user on the machine,
/// whichever engine runs it.
pub const INSTALL: &str = "install";
/// The scope kind of a Daemon: every run it runs; its ID is its major's name.
pub const DAEMON: &str = "daemon";
/// The scope kind of a person: every run they start.
pub const PERSON: &str = "person";
/// The scope kind of a named token: every run a Connection holding it starts.
pub const TOKEN: &str = "token";
/// The scope kind of a Client connection: every run it starts while it lasts.
pub const CONNECTION: &str = "connection";
/// The scope kind of a Run: the Run its program gave the Limit at its
/// start; its ID is the Run's, and its Limits have no period.
pub const RUN: &str = "run";

/// The Usage kind of the requests a run sends: a crawl's Crawl requests
/// and, with the browser, a page's navigations.
pub const REQUESTS: &str = "requests";

/// The Usage kind of the bytes a run sent and received through a proxy,
/// counted per proxy kind (area 8 charges it).
pub const PROXY_BYTES: &str = "proxy_bytes";

/// The Usage kind of paid solves, counted by currency at the solving
/// service's configured price (ADR 0035; area 8 reserves it).
pub const PAID_SOLVES: &str = "paid_solves";

/// The Usage kind of the model calls a run made, retries and Repairs
/// included (ADR 0101; area 9 charges it).
pub const MODEL_CALLS: &str = "model_calls";

/// The Usage kind of the model tokens a run used, input and output (ADR
/// 0101; area 9 charges it).
pub const MODEL_TOKENS: &str = "model_tokens";

/// The Usage kind of the time a run held a browser, in milliseconds (ADR
/// 0101; area 5 charges it).
pub const BROWSER_TIME: &str = "browser_time";

/// The Usage kind of a run's crashes: one per renderer that crashed and
/// one per browser process that crashed, however many pages it held (ADR
/// 0040; area 5 charges it).
pub const PAGE_CRASHES: &str = "page_crashes";

/// One Limit of the Usage ledger: a generic row, which a newer engine may
/// write with a scope kind, a Usage kind, a currency or a period this one
/// does not know (ADR 0041).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Limit {
    /// What it is on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: LimitScope,
    /// The Usage kind it caps, a value of a type both Products share.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps the Usage kind's price in, an ISO 4217 code;
    /// none for a Limit on the Usage itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// The period it counts over; none for a Limit that never starts again,
    /// such as a Client connection's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
    /// How much it allows per period: a count of the Usage kind, or
    /// millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
    /// Whether an engine that cannot evaluate it must refuse the runs it
    /// cannot place outside it, with `limit_not_understood`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub must_understand: bool,
    /// Who set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub set_by: LimitSetter,
}

/// What the engine that read a Limit makes of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitStanding {
    /// It holds its runs to it.
    Held,
    /// It is on a value only Foxhound carries, such as `paid_fetch`, which
    /// a Collie engine keeps untouched (ADR 0098).
    Foxhound,
    /// It cannot evaluate it: a newer engine wrote it.
    NotUnderstood,
}

/// One Limit with the Usage counted against it in its current period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitUsage {
    /// The Limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Limit,
    /// What the engine that read it makes of it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub standing: LimitStanding,
    /// The Usage charged in the current period: what ended runs used, and
    /// what was reclaimed from dead engines' Slices.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub charged: u64,
    /// The Usage live runs hold in Slices they have not returned yet.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub held: u64,
    /// What is left of the amount in the current period.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub left: u64,
    /// When the current period started; none for a Limit without one.
    #[serde(
        with = "crate::point_in_time::nullable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub period_started_at: Option<SystemTime>,
    /// When the next period starts; none for a Limit without one.
    #[serde(
        with = "crate::point_in_time::nullable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub resets_at: Option<SystemTime>,
    /// The major of the managed Operator policy whose row it is; none for
    /// a row no managed policy wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub managed_major: Option<String>,
}

/// What `usage.list` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageListParams {}

/// What `usage.list` answers, and what `usage --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageListResult {
    /// Every Limit of the Usage ledger, by scope, with its Usage.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Vec<LimitUsage>,
}

/// What `usage.set` takes: a Limit of the caller's own, which holds beside
/// the others, so the tightest one holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageSetParams {
    /// The scope's kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: String,
    /// Which one of its kind; the caller's own when absent (its Client
    /// connection, its person, the engine's Daemon, the install).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope_id: Option<String>,
    /// The Usage kind it caps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps the price in, an ISO 4217 code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// The period it counts over; none for a Limit that never starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
    /// How much it allows per period: a count of the Usage kind, or
    /// millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
}

/// An install Limit a managed Operator policy names (ADRs 0031 and 0041):
/// each engine of the policy's major writes it to the Usage ledger when it
/// starts, as a row of its own keyed by that major and set by
/// [`LimitSetter::ManagedPolicy`], which only the policy and
/// `upgrade-files` change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedLimit {
    /// The Usage kind it caps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps the price in, an ISO 4217 code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// The period it counts over; none for a Limit that never starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
    /// How much it allows per period: a count of the Usage kind, or
    /// millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
}

impl ManagedLimit {
    /// Its row: on the install, flagged *must understand*, set by a
    /// managed policy.
    pub fn limit(&self) -> Limit {
        Limit {
            scope: LimitScope {
                kind: INSTALL.to_owned(),
                id: String::new(),
            },
            usage_kind: self.usage_kind.clone(),
            currency: self.currency.clone(),
            period: self.period.clone(),
            amount: self.amount,
            must_understand: true,
            set_by: LimitSetter::ManagedPolicy,
        }
    }
}

/// What an Operator policy's `[managed]` table holds, which marks the
/// policy *managed*: deployed by MDM or Ansible, read-only for every person surface
/// (ADR 0031).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Managed {
    /// The install Limits it names (ADR 0041).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub install_limits: Vec<ManagedLimit>,
}

/// What `usage.raise` takes: a Limit a person sets, which replaces every
/// Limit of its scope, Usage kind, currency and period that is not a
/// managed Operator policy's, keeping the Usage counted against them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageRaiseParams {
    /// The scope's kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: String,
    /// Which one of its kind; the person's own when absent (their Client
    /// connection, their person, the engine's Daemon, the install).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope_id: Option<String>,
    /// The Usage kind it caps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps the price in, an ISO 4217 code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// The period it counts over; none for a Limit that never starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
    /// How much it allows per period from now on: a count of the Usage
    /// kind, or millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
}

/// What `limit_raise_refused` details: the Limit a start gave above the
/// Run's Limit as it stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitRaiseRefusedDetails {
    /// The Usage kind of the Limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// Its currency, an ISO 4217 code; none for a count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// The amount the start gave.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub given: u64,
    /// The amount of the Limit as it stands.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub standing: u64,
}

/// What `limit_not_understood` details: the Limit this engine cannot
/// evaluate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitNotUnderstoodDetails {
    /// The Limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Limit,
}

/// `usage.list`.
pub const USAGE_LIST: Operation = Operation {
    name: "usage.list",
    description: "Lists every Limit of the Usage ledger, by scope, with the Usage charged and held against it in its current period and what is left.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<UsageListParams>,
    result: schema_of::<UsageListResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `usage.set`.
pub const USAGE_SET: Operation = Operation {
    name: "usage.set",
    description: "Adds a Limit of the caller's own to the Usage ledger, which holds beside the others: the tightest one holds, so it can only lower what applies. Answers it with its Usage.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<UsageSetParams>,
    result: schema_of::<LimitUsage>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `usage.raise`.
pub const USAGE_RAISE: Operation = Operation {
    name: "usage.raise",
    description: "Sets a Limit to a person's amount, in place of every Limit of its scope, Usage kind, currency and period but a managed Operator policy's, keeping the Usage counted. A person's answer: offered only in the Kennel and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<UsageRaiseParams>,
    result: schema_of::<LimitUsage>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `limit_reached`.
pub const LIMIT_REACHED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "limit_reached",
    description: "A Limit that covers the run has nothing left in its period; the details name its scope, its Usage kind, its period and when it starts again. Wait for the next period, or ask a person to raise it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: Some(16),
    arrived_in: ARRIVED_IN,
};

/// `limit_not_understood`.
pub const LIMIT_NOT_UNDERSTOOD: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "limit_not_understood",
    description: "A newer engine wrote a Limit this engine cannot evaluate, and it may cover the run; the details name it. Upgrade the engine.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `limit_raise_refused`.
pub const LIMIT_RAISE_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "limit_raise_refused",
    description: "A start that names an existing Run gave a Limit above the Run's Limit as it stands; the details name its Usage kind, its currency, the amount given and the one standing. A program only lowers a Limit: ask a person to raise it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[USAGE_LIST, USAGE_SET, USAGE_RAISE],
    events: &[],
    handler_requests: &[],
    error_codes: &[LIMIT_REACHED, LIMIT_NOT_UNDERSTOOD, LIMIT_RAISE_REFUSED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: REQUESTS,
            description: "Requests a run sent: a crawl's Crawl requests, and a page's navigations.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: PROXY_BYTES,
            description: "Bytes a run sent and received through a proxy, per proxy kind.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: PAID_SOLVES,
            description: "Paid solves and their price, by currency (ADR 0035); its Limits carry must understand.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: MODEL_CALLS,
            description: "Model calls a run made, retries and Repairs included (ADR 0101).",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: MODEL_TOKENS,
            description: "Model tokens a run used, input and output (ADR 0101).",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: BROWSER_TIME,
            description: "The time a run held a browser, in milliseconds (ADR 0101).",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::UsageKind,
            name: PAGE_CRASHES,
            description: "Crashes of a run: one per renderer that crashed, and one per browser process that crashed, however many pages it held (ADR 0040). A crash racing the page's close does not count.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: INSTALL,
            description: "Every run of the user on the machine, whichever engine runs it.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: DAEMON,
            description: "Every run a Daemon runs.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: PERSON,
            description: "Every run a person starts.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: TOKEN,
            description: "Every run a Connection holding a named token starts.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: CONNECTION,
            description: "Every run a Client connection starts, while it lasts.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::LimitScope,
            name: RUN,
            description: "The Run whose program gave the Limit at its start; it has no period.",
            arrived_in: ARRIVED_IN,
        },
    ],
};
