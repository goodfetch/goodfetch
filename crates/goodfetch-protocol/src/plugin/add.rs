//! Installing a Plugin (ADR 0033; ticket #889): `plugin.add` pulls the
//! artifact for the host's platform from the Plugin's multi-platform OCI
//! image index, verifies its digest, unpacks it into the Plugin store and
//! pins it in the Plugin lockfile, executing nothing: a person's answer
//! (ADR 0099).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `plugin.add` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginAddParams {
    /// The Plugin's OCI reference,
    /// `<registry>/<repository>@sha256:<hex>`: the repository's last
    /// segment is `goodfetch-plugin-<name>`, and the digest is its image
    /// index's. No tag names one, since a tag moves.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The project configuration the Client found, layer 3 (ADR 0083): the
    /// lockfile goes beside it, else in the Goodfetch home's configuration
    /// folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// What `plugin.add` answers once the Plugin is installed and pinned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginAdded {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its version, as its manifest names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The digest of its multi-platform image index, `sha256:<hex>`, which
    /// the lockfile pins.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The lockfile that pins it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lockfile: String,
}

/// `plugin.add`.
pub const PLUGIN_ADD: Operation = Operation {
    name: "plugin.add",
    description: "Installs a Plugin from its OCI registry, a person's answer: it pulls the multi-platform image index, then the artifact for the host's platform, verifies each digest, unpacks the bundle into the Plugin store and pins the index's digest in the Plugin lockfile, `goodfetch-plugins.lock`, beside the project's configuration or in the Goodfetch home's configuration folder. It executes nothing it pulled. Adding a Plugin the lockfile pins to the same digest and the store holds answers it again without a pull. A bundle with a path that leaves it, a link whose target leaves it or a device file is refused whole with `plugin_artifact_refused`; a name the lockfile pins to another digest with `plugin_digest_mismatch`; a Plugin the store holds that the lockfile does not pin with `plugin_installed`; a registry that does not serve it with `artifact_download_failed`, and a pull `egress.offline` forbids with `artifact_unavailable_offline`. It is offered to a person only, never to an agent or a program (ADR 0099): any other surface is refused with `person_required`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PluginAddParams>,
    result: schema_of::<PluginAdded>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin whose index digest differs from the one its lockfile pins (ADR
/// 0033).
pub const PLUGIN_DIGEST_MISMATCH: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_digest_mismatch",
    description: "The Plugin lockfile pins the Plugin's name to another image index digest than the one its reference gives, so nothing is installed and the lockfile is left as it was. Give the reference with the pinned digest, or remove the Plugin's entry from the lockfile to accept the new one.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_digest_mismatch` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginDigestMismatchDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The digest the lockfile pins.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locked: String,
    /// The digest the reference gives.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found: String,
    /// The lockfile.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lockfile: String,
    /// What to do about it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A Plugin artifact the engine refuses to install (ADR 0033).
pub const PLUGIN_ARTIFACT_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_artifact_refused",
    description: "The Plugin's registry served an artifact the engine refuses to install: an image index or manifest it does not read, no artifact for the host's platform, or a bundle with an entry whose path leaves it, a link whose target leaves it, a device file, or more entries or bytes than a bundle may hold. The message names the reason; nothing is left in the Plugin store.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin lockfile the engine cannot read (ADR 0033).
pub const PLUGIN_LOCKFILE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_lockfile_invalid",
    description: "The Plugin lockfile, `goodfetch-plugins.lock`, is not TOML, holds a field a lockfile has not, or pins one Plugin name twice, so the engine leaves it as it is and installs nothing. The message names the fault; correct the file, or restore it from version control. A lockfile of a version this engine does not read fails with `format_version_unsupported` instead.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin the Plugin store holds that the lockfile does not pin (ADR
/// 0033).
pub const PLUGIN_INSTALLED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_installed",
    description: "The Plugin store holds a version of the Plugin already, and the Plugin lockfile does not pin its name to the reference's digest, so nothing is pulled and the lockfile is left as it was. `plugin update` replaces an installed Plugin.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[PLUGIN_ADD],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        PLUGIN_DIGEST_MISMATCH,
        PLUGIN_ARTIFACT_REFUSED,
        PLUGIN_LOCKFILE_INVALID,
        PLUGIN_INSTALLED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
