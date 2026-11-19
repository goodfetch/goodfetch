//! The Error codes of running a process Plugin (ADR 0033): one that
//! crashed, and one the host cannot confine, as a container runtime that
//! runs as root cannot confine a Bridge Sink.

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

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
