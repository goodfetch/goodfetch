//! Installing a Plugin (ADR 0033; ticket #889): `plugin.add` pulls the
//! artifact for the host's platform from the Plugin's multi-platform OCI
//! image index, verifies its digest, unpacks it into the Plugin store and
//! pins it in the Plugin lockfile, executing nothing: a person's answer
//! (ADR 0099).

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::plugin::approvals::{StatedPermission, is_false};
use crate::plugin::index::SignerIdentity;
use crate::plugin::manifest::{ManifestBrowser, NetworkLevel, Spend};
use crate::plugin::names::PluginKindName;
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `plugin.add` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginAddParams {
    /// The Plugin's OCI reference,
    /// `<registry>/<repository>@sha256:<hex>` (where the repository's last
    /// segment is `goodfetch-plugin-<name>` and the digest is its image
    /// index's), or a Plugin name listed in the Plugin index (which resolves
    /// to its pinned digest; a look-alike or ambiguous name without an exact
    /// digest is refused with `plugin_name_ambiguous`). No tag names one,
    /// since a tag moves.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The project configuration the Client found, layer 3 (ADR 0083): the
    /// lockfile goes beside it, else in the Goodfetch home's configuration
    /// folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
    /// Whether the person approved the Plugin and each permission its
    /// manifest declares, as they were shown before the call (ADR 0033):
    /// the engine then allows it, pins it to its digest and approves those
    /// permissions in the Operator policy, or answers the entry to put in
    /// a policy managed elsewhere.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approve: bool,
}

/// Where a person's approval of a Plugin went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginApproval {
    /// Written into the Operator policy, and recorded in the Audit log.
    Written {
        /// The Operator policy file.
        #[schemars(extend("arrived_in" = "0.1"))]
        policy: String,
    },
    /// Not written: the Operator policy is managed elsewhere, by a device
    /// manager or a configuration tool, so the person adds this entry
    /// there.
    Managed {
        /// The policy's `plugins.<name>` entry, as TOML.
        #[schemars(extend("arrived_in" = "0.1"))]
        entry: String,
    },
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
    /// The Plugin kinds its manifest declares.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: BTreeSet<PluginKindName>,
    /// What its manifest lets it reach on the network.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network_level: NetworkLevel,
    /// The environment variables its manifest names, each by the setting
    /// that values it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub env: BTreeMap<String, String>,
    /// The paths outside its bundle its manifest declares it may read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub files: Vec<String>,
    /// The executables its manifest declares it may start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub processes: Vec<String>,
    /// The browsers its manifest adds, each with its downloads per
    /// platform.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browsers: Vec<ManifestBrowser>,
    /// The spend units its manifest declares outside the engine, each with
    /// its maximum per call and optional price (ADR 0033).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub spend: BTreeMap<String, Spend>,
    /// Each permission its manifest declares, with what it lets the Plugin
    /// do and what the engine cannot check.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub permissions: Vec<StatedPermission>,
    /// The verified signer identity recorded from the artifact's provenance
    /// attestation, when verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: Option<SignerIdentity>,
    /// Where the person's approval went, when they gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approval: Option<PluginApproval>,
}

/// `plugin.add`.
pub const PLUGIN_ADD: Operation = Operation {
    name: "plugin.add",
    description: "Installs a Plugin from its OCI registry, a person's answer: it pulls the multi-platform image index, then the artifact for the host's platform, verifies each digest, unpacks the bundle into the Plugin store and pins the index's digest in the Plugin lockfile, `goodfetch-plugins.lock`, beside the project's configuration or in the Goodfetch home's configuration folder. It executes nothing it pulled. Adding a Plugin the lockfile pins to the same digest and the store holds answers it again without a pull. The answer states the manifest's kinds, network level, environment variables, files, processes, browsers and each permission it declares, with what it lets the Plugin do and what the engine cannot check. With `approve`, given once the person saw those, the engine allows the Plugin, pins it to the index's digest and approves each of those permissions in the Operator policy's `plugins.<name>` entry, recorded in the Audit log as `plugin_approved`; an Operator policy managed elsewhere is left as it is and the answer carries the entry to add there. A bundle with a path that leaves it, a link whose target leaves it or a device file is refused whole with `plugin_artifact_refused`; a name the lockfile pins to another digest with `plugin_digest_mismatch`; a Plugin the store holds that the lockfile does not pin with `plugin_installed`; a registry that does not serve it with `artifact_download_failed`, and a pull `egress.offline` forbids with `artifact_unavailable_offline`. It is offered to a person only, never to an agent or a program (ADR 0099): any other surface is refused with `person_required`.",
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

/// Why a Plugin artifact's provenance attestation was refused (ADR 0033,
/// issue 916).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttestationFailureReason {
    /// No provenance attestation was attached to the artifact.
    Missing,
    /// The attestation's signature did not verify under the trust root.
    BadSignature,
    /// The attestation's signer identity did not match the signer recorded in
    /// the Plugin index (or the Plugin has no index-recorded signer).
    SignerMismatch,
    /// The attestation's subject digest did not match the artifact's image
    /// index digest.
    DigestMismatch,
    /// The attestation document could not be parsed.
    Malformed,
}

/// A Plugin artifact whose provenance attestation is missing or invalid
/// (ADR 0033, issue 916).
pub const PLUGIN_ATTESTATION_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_attestation_invalid",
    description: "A third-party Plugin's OCI artifact carried no valid provenance attestation matching the signer identity recorded in the Plugin index, so nothing was installed. Allow its exact digest in Operator policy (`plugins.<name>.allow = true` and `plugins.<name>.digest`) only if you trust that exact artifact without an index-matched attestation.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_attestation_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginAttestationInvalidDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The artifact's image index digest, `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// Why verification failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: AttestationFailureReason,
    /// The signer identity recorded in the Plugin index, when listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expected_signer: Option<SignerIdentity>,
    /// The signer identity carried by the attestation, when readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found_signer: Option<SignerIdentity>,
    /// What resolves the refusal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A Plugin name that is ambiguous or a look-alike of an index entry without
/// an exact digest (ADR 0033, issue 916).
pub const PLUGIN_NAME_AMBIGUOUS: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_name_ambiguous",
    description: "The Plugin name is ambiguous or closely resembles an entry in the Plugin index (typosquatting protection) and was given without an exact `@sha256:<hex>` digest, so nothing was installed. Use the exact index entry name or pass the full OCI reference pinned by its `@sha256:<hex>` digest.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_name_ambiguous` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginNameAmbiguousDetails {
    /// The Plugin name that was requested.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The index entry name(s) that match or closely resemble `name`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub similar: Vec<String>,
    /// What resolves the refusal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

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
        PLUGIN_ATTESTATION_INVALID,
        PLUGIN_NAME_AMBIGUOUS,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
