//! A crawl request's domain key (ADR 0004): the registrable domain of its
//! URL's host under the full Public Suffix List, private entries included,
//! so subdomains of one site share one queue and two sites under a shared
//! host such as `github.io` do not; an address is its own key. A Recipe
//! overrides the key through `crawl.domain_keys`: a host at or under one of
//! its names takes that name's key, the longest name winning.

use std::collections::BTreeMap;

use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::CRAWL_DOMAIN_KEYS;

/// How a crawl keys its requests' domains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainKeys {
    overrides: BTreeMap<String, String>,
}

impl DomainKeys {
    /// Keys under the Public Suffix List, with `overrides`: each host name,
    /// and the hosts under it, mapped to the key they share.
    pub fn new(overrides: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            overrides: overrides
                .into_iter()
                .map(|(host, key)| (normalized(&host), key))
                .collect(),
        }
    }

    /// Keys under the Public Suffix List, with the overrides
    /// `crawl.domain_keys` sets in `settings`.
    pub fn of(settings: &Resolved) -> Self {
        let overrides = settings
            .value(&CRAWL_DOMAIN_KEYS)
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(host, key)| Some((host.clone(), key.as_str()?.to_owned())));
        Self::new(overrides)
    }

    /// The domain key of a request to `url`.
    pub fn key(&self, url: &str) -> String {
        let host = normalized(host_of(url));
        let overridden = self
            .overrides
            .iter()
            .filter(|(name, _)| {
                host == **name
                    || host
                        .strip_suffix(name.as_str())
                        .is_some_and(|rest| rest.ends_with('.'))
            })
            .max_by_key(|(name, _)| name.len());
        if let Some((_, key)) = overridden {
            return key.clone();
        }
        if host.parse::<std::net::IpAddr>().is_ok() {
            return host;
        }
        match psl::domain(host.as_bytes()) {
            Some(domain) => String::from_utf8_lossy(domain.as_bytes()).into_owned(),
            None => host,
        }
    }
}

/// The host of `url`: what follows its scheme, without credentials, port,
/// path, query or fragment, and an IPv6 address without its brackets.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(bracketed) = host_and_port.strip_prefix('[') {
        return bracketed.split(']').next().unwrap_or_default();
    }
    host_and_port.split(':').next().unwrap_or_default()
}

/// A host name as keys compare it: lowercase, with no trailing dot.
fn normalized(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests;
