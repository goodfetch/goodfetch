//! Settings and shared type values for the Model Context Protocol server
//! (ADRs 0070, 0090, 0095, 0099; spec #608, issue 1363).

use crate::catalog::{
    Authority, Entries, ProtocolVersion, Setting, SharedType, SharedTypeValue, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The five Collie MCP profiles, carried identically by Collie and Foxhound
/// (ADRs 0090, 0095; issue 1363).
pub const PROFILES: &[&str] = &["browser", "agent", "run", "code", "full"];

fn browser() -> serde_json::Value {
    serde_json::Value::String("browser".to_owned())
}

/// `mcp.profile` (ADRs 0070, 0090, 0095; issue 1363).
pub const PROFILE: Setting = Setting {
    name: "mcp.profile",
    description: "Which tool profile `mcp` serves when `--profile` is not passed: `browser` (the default), `agent`, `run`, `code` or `full`. A Collie profile offers the same tools in both Collie and Foxhound executables.",
    kind: ValueKind::Choice(PROFILES),
    default: browser,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The shared type values of `SharedType::McpProfile` for Collie's five
/// profiles (ADR 0095, issue 1363).
pub const PROFILE_VALUES: &[SharedTypeValue] = &[
    SharedTypeValue {
        shared_type: SharedType::McpProfile,
        name: "browser",
        description: "Collie's default MCP profile: Run lifecycle, Session and page opening/closing, navigation and reading, locator actions, and Saved identity listing.",
        arrived_in: ARRIVED_IN,
    },
    SharedTypeValue {
        shared_type: SharedType::McpProfile,
        name: "agent",
        description: "The `browser` MCP profile plus `page_agent`.",
        arrived_in: ARRIVED_IN,
    },
    SharedTypeValue {
        shared_type: SharedType::McpProfile,
        name: "run",
        description: "Runs, Run reports, `recipe_start`, `catalog_run_start`, `schedule_*`, `usage_*`, and Run index reads, with no page tools.",
        arrived_in: ARRIVED_IN,
    },
    SharedTypeValue {
        shared_type: SharedType::McpProfile,
        name: "code",
        description: "Code mode (`code_mode_run`) and product documentation resources, with no per-operation tools.",
        arrived_in: ARRIVED_IN,
    },
    SharedTypeValue {
        shared_type: SharedType::McpProfile,
        name: "full",
        description: "Every agent-surface operation of the Product, excluding person-only and granting operations.",
        arrived_in: ARRIVED_IN,
    },
];

/// The MCP server's catalog entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[PROFILE],
    hook_stages: &[],
    shared_type_values: PROFILE_VALUES,
};
