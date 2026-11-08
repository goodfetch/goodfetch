//! `proxy.bypass` read as the browser reads it (Chromium's proxy bypass
//! rules, which Playwright's `bypass` hands it): a URL the rules match goes
//! to its site directly, never through the Session's proxy, so the engine
//! neither counts it against the proxy's limits nor reads its answer as the
//! proxy's. A rule is a host pattern, `*` matching any run of characters
//! and a leading dot read as `*.`, with an optional `scheme://` before it
//! and `:port` after it; an IP range, `address/prefix`; or `<local>`, a
//! name without a dot. `<-loopback>`, which keeps loopback on the proxy,
//! matches nothing.

use std::net::IpAddr;

/// Whether a request to `url` skips the proxy under `bypass`, the
/// Session's `proxy.bypass`.
pub(crate) fn bypassed(bypass: Option<&str>, url: &str) -> bool {
    let (Some(bypass), Ok(url)) = (bypass, url::Url::parse(url)) else {
        return false;
    };
    let Some(host) = host_of(&url) else {
        return false;
    };
    rules(bypass).any(|rule| matches(rule, &url, &host))
}

/// The rules of `bypass`, the Session's `proxy.bypass`, each trimmed: the
/// list split on commas, as Playwright splits it, and on semicolons, as
/// Chromium splits the list it is then handed, so the browser and the
/// engine read the same rules.
pub(crate) fn rules(bypass: &str) -> impl Iterator<Item = &str> {
    bypass
        .split([',', ';'])
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
}

/// `url`'s host, lowercase, an IPv6 address without its brackets.
fn host_of(url: &url::Url) -> Option<String> {
    Some(match url.host()? {
        url::Host::Domain(name) => name.to_ascii_lowercase(),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    })
}

/// Whether the one rule `rule` matches `url`, whose host is `host`.
fn matches(rule: &str, url: &url::Url, host: &str) -> bool {
    match rule {
        "<-loopback>" => return false,
        "<local>" => return !host.contains(['.', ':']),
        _ => {}
    }
    if let Some((network, prefix)) = rule.split_once('/')
        && !rule.contains("://")
    {
        return in_range(network, prefix, host);
    }
    let (scheme, rest) = match rule.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, rule),
    };
    if scheme.is_some_and(|scheme| !scheme.eq_ignore_ascii_case(url.scheme())) {
        return false;
    }
    let (pattern, port) = split_port(rest);
    if let Some(port) = port
        && url.port_or_known_default() != Some(port)
    {
        return false;
    }
    let pattern = pattern.to_ascii_lowercase();
    let pattern = if pattern.starts_with('.') {
        format!("*{pattern}")
    } else {
        pattern
    };
    glob(&pattern, host)
}

/// A host pattern and the port after it, if it names one: `[::1]:8080`,
/// `host:8080`, or a pattern alone.
fn split_port(rest: &str) -> (&str, Option<u16>) {
    if let Some(inner) = rest.strip_prefix('[')
        && let Some((address, after)) = inner.split_once(']')
    {
        return (
            address,
            after.strip_prefix(':').and_then(|port| port.parse().ok()),
        );
    }
    match rest.rsplit_once(':') {
        Some((pattern, port)) if !pattern.contains(':') => match port.parse() {
            Ok(port) => (pattern, Some(port)),
            Err(_) => (rest, None),
        },
        _ => (rest, None),
    }
}

/// Whether `host`, an IP address, is in the range `network/prefix`.
fn in_range(network: &str, prefix: &str, host: &str) -> bool {
    let network = network.trim_start_matches('[').trim_end_matches(']');
    let (Ok(network), Ok(prefix), Ok(address)) = (
        network.parse::<IpAddr>(),
        prefix.parse::<u32>(),
        host.parse::<IpAddr>(),
    ) else {
        return false;
    };
    match (network, address) {
        (IpAddr::V4(network), IpAddr::V4(address)) if prefix <= 32 => {
            let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
            u32::from(network) & mask == u32::from(address) & mask
        }
        (IpAddr::V6(network), IpAddr::V6(address)) if prefix <= 128 => {
            let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
            u128::from(network) & mask == u128::from(address) & mask
        }
        _ => false,
    }
}

/// Whether `text` matches `pattern`, whose `*` matches any run of
/// characters, the empty one included.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests;
