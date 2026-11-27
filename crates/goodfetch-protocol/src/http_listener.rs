//! The Daemon's HTTP JSON-RPC listener contracts (ADRs 0027, 0030, 0049 and
//! 0099; CL-03): the three machine-only settings that switch it on and choose
//! its address and per-major port, the route and header names every HTTP
//! client sends, and the default port helper. Both Products' Daemons carry
//! the listener unchanged, so everything here is Collie's.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::catalog::{Authority, Entries, Product, ProtocolVersion, Setting, ValueKind};
use crate::connection::{Form, ProductVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The route that accepts one JSON-RPC request per HTTP request.
pub const RPC_PATH: &str = "/v1/rpc";

/// The route that streams a Run's (or engine's) events as Server-Sent Events
/// (`GET /v1/events?run=<id>&after=<position>`, ADRs 0027 and 0024).
pub const EVENTS_PATH: &str = "/v1/events";

/// The HTTP header carrying the Client's protocol version (`major.minor`),
/// matching `hello`'s `protocol_version` (ADRs 0027 and 0030).
pub const HEADER_PROTOCOL: &str = "goodfetch-protocol";

/// The HTTP header carrying the Product the Client needs (`collie` or
/// `foxhound`), matching `hello`'s `needs` (ADRs 0027 and 0095).
pub const HEADER_PRODUCT: &str = "goodfetch-product";

/// The HTTP header carrying the form the Client writes names in (`catalog` by
/// default), matching `hello`'s `form` (ADR 0099).
pub const HEADER_FORM: &str = "goodfetch-form";

/// The HTTP listener's ports: a release major's is this plus the major,
/// apart from `daemon.address`'s 7900 block (ADR 0030).
const RELEASE_PORT_BASE: u16 = 7800;

/// A `0.y` preview's is this plus the minor: each preview minor counts as a
/// major (ADR 0030), apart from `daemon.address`'s 7600 block.
const PREVIEW_PORT_BASE: u16 = 7700;

/// The default port of the HTTP listener of `version`'s major: `7800` plus
/// the major, or `7700` plus the minor during `0.y` previews, so the HTTP
/// listeners of two majors never share a port.
#[must_use]
pub fn default_port(version: &ProductVersion) -> u16 {
    version
        .loopback_of_its_major(RELEASE_PORT_BASE, PREVIEW_PORT_BASE)
        .port()
}

/// Where the HTTP listener of `version`'s major listens by default:
/// `127.0.0.1` on [`default_port`].
#[must_use]
pub fn default_address(version: &ProductVersion) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, default_port(version)))
}

fn off() -> serde_json::Value {
    serde_json::Value::Bool(false)
}

fn loopback() -> serde_json::Value {
    serde_json::Value::String(IpAddr::V4(Ipv4Addr::LOCALHOST).to_string())
}

fn port_of_this_version() -> serde_json::Value {
    let port = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .map_or(RELEASE_PORT_BASE, |version| default_port(&version));
    serde_json::json!(port)
}

/// `http_listener.enabled` (ADR 0027): whether the Daemon serves its HTTP
/// JSON-RPC listener.
pub const ENABLED: Setting = Setting {
    name: "http_listener.enabled",
    description: "Whether the Daemon opens its HTTP JSON-RPC listener (`POST /v1/rpc`, ADR 0027). Off (`false`) by default so workflow tools reach a Daemon only when enabled. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `http_listener.address` (ADRs 0027 and 0049): the IP address the Daemon's
/// HTTP listener binds to.
pub const ADDRESS: Setting = Setting {
    name: "http_listener.address",
    description: "The IP address the Daemon's HTTP listener binds to when `http_listener.enabled` is on: `127.0.0.1` by default. Any address other than loopback requires TLS through the Exposed Daemon's keys (`daemon.tls.cert_file` and `daemon.tls.key_file`, or `daemon.tls.upstream`), so no token crosses a network in clear. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: loopback,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `http_listener.port` (ADRs 0027 and 0030): the port the Daemon's HTTP
/// listener binds to.
pub const PORT: Setting = Setting {
    name: "http_listener.port",
    description: "The port the Daemon's HTTP listener binds to when `http_listener.enabled` is on: 7800 plus the major (7700 plus the minor during `0.y` previews), so each major's listener has a port of its own. Port 0 takes a free port, which the Daemon's log and `daemon.status` report. Set in user configuration or an Operator policy only.",
    kind: ValueKind::CountUpTo(65_535),
    default: port_of_this_version,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Parses a `Goodfetch-Product` header value into a [`Product`].
#[must_use]
pub fn parse_product(value: &str) -> Option<Product> {
    Product::ALL
        .into_iter()
        .find(|product| product.as_str() == value)
}

/// Parses a `Goodfetch-Form` header value into a [`Form`].
#[must_use]
pub fn parse_form(value: &str) -> Option<Form> {
    Form::ALL.into_iter().find(|form| form.as_str() == value)
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[ENABLED, ADDRESS, PORT],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
