//! The `config.*` entries that change the machine's own layers from the
//! Console (ADR 0031; spec #592): `config.policy_read` reads the Operator
//! policy with its digest, `config.policy_write` writes changes to it only
//! if its digest has not changed since that read, so a hand edit is never
//! lost, and `config.user_write` writes user-level values. The Console never
//! writes project configuration or a Recipe's manifest, which are files
//! under version control. Each is a person's entry, answered in the Console
//! or an interactive CLI prompt (ADR 0029), and each write goes to the
//! Audit log with its values before and after.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, SharedType, SharedTypeValue, Stability, schema_of,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A table of the Operator policy: what it sets, what it sets and locks, and
/// its caps.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PolicyTable {
    /// `[settings]`: values it sets, which a higher layer may change.
    Settings,
    /// `[lock]`: values it sets and locks.
    Lock,
    /// `[cap]`: the most a count or a duration may be.
    Cap,
}

impl PolicyTable {
    /// Its table's name in the file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Lock => "lock",
            Self::Cap => "cap",
        }
    }
}

/// One entry of the Operator policy: a key of one of its tables, and its
/// value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyEntry {
    /// Its table.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub table: PolicyTable,
    /// Its key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
}

/// What `config.policy_read` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyReadParams {}

/// The Operator policy as `config.policy_read` reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OperatorPolicyRead {
    /// Its file's path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Whether the file exists; a policy that does not sets nothing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found: bool,
    /// The digest of the file as read, which `config.policy_write` takes
    /// back. It is the policy version a Run report names, `sha256:` and 16
    /// hex digits, of the file's text, or of no text when there is none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// Every entry of its three tables, by table, then key, the other
    /// Product's keys included.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: Vec<PolicyEntry>,
}

/// One change `config.policy_write` makes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyChange {
    /// The table it changes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub table: PolicyTable,
    /// The key it changes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The key's new value; `null` removes it from the table.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
}

/// What `config.policy_write` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyWriteParams {
    /// The digest `config.policy_read` answered: the write is made only if
    /// the file still has it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The changes, made in order, in one write.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub changes: Vec<PolicyChange>,
}

/// One key's values before and after a write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingDiff {
    /// The Operator policy's table, for a change to it; none for user
    /// configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub table: Option<PolicyTable>,
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its value before; `null` when the file did not hold it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub before: Value,
    /// Its value after; `null` when the write removed it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Value,
}

/// What `config.policy_write` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyWritten {
    /// The file's digest after the write, the version new Runs name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// Each key the write changed, with its values before and after, as the
    /// Audit log records them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub diff: Vec<SettingDiff>,
}

/// One user-level value `config.user_write` writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserChange {
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its new value; `null` removes it from user configuration.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
}

/// What `config.user_write` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserWriteParams {
    /// The changes, made in order, in one write.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub changes: Vec<UserChange>,
}

/// What `config.user_write` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserConfigurationWritten {
    /// Each key the write changed, with its values before and after.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub diff: Vec<SettingDiff>,
}

/// `config.policy_read`.
pub const CONFIG_POLICY_READ: Operation = Operation {
    name: "config.policy_read",
    description: "Reads the Operator policy of this engine's major: every entry of its `settings`, `lock` and `cap` tables, and its digest, which `config.policy_write` takes back. A person's entry: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PolicyReadParams>,
    result: schema_of::<OperatorPolicyRead>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `config.policy_write`.
pub const CONFIG_POLICY_WRITE: Operation = Operation {
    name: "config.policy_write",
    description: "Writes changes to the Operator policy in one atomic write, only if its digest is still the one `config.policy_read` gave, every other byte of the file kept, and records the values before and after in the Audit log; a changed digest answers `policy_conflict` and writes nothing. A tightened cap reaches running Runs at their next admission point; every other change applies to new Runs. A person's entry: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PolicyWriteParams>,
    result: schema_of::<PolicyWritten>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `config.user_write`.
pub const CONFIG_USER_WRITE: Operation = Operation {
    name: "config.user_write",
    description: "Writes user-level values to user configuration in one atomic write, every other byte of the file kept, once the resolver accepts them under the Operator policy, and records the values before and after in the Audit log; they apply to new Runs. A person's entry: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<UserWriteParams>,
    result: schema_of::<UserConfigurationWritten>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `policy_conflict`.
pub const POLICY_CONFLICT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "policy_conflict",
    description: "The Operator policy's digest changed since it was read, as by a hand edit, so nothing was written; `fix` says to read it again.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `policy_conflict` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyConflictDetails {
    /// The digest the write was made against.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The file's digest now.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub current: String,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A kind of Audit log entry: a value of the shared type `audit_kind`.
const fn kind(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::AuditKind,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// A person wrote the Operator policy.
pub const OPERATOR_POLICY_WRITTEN: SharedTypeValue = kind(
    "operator_policy_written",
    "A person wrote the Operator policy through `config.policy_write`, with its digest before and after and each key's values before and after.",
);

/// A person wrote user configuration.
pub const USER_CONFIGURATION_WRITTEN: SharedTypeValue = kind(
    "user_configuration_written",
    "A person wrote user-level values through `config.user_write`, with each key's values before and after.",
);

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CONFIG_POLICY_READ, CONFIG_POLICY_WRITE, CONFIG_USER_WRITE],
    events: &[],
    handler_requests: &[],
    error_codes: &[POLICY_CONFLICT],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[OPERATOR_POLICY_WRITTEN, USER_CONFIGURATION_WRITTEN],
};
