//! Accounts (ADR 0149): whether a Daemon knows its person by an Account,
//! `accounts.enabled`, and what its Accounts store holds. The store is one
//! SQLite file per user and major, beside the Operator policy, which only
//! the Daemon opens. It holds the person's one Account, its passkeys'
//! public keys and the SHA-256 of each of its recovery codes and Named
//! tokens: what checks a credential, never a value that signs anyone in.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Authority, Entries, ProtocolVersion, Setting, ValueKind};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn none() -> Value {
    Value::Null
}

/// `accounts.enabled` (ADR 0149).
pub const ACCOUNTS_ENABLED: Setting = Setting {
    name: "accounts.enabled",
    description: "Whether the Daemon knows its person by an Account, kept in its Accounts store beside the Operator policy. Unset, the default, it is on for `daemon run`, the Exposed Daemon, and off for a machine's Daemon. Off, the Daemon opens no Accounts store and its person is the OS user, on its own machine only. Read once, as the Daemon starts. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Boolean,
    default: none,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What an Accounts store holds, table by table: the schema of the
/// `accounts-store` format.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredAccounts {
    /// Its Account, the only one; none until its person makes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub account: Option<StoredAccount>,
    /// The Account's passkeys.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub passkeys: Vec<StoredPasskey>,
    /// The Account's recovery codes, each by its hash.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recovery_codes: Vec<StoredRecoveryCode>,
    /// The Account's Named tokens, each by its hash.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub named_tokens: Vec<StoredNamedToken>,
}

/// The Account, as its `account` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredAccount {
    /// The name its person gave it, which a passkey shows.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// When it was made, as RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub created: String,
}

/// One passkey, as its `passkeys` row holds it: what checks its
/// assertions, never its private key, which stays with its authenticator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredPasskey {
    /// Its credential ID, in unpadded base64url.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub credential_id: String,
    /// Its public key, as the COSE key its registration gave, in unpadded
    /// base64url.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub public_key: String,
    /// When it was added, as RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: String,
}

/// One recovery code, as its `recovery_codes` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredRecoveryCode {
    /// The SHA-256 of the code, in lowercase hex.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// When it was used, as RFC 3339; none while it is unused. A code is
    /// used once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub used: Option<String>,
}

/// One Named token, as its `named_tokens` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredNamedToken {
    /// Its name, unique in the Account.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The SHA-256 of its value, in lowercase hex.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// When it was made, as RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub created: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[ACCOUNTS_ENABLED],
    hook_stages: &[],
    shared_type_values: &[],
};
