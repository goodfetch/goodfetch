//! What resolving a Secret reference answers (ADRs 0015, 0092 and 0129):
//! the Error codes a resolution fails with, each naming the placeholder and
//! the reference and never a value, and the settings the Secret-provider
//! driver reads. The references' grammar is `secret_reference`'s.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a resolution failed on, which `secret_unavailable` carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretFailure {
    /// The provider holds no value for the reference, such as an unset
    /// variable.
    Missing,
    /// The engine has no such provider, or its `secret_providers.<scheme>.*`
    /// configuration does not let it answer.
    NotConfigured,
    /// The provider failed in a way the engine does not know.
    ProviderFailed,
    /// The caller's `resolveSecret` is missing, threw or gave no answer.
    ResolverFailed,
    /// The provider gave no answer within `timeout.secret`.
    TimedOut,
}

impl SecretFailure {
    /// Its name as the wire writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::NotConfigured => "not_configured",
            Self::ProviderFailed => "provider_failed",
            Self::ResolverFailed => "resolver_failed",
            Self::TimedOut => "timed_out",
        }
    }
}

/// A provider gave no value (ADR 0092).
pub const SECRET_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_unavailable",
    description: "A Secret provider gave no value for a reference: a missing variable, a locked keychain, an unreachable vault, the caller's `resolveSecret` missing or throwing, no answer within `timeout.secret`. `failure` says which, and the details name the placeholder or setting, the reference and the provider, never a value.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `secret_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretUnavailableDetails {
    /// The Secret placeholder the value was for, or the setting's key where
    /// no placeholder names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The Secret reference.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The provider, by its scheme, such as `env`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provider: String,
    /// What failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: SecretFailure,
    /// What makes the reference resolve.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// No scope binds the placeholder (ADR 0092).
pub const SECRET_UNBOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_unbound",
    description: "No scope binds the Secret placeholder a call names. `bound` lists the placeholders bound, never a value.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `secret_unbound` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretUnboundDetails {
    /// The placeholder no scope binds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The placeholders bound where it was named.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bound: Vec<String>,
}

/// The receiving domain is outside the binding's (ADR 0092).
pub const SECRET_DOMAIN_NOT_ALLOWED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_domain_not_allowed",
    description: "A Secret placeholder's value would go to a domain outside its binding's `domains`: the field's frame or the request's URL. The details name the placeholder, the reference and the domain, never a value.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `secret_domain_not_allowed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretDomainNotAllowedDetails {
    /// The placeholder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The Secret reference.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The host the value would have gone to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// The binding's domains.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<String>,
}

fn null() -> Value {
    Value::Null
}

fn thirty_seconds() -> Value {
    json!(30_000)
}

/// `run_state.key_reference` (ADR 0015).
pub const RUN_STATE_KEY_REFERENCE: Setting = Setting {
    name: "run_state.key_reference",
    description: "The Secret reference of the key-encryption key that wraps each Run's data key, resolved when a Run starts: a Run whose reference does not resolve is refused with `secret_unavailable`. Taken only from an Operator policy and user and project configuration; none by default.",
    kind: ValueKind::SecretReference,
    default: null,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.secret` (ADR 0092).
pub const TIMEOUT_SECRET: Setting = Setting {
    name: "timeout.secret",
    description: "How long a Secret provider may take to answer one reference before the resolution fails with `secret_unavailable`, `failure: \"timed_out\"`: 30 s.",
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
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        SECRET_UNAVAILABLE,
        SECRET_UNBOUND,
        SECRET_DOMAIN_NOT_ALLOWED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[RUN_STATE_KEY_REFERENCE, TIMEOUT_SECRET],
    hook_stages: &[],
    shared_type_values: &[],
};
