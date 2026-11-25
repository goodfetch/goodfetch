//! Running a process Plugin (ADR 0033): the plugin protocol's handshake and
//! the engine methods every Plugin answers, the Plugin manifest it is held
//! to, and the Error codes of a Plugin refused at its `hello`, of one that
//! crashed and of one the host cannot confine, as a container runtime that
//! runs as root cannot confine a Bridge Sink.
//!
//! The plugin protocol is newline-delimited JSON-RPC 2.0 over the Plugin's
//! stdio. Whoever starts a Plugin, the engine or the conformance harness,
//! sets [`COOKIE_VARIABLE`] to [`COOKIE`] in its environment, as go-plugin
//! does, so that a Plugin started by hand refuses to run (not a security
//! boundary), then sends [`HELLO`] as the first message, with the protocol
//! range it speaks, the Product, the run ID and the network level. The
//! Plugin answers with the version it speaks, its kinds and each kind's
//! capabilities, which its kind's conformance suite proves one at a time,
//! and the Hook stages it implements; [`check_hello`] holds the answer to
//! the approved manifest (ADR 0066). Every request the engine sends may
//! carry `_meta` with the Trace context (ADR 0132), which the schema bundle
//! declares.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::error_code::ErrorCode;
use crate::jsonrpc::RequestId;
use crate::run::RunId;

mod manifest;
mod names;

pub use manifest::{
    Download, FILE_NAME, Grant, LOADER_VARIABLES, Limits, MANIFEST_VERSION, MEDIA_TYPE,
    ManifestBrowser, ManifestError, ManifestModel, NetworkLevel, Permissions, PluginManifest,
    Runtime, Spend, StructuredOutputStrategy,
};
pub use names::{HookStageName, PluginKindName};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The variable of a process Plugin's environment that holds [`COOKIE`].
pub const COOKIE_VARIABLE: &str = "GOODFETCH_PLUGIN_COOKIE";

/// The value of [`COOKIE_VARIABLE`] whoever starts a Plugin sets: a fixed
/// value, which tells the Plugin it was started to speak the plugin
/// protocol, never a secret.
pub const COOKIE: &str = "goodfetch-plugin-protocol";

/// The plugin protocol's version, `major.minor` (ADR 0030).
pub const PLUGIN_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The plugin protocol versions this engine speaks.
pub const PLUGIN_PROTOCOL: PluginProtocolRange = PluginProtocolRange {
    min: PLUGIN_PROTOCOL_VERSION,
    max: PLUGIN_PROTOCOL_VERSION,
};

/// The method of the plugin protocol's first message.
pub const HELLO: &str = "hello";

/// The engine's method that stops a call it no longer awaits.
pub const CANCEL: &str = "$/cancel";

/// The engine's heartbeat, sent every [`PING_INTERVAL`].
pub const PING: &str = "$/ping";

/// How often the engine sends [`PING`] to a ready Plugin.
pub const PING_INTERVAL: Duration = Duration::from_secs(5);

/// How many pings in a row a Plugin leaves unanswered before the engine
/// restarts it.
pub const MISSED_PINGS: u32 = 3;

/// A range of plugin protocol versions, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginProtocolRange {
    /// The oldest version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub min: ProtocolVersion,
    /// The newest version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max: ProtocolVersion,
}

impl PluginProtocolRange {
    /// Whether it holds `version`.
    #[must_use]
    pub fn contains(self, version: ProtocolVersion) -> bool {
        self.min <= version && version <= self.max
    }

    /// The newest version both it and `other` hold, which a Plugin speaks;
    /// none when they share none.
    #[must_use]
    pub fn newest_shared(self, other: Self) -> Option<ProtocolVersion> {
        let newest = self.max.min(other.max);
        (newest >= self.min.max(other.min)).then_some(newest)
    }
}

impl fmt::Display for PluginProtocolRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} to {}", self.min, self.max)
    }
}

/// What the starter of a Plugin says in its `hello`. A field a newer
/// starter adds is read as absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginHelloParams {
    /// The plugin protocol versions the starter speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol: PluginProtocolRange,
    /// The Product whose executable runs the Plugin.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// The Run the instance serves.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The network level the instance is confined to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network_level: NetworkLevel,
}

/// What a Plugin answers to `hello`, which the engine holds to the approved
/// manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHelloResult {
    /// The plugin protocol version the Plugin speaks, the newest it shares
    /// with the starter.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The Plugin kinds it implements.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: Vec<DeclaredKind>,
    /// The Hook stages it implements, for a Hook.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook_stages: BTreeSet<HookStageName>,
    /// The network level it was built to run at.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network_level: NetworkLevel,
}

/// A Plugin kind a Plugin implements, and the capabilities of it that the
/// Plugin claims: its kind's suite proves each, and a capability not
/// declared stays unproved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredKind {
    /// The Plugin kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: PluginKindName,
    /// The capabilities it claims, as its kind's suite names them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub capabilities: Vec<String>,
}

/// What `$/cancel` names: the call the engine no longer awaits, whose late
/// result it refuses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginCancelParams {
    /// The call's id.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call_id: RequestId,
    /// Whether the call passed its deadline, rather than being stopped.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub deadline: bool,
}

/// What `$/ping` says: nothing but the Trace context it may carry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginPingParams {}

/// What a Plugin answers to `$/ping`: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginPingResult {}

/// The part of a `hello` result the approved manifest fixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestField {
    /// The Plugin kinds.
    Kinds,
    /// The Hook stages.
    HookStages,
    /// The plugin protocol range, which the version spoken lies outside.
    Protocol,
    /// The network level.
    NetworkLevel,
}

impl ManifestField {
    /// Its name in the manifest.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kinds => "kinds",
            Self::HookStages => "hook_stages",
            Self::Protocol => "protocol",
            Self::NetworkLevel => "network_level",
        }
    }
}

/// Why the engine refuses a Plugin's `hello` result.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HelloRefusal {
    /// The Plugin speaks a version the engine does not.
    #[error("the Plugin speaks plugin protocol {found}, and this engine speaks {engine}")]
    ProtocolUnsupported {
        /// The version the Plugin speaks.
        found: ProtocolVersion,
        /// The versions the engine speaks.
        engine: PluginProtocolRange,
    },
    /// The result claims what the approved manifest does not.
    #[error("the Plugin's `hello` differs from its approved manifest in `{}`", field.as_str())]
    ManifestMismatch {
        /// The first part that differs.
        field: ManifestField,
    },
}

impl HelloRefusal {
    /// Its Error code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::ProtocolUnsupported { .. } => ErrorCode::PLUGIN_PROTOCOL_UNSUPPORTED,
            Self::ManifestMismatch { .. } => ErrorCode::PLUGIN_MANIFEST_MISMATCH,
        }
    }
}

/// Holds a Plugin's `hello` result to the versions the engine speaks and to
/// its approved manifest: the version it speaks must be one of `engine`'s,
/// and its kinds, Hook stages, version and network level what `approved`
/// declares.
///
/// # Errors
///
/// [`HelloRefusal::ProtocolUnsupported`] when the version is none of
/// `engine`'s, and otherwise [`HelloRefusal::ManifestMismatch`] naming the
/// first part that differs, in the manifest's order.
pub fn check_hello(
    engine: PluginProtocolRange,
    approved: &PluginManifest,
    result: &PluginHelloResult,
) -> Result<(), HelloRefusal> {
    if !engine.contains(result.protocol_version) {
        return Err(HelloRefusal::ProtocolUnsupported {
            found: result.protocol_version,
            engine,
        });
    }
    let kinds: BTreeSet<PluginKindName> =
        result.kinds.iter().map(|declared| declared.kind).collect();
    let mismatch = if kinds != approved.kinds || kinds.len() != result.kinds.len() {
        Some(ManifestField::Kinds)
    } else if result.hook_stages != approved.hook_stages {
        Some(ManifestField::HookStages)
    } else if !approved.protocol.contains(result.protocol_version) {
        Some(ManifestField::Protocol)
    } else if result.network_level != approved.network_level {
        Some(ManifestField::NetworkLevel)
    } else {
        None
    };
    match mismatch {
        Some(field) => Err(HelloRefusal::ManifestMismatch { field }),
        None => Ok(()),
    }
}

/// A Plugin whose `hello` claims more than was approved (ADRs 0033 and
/// 0066).
pub const PLUGIN_MANIFEST_MISMATCH: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_manifest_mismatch",
    description: "The Plugin's answer to `hello` differs from its approved manifest in its kinds, its Hook stages, its plugin protocol range or its network level, so the engine stops the instance before any call. Approve the Plugin's current manifest again, or install the version that was approved.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin that speaks no plugin protocol version the engine speaks (ADR
/// 0066).
pub const PLUGIN_PROTOCOL_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_protocol_unsupported",
    description: "The Plugin and the engine share no plugin protocol version, which the reply to `hello` says before any other message. Upgrade the older side: the Plugin, or the engine.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A process Plugin that crashed while it held the call (ADR 0033).
pub const PLUGIN_CRASHED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_crashed",
    description: "The process Plugin crashed while it held the call, and restarts with backoff. A read or an idempotent write fails with it and is retried under its lease; an Effect with no check is never retried blindly, but reported uncertain for Reconciliation instead. Repeated crashes quarantine the Plugin.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin whose confinement the host cannot provide (ADR 0033).
pub const CONFINEMENT_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "confinement_unavailable",
    description: "The host cannot confine the Plugin as it must run: it has no seccomp at all (WSL1), or neither user namespaces nor Landlock ABI 6 (a container under Docker's default seccomp profile on a kernel before 6.12), or it cannot provide the network level the Plugin's manifest declares, which is never silently weakened. A third-party Plugin is always refused; a first-party one, shipped in the engine's own release, too, unless the Operator policy's `plugins.<name>.reduced_confinement` lets it run with the confinement the host offers. A Bridge Sink is refused with it under a container runtime that runs as root.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Error codes of running a process Plugin.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        PLUGIN_MANIFEST_MISMATCH,
        PLUGIN_PROTOCOL_UNSUPPORTED,
        PLUGIN_CRASHED,
        CONFINEMENT_UNAVAILABLE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
