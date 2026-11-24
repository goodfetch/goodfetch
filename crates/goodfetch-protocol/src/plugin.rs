//! Running a process Plugin (ADR 0033): the plugin protocol's handshake,
//! and the Error codes of one that crashed and of one the host cannot
//! confine, as a container runtime that runs as root cannot confine a
//! Bridge Sink.
//!
//! The plugin protocol is newline-delimited JSON-RPC 2.0 over the Plugin's
//! stdio. Whoever starts a Plugin, the engine or the conformance harness,
//! sets [`COOKIE_VARIABLE`] to [`COOKIE`] in its environment, as go-plugin
//! does, so that a Plugin started by hand refuses to run (not a security
//! boundary), then sends [`HELLO`] as the first message; the Plugin answers
//! with the kinds it implements and each kind's capabilities, which its
//! kind's conformance suite proves one at a time. The approved manifest the
//! answer must match, the protocol ranges and the schema bundle come with
//! the handshake's own ticket (#810).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The variable of a process Plugin's environment that holds [`COOKIE`].
pub const COOKIE_VARIABLE: &str = "GOODFETCH_PLUGIN_COOKIE";

/// The value of [`COOKIE_VARIABLE`] whoever starts a Plugin sets: a fixed
/// value, which tells the Plugin it was started to speak the plugin
/// protocol, never a secret.
pub const COOKIE: &str = "goodfetch-plugin-protocol";

/// The plugin protocol's version, `major.minor` (ADR 0030).
pub const PLUGIN_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The method of the plugin protocol's first message.
pub const HELLO: &str = "hello";

/// What the starter of a Plugin says in its `hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginHelloParams {
    /// The plugin protocol version the starter speaks.
    pub protocol_version: ProtocolVersion,
    /// The Product whose executable runs the Plugin.
    pub product: Product,
}

/// What a Plugin answers to `hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginHelloResult {
    /// The plugin protocol version the Plugin speaks.
    pub protocol_version: ProtocolVersion,
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The Plugin kinds it implements.
    pub kinds: Vec<DeclaredKind>,
}

/// A Plugin kind a Plugin implements, and the capabilities of it that the
/// Plugin claims: its kind's suite proves each, and a capability not
/// declared stays unproved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredKind {
    /// The Plugin kind's `snake_case` name, such as `sample_store`.
    pub kind: String,
    /// The capabilities it claims, as its kind's suite names them.
    pub capabilities: Vec<String>,
}

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
    error_codes: &[PLUGIN_CRASHED, CONFINEMENT_UNAVAILABLE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
