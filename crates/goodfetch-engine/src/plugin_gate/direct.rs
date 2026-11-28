//! Pre-start ADR 0017 destination extraction and verification for `direct`
//! Plugins (ADR 0033, issue 855).

use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::plugin::approvals::Origin as PluginOrigin;
use serde_json::{Map, Value};
use url::Url;

use crate::call_error::CallError;
use crate::egress::{EgressClass, Gate, Policy, Refused, RequestKind};
use crate::http_client::{EndpointSettings, WholeFailure};

/// A destination extracted from a `direct` Plugin's validated settings for
/// pre-start ADR 0017 verification (ADR 0033, issue 855).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DirectDestination {
    /// A URL with a forbidden non-network scheme (`file:`, `data:`,
    /// `javascript:`, `gopher:`).
    ForbiddenScheme(String),
    /// A destination containing an IPv6 zone ID (`%`).
    ZoneId(String),
    /// A URL containing `://` that failed parsing.
    InvalidUrl(String),
    /// A host (and optional port, defaulting to 1 for resolution) to judge
    /// under ADR 0017.
    Host {
        named: String,
        host: String,
        port: u16,
    },
}

/// Checks every configured destination in `values` (skipping `secret_paths`)
/// under ADR 0017 before a `direct` Plugin starts.
pub(crate) async fn check_direct_destinations(
    settings_class: EgressClass,
    endpoint_settings: &EndpointSettings,
    page: &Policy,
    values: &Map<String, Value>,
    secret_paths: &[Vec<String>],
    origin: PluginOrigin,
    on_failure: impl Fn(&str, EgressClass, WholeFailure) -> CallError,
) -> Result<(), CallError> {
    let destinations = extract_direct_destinations(values, secret_paths);
    if destinations.is_empty() {
        return Ok(());
    }
    let class = if settings_class == EgressClass::OperatorConfigured
        || origin == PluginOrigin::FirstParty
    {
        EgressClass::OperatorConfigured
    } else {
        EgressClass::PageReachable
    };
    let gate = match class {
        EgressClass::OperatorConfigured => Gate::PluginEndpoint(endpoint_settings.policy()),
        EgressClass::PageReachable => Gate::Session {
            policy: page,
            kind: RequestKind::Fetch,
        },
    };
    for destination in destinations {
        match destination {
            DirectDestination::ForbiddenScheme(url) => {
                return Err(Refused {
                    url,
                    rule: EgressRule::Scheme,
                    address: None,
                    setting: None,
                }
                .error());
            }
            DirectDestination::ZoneId(url) => {
                return Err(Refused {
                    url,
                    rule: EgressRule::PrivateAddress,
                    address: None,
                    setting: None,
                }
                .error());
            }
            DirectDestination::InvalidUrl(url) => {
                let rule = if looks_like_zone_id(&url) {
                    EgressRule::PrivateAddress
                } else {
                    EgressRule::DomainPolicy
                };
                return Err(Refused {
                    url,
                    rule,
                    address: None,
                    setting: None,
                }
                .error());
            }
            DirectDestination::Host { named, host, port } => {
                if let Err(failure) = endpoint_settings
                    .check_host(gate, &named, &host, port)
                    .await
                {
                    return Err(on_failure(&named, class, failure));
                }
            }
        }
    }
    Ok(())
}

/// Extracts every configured destination from `values`, skipping fields whose
/// path is marked `"secret": true` in `secret_paths`.
pub(crate) fn extract_direct_destinations(
    values: &Map<String, Value>,
    secret_paths: &[Vec<String>],
) -> Vec<DirectDestination> {
    let mut destinations = Vec::new();
    let mut path = Vec::new();
    for (key, value) in values {
        path.push(key.clone());
        collect_destinations(key, value, &mut path, secret_paths, &mut destinations);
        path.pop();
    }
    destinations
}

fn collect_destinations(
    key: &str,
    value: &Value,
    path: &mut Vec<String>,
    secret_paths: &[Vec<String>],
    out: &mut Vec<DirectDestination>,
) {
    if secret_paths.iter().any(|secret| secret == path) {
        return;
    }
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            if trimmed.contains("://") {
                if let Ok(url) = Url::parse(trimmed) {
                    if matches!(url.scheme(), "file" | "data" | "javascript" | "gopher") {
                        out.push(DirectDestination::ForbiddenScheme(trimmed.to_owned()));
                    } else if let Some(host) = url.host_str() {
                        let stripped = host
                            .strip_prefix('[')
                            .and_then(|s| s.strip_suffix(']'))
                            .unwrap_or(host);
                        out.push(DirectDestination::Host {
                            named: trimmed.to_owned(),
                            host: stripped.to_owned(),
                            port: url.port_or_known_default().unwrap_or(1),
                        });
                    }
                } else {
                    out.push(DirectDestination::InvalidUrl(trimmed.to_owned()));
                }
            } else if is_destination_key(key) {
                for raw in trimmed.split(',') {
                    let item = raw.trim();
                    if item.is_empty() {
                        continue;
                    }
                    if looks_like_zone_id(item) {
                        out.push(DirectDestination::ZoneId(item.to_owned()));
                        continue;
                    }
                    let (host, port) = split_host_port(item);
                    out.push(DirectDestination::Host {
                        named: item.to_owned(),
                        host: host.to_owned(),
                        port,
                    });
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_destinations(key, item, path, secret_paths, out);
            }
        }
        Value::Object(fields) => {
            for (child_key, child_value) in fields {
                path.push(child_key.clone());
                collect_destinations(child_key, child_value, path, secret_paths, out);
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_destination_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "host"
            | "hosts"
            | "hostname"
            | "endpoint"
            | "endpoints"
            | "broker"
            | "brokers"
            | "bootstrap_servers"
            | "server"
            | "servers"
            | "address"
            | "addresses"
            | "destination"
            | "destinations"
            | "dsn"
            | "url"
            | "uri"
    )
}

fn looks_like_zone_id(text: &str) -> bool {
    let Some(open) = text.find('[') else {
        return false;
    };
    let rest = &text[open + 1..];
    let Some(close) = rest.find(']') else {
        return false;
    };
    rest[..close].contains('%')
}

fn split_host_port(item: &str) -> (&str, u16) {
    if let Some(rest) = item.strip_prefix('[')
        && let Some((addr, after)) = rest.split_once(']')
    {
        let port = after
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|&p| p != 0)
            .unwrap_or(1);
        return (addr, port);
    }
    if item.parse::<std::net::IpAddr>().is_ok() {
        return (item, 1);
    }
    if let Some((host, port_str)) = item.rsplit_once(':')
        && let Ok(port) = port_str.parse::<u16>()
        && port != 0
    {
        return (host, port);
    }
    (item, 1)
}
