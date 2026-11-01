//! Who uses the Console, and for how long (ADRs 0031 and 0098; spec #592):
//! the sign-ins a Daemon keeps in its accounts store, which outlive its
//! restarts and end after `console.sign_in.idle_expiry` without use, when
//! revoked, or when the Console key rotates; the rate at which one address
//! may try to sign in; and the person-only entries that list and revoke
//! sign-ins and rotate the key, answered only in the Console or an
//! interactive CLI prompt (ADR 0029). One Console serves both Products, so
//! everything here is Collie's.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting,
    SharedType, SharedTypeValue, Stability, ValueKind, schema_of,
};
use crate::run::PointInTime;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn thirty_days() -> serde_json::Value {
    serde_json::json!(30_u64 * 24 * 60 * 60 * 1000)
}

fn ten() -> serde_json::Value {
    serde_json::json!(10_u64)
}

/// `console.sign_in.idle_expiry`: how long a sign-in lasts unused.
pub const SIGN_IN_IDLE_EXPIRY: Setting = Setting {
    name: "console.sign_in.idle_expiry",
    description: "How long a Console sign-in lasts without being used, in milliseconds: 30 days. Each use starts it again; a sign-in unused for longer ends, and its person signs in again. Set in an Operator policy only.",
    kind: ValueKind::Duration,
    default: thirty_days,
    clearable: false,
    authority: Authority::OperatorPolicy,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `console.sign_in.rate`: how many sign-in attempts one address may make
/// in a minute.
pub const SIGN_IN_RATE: Setting = Setting {
    name: "console.sign_in.rate",
    description: "How many times one address may try to sign in to the Console in a minute: 10. A further attempt in that minute is refused, whatever its credential. Set in an Operator policy only.",
    kind: ValueKind::Count,
    default: ten,
    clearable: false,
    authority: Authority::OperatorPolicy,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// What `sign_in.list` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignInListParams {}

/// One sign-in, as `sign_in.list` shows it: never its cookie.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SignInSummary {
    /// Its identifier, which `sign_in.revoke` takes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// The person it signs in, by the name their account or OS user goes
    /// by.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub person: String,
    /// When it was created.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub created: PointInTime,
    /// When it was last used, from which its idle expiry counts.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last_used: PointInTime,
}

/// What `sign_in.list` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SignInList {
    /// Every sign-in that has not ended, oldest first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sign_ins: Vec<SignInSummary>,
}

/// What `sign_in.revoke` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignInRevokeParams {
    /// The sign-in's identifier, as `sign_in.list` gives it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
}

/// What `sign_in.revoke` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SignInRevoked {
    /// Whether a sign-in ended: `false` when none had that identifier, or
    /// it had already ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revoked: bool,
}

/// What `console.rotate_key` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConsoleRotateKeyParams {}

/// What `console.rotate_key` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleKeyRotated {
    /// How many sign-ins it ended: every one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signed_out: u64,
}

/// `sign_in.list`.
pub const SIGN_IN_LIST: Operation = Operation {
    name: "sign_in.list",
    description: "Lists the Console's sign-ins that have not ended, each with its person, when it was created and when it was last used, never its cookie. A person's entry: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::PersonOnly,
    params: schema_of::<SignInListParams>,
    result: schema_of::<SignInList>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `sign_in.revoke`.
pub const SIGN_IN_REVOKE: Operation = Operation {
    name: "sign_in.revoke",
    description: "Ends one Console sign-in, so its cookie signs nobody in, such as a lost laptop's. A person's entry: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<SignInRevokeParams>,
    result: schema_of::<SignInRevoked>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `console.rotate_key`.
pub const CONSOLE_ROTATE_KEY: Operation = Operation {
    name: "console.rotate_key",
    description: "Replaces the Console key, which every major of the user shares, so every Console sign-in ends and everyone signs in again. A person's entry: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<ConsoleRotateKeyParams>,
    result: schema_of::<ConsoleKeyRotated>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A kind of Audit log entry: a value of the shared type `audit_kind`.
const fn kind(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::AuditKind,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// A person signed in to the Console.
pub const SIGNED_IN: SharedTypeValue = kind(
    "signed_in",
    "A person signed in to the Console, with the sign-in's identifier and how they proved who they are, such as the single-user link.",
);

/// A sign-in ended before its idle expiry.
pub const SIGN_IN_REVOKED: SharedTypeValue = kind(
    "sign_in_revoked",
    "A Console sign-in was ended, by its person signing out or by `sign_in.revoke`, with its identifier.",
);

/// The Console key rotated.
pub const CONSOLE_KEY_ROTATED: SharedTypeValue = kind(
    "console_key_rotated",
    "The Console key was replaced, which ended every Console sign-in, with how many it ended.",
);

/// One sign-in as the accounts store keeps it: the schema of the
/// `accounts-store` format. Never its cookie: the store keeps a verifier,
/// an HMAC of the cookie under the Console key, so a stolen store signs
/// nobody in and a rotated key ends every sign-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SignInRow {
    /// Its identifier, the first part of its cookie.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// HMAC-SHA-256 of its cookie under the Console key, in hex.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verifier: String,
    /// The person it signs in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub person: String,
    /// When it was created, as RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub created: String,
    /// When it was last used, as RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last_used: String,
    /// The key, in hex, that wraps the sign-in's offline cache key in the
    /// browser, which the Daemon unwraps only for its cookie (ADR 0031).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wrapping_key: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[SIGN_IN_LIST, SIGN_IN_REVOKE, CONSOLE_ROTATE_KEY],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[SIGN_IN_IDLE_EXPIRY, SIGN_IN_RATE],
    hook_stages: &[],
    shared_type_values: &[SIGNED_IN, SIGN_IN_REVOKED, CONSOLE_KEY_ROTATED],
};
