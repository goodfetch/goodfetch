//! The Error codes of starting an executable Goodfetch did not build, such
//! as a browser, a third-party process Plugin or an interpreter from the
//! runtime cache, which the operating system may refuse (ADR 0039); and the
//! per-user list of installed executables in Goodfetch home, a format of the
//! Usage ledger's family, in which each Product's executable records itself
//! when it runs (ADR 0098).
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::browser_cache::ArtifactId;
use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::connection::ProductVersion;

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

/// The list of installed executables, `installed-executables.json` in
/// Goodfetch home (ADR 0098). Newer versions of the format only add, so an
/// engine keeps every entry and field it does not know when it rewrites the
/// list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledExecutables {
    /// The version of this format the list was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format_version: ProtocolVersion,
    /// One entry per executable, at one path and one Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub executables: Vec<InstalledExecutable>,
}

/// One executable that ran on this machine (ADR 0098).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledExecutable {
    /// What it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: ExecutableKind,
    /// The Product it is, or whose Rust crate it links.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// Its absolute path when it last ran.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// The size of its file in bytes then, which with `modified_at` tells
    /// whether the file at `path` is still this executable.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
    /// When its file was last modified then, in RFC 3339.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub modified_at: SystemTime,
    /// The browser builds and other cache downloads its version pins, its
    /// versioned copy included once a Daemon runs from it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pins: Vec<ArtifactId>,
    /// Its versioned copy in the Browser cache, from which a Daemon runs
    /// (ADR 0024), once one was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub copy: Option<ArtifactId>,
    /// When it last ran, in RFC 3339.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub last_seen: SystemTime,
}

/// What an installed executable is (ADR 0098).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutableKind {
    /// A Product's executable, which can run its major's Daemon.
    ProductExecutable,
    /// A program that links the engine through a Product's Rust crate: its
    /// builds are kept, but it never runs the Daemon.
    LinkedProgram,
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
