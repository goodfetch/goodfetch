//! The MCP specification revisions this server supports (ADR 0070, spec #608):
//! `2025-03-26`, `2025-06-18`, `2025-11-25` and `2026-07-28`.

use rmcp::model::ProtocolVersion;

/// The four MCP revisions supported by Goodfetch's MCP server, oldest first.
pub const SUPPORTED: &[ProtocolVersion] = &[
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2026_07_28,
];

/// The newest revision that still uses the `initialize` handshake.
pub const DEFAULT_HANDSHAKE_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;
