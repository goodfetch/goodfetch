//! The Console's contracts (ADRs 0030, 0031 and 0057): the address the
//! Daemon serves it on, one port per major so that two majors never share a
//! Console, the Error code of a port another program holds, and the state
//! of a Daemon's Console as its runtime file and `daemon status` report it.
//! One Console serves both Products (ADR 0098), so everything here is
//! Collie's.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::net::{Ipv4Addr, SocketAddr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};
use crate::connection::ProductVersion;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The port of a release major's Console is this plus the major.
const RELEASE_PORT_BASE: u16 = 7800;
/// The port of a `0.y` preview's Console is this plus the minor: each
/// preview minor counts as a major (ADR 0030).
const PREVIEW_PORT_BASE: u16 = 7700;

/// The Console's address on the Daemon of `version`'s major: `127.0.0.1`
/// on port 7800 plus the major, or 7700 plus the minor during `0.y`
/// previews, so each major's Console has its own port.
pub fn default_address(version: &ProductVersion) -> SocketAddr {
    let mut numbers = version.as_str().split(['.', '-']);
    let major = numbers.next().and_then(|number| number.parse::<u16>().ok());
    let minor = numbers.next().and_then(|number| number.parse::<u16>().ok());
    let port = match (major, minor) {
        (Some(0), Some(minor)) => PREVIEW_PORT_BASE.saturating_add(minor),
        (Some(major), _) => RELEASE_PORT_BASE.saturating_add(major),
        (None, _) => RELEASE_PORT_BASE,
    };
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

fn default_of_this_version() -> serde_json::Value {
    let address = ProductVersion::parse(env!("CARGO_PKG_VERSION")).map_or_else(
        |_| SocketAddr::from((Ipv4Addr::LOCALHOST, RELEASE_PORT_BASE)),
        |version| default_address(&version),
    );
    serde_json::Value::String(address.to_string())
}

/// `console.address` (ADR 0031): where the Daemon serves the Console.
pub const ADDRESS: Setting = Setting {
    name: "console.address",
    description: "Where the Daemon serves the Console, as `host:port`: `127.0.0.1` on port 7800 plus the major (7700 plus the minor during `0.y` previews), so each major's Console has its own port. Port 0 takes a free port, which the Daemon's runtime file and `daemon status` report. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: default_of_this_version,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// The Console's port, taken by another program.
pub const CONSOLE_PORT_IN_USE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "console_port_in_use",
    description: "The Console did not start: another program holds the port of `console.address`. The Daemon still serves its Clients. Stop the other program, or set `console.address` to a free port (port 0 takes any free one) and restart the Daemon.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `console_port_in_use` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsolePortInUseDetails {
    /// The address whose port is taken.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: String,
    /// What to do instead: the error's `fix` (ADR 0079), which names
    /// `console.address`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// Where a Daemon's Console is, as its runtime file and `daemon status`
/// report it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleState {
    /// The Console is served at this URL, such as `http://127.0.0.1:7801/`.
    Serving(#[schemars(extend("arrived_in" = "0.1"))] String),
    /// The Console did not start; the Daemon still serves its Clients.
    Failed(#[schemars(extend("arrived_in" = "0.1"))] ConsoleFailure),
    /// The engine carries no Console, as an SDK crate's linked engine.
    Absent,
}

/// Why a Daemon's Console did not start: an Error code, its message and
/// what to do instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleFailure {
    /// The Error code, such as `console_port_in_use`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// What happened.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `console --json` prints (ADR 0069): the Console's sign-in link,
/// whose fragment carries the token the page swaps once for its cookie
/// (ADR 0024), the URL the command opened the browser on, or would have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleOpened {
    /// The sign-in link, such as `http://127.0.0.1:7801/#token=…`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[CONSOLE_PORT_IN_USE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[ADDRESS],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
