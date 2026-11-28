//! Installing the platform confinement helper (`goodfetch-sandbox` + AppArmor
//! profile on Linux, or `goodfetch-filter-broker` Windows service on Windows;
//! ADRs 0033, 0039 and 0099; issues 884 and 879): `plugin.setup` verifies the
//! Sigstore provenance attestation of the helper/broker (and optional Linux
//! AppArmor profile) against the bundled trust root before installing it.
//! It is person-only (ADR 0099): any non-interactive or programmatic caller is
//! refused with `person_required`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::plugin::index::SignerIdentity;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Which platform confinement helper `plugin.setup` verifies and installs
/// (ADRs 0033 and 0039; issues 884 and 879).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginSetupComponent {
    /// Root-owned, non-setuid Linux `goodfetch-sandbox` helper and its
    /// AppArmor profile granting `userns` on Ubuntu 23.10 and later.
    #[default]
    SandboxHelper,
    /// Elevated Windows `goodfetch-filter-broker` service managing persistent
    /// Windows Filtering Platform filters for `proxy_tcp` and `direct`.
    FilterBroker,
}

/// What `plugin.setup` takes (ADRs 0033 and 0039; issues 884 and 879).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginSetupParams {
    /// Which confinement helper to verify and install; defaults to
    /// `filter_broker` on Windows and `sandbox_helper` on Linux.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub component: Option<PluginSetupComponent>,
    /// Path to the `goodfetch-sandbox` helper or `goodfetch-filter-broker.exe`
    /// executable to verify and install.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub helper: String,
    /// Path to the Sigstore provenance attestation JSON for `helper`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attestation: String,
    /// Optional path to a Linux AppArmor profile file to verify and install.
    /// When omitted on Linux, `plugin.setup` writes the standard AppArmor
    /// profile granting `userns` exclusively to the installed helper path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile: Option<String>,
    /// Path to the Sigstore provenance attestation JSON for `profile`;
    /// required whenever `profile` is provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_attestation: Option<String>,
}

/// What `plugin.setup` answers once the helper or Windows filter broker is
/// verified and installed (ADRs 0033 and 0039; issues 884 and 879).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSetupResult {
    /// Which component was verified and installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub component: PluginSetupComponent,
    /// Where the verified `goodfetch-sandbox` helper or
    /// `goodfetch-filter-broker.exe` binary was installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub helper_path: String,
    /// The SHA-256 digest (`sha256:<hex>`) of the verified binary bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub helper_digest: String,
    /// Where the Linux AppArmor profile was installed (empty for Windows
    /// `filter_broker`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_path: String,
    /// Whether `apparmor_parser --replace` loaded the Linux profile into the
    /// kernel.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_loaded: bool,
    /// Windows Service Control Manager name registered for `filter_broker`, if
    /// applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub service_name: Option<String>,
    /// Whether the Windows filter broker service was registered and started.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub service_installed: bool,
    /// The verified signer identity carried by the attestation.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: SignerIdentity,
}

/// `plugin.setup` (ADRs 0033, 0039 and 0099; issues 884 and 879).
pub const PLUGIN_SETUP: Operation = Operation {
    name: "plugin.setup",
    description: "Verifies the Sigstore provenance attestation of the platform confinement helper (`goodfetch-sandbox` and optional AppArmor profile on Linux, or `goodfetch-filter-broker` Windows service on Windows) against the bundled trust root, then installs it. Refuses with `plugin_attestation_invalid` and writes nothing when verification fails. Offered to a person only, never to an agent or a program (ADR 0099): any other surface is refused with `person_required`.",
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
