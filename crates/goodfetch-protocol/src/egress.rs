//! The egress policy's contracts (ADRs 0014, 0017, 0043, 0100 and 0120):
//! the domain policy's two settings, `egress.allowed_domains` and
//! `egress.blocked_domains`, `egress.offline`, and `egress_refused`, the
//! Error code a request the policy refuses ends the call that waited for it
//! with, whose details name the URL, the rule, the address and the setting
//! that would allow it.
//!
//! The policy sorts connections into two [`EgressClass`]es (ADR 0017):
//! addresses a page can reach, and Operator-configured endpoints, each
//! [`ConfiguredKind`] classed by [`CONFIGURED_KINDS`], which
//! `egress.no_private_endpoints`, set in the Operator policy alone, holds
//! to globally reachable addresses.
//!
//! A `cdp_url` Session source's address is judged by ADR 0056's address
//! rules before any connection, and one they refuse raises
//! `cdp_url_not_allowed` (ADR 0115), naming the rule, the address and the
//! settings layer that gave it.
//!
//! [`private_ranges`] holds `egress.private_ranges`, the ranges and hosts
//! a page may reach although not globally reachable, and the Operator
//! policy's allowance for a Recipe to set it.
//!
//! [`local_addresses`] holds ADR 0044's: `egress.local_addresses`, the
//! rotation's block sizes and `local_address_unavailable`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, HostList, ProtocolVersion, Setting,
    ValueKind,
};
use crate::settings::SettingsLayer;

mod local_addresses;
mod private_ranges;

pub use local_addresses::{
    EGRESS_LOCAL_ADDRESSES, EGRESS_ROTATION_BLOCK_IPV4, EGRESS_ROTATION_BLOCK_IPV6,
    IPV4_BLOCK_SHAPE, IPV6_BLOCK_SHAPE, Ipv6Prefix, LOCAL_ADDRESS_UNAVAILABLE,
    LOCAL_ADDRESSES_SHAPE, LocalAddressFailure, LocalAddressUnavailableDetails, LocalAddresses,
};
pub use private_ranges::{
    EGRESS_PRIVATE_RANGES, EGRESS_RECIPE_PRIVATE_RANGES, PRIVATE_RANGES_SHAPE, PrivateRange,
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
    description: "The hosts a run's requests may reach: each entry its host and their subdomains, matched on the host as a browser's URL parser leaves it (punycode, dotted-decimal IPv4), and `*` every host, only when written. `[\"*\"]` by default; set for a connection or a Run, and only narrowed by a Recipe or a Client acting for a model, whose value listing a host the value below does not cover is `setting_locked`. A request to a host it does not list fails with `egress_refused`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: every_host,
    clearable: false,
    authority: Authority::HostsNarrowedByRecipeOrModel(HostList::Allows),
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `egress.blocked_domains` (ADRs 0014 and 0100).
pub const EGRESS_BLOCKED_DOMAINS: Setting = Setting {
    name: "egress.blocked_domains",
    description: "The hosts no request of a run may reach, whatever `egress.allowed_domains` allows: each entry its host and their subdomains, matched as `egress.allowed_domains` matches, and `*` every host. Empty by default; set for a connection or a Run, and only added to by a Recipe or a Client acting for a model, whose value that leaves a host the value below lists uncovered is `setting_locked`. A request to a host it lists fails with `egress_refused`.",
    kind: ValueKind::List(&ValueKind::Text),
    default: empty_list,
    clearable: false,
    authority: Authority::HostsNarrowedByRecipeOrModel(HostList::Blocks),
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

/// `egress.no_private_endpoints` (ADR 0017's "no private endpoints"
/// policy).
pub const EGRESS_NO_PRIVATE_ENDPOINTS: Setting = Setting {
    name: "egress.no_private_endpoints",
    description: "Whether Operator-configured endpoints (Model providers, Secret providers, proxies, Notification webhooks, OTLP collectors, the Plugin index, a configured `cdp_url`) are refused every address that is not globally reachable, as the addresses a page can reach are, so a tenant's endpoint never reaches the network the engine runs in. `false` by default, so an endpoint on localhost or a private network works; set only in an Operator policy, as Goodfetch Cloud sets it. A refused endpoint fails with `egress_refused` before it connects.",
    kind: ValueKind::Boolean,
    default: no,
    clearable: false,
    authority: Authority::OperatorPolicy,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// The two classes of outgoing connection (ADR 0017), which the egress
/// policy judges each by its own rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EgressClass {
    /// An address a page can reach: a start URL, a navigation, a redirect,
    /// a subresource, a worker, a WebSocket, a page's own request. The
    /// run's domain policy judges it, and no address that is not globally
    /// reachable is reached unless `egress.private_ranges` lists it.
    PageReachable,
    /// An Operator-configured endpoint: private ranges are reached unless
    /// `egress.no_private_endpoints` refuses them, and no run's domain
    /// policy or certificate exception applies.
    OperatorConfigured,
}

/// A kind of destination the operator configures (ADRs 0017 and 0043),
/// which the engine connects to on its own behalf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfiguredKind {
    /// A Model provider, such as a model on localhost.
    ModelProvider,
    /// An external Secret provider: a vault, a cloud's secret manager.
    SecretProvider,
    /// A Session's upstream proxy: the proxy's own address.
    Proxy,
    /// A Notification webhook.
    NotificationWebhook,
    /// An OTLP collector: `otlp.traces_endpoint`, `otlp.metrics_endpoint`.
    Otlp,
    /// The Plugin index.
    PluginIndex,
    /// A `cdp_url` to connect to a browser through.
    CdpUrl,
    /// A source of the Clean-up rules, which `clean_up.update`'s `daily`
    /// and `upstream` make a configured destination (ADR 0047).
    CleanUpRulesSource,
}

/// The layers whose value makes a destination Operator-configured: the
/// Operator policy and user and project configuration.
const CONFIGURATION: &[SettingsLayer] = &[
    SettingsLayer::OperatorPolicy,
    SettingsLayer::UserConfiguration,
    SettingsLayer::ProjectConfiguration,
];

/// One row of [`CONFIGURED_KINDS`]: a kind of configured destination and
/// how the egress policy classes it.
#[derive(Clone, Copy, Debug)]
pub struct ConfiguredKindClass {
    /// The kind.
    pub kind: ConfiguredKind,
    /// Its name in prose and in the engine's logs.
    pub name: &'static str,
    /// The layers whose value makes it an
    /// [`EgressClass::OperatorConfigured`] endpoint; from any other, it is
    /// an address a page can reach.
    pub operator_layers: &'static [SettingsLayer],
}

impl ConfiguredKindClass {
    /// The class of a destination of this kind whose value `layer` set.
    pub fn class(&self, layer: SettingsLayer) -> EgressClass {
        if self.operator_layers.contains(&layer) {
            EgressClass::OperatorConfigured
        } else {
            EgressClass::PageReachable
        }
    }
}

/// How each kind of configured destination is classed (ADRs 0017, 0043,
/// 0047 and 0056): an Operator-configured endpoint when the Operator policy
/// or user or project configuration sets it. Only a `cdp_url` can come
/// from a Recipe or a run, and is then an address a page can reach unless
/// the Operator policy allows it; so is one in the project configuration a
/// Client acting for a model sends (ADR 0056, read where `cdp_url` is
/// judged). Every caller of the engine's own HTTP client but its pinned
/// downloads and a run's own requests connects to one of these kinds.
pub const CONFIGURED_KINDS: [ConfiguredKindClass; 8] = [
    ConfiguredKindClass {
        kind: ConfiguredKind::ModelProvider,
        name: "Model provider",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::SecretProvider,
        name: "Secret provider",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::Proxy,
        name: "proxy",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::NotificationWebhook,
        name: "Notification webhook",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::Otlp,
        name: "OTLP",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::PluginIndex,
        name: "Plugin index",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::CdpUrl,
        name: "cdp_url",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::CleanUpRulesSource,
        name: "Clean-up rules source",
        operator_layers: CONFIGURATION,
    },
];

impl ConfiguredKind {
    /// Its row of [`CONFIGURED_KINDS`].
    pub fn classed(self) -> &'static ConfiguredKindClass {
        CONFIGURED_KINDS
            .iter()
            .find(|row| row.kind == self)
            .expect("invariant: every configured kind has its row")
    }
}

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

/// A `cdp_url` whose address ADR 0056's rules refuse (ADR 0115).
pub const CDP_URL_NOT_ALLOWED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "cdp_url_not_allowed",
    description: "A `cdp_url` Session source names an address ADR 0056's rules refuse, before any connection: a private address from a value that is not Operator-configured, loopback from a Client acting for a model (its project configuration included) or a Recipe, or a private address under `egress.no_private_endpoints`. The details name the address without its query or credentials, the settings layer that gave it, the rule, the address it resolved to and the fix: set the address in Operator policy or in user configuration, or give a public address.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The rule of ADR 0056's that refused a `cdp_url`'s address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdpUrlRule {
    /// Loopback, from a value that may not use it: a Client acting for a
    /// model's, its project configuration's included, or a Recipe's.
    Loopback,
    /// An address that is not globally reachable, from a value that is not
    /// Operator-configured, or under `egress.no_private_endpoints`.
    PrivateAddress,
}

/// What `cdp_url_not_allowed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpUrlNotAllowedDetails {
    /// The `cdp_url`, without its query string or any credentials in it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The settings layer that gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// The rule that refused it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rule: CdpUrlRule,
    /// The address it resolved to that the rule refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: String,
    /// The setting that refused it, or whose value would allow it, when one
    /// does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub setting: Option<String>,
    /// What would allow it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        EGRESS_REFUSED,
        CDP_URL_NOT_ALLOWED,
        LOCAL_ADDRESS_UNAVAILABLE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        EGRESS_ALLOWED_DOMAINS,
        EGRESS_BLOCKED_DOMAINS,
        EGRESS_OFFLINE,
        EGRESS_NO_PRIVATE_ENDPOINTS,
        EGRESS_PRIVATE_RANGES,
        EGRESS_RECIPE_PRIVATE_RANGES,
        EGRESS_LOCAL_ADDRESSES,
        EGRESS_ROTATION_BLOCK_IPV4,
        EGRESS_ROTATION_BLOCK_IPV6,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
