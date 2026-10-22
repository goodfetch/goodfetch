//! The egress policy's contracts (ADRs 0014, 0017, 0043, 0100 and 0120):
//! the domain policy's two settings, `egress.allowed_domains` and
//! `egress.blocked_domains`, `egress.offline`, and `egress_refused`, the
//! Error code a request the policy refuses ends the call that waited for it
//! with, whose details name the URL, the rule, the address and the setting
//! that would allow it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What an entry of either domain list allows or blocks when written alone:
/// every host.
pub const EVERY_HOST: &str = "*";

fn every_host() -> Value {
    json!([EVERY_HOST])
}

fn empty_list() -> Value {
    json!([])
}

fn no() -> Value {
    Value::Bool(false)
}

/// `egress.allowed_domains` (ADRs 0014 and 0100).
pub const EGRESS_ALLOWED_DOMAINS: Setting = Setting {
    name: "egress.allowed_domains",
    description: "The hosts a run's requests may reach: each entry its host and their subdomains, matched on the host as a browser's URL parser leaves it (punycode, dotted-decimal IPv4), and `*` every host, only when written. `[\"*\"]` by default; set for a connection or a Run. A request to a host it does not list fails with `egress_refused`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: every_host,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `egress.blocked_domains` (ADRs 0014 and 0100).
pub const EGRESS_BLOCKED_DOMAINS: Setting = Setting {
    name: "egress.blocked_domains",
    description: "The hosts no request of a run may reach, whatever `egress.allowed_domains` allows: each entry its host and their subdomains, matched as `egress.allowed_domains` matches, and `*` every host. Empty by default; set for a connection or a Run. A request to a host it lists fails with `egress_refused`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `egress.offline` (ADR 0043's `offline`).
pub const EGRESS_OFFLINE: Setting = Setting {
    name: "egress.offline",
    description: "Whether the engine opens no connection nobody configured: every pinned download, such as a Pinned artifact missing from the Browser cache, fails with `artifact_unavailable_offline` before it connects, naming the command that supplies it beforehand. `false` by default; set at every layer, and an Operator policy can lock it.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// A request the egress policy refused.
pub const EGRESS_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "egress_refused",
    description: "The run's egress policy refused a request before it connected: a host the domain policy does not allow, an address that is not globally reachable, or a scheme no request may use. The details name the URL, the rule, the address when there is one and the setting that would allow it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The rule of the egress policy that refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EgressRule {
    /// `egress.allowed_domains` does not list the host, or
    /// `egress.blocked_domains` does, or the request could not be judged.
    DomainPolicy,
    /// The address the connection would use is not globally reachable.
    PrivateAddress,
    /// The URL's scheme is one no request may use.
    Scheme,
}

impl EgressRule {
    /// Its name as the wire writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DomainPolicy => "domain_policy",
            Self::PrivateAddress => "private_address",
            Self::Scheme => "scheme",
        }
    }
}

/// What `egress_refused` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EgressRefusedDetails {
    /// The URL of the request refused, as the run gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The rule that refused it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rule: EgressRule,
    /// The address the connection would have used, when the rule judged
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: Option<String>,
    /// The setting whose value would allow the request; none when no
    /// setting would, as for a URL that does not parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub setting: Option<String>,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[EGRESS_REFUSED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        EGRESS_ALLOWED_DOMAINS,
        EGRESS_BLOCKED_DOMAINS,
        EGRESS_OFFLINE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
