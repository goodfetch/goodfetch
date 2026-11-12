//! The limits on what the engine reads from outside (ADR 0067, SC-05,
//! pitfall H14): `network.max_body_size`, the decoded bytes of every body
//! the engine itself reads, never a download, which the disk governor
//! bounds (ADR 0122); `network.max_decompression_ratio`, the decoded bytes
//! per received byte once a decoded body passes 1 MiB; and
//! `network.max_json_depth`, the nesting of JSON the engine parses from
//! outside. Past any of them the call fails with `response_limit_exceeded`,
//! whose details name the setting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `network.max_body_size`'s default: 64 MiB.
pub const DEFAULT_MAX_BODY_SIZE: u64 = 64 * 1024 * 1024;

/// `network.max_decompression_ratio`'s default.
pub const DEFAULT_MAX_DECOMPRESSION_RATIO: u64 = 100;

/// `network.max_json_depth`'s default.
pub const DEFAULT_MAX_JSON_DEPTH: u64 = 128;

/// The deepest `network.max_json_depth` takes, so a deeper one is
/// `setting_invalid` rather than a limit the engine would not apply: past
/// it the engine's parser, which recurses, might not fit a thread's stack.
pub const MAX_JSON_DEPTH_CEILING: u64 = 1024;

fn sixty_four_mebibytes() -> Value {
    json!(DEFAULT_MAX_BODY_SIZE)
}

fn one_hundred() -> Value {
    json!(DEFAULT_MAX_DECOMPRESSION_RATIO)
}

fn one_hundred_twenty_eight() -> Value {
    json!(DEFAULT_MAX_JSON_DEPTH)
}

/// `network.max_body_size` (SC-05).
pub const NETWORK_MAX_BODY_SIZE: Setting = Setting {
    name: "network.max_body_size",
    description: "The most decoded bytes of one body the engine itself reads: a page `request`'s response, a kept response, an Operator-configured endpoint's reply. Never a download, which the disk governor bounds. 64 MiB (67108864) by default; set for a connection or a Run. A longer body fails the call with `response_limit_exceeded`.",
    kind: ValueKind::Count,
    default: sixty_four_mebibytes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `network.max_decompression_ratio` (SC-05).
pub const NETWORK_MAX_DECOMPRESSION_RATIO: Setting = Setting {
    name: "network.max_decompression_ratio",
    description: "The most decoded bytes per received byte of a body the engine reads, once its decoded bytes pass 1 MiB, so a decompression bomb is stopped as it expands. 100 by default; set for a connection or a Run. A body past it fails the call with `response_limit_exceeded`.",
    kind: ValueKind::Count,
    default: one_hundred,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `network.max_json_depth` (SC-05).
pub const NETWORK_MAX_JSON_DEPTH: Setting = Setting {
    name: "network.max_json_depth",
    description: "The deepest nesting of arrays and objects in JSON the engine parses from outside, from 0 to 1024: 128 by default; set for a connection or a Run. Deeper JSON fails the call with `response_limit_exceeded`.",
    kind: ValueKind::CountUpTo(MAX_JSON_DEPTH_CEILING),
    default: one_hundred_twenty_eight,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `response_limit_exceeded` (SC-05).
pub const RESPONSE_LIMIT_EXCEEDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "response_limit_exceeded",
    description: "A body or JSON the engine read from outside passed one of its limits, so the engine stopped reading it: more decoded bytes than `network.max_body_size`, more decoded bytes per received byte than `network.max_decompression_ratio` once past 1 MiB, or JSON nested deeper than `network.max_json_depth`. The details name the URL, the setting and its value; raise the setting for a site you trust.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The setting whose limit a body or JSON passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum LimitSetting {
    /// `network.max_body_size`.
    #[serde(rename = "network.max_body_size")]
    MaxBodySize,
    /// `network.max_decompression_ratio`.
    #[serde(rename = "network.max_decompression_ratio")]
    MaxDecompressionRatio,
    /// `network.max_json_depth`.
    #[serde(rename = "network.max_json_depth")]
    MaxJsonDepth,
}

impl LimitSetting {
    /// The setting's definition.
    pub fn setting(self) -> &'static Setting {
        match self {
            Self::MaxBodySize => &NETWORK_MAX_BODY_SIZE,
            Self::MaxDecompressionRatio => &NETWORK_MAX_DECOMPRESSION_RATIO,
            Self::MaxJsonDepth => &NETWORK_MAX_JSON_DEPTH,
        }
    }
}

/// What `response_limit_exceeded` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResponseLimitExceededDetails {
    /// The URL whose body passed the limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The setting whose limit it passed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub setting: LimitSetting,
    /// The setting's value the body passed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: u64,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[RESPONSE_LIMIT_EXCEEDED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        NETWORK_MAX_BODY_SIZE,
        NETWORK_MAX_DECOMPRESSION_RATIO,
        NETWORK_MAX_JSON_DEPTH,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
