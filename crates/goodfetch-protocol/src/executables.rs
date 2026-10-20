//! The Error codes of starting an executable Goodfetch did not build, such
//! as a browser, a third-party process Plugin or an interpreter from the
//! runtime cache, which the operating system may refuse (ADR 0039).
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Windows' refusal to start an executable Goodfetch did not build.
pub const EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "executable_blocked_by_smart_app_control",
    description: "Windows refused to start the file: Smart App Control, or another App Control policy, blocks executables without a signature or reputation it trusts. Goodfetch never re-signs a file it did not build. For a browser, use the Goodfetch Chromium or an installed Chrome by channel; otherwise turn Smart App Control off, or ask the device's administrator to allow the file.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `executable_blocked_by_smart_app_control` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutableBlockedBySmartAppControlDetails {
    /// The path of the file Windows refused to start.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub file: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
