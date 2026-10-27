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
    description: "Where the Daemon serves the Console, as `host:port`: `127.0.0.1` on port 7800 plus the major (7700 plus the minor during `0.y` previews), so each major's Console has its own port. Port 0 takes a free port, which the Daemon's runtime file and `daemon status` report. Set in user configuration or an Operator policy only. An Attached engine serves no Console unless the script that starts it asks for one at an address of its own, which `hello` answers with the Console's sign-in link.",
    kind: ValueKind::Text,
    default: default_of_this_version,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

fn none() -> serde_json::Value {
    serde_json::Value::Null
}

/// `console.content_address` (ADR 0031): where the Daemon serves the
/// Console's content origin, the second origin that shows scraped HTML,
/// screenshots, DOM snapshots and page bodies.
pub const CONTENT_ADDRESS: Setting = Setting {
    name: "console.content_address",
    description: "Where the Daemon serves the Console's content origin, which shows scraped HTML, screenshots, DOM snapshots and page bodies, as `host:port`. None by default: the Console's host on a free port, or, when `console.content_public_url` is set, the Console's own listener, which tells the two origins apart by the `Host` each request names. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `console.content_public_url` (ADR 0049): the content origin's address
/// behind a proxy.
pub const CONTENT_PUBLIC_URL: Setting = Setting {
    name: "console.content_public_url",
    description: "The origin a browser reaches the Console's content origin at behind a proxy, such as `https://content.example.com`, which the addresses the Console signs name. None by default: the content origin's own listener. With no `console.content_address`, the content origin shares the Console's listener and answers the requests that name this origin's host. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `console.public_url` (ADR 0031): the address links to the Console
/// name, such as an Exposed Daemon's TLS address behind a proxy.
pub const PUBLIC_URL: Setting = Setting {
    name: "console.public_url",
    description: "The address links to the Console name, such as `https://console.example.com`: the links in Notifications and a run's `console_url`. None by default: the Console's own address. A link never carries a token or a cookie, so a forwarded one grants nothing. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

fn on() -> serde_json::Value {
    serde_json::Value::Bool(true)
}

/// `console.offline_cache` (ADR 0031): whether the Console caches run
/// summaries in the browser, encrypted under its sign-in's key.
pub const OFFLINE_CACHE: Setting = Setting {
    name: "console.offline_cache",
    description: "Whether the Console keeps run summaries and their metadata in the browser, so that a page goes on showing them while it has no connection to the Daemon: `true`. The cache is encrypted under a key that only the sign-in's cookie unwraps, holds no secret field, and is wiped at sign-out; `false` writes nothing and wipes what an earlier page wrote. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Boolean,
    default: on,
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
    settings: &[
        ADDRESS,
        CONTENT_ADDRESS,
        CONTENT_PUBLIC_URL,
        OFFLINE_CACHE,
        PUBLIC_URL,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
