//! Settings and their layers (ADRs 0028, 0083, 0087 and 0098), which every
//! Product carries: the five layers a value resolves through, the refusals
//! of a value that cannot take effect where it is set, `config.explain`
//! with the explanation it answers and `config explain --json` prints, the
//! files of the two layers kept in Goodfetch home (Operator policy and user
//! configuration, per major, shared by both Products), the Recipe
//! manifest's Product and settings, the closed list of deployment
//! variables, and the keys of areas whose own modules do not exist yet,
//! each with its area first (ADR 0083). Each later area moves its keys into
//! its own module.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    Product, ProtocolVersion, Setting, Stability, ValueKind, schema_of,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The layers settings resolve through, lowest first (ADRs 0028 and 0083).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SettingsLayer {
    /// The built-in default.
    Default,
    /// An Operator policy.
    OperatorPolicy,
    /// User configuration.
    UserConfiguration,
    /// Project configuration.
    ProjectConfiguration,
    /// A Recipe.
    Recipe,
    /// The run: a program's settings, a call's argument, a CLI flag or an
    /// Integration's form.
    Run,
}

impl SettingsLayer {
    /// Its name as the catalog and messages write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::OperatorPolicy => "operator_policy",
            Self::UserConfiguration => "user_configuration",
            Self::ProjectConfiguration => "project_configuration",
            Self::Recipe => "recipe",
            Self::Run => "run",
        }
    }

    /// Whether it is one of the configuration layers: Operator policy, user
    /// or project configuration (ADR 0028).
    pub fn is_configuration(self) -> bool {
        matches!(
            self,
            Self::OperatorPolicy | Self::UserConfiguration | Self::ProjectConfiguration
        )
    }
}

/// Where a run-layer value came from (ADR 0083).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunOrigin {
    /// A call's argument.
    Argument,
    /// A scope's `settings` option.
    Settings,
    /// A CLI flag.
    Flag,
    /// An Integration's form.
    Form,
}

/// A setting that decided an outcome: its key, its value and the layer that
/// set it (ADRs 0028 and 0079).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingLimit {
    /// The setting's key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
    /// The layer that set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
}

/// An unknown key, or an unknown `GOODFETCH_` variable (ADR 0083).
pub const SETTING_UNKNOWN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "setting_unknown",
    description: "A key no Product knows, or an unknown `GOODFETCH_` variable. The message names the closest valid one.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `setting_unknown` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingUnknownDetails {
    /// The key or variable as written.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The layer that held it; `None` for a variable of the environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: Option<SettingsLayer>,
    /// The closest known key or variable, when one is close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub closest: Option<String>,
}

/// A value of the wrong kind (ADR 0083).
pub const SETTING_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "setting_invalid",
    description: "A value of the wrong type or out of the setting's own range, an empty or unparsable deployment variable, or a settings file that does not parse.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `setting_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingInvalidDetails {
    /// The key or variable, or the file's path when the file does not
    /// parse.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The layer that held it; `None` for a variable of the environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: Option<SettingsLayer>,
    /// What is wrong with it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
}

/// A value set where it cannot take effect (ADR 0083).
pub const SETTING_NOT_APPLICABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "setting_not_applicable",
    description: "A setting given where it cannot take effect: in a layer that may not hold it, such as `browser_cache.max_unused_age` outside user configuration and an Operator policy, or on a Run it does not apply to, such as a crawl's setting on a Collie Run.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `setting_not_applicable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingNotApplicableDetails {
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The layer that held it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// Where it can be set instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A value a lower layer locked or capped, outside a Recipe's exposed
/// bounds, or reserved to a lower layer (ADRs 0028 and 0083).
pub const SETTING_LOCKED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "setting_locked",
    description: "A value a lower layer locked or capped, outside a Recipe's exposed bounds, or one only a lower layer may set or widen, such as `tls.trusted_cas` from a run. `limit` names the setting and the layer that holds it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `setting_locked` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingLockedDetails {
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The layer whose value was refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// What holds the value: the lock's or the cap's value and layer, or
    /// the lower layers' value a narrowing-only key was held to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A layer's file, as `config explain` names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingsFile {
    /// Its layer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// Its path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Whether it exists; a missing file sets nothing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found: bool,
}

/// One setting as `config explain` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExplainedSetting {
    /// Its key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The Products that honour it, from its Product mark (ADR 0095).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub products: Vec<Product>,
    /// Whether this engine applies it: `false` for a key only the other
    /// Product honours, which a shared file may hold (ADR 0087).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub applied: bool,
    /// The effective value, or for a key this engine does not apply, the
    /// value its highest layer wrote.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
    /// The layer that set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// For the run layer, where the value came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: Option<RunOrigin>,
    /// The layer that locked it, if any: an Operator policy's lock, a
    /// Recipe that declares it without exposing it, or the default for an
    /// honesty guarantee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locked_by: Option<SettingsLayer>,
    /// An Operator policy's cap on it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cap: Option<Value>,
    /// Whether its value weakens a guarantee class or switches off a costly
    /// control, which the results it touches are marked with (ADR 0028).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub weakens: bool,
}

/// What `config.explain` answers and `config explain --json` prints:
/// every setting's effective value, the layer that set it and the layer
/// that locked it (ADR 0028), and the files read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingsExplanation {
    /// The Operator policy's version, `sha256:` and the first 16 hex digits
    /// of its file's digest, which the run report names; `None` without a
    /// policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy_version: Option<String>,
    /// The files of the configuration layers, lowest first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub files: Vec<SettingsFile>,
    /// Each setting, by key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Vec<ExplainedSetting>,
}

/// A project configuration's values, which a Client reads where it runs and
/// sends with a scope it opens (ADR 0083).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettings {
    /// The file's path, as the Client found it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Its settings, nested as the file nests them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
}

/// What `config.explain` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigExplainParams {
    /// The keys to explain, each a key or a prefix of keys, such as
    /// `timeout`; every key when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub keys: Vec<String>,
    /// The project configuration to explain with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
    /// Run-layer values to explain with, as a Run's `settings` would give
    /// them.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// Where those run values come from: a scope's `settings` option by
    /// default, or the CLI's flags when the CLI asks a Daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: Option<RunOrigin>,
}

/// `config.explain`.
pub const CONFIG_EXPLAIN: Operation = Operation {
    name: "config.explain",
    description: "Shows each setting's effective value, the layer that set it and the layer that locked it, from the built-in defaults, the Operator policy, user configuration and the project configuration and run values given, and lists the keys only the other Product honours as that Product's.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    params: schema_of::<ConfigExplainParams>,
    result: schema_of::<SettingsExplanation>,
    events: &[],
    cancellable: false,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Recipe's exposed setting (ADR 0028): its users may set it, within its
/// bounds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exposure {
    /// The smallest value its users may give, for a count or a duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub minimum: Option<u64>,
    /// The largest value its users may give, for a count or a duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub maximum: Option<u64>,
    /// The values its users may give, when only these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub one_of: Vec<Value>,
}

/// What a Recipe's manifest says about its Product and its settings
/// (ADRs 0028 and 0096).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeManifest {
    /// The Recipe's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The Product the Recipe needs, which `init` writes (ADR 0096).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// The settings it declares, each locked for its users unless exposed.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The settings its users may set, by key, each with its bounds.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expose: BTreeMap<String, Exposure>,
}

/// The deployment variables (ADRs 0059, 0083 and 0090): the closed list of
/// `GOODFETCH_` variables, each a value that differs by machine, read from
/// the environment map and never a setting. Both engine-path variables are
/// on it, so neither is unknown to either Product; each SDK and crate reads
/// only its own Product's.
pub const DEPLOYMENT_VARIABLES: &[&str] = &[
    "GOODFETCH_BROWSERS_PATH",
    "GOODFETCH_COLLIE_ENGINE_PATH",
    "GOODFETCH_FOXHOUND_ENGINE_PATH",
    "GOODFETCH_HOME",
    "GOODFETCH_LOG",
    "GOODFETCH_LOG_OUTPUT",
    "GOODFETCH_TEST_SEED",
    "GOODFETCH_TOKEN",
    "GOODFETCH_URL",
];

/// The prefix every deployment variable has, which ADR 0034 reserves.
pub const DEPLOYMENT_PREFIX: &str = "GOODFETCH_";

/// A user configuration or project configuration file: its format version
/// and its settings, nested by key (ADRs 0028 and 0083).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationFile {
    /// The version of this format the file was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProtocolVersion,
    /// Its settings, nested by key: `[settings.crawl.handler]` holds
    /// `concurrency`.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
}

/// An Operator policy file: what it sets, what it locks and what it caps,
/// each nested by key (ADR 0028).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperatorPolicyFile {
    /// The version of this format the file was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProtocolVersion,
    /// The values it sets, which higher layers may change.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The values it sets and locks: no higher layer changes them.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lock: Map<String, Value>,
    /// The largest value a higher layer may give a count or a duration;
    /// a run already going takes a tightened cap at its next admission.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cap: Map<String, Value>,
}

fn null() -> Value {
    Value::Null
}

fn empty_list() -> Value {
    json!([])
}

fn empty_table() -> Value {
    json!({})
}

fn yes() -> Value {
    Value::Bool(true)
}

fn no() -> Value {
    Value::Bool(false)
}

fn thirty_seconds() -> Value {
    json!(30_000)
}

/// `egress.offline` (ADR 0043's `offline`).
pub const EGRESS_OFFLINE: Setting = Setting {
    name: "egress.offline",
    description: "Whether the engine opens no connection nobody configured: every pinned download, such as a Pinned artifact missing from the Browser cache, fails with `artifact_unavailable_offline` before it connects, naming the command that supplies it beforehand. `false` by default; set at every layer, and an Operator policy can lock it.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `network.resolve` (ADR 0059).
pub const NETWORK_RESOLVE: Setting = Setting {
    name: "network.resolve",
    description: "Host names mapped to addresses before any lookup, in the engine's resolver and in Chromium, each address still checked by ADR 0017's resolved-address rule. Empty by default; an Operator policy can lock it.",
    kind: ValueKind::Map(&ValueKind::List(&ValueKind::Address)),
    default: empty_table,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `tls.trusted_cas` (ADR 0042).
pub const TLS_TRUSTED_CAS: Setting = Setting {
    name: "tls.trusted_cas",
    description: "Extra certificate authorities by domain: certificate files or Secret references. Taken only from an Operator policy and user and project configuration, never from a Recipe or a run.",
    kind: ValueKind::Map(&ValueKind::List(&ValueKind::Text)),
    default: empty_table,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

fn trust_on_first_use_only(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|entries| entries.values().all(Value::is_null))
}

fn accepts_any_invalid_certificate(value: &Value) -> bool {
    value.as_object().is_some_and(|entries| !entries.is_empty())
}

/// `tls.accept_invalid_certs` (ADR 0042).
pub const TLS_ACCEPT_INVALID_CERTS: Setting = Setting {
    name: "tls.accept_invalid_certs",
    description: "Domains whose certificate may fail verification, each with a pin, `any`, or `null` for trust on first use. A pin and `any` are taken only from an Operator policy and user and project configuration; a Recipe or a run may list a domain only for trust on first use.",
    kind: ValueKind::Map(&ValueKind::Optional(&ValueKind::Text)),
    default: empty_table,
    clearable: false,
    authority: Authority::ConfigurationExcept(trust_on_first_use_only),
    weakens: Some(accepts_any_invalid_certificate),
    arrived_in: ARRIVED_IN,
};

/// `consent.domains` (ADR 0048).
pub const CONSENT_DOMAINS: Setting = Setting {
    name: "consent.domains",
    description: "The domains whose consent prompts a run accepts instead of refusing. A Recipe or a run only narrows it; it is widened only in configuration or by a person's answer.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Narrowed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `wall.domains` (ADR 0052).
pub const WALL_DOMAINS: Setting = Setting {
    name: "wall.domains",
    description: "The domains whose Walls a run may read past by generic means. A Recipe or a run only narrows it; it is widened only in configuration or by a person's answer.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Narrowed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `wall.refetch_domains` (ADR 0052).
pub const WALL_REFETCH_DOMAINS: Setting = Setting {
    name: "wall.refetch_domains",
    description: "The domains on which reading past a Wall may fetch a page again. A Recipe or a run only narrows it; it is widened only in configuration, never by an answer.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Narrowed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `wall.denied_domains` (ADR 0052).
pub const WALL_DENIED_DOMAINS: Setting = Setting {
    name: "wall.denied_domains",
    description: "The domains whose Walls no run reads past, whatever another layer allows. Set only in an Operator policy.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::OperatorPolicy,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `filter_lists.extra` (ADR 0047).
pub const FILTER_LISTS_EXTRA: Setting = Setting {
    name: "filter_lists.extra",
    description: "Extra Filter lists, by URL or path. Their scriptlets run only when the value comes from an Operator policy or user or project configuration; from a Recipe or a run, only their blocking rules apply.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `session.source` (ADR 0056).
pub const SESSION_SOURCE: Setting = Setting {
    name: "session.source",
    description: "Where a Session's login comes from. A Recipe never sets or locks it; it may only expose it to its users.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::NotByRecipe,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `secrets.by_reference` (ADR 0028).
pub const SECRETS_BY_REFERENCE: Setting = Setting {
    name: "secrets.by_reference",
    description: "Secrets are given by Secret reference, never as values, an honesty guarantee that stays locked whatever the layer.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Fixed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `webhook.signing` (ADR 0028).
pub const WEBHOOK_SIGNING: Setting = Setting {
    name: "webhook.signing",
    description: "Every webhook Goodfetch sends is signed, an honesty guarantee that stays locked whatever the layer.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Fixed,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.run` (ADRs 0080 and 0083).
pub const TIMEOUT_RUN: Setting = Setting {
    name: "timeout.run",
    description: "How long a Run may go on before it checkpoints and stops at its next safe point, resumable; none by default.",
    kind: ValueKind::Duration,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.navigation` (ADRs 0080 and 0083).
pub const TIMEOUT_NAVIGATION: Setting = Setting {
    name: "timeout.navigation",
    description: "How long a navigation may take before its attempt fails with `deadline_exceeded`: 30 s, Playwright's default.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.first_byte` (ADRs 0080 and 0083).
pub const TIMEOUT_FIRST_BYTE: Setting = Setting {
    name: "timeout.first_byte",
    description: "How long a request may wait for its response's first byte before its attempt fails with `deadline_exceeded`: 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CONFIG_EXPLAIN],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        SETTING_UNKNOWN,
        SETTING_INVALID,
        SETTING_NOT_APPLICABLE,
        SETTING_LOCKED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        EGRESS_OFFLINE,
        NETWORK_RESOLVE,
        TLS_TRUSTED_CAS,
        TLS_ACCEPT_INVALID_CERTS,
        CONSENT_DOMAINS,
        WALL_DOMAINS,
        WALL_REFETCH_DOMAINS,
        WALL_DENIED_DOMAINS,
        FILTER_LISTS_EXTRA,
        SESSION_SOURCE,
        SECRETS_BY_REFERENCE,
        WEBHOOK_SIGNING,
        TIMEOUT_RUN,
        TIMEOUT_NAVIGATION,
        TIMEOUT_FIRST_BYTE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
