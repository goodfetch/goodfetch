//! Installing the Linux `goodfetch-sandbox` helper and its AppArmor profile
//! (ADRs 0033, 0039 and 0099; issue 884): `plugin.setup` verifies the
//! Sigstore provenance attestation of the helper (and optional AppArmor
//! profile) against the bundled trust root before installing a root-owned,
//! non-setuid copy and loading its AppArmor profile on Ubuntu 23.10 and later.
//! It is person-only (ADR 0099): any non-interactive or programmatic caller is
//! refused with `person_required`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::plugin::index::SignerIdentity;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `plugin.setup` takes (ADRs 0033 and 0039; issue 884).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginSetupParams {
    /// Path to the `goodfetch-sandbox` helper executable to verify and install.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub helper: String,
    /// Path to the Sigstore provenance attestation JSON for `helper`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attestation: String,
    /// Optional path to an AppArmor profile file to verify and install. When
    /// omitted, `plugin.setup` writes the standard AppArmor profile granting
    /// `userns` exclusively to the installed helper path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile: Option<String>,
    /// Path to the Sigstore provenance attestation JSON for `profile`;
    /// required whenever `profile` is provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_attestation: Option<String>,
}

/// What `plugin.setup` answers once the helper and its AppArmor profile are
/// verified and installed (ADRs 0033 and 0039; issue 884).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSetupResult {
    /// Where the verified `goodfetch-sandbox` helper was installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub helper_path: String,
    /// The SHA-256 digest (`sha256:<hex>`) of the verified helper bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub helper_digest: String,
    /// Where the AppArmor profile was installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_path: String,
    /// Whether `apparmor_parser --replace` loaded the profile into the kernel.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_loaded: bool,
    /// The verified signer identity carried by the helper's attestation.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: SignerIdentity,
}

/// `plugin.setup` (ADRs 0033, 0039 and 0099; issue 884).
pub const PLUGIN_SETUP: Operation = Operation {
    name: "plugin.setup",
    description: "Verifies the Sigstore provenance attestation of the Linux `goodfetch-sandbox` helper (and optional AppArmor profile) against the bundled trust root, then installs a non-setuid copy of the helper and its AppArmor profile granting `userns` on Ubuntu 23.10 and later. Refuses with `plugin_attestation_invalid` and writes nothing when verification fails. Offered to a person only, never to an agent or a program (ADR 0099): any other surface is refused with `person_required`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PluginSetupParams>,
    result: schema_of::<PluginSetupResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[PLUGIN_SETUP],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
