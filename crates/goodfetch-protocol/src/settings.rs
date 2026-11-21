//! Settings and their layers (ADRs 0028, 0083, 0087 and 0098), which every
//! Product carries: the five layers a value resolves through, the refusals
//! of a value that cannot take effect where it is set, `config.explain`
//! with the explanation it answers and `config explain --json` prints, the
//! files of the two layers kept in Goodfetch home (Operator policy and user
//! configuration, per major, shared by both Products), the Recipe
//! manifest's Product and settings, the closed list of deployment
//! variables (kept in `deployment`), and the keys of areas whose own modules
//! do not exist yet, each with its area first (ADR 0083). Each later area moves its keys into
//! its own module.
//!
//! Size: one catalog module: the contract of settings and their layers, and
//! the flat declarations of the keys whose areas have no module yet, which
//! leave as those modules arrive.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    Product, ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
};

mod deployment;
pub mod edit;
pub mod page_check;
pub mod session_source;

pub use deployment::{
    BROWSERS_PATH_VARIABLE, CHANNEL_VARIABLE, COLLIE_ENGINE_PATH_VARIABLE, DAEMON_ADDRESS_VARIABLE,
    DAEMON_DRAIN_GRACE_VARIABLE, DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES,
    FOXHOUND_ENGINE_PATH_VARIABLE, HOME_VARIABLE, OPS_ADDRESS_VARIABLE, TEST_SEED_VARIABLE,
    TOKEN_VARIABLE, URL_VARIABLE,
};

/// A field that is present, `null` included, as `Some`; one left out stays
/// `None` through `#[serde(default)]`.
#[expect(
    clippy::option_option,
    reason = "three states (ADRs 0080 and 0083): left out inherits, `null` clears, a value sets"
)]
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

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
    /// Every value, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Default,
        Self::OperatorPolicy,
        Self::UserConfiguration,
        Self::ProjectConfiguration,
        Self::Recipe,
        Self::Run,
    ];

    /// Its name as the catalog and messages write it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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
    /// A program's only SDK handler of a kind that one setting chooses,
    /// registered on its connection: the lowest of the run layer's origins,
    /// which every other replaces and configuration does not. A lock wins
    /// over it with no refusal, since the program never wrote the key (ADR
    /// 0104's amendment of ADR 0083).
    Registered,
}

impl RunOrigin {
    /// Every value, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Argument,
        Self::Settings,
        Self::Flag,
        Self::Form,
        Self::Registered,
    ];

    /// Its name as the wire and messages write it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// A setting that decided an outcome: its key, its value, the layer that
/// set it and, for the run layer, where the value came from (ADRs 0028,
/// 0079 and 0083).
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
    /// For the run layer, where the value came from, such as `argument`
    /// for a call's own `timeout` (ADR 0083); absent when unknown or for
    /// another layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: Option<RunOrigin>,
}

/// A setting that weakened a safeguard for a Run (ADR 0028): its key, its
/// value, the layer that set it and, for the run layer, where the value came
/// from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WeakenedSetting {
    /// The setting's key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
    /// The layer that set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// For the run layer, where the value came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: Option<RunOrigin>,
    /// When a level's preset gave the value, at `layer`, that level, such
    /// as `run_state.durability: off` (ADR 0142).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub preset: Option<SettingPreset>,
}

/// The setting whose value names the preset that gave another setting its
/// value: `run_state.durability` and its level unfold into the
/// `run_state.batch.*` keys at the layer that set the level (ADR 0142).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingPreset {
    /// The key that names the preset, such as `run_state.durability`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its value, the preset's name, such as `full`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
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
    /// The value refused, as the setting it would have set: a call's own
    /// `timeout` of 0 names its timeout setting at the run layer, origin
    /// `argument` (ADR 0080); absent where no value was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<SettingLimit>,
    /// What to give instead, such as the null form that sets no timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: Option<String>,
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

/// A capability this engine cannot provide where it was asked for (ADR
/// 0083).
pub const CAPABILITY_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "capability_unavailable",
    description: "A capability this engine cannot provide where a setting asks for it, such as a Session opening from a Session source of a kind this engine does not read: the details name the setting, the layer that set it and what it asked for, and its fix names the value that does without it.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `capability_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityUnavailableDetails {
    /// The setting that asks for it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The layer that set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// What it asks for, such as a Session source's kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// The value that does without it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

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
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent fact of the key on the wire, not a state of one machine"
)]
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
    /// When a level's preset gave the value, at `layer`, that level, which
    /// `config explain` shows as "preset of `run_state.durability: full`"
    /// in place of the layer (ADR 0142).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub preset: Option<SettingPreset>,
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
    /// Whether user configuration may set it: its authority lets user
    /// configuration hold it and no lower layer locked it, so a person
    /// surface may offer its user-level value for editing (ADR 0031).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub user_settable: bool,
    /// Whether it is fixed for a Run's life: a resume that changes it is
    /// refused with `setting_not_applicable`, and only a new Run takes
    /// another value (ADRs 0102 and 0120).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fixed_for_run: bool,
    /// The setting its value yields to, with that setting's value and
    /// layer, when one that is set keeps its value from taking effect:
    /// `fetch.http_from` away from `browser` yields to the Run's
    /// `session.source`, whose Sessions hold a login and so send their HTTP
    /// from the browser (#2418).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub yields_to: Option<SettingLimit>,
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
    /// The path of the project configuration the Client was named, in place
    /// of `project`, when no file is there: it sets nothing, and the answer
    /// lists it as not found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub missing_project: Option<String>,
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
    /// The Run ID of a live Run to explain, with the settings it opened
    /// with; none explains the Connection's own, the run values given set
    /// over them (ADR 0083). One that is no Run ID fails with
    /// `run_id_invalid`, and one given with `settings`, `origin`,
    /// `project` or `missing_project` with `invalid_params`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<String>,
}

/// `config.explain`.
pub const CONFIG_EXPLAIN: Operation = Operation {
    name: "config.explain",
    description: "Shows each setting's effective value, the layer that set it and the layer that locked it, from the built-in defaults, the Operator policy, user configuration, the project configuration given and the run layer: the Connection's own settings with the run values given over them, or a live Run's as it opened when `run_id` names one; and lists the keys only the other Product honours as that Product's.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<ConfigExplainParams>,
    result: schema_of::<SettingsExplanation>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
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
    /// The largest value a higher layer may give a count or a duration; a
    /// Run takes the cap it opens under, so a tightened cap reaches the next
    /// Run.
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

fn no() -> Value {
    Value::Bool(false)
}

fn yes() -> Value {
    Value::Bool(true)
}

fn thirty_seconds() -> Value {
    json!(30_000)
}

fn ten_seconds() -> Value {
    json!(10_000)
}

fn one_minute() -> Value {
    json!(60_000)
}

fn two_minutes() -> Value {
    json!(120_000)
}

fn ten_minutes() -> Value {
    json!(600_000)
}

fn a_day() -> Value {
    json!(86_400_000)
}

/// `network.resolve` (ADR 0059).
pub const NETWORK_RESOLVE: Setting = Setting {
    name: "network.resolve",
    description: "Host names mapped to addresses before any lookup, in the engine's resolver and in Chromium, each address still checked by ADR 0017's resolved-address rule. Empty by default; an Operator policy can lock it.",
    kind: ValueKind::Map(&ValueKind::List(&ValueKind::Address)),
    default: empty_table,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The values `consent.handling` takes (ADR 0048, #2434).
pub const CONSENT_HANDLING_VALUES: &[&str] = &["answer", "detect", "off"];

/// `consent.handling` (ADR 0048, #2434).
pub const CONSENT_HANDLING: Setting = Setting {
    name: "consent.handling",
    description: "What a browser Session does with Consent prompts: `answer` (the default, in every Access mode) refuses each prompt in every frame and hides one with no refusal that leaves the content readable; `detect` only looks: nothing is clicked or hidden, the wall test still runs, and a consent wall is still `consent_required`; `off` registers and starts no Consent prompt runner and lets no cosmetic rule act on the live page or in extracted content, so no consent wall is reported and the page's text is returned as served, while the Filter lists still block requests; the Run report lists `off` in `weakened`. A Recipe, and a Client acting for a model, never set `off` unless the layers below already give it: `setting_locked`; set it in the program's own code, in configuration or in an Operator policy.",
    kind: ValueKind::Choice(CONSENT_HANDLING_VALUES),
    default: answer,
    clearable: false,
    authority: Authority::NotWeakenedByRecipeOrModel,
    weakens: Some(handling_off),
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn answer() -> Value {
    Value::String("answer".to_owned())
}

/// `consent.handling: off`, which leaves the page as served (#2434).
fn handling_off(value: &Value) -> bool {
    value.as_str() == Some("off")
}

/// `consent.domains` (ADR 0048).
pub const CONSENT_DOMAINS: Setting = Setting {
    name: "consent.domains",
    description: "The registrable domains on which a run accepts a consent wall, a Consent prompt with no refusal that covers the content or offers payment instead, rather than raising `blocked` with `consent_required`; `*` only when written. A prompt that offers a refusal is still refused there. A Recipe or a run only narrows it; it is widened only in an Operator policy, user or project configuration, or by a person's answer.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Narrowed,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The values `consent.on_wall` takes (ADR 0048).
pub const CONSENT_ON_WALL_VALUES: &[&str] = &["report", "ask"];

/// `consent.on_wall` (ADR 0048).
pub const CONSENT_ON_WALL: Setting = Setting {
    name: "consent.on_wall",
    description: "What a consent wall on a domain outside `consent.domains` does: `report` (the default) leaves the page at `consent_required`, raised as `blocked`; `ask` raises a Human pause for a person's answer, for this run or always for the domain. Until the Human pause takes a person's answer, `ask` opens no page: `capability_unavailable`.",
    kind: ValueKind::Choice(CONSENT_ON_WALL_VALUES),
    default: report,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn report() -> Value {
    Value::String("report".to_owned())
}

/// `wall.domains` (ADR 0052).
pub const WALL_DOMAINS: Setting = Setting {
    name: "wall.domains",
    description: "The domains whose Walls a run may read past by generic means. A Recipe or a run only narrows it; it is widened only in configuration or by a person's answer.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Narrowed,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("wall_reading"),
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The values `clean_up.update` takes (ADR 0047).
pub const CLEAN_UP_UPDATES: &[&str] = &["release", "daily", "upstream"];

/// `clean_up.update` (ADR 0047).
pub const CLEAN_UP_UPDATE: Setting = Setting {
    name: "clean_up.update",
    description: "Where a new Run's Clean-up rules come from: `release` (the default), the set this release pins, with the embedded part as fallback; `daily`, the opt-in daily channel, whose registry it makes a configured destination, refused with `capability_unavailable` until the engine checks the channel's signature; `upstream`, the Filter lists from their authors' URLs, which it makes configured destinations. A Run holds to the version it starts with for its whole life, a resume included; `egress.offline` turns the fetching off. Operator policy can lock it.",
    kind: ValueKind::Choice(CLEAN_UP_UPDATES),
    default: release,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `clean_up.version` (ADR 0047).
pub const CLEAN_UP_VERSION: Setting = Setting {
    name: "clean_up.version",
    description: "The exact version of the Clean-up rules a new Run holds to, by its digest `sha256:<hex>` as Provenance names it, instead of the newest one `clean_up.update` gives: a daily version it names is fetched from the daily channel, and it is exempt from the channel's expiry and its refusal of older versions. None by default.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn release() -> Value {
    Value::String("release".to_owned())
}

/// `clean_up.channel` (ADR 0047).
pub const CLEAN_UP_CHANNEL: Setting = Setting {
    name: "clean_up.channel",
    description: "The OCI repository the daily channel is pulled from, `host[:port]/path`: Goodfetch's own on ghcr.io by default, or a mirror of it, which `clean_up.update = daily` makes the configured destination. Only Operator policy and user and project configuration set it: a Recipe's or a run's value is `setting_locked`.",
    kind: ValueKind::Text,
    default: daily_channel,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The daily channel's OCI repository by default.
pub const DAILY_CHANNEL: &str = "ghcr.io/goodfetch/clean-up-rules";

fn daily_channel() -> Value {
    Value::String(DAILY_CHANNEL.to_owned())
}

/// `clean_up.lists` (ADR 0047).
pub const CLEAN_UP_LISTS: Setting = Setting {
    name: "clean_up.lists",
    description: "The URL `clean_up.update = upstream` fetches each Filter list from, by list: `ads` (EasyList), `trackers` (EasyPrivacy), `annoyances` (Fanboy Annoyance) and `unbreak` (Brave's unbreak list); their authors' own URLs by default, or a mirror's. Each must be `https`. Only Operator policy and user and project configuration set it: a Recipe's or a run's value is `setting_locked`.",
    kind: ValueKind::Map(&ValueKind::Text),
    default: upstream_lists,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `clean_up.fetch_timeout` (ADRs 0047 and 0080).
pub const CLEAN_UP_FETCH_TIMEOUT: Setting = Setting {
    name: "clean_up.fetch_timeout",
    description: "How long fetching a version of the Clean-up rules from its sources may take at a Run's start or resume: past it, a Run holds to the newest version this machine has, and a `clean_up.version` it cannot fetch refuses the start. 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The Filter lists' authors' own URLs, by list, which `clean_up.lists`
/// holds by default.
pub const UPSTREAM_LISTS: [(&str, &str); 4] = [
    ("ads", "https://easylist.to/easylist/easylist.txt"),
    ("trackers", "https://easylist.to/easylist/easyprivacy.txt"),
    (
        "annoyances",
        "https://secure.fanboy.co.nz/fanboy-annoyance.txt",
    ),
    (
        "unbreak",
        "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-unbreak.txt",
    ),
];

fn upstream_lists() -> Value {
    Value::Object(
        UPSTREAM_LISTS
            .iter()
            .map(|(list, url)| ((*list).to_owned(), Value::String((*url).to_owned())))
            .collect(),
    )
}

/// `filter_lists.ads` (ADR 0047).
pub const FILTER_LISTS_ADS: Setting = Setting {
    name: "filter_lists.ads",
    description: "Whether a browser Session blocks ads by EasyList, as a blocker does: on by default in every Access mode. A blocked request fails as `BlockedByClient`; the never-block list keeps challenge, anti-bot, anti-fraud and payment scripts loading.",
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

/// `filter_lists.trackers` (ADR 0047).
pub const FILTER_LISTS_TRACKERS: Setting = Setting {
    name: "filter_lists.trackers",
    description: "Whether a browser Session blocks trackers by EasyPrivacy: on by default in every Access mode. A blocked request fails as `BlockedByClient`; the never-block list keeps challenge, anti-bot, anti-fraud and payment scripts loading.",
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

/// `filter_lists.annoyances` (ADR 0047).
pub const FILTER_LISTS_ANNOYANCES: Setting = Setting {
    name: "filter_lists.annoyances",
    description: "Whether a browser Session blocks page annoyances (social widgets, newsletter and notification prompts) by Fanboy Annoyance, its cookie sections left to the Consent prompt runner: on by default in every Access mode.",
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

/// `filter_lists.first_party_scripts` (ADR 0047).
pub const FILTER_LISTS_FIRST_PARTY_SCRIPTS: Setting = Setting {
    name: "filter_lists.first_party_scripts",
    description: "Whether a site's own scripts load whatever the Filter lists say: off by default.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `filter_lists.page_scriptlets` (ADR 0047).
pub const FILTER_LISTS_PAGE_SCRIPTLETS: Setting = Setting {
    name: "filter_lists.page_scriptlets",
    description: "Whether the Filter lists' scriptlets that patch a page's JavaScript run, from uBlock Origin's resources downloaded with the lists that name them: off by default, since a site can see them.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `filter_lists.host_cache` (spec #2164).
pub const FILTER_LISTS_HOST_CACHE: Setting = Setting {
    name: "filter_lists.host_cache",
    description: "How many hosts' Clean-up rules this Run keeps built: 1,024, the least used dropped first, and none once the Resource governor finds memory critical. A page on a host it keeps builds no rule list. 0 builds each page's rule list, as before.",
    kind: ValueKind::Count,
    default: hosts_kept,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn hosts_kept() -> Value {
    Value::from(1_024_u64)
}

/// `session.source` (ADR 0056).
pub const SESSION_SOURCE: Setting = Setting {
    name: "session.source",
    description: "Where a Session's login comes from: exactly one kind of ADR 0056's tagged union, or none for a fresh Session. Set at a Run or a connection, it is the default of the Sessions opened beneath; opening a Session takes its own as the call's `source`, and in that Session's own `settings` it is refused with `setting_not_applicable`. A Recipe never sets or locks it; it may only expose it to its users.",
    kind: ValueKind::Shape(&session_source::SESSION_SOURCE_SHAPE),
    default: null,
    clearable: true,
    authority: Authority::NotByRecipe,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.page_check` (ADRs 0006 and 0092).
pub const SESSION_PAGE_CHECK: Setting = Setting {
    name: "session.page_check",
    description: "The Page check every page of a Session is judged against once its navigation reached its document: `login_page` and `expected_page`, each optional and each a page match, a URL glob or regular expression, a CSS or XPath selector, or a text. A page matching its login page raises `logged_out` from the call that waited for its navigation; one matching neither, when an expected page is given, raises `page_check_failed`. None by default: the built-in detector then judges a page a login page when the navigation was redirected to a login path, or when the page shows one password field outside a login path, and Provenance marks each page it judged as weakly checked. Set at a Run, a connection or a Session's own `settings`.",
    kind: ValueKind::Shape(&page_check::PAGE_CHECK_SHAPE),
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.connect` (ADRs 0080 and 0083), which `connect()` and the CLI
/// read from the project file before they connect.
pub const TIMEOUT_CONNECT: Setting = Setting {
    name: "timeout.connect",
    description: "How long `connect()` and the CLI wait for the engine to answer `hello` before they fail with `deadline_exceeded`: 30 s. Set only in the project file, which they read before they connect; a connection's own is `connect()`'s `timeout`.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::ProjectFileAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.close` (ADRs 0080 and 0083).
pub const TIMEOUT_CLOSE: Setting = Setting {
    name: "timeout.close",
    description: "How long a stop of a Run, `run.stop` or a scope's close, waits for its next safe point before it fails with `deadline_exceeded`, the Run still stopping there: 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.action` (ADRs 0019, 0080 and 0083), the guard's window.
pub const TIMEOUT_ACTION: Setting = Setting {
    name: "timeout.action",
    description: "How long a locator action waits for its target to be unique and actionable, and replays a gesture that landed elsewhere, before it fails with `deadline_exceeded`, or `target_not_unique` for a target still ambiguous: 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.launch` (ADR 0080's note of issue 2813), a browser's bring-up.
pub const TIMEOUT_LAUNCH: Setting = Setting {
    name: "timeout.launch",
    description: "How long a browser the engine launches, or a Borrowed one it connects to, may leave each command of its bring-up unanswered, its identity read's page load included, before the call that needs it fails with `deadline_exceeded`: 30 s, Playwright's default.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.wait` (ADRs 0080 and 0083), an app-state wait's.
pub const TIMEOUT_WAIT: Setting = Setting {
    name: "timeout.wait",
    description: "How long a wait for an application state, such as `page.wait_for_request_finished`, may take before it fails with `deadline_exceeded`: 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
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
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.hook` (ADRs 0080, 0083 and 0104).
pub const TIMEOUT_HOOK: Setting = Setting {
    name: "timeout.hook",
    description: "How long each call of a Hook may run before it fails with `deadline_exceeded`: 10 s. A crawl's Label may set it for the Hooks on its requests.",
    kind: ValueKind::Duration,
    default: ten_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.sink_write` (ADRs 0080 and 0083), refused away from its default until its phase is
/// built.
pub const TIMEOUT_SINK_WRITE: Setting = Setting {
    name: "timeout.sink_write",
    description: "How long one write to a Sink may take before it fails with `deadline_exceeded`: 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("sinks"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.agent` (ADRs 0080 and 0100), refused away from its default until its phase is
/// built.
pub const TIMEOUT_AGENT: Setting = Setting {
    name: "timeout.agent",
    description: "How long an Agent's task may run, its actions and model calls nested inside, before it stops with `ended: stopped` and the reason `timeout`: 10 min.",
    kind: ValueKind::Duration,
    default: ten_minutes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("agent"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.ai` (ADRs 0080 and 0091), refused away from its default until its phase is
/// built.
pub const TIMEOUT_AI: Setting = Setting {
    name: "timeout.ai",
    description: "How long an AI verb may take from its call to its return, its model calls and a Repair included, before it fails with `deadline_exceeded`: 120 s.",
    kind: ValueKind::Duration,
    default: two_minutes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("ai_verbs"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.model` (ADRs 0080 and 0101), refused away from its default until its phase is
/// built.
pub const TIMEOUT_MODEL: Setting = Setting {
    name: "timeout.model",
    description: "How long one attempt of one model request may take before it is retried within the call that made it: 60 s.",
    kind: ValueKind::Duration,
    default: one_minute,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("model_calls"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.human_pause` (ADRs 0080 and 0092), refused away from its default until its phase is
/// built.
pub const TIMEOUT_HUMAN_PAUSE: Setting = Setting {
    name: "timeout.human_pause",
    description: "How long a Human pause may wait for a person before it fails with `deadline_exceeded`: 24 h; in a scheduled run, ADR 0036's pause timeout governs too, and the first to expire acts.",
    kind: ValueKind::Duration,
    default: a_day,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("human_pause"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `admission.max_requests_per_minute` (RT-02, ADR 0004).
pub const ADMISSION_MAX_REQUESTS_PER_MINUTE: Setting = Setting {
    name: "admission.max_requests_per_minute",
    description: "How many requests a Run admits in a minute at most, spaced evenly (GCRA), whatever its concurrency; none by default. An Operator policy cap bounds it even when unset.",
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
        CAPABILITY_UNAVAILABLE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        NETWORK_RESOLVE,
        CONSENT_HANDLING,
        CONSENT_DOMAINS,
        CONSENT_ON_WALL,
        WALL_DOMAINS,
        WALL_REFETCH_DOMAINS,
        WALL_DENIED_DOMAINS,
        FILTER_LISTS_ADS,
        FILTER_LISTS_TRACKERS,
        FILTER_LISTS_ANNOYANCES,
        FILTER_LISTS_FIRST_PARTY_SCRIPTS,
        FILTER_LISTS_PAGE_SCRIPTLETS,
        FILTER_LISTS_HOST_CACHE,
        FILTER_LISTS_EXTRA,
        CLEAN_UP_UPDATE,
        CLEAN_UP_VERSION,
        CLEAN_UP_CHANNEL,
        CLEAN_UP_LISTS,
        CLEAN_UP_FETCH_TIMEOUT,
        SESSION_SOURCE,
        SESSION_PAGE_CHECK,
        SECRETS_BY_REFERENCE,
        WEBHOOK_SIGNING,
        TIMEOUT_RUN,
        TIMEOUT_CONNECT,
        TIMEOUT_CLOSE,
        TIMEOUT_ACTION,
        TIMEOUT_NAVIGATION,
        TIMEOUT_LAUNCH,
        TIMEOUT_WAIT,
        TIMEOUT_FIRST_BYTE,
        TIMEOUT_HOOK,
        TIMEOUT_SINK_WRITE,
        TIMEOUT_AGENT,
        TIMEOUT_AI,
        TIMEOUT_MODEL,
        TIMEOUT_HUMAN_PAUSE,
        ADMISSION_MAX_REQUESTS_PER_MINUTE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
