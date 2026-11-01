//! Versions and upgrades (ADRs 0030 and 0066): what `upgrade-files` prints
//! and its refusal, and the setting that turns the age notice off.
//!
//! `upgrade-files` gives the executable's major its own copy of an older
//! major's per-user and per-install files, never changing the older
//! major's, from a major `compat/upgrade-paths.toml` lists. An executable
//! more than three months past its release date prints one line a day on
//! stderr saying so, unless `version.age_notice` is `false`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `upgrade-files` refused its source (ADR 0066).
pub const UPGRADE_SOURCE_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "upgrade_source_unsupported",
    description: "`upgrade-files` does not upgrade from this major: one this release's upgrade paths do not list, whose message names the release to upgrade through first, or a newer major than the executable's, whose message says to upgrade. No file is changed.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `upgrade-files --json` prints (spec #608): the files of the new
/// major's copy, by their path in Goodfetch home; every list empty when
/// Goodfetch home holds no older major's files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesUpgraded {
    /// The files it gave the new major from the older major's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub upgraded: Vec<String>,
    /// The files the new major already held as the older major has them,
    /// as after an earlier run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unchanged: Vec<String>,
    /// The files the new major already held otherwise, which it refused
    /// to overwrite and kept.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub refused: Vec<String>,
}

fn yes() -> serde_json::Value {
    serde_json::Value::Bool(true)
}

/// `version.age_notice` (ADR 0030).
pub const VERSION_AGE_NOTICE: Setting = Setting {
    name: "version.age_notice",
    description: "Whether an executable more than three months past its release date prints one line a day on stderr saying so, from its own release date alone, with no network call. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Boolean,
    default: yes,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[UPGRADE_SOURCE_UNSUPPORTED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[VERSION_AGE_NOTICE],
    hook_stages: &[],
    shared_type_values: &[],
};
