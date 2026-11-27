//! The Secret provider, a Plugin kind (ADRs 0015, 0033, 0092 and 0129): an
//! implementation answers `secret.resolve` with `{ value }` for a Secret
//! reference under the scheme its manifest declares (`plugin:<scheme>:<rest>`).
//!
//! The engine records only the reference, the outcome and the time — never the
//! answer — and wraps the value immediately in a zeroizing `SecretString` that
//! joins the Run's mask and never reaches a model, journal, trace, Usage or
//! log. Any failure or unexpected shape becomes `secret_unavailable`.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, PluginKind, PluginMethod, ProtocolVersion, Stability, schema_of};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Secret provider's method, its wire name with its kind's.
pub const SECRET_RESOLVE: &str = "secret.resolve";

/// What `secret.resolve` asks a Secret provider Plugin for: the Secret
/// reference to resolve, under the scheme its manifest declares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretResolveParams {
    /// The Secret reference to resolve (`plugin:<scheme>:<rest>`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
}

/// What a Secret provider Plugin answers to `secret.resolve`: the secret's
/// value, and no other field. Its `Debug` hides the value so a decoded answer
/// never reaches a log or panic message.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretResolveResult {
    /// The resolved secret value, immediately wrapped in a zeroizing
    /// `SecretString` by the engine's driver.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
}

impl fmt::Debug for SecretResolveResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretResolveResult")
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// The Secret provider kind.
pub const SECRET_PROVIDER: PluginKind = PluginKind {
    name: "secret",
    description: "Resolves Secret references under its manifest's declared scheme from a vault Goodfetch does not ship.",
    methods: &[PluginMethod {
        name: "resolve",
        description: "Resolves one Secret reference and returns its value.",
        params: schema_of::<SecretResolveParams>,
        result: schema_of::<SecretResolveResult>,
    }],
    chosen_by: None,
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// The Secret provider kind's catalog entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[SECRET_PROVIDER],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
