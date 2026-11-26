//! Installing a Plugin (ADR 0033; ticket #889): `plugin.add` pulls the
//! artifact for the host's platform from the Plugin's multi-platform OCI
//! image index, verifies its digest, unpacks it into the Plugin store and
//! pins it in the Plugin lockfile, executing nothing. These are the codes
//! it refuses with.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

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

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        PLUGIN_DIGEST_MISMATCH,
        PLUGIN_ARTIFACT_REFUSED,
        PLUGIN_LOCKFILE_INVALID,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
