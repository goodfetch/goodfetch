//! Politeness shared on one machine (ADRs 0053, 0086 and 0088): the setting
//! that says how a Run shares it, which both Products honour since the
//! per-proxy limits it also governs are Collie's, those per-proxy limits,
//! which hold a program's own navigation through its proxy too, and the rows of the
//! per-user Politeness file, a format of the Usage ledger's family (ADR
//! 0066), which every engine of a user, of either Product, writes.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, Event, EventClass, Instrument, Metric, ProtocolVersion, Setting, Stability,
    ValueKind, schema_of,
};
use crate::run::RunId;

pub mod store;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `politeness.sharing`'s values, in the catalog's order.
pub const SHARING_VALUES: &[&str] = &["shared", "per_daemon", "run"];

fn null() -> Value {
    Value::Null
}

fn shared() -> Value {
    Value::String("shared".to_owned())
}

fn not_shared(value: &Value) -> bool {
    value.as_str().is_some_and(|sharing| sharing != "shared")
}

/// `politeness.sharing` (ADRs 0053, 0086 and 0088).
pub const POLITENESS_SHARING: Setting = Setting {
    name: "politeness.sharing",
    description: "How a Run shares Politeness with the other runs of its user: `shared` with every engine of the user (through the Politeness store when one is configured, else on the machine), `per_daemon` on the machine only, ignoring a store, or `run` for Politeness of its own. Anything but `shared` is listed in the Run's result.",
    kind: ValueKind::Choice(SHARING_VALUES),
    default: shared,
    clearable: false,
    authority: Authority::Every,
    weakens: Some(not_shared),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn yes() -> Value {
    Value::Bool(true)
}

/// `politeness.delegation` (ADR 0142).
pub const POLITENESS_DELEGATION: Setting = Setting {
    name: "politeness.delegation",
    description: "Whether a Run sharing Politeness takes each key's Delegation: while no other engine of the user uses a key, its engine keeps the key's Permits and state in memory, each Permit with no transaction on the Politeness file, and writes to the file at once what a site asked (a 429, a `Retry-After`, a backoff, a halved ceiling, a new Crawl-delay). Another engine that needs the key recalls it, and one whose holder is gone reclaims it, its delay counted from the reclaim. `false` takes and gives back each Permit in transactions of the file, unless the Run's engine already holds the key's Delegation for another Run: the Permit then comes from memory too, so the key's limits hold across both Runs. Ignored at `politeness.sharing: run`.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `politeness.proxy.concurrency` (ADRs 0053, 0086 and 0088).
pub const POLITENESS_PROXY_CONCURRENCY: Setting = Setting {
    name: "politeness.proxy.concurrency",
    description: "How many page navigations may be in flight at once through one proxy endpoint and credential, counted across every run of the user that shares Politeness, each redirect hop of a page's main frame counted, a program's own navigation included (a page's other requests and `page.request` are not counted): a run sends one only while that combined count is below its own value and below the ceiling the proxy's own refusals taught. At least 1; none by default, for no cap.",
    kind: ValueKind::Count,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `politeness.proxy.rate` (ADRs 0053, 0086 and 0088).
pub const POLITENESS_PROXY_RATE: Setting = Setting {
    name: "politeness.proxy.rate",
    description: "How many page navigations a second may start through one proxy endpoint and credential, counted across every run of the user that shares Politeness, each redirect hop of a page's main frame counted, a program's own navigation included (a page's other requests and `page.request` are not counted): a run starts one only once a second divided by its own value has passed since the last start there by any sharing run. None by default.",
    kind: ValueKind::Count,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// One Permit in the per-user Politeness file (ADR 0053): a request in
/// flight under one key, such as a domain key, with the Run and run epoch
/// that hold it, so an engine reclaims the Permits of a Run whose writer
/// lock is free or held at a later epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PermitRow {
    /// Its number in the file.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub permit_id: i64,
    /// The key it counts under, such as a domain key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The Run that holds it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: String,
    /// The run epoch it was taken under (ADR 0003).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub epoch: u64,
    /// The Run's folder, whose writer lock tells whether the Run is live.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_folder: String,
    /// The engine that took it, which gives it back itself.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub engine: String,
    /// When it was taken, in milliseconds since the Unix epoch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub taken_at: i64,
}

fn ten_minutes() -> Value {
    json!(600_000)
}

/// `politeness.degraded_max` (ADRs 0053 and 0086).
pub const POLITENESS_DEGRADED_MAX: Setting = Setting {
    name: "politeness.degraded_max",
    description: "How long a cut-off engine keeps admitting requests to shared keys within its last granted share when the Politeness store is unreachable, counted on each side's clock from the moment the engine sent its last acknowledged heartbeat: 10 minutes by default. Past it, the cut-off engine admits nothing to shared keys until it reconnects, while its Runs stay live and resumable, and the store frees its reservation once `politeness.degraded_max` plus its longest reported request timeout has passed. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: ten_minutes,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Payload of `politeness.store_degraded` (ADRs 0024 and 0053): emitted on
/// each affected Run's event stream when the Politeness store becomes
/// unreachable, when degraded admission stops at `politeness.degraded_max`,
/// and when the store reconnects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolitenessStoreDegraded {
    /// The affected Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Whether the Politeness store is currently unreachable (degraded).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub degraded: bool,
    /// Whether `politeness.degraded_max` has elapsed and admission to shared
    /// keys is stopped until the store reconnects.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub admission_stopped: bool,
    /// The configured `politeness.degraded_max` window in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub degraded_max_ms: u64,
    /// Why the Politeness store became unreachable, when `degraded` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: Option<String>,
}

/// Lossless event emitted on each affected Run's stream when the Politeness
/// store enters degraded mode, stops degraded admission at
/// `politeness.degraded_max`, or reconnects (ADRs 0024 and 0053).
pub const POLITENESS_STORE_DEGRADED_EVENT: Event = Event {
    name: "politeness.store_degraded",
    description: "The Politeness store became unreachable (entering bounded degraded admission within granted shares), reached `politeness.degraded_max` and stopped admission to shared keys, or reconnected (ADR 0053).",
    class: EventClass::Lossless,
    payload: schema_of::<PolitenessStoreDegraded>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.politeness.store_degraded`: whether this engine's Politeness
/// store is currently unreachable and running in degraded mode (ADRs 0050 and
/// 0053).
pub const POLITENESS_STORE_DEGRADED_METRIC: Metric = Metric {
    name: "goodfetch.politeness.store_degraded",
    description: "1 while this engine's Politeness store is unreachable and running in degraded mode, else 0 (ADR 0053).",
    instrument: Instrument::Gauge,
    unit: "{engine}",
    labels: &[],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[POLITENESS_STORE_DEGRADED_EVENT],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[POLITENESS_STORE_DEGRADED_METRIC],
    settings: &[
        POLITENESS_SHARING,
        POLITENESS_DELEGATION,
        POLITENESS_DEGRADED_MAX,
        POLITENESS_PROXY_CONCURRENCY,
        POLITENESS_PROXY_RATE,
        store::POLITENESS_STORE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
