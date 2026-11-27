//! Listing and removing installed Plugins (ADRs 0033, 0094, 0098 and 0099;
//! issue 928): `plugin.list` (a read, every surface) reports every installed
//! Plugin in the shared Plugin store with its kinds, digest, signer, network
//! level, approvals, conformance results, `settings_schema`, and whether its
//! kinds belong to the running Product; `plugin.remove` (person-only) deletes
//! a Plugin from the store, its lockfile entry and its Operator policy
//! approvals, and is refused while a live Run uses it.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, SharedType,
    SharedTypeValue, Stability, schema_of,
};
use crate::plugin::approvals::Approvals;
use crate::plugin::index::ConformanceResult;
use crate::plugin::manifest::NetworkLevel;
use crate::plugin::names::PluginKindName;
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `plugin.list` takes (ADR 0098, issue 928).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginListParams {
    /// The project configuration the Client found, layer 3 (ADR 0083): used
    /// to read the project's lockfile and settings when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// The Operator policy approval state recorded for an installed Plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledPluginApprovals {
    /// Whether Operator policy allows (`true`) or denies (`false`) loading it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub allow: Option<bool>,
    /// The digest pinned in Operator policy, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: Option<String>,
    /// The permissions approved in Operator policy.
    #[serde(default, skip_serializing_if = "Approvals::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approved: Approvals,
}

/// One installed Plugin in the shared Plugin store (ADRs 0033 and 0098;
/// issue 928).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledPluginInfo {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its installed version, from its manifest.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The Plugin kinds its manifest declares.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: BTreeSet<PluginKindName>,
    /// The image index digest recorded beside its bundle, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: Option<String>,
    /// The signer identity recorded in the lockfile or Plugin index, when
    /// known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: Option<String>,
    /// What its manifest lets it reach on the network.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network_level: NetworkLevel,
    /// The Operator policy approval state for this Plugin, when any policy
    /// fields are set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approvals: Option<InstalledPluginApprovals>,
    /// Conformance suite results recorded in the cached Plugin index for this
    /// Plugin, when listed.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub conformance: Vec<ConformanceResult>,
    /// The JSON Schema of the Plugin's settings from its manifest, so a
    /// surface can render its fields (ADR 0033).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings_schema: Option<Value>,
    /// Whether at least one of the Plugin's kinds is carried by the running
    /// Product (`false` marks e.g. a Sink or Fetch backend Plugin on Collie).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub available_in_product: bool,
    /// Kinds declared by the Plugin that the running Product does not carry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unsupported_kinds: Vec<PluginKindName>,
}

/// What `plugin.list` answers (ADR 0098, issue 928).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginListResult {
    /// The Product that answered the list (`collie` or `foxhound`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Every installed Plugin in the shared Plugin store, sorted by name.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: Vec<InstalledPluginInfo>,
}

/// `plugin.list` (ADRs 0033, 0094 and 0098; issue 928).
pub const PLUGIN_LIST: Operation = Operation {
    name: "plugin.list",
    description: "Lists every Plugin installed in the shared Plugin store with its kinds, digest, signer, network level, approvals, conformance results and `settings_schema`, marking Plugins of a kind the running Product lacks (`available_in_product: false`, `unsupported_kinds`) without deleting them. Available on every surface.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PluginListParams>,
    result: schema_of::<PluginListResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin.remove` takes (ADR 0033, issue 928).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRemoveParams {
    /// The name of the Plugin to remove.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The project configuration the Client found, layer 3 (ADR 0083): the
    /// lockfile beside it is updated, else the one in the Goodfetch home's
    /// configuration folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// What happened to the Plugin's approvals in the Operator policy on removal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginApprovalsRemoval {
    /// Removed from the Operator policy and recorded in the Audit log.
    Removed {
        /// The Operator policy file.
        #[schemars(extend("arrived_in" = "0.1"))]
        policy: String,
    },
    /// No Operator policy approvals were present for the Plugin.
    Absent,
    /// The Operator policy is managed elsewhere, so its entry was left alone.
    Managed {
        /// The Operator policy key to remove there (`plugins.<name>`).
        #[schemars(extend("arrived_in" = "0.1"))]
        key: String,
    },
}

/// What `plugin.remove` answers once the Plugin is removed (ADR 0033,
/// issue 928).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginRemoved {
    /// The removed Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Whether an unpacked bundle was deleted from the Plugin store.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed_from_store: bool,
    /// Whether an entry was removed from the Plugin lockfile.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed_from_lockfile: bool,
    /// What happened to its Operator policy approvals.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approvals: PluginApprovalsRemoval,
}

/// `plugin.remove` (ADR 0033, issue 928).
pub const PLUGIN_REMOVE: Operation = Operation {
    name: "plugin.remove",
    description: "Deletes a Plugin from the shared Plugin store, removes its entry from the Plugin lockfile and clears its approvals from the Operator policy (audited as `plugin_removed`). Refused with `run_live` while any live Run is using the Plugin. Person-only (ADR 0099): any non-person surface is refused with `person_required`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PluginRemoveParams>,
    result: schema_of::<PluginRemoved>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin's approvals were removed from the Operator policy.
pub const PLUGIN_REMOVED_AUDIT: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "plugin_removed",
    description: "A person removed an installed Plugin from the Plugin store, its lockfile entry and its Operator policy approvals.",
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[PLUGIN_LIST, PLUGIN_REMOVE],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[PLUGIN_REMOVED_AUDIT],
};
