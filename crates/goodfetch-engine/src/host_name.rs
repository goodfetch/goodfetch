//! Host names as every module keys and matches them (ADRs 0014, 0042 and
//! 0059): one reading of a host, the WHATWG URL parser's, so `Bücher.Test.`
//! and `xn--bcher-kva.test` are one key wherever a setting lists a host,
//! the `resolve` mapping holds one, a certificate is pinned to one or a Run
//! records one; and one rule for which listed domain covers a host: the
//! host itself or a domain it is a subdomain of, an address only itself.

use std::collections::BTreeMap;
use std::net::IpAddr;

use url::Host;

#[cfg(test)]
mod tests;

/// `host` as the URL parser leaves a host: in lower case and punycode, an
/// IPv4 address in its dotted form, an IPv6 address without brackets (given
/// with or without them), and without the one trailing dot that makes a
/// name fully qualified; `None` when it names no host.
pub fn key(host: &str) -> Option<String> {
    let parsed = if host.contains(':') && !host.starts_with('[') {
        Host::parse(&format!("[{host}]")).ok()?
    } else {
        Host::parse(host).ok()?
    };
    let key = match parsed {
        Host::Domain(name) => without_root(&name).to_owned(),
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => address.to_string(),
    };
    (!key.is_empty()).then_some(key)
}

/// `name` as [`key`] keys it, or, when the URL parser reads no host in it,
/// in lower case without its trailing dot: the key a lookup of any text
/// takes, so it matches only a host written the same way.
pub fn lookup_key(name: &str) -> String {
    key(name).unwrap_or_else(|| without_root(name).to_ascii_lowercase())
}

/// A name without the one trailing dot that makes it fully qualified: the
/// same host either way.
pub fn without_root(name: &str) -> &str {
    name.strip_suffix('.').unwrap_or(name)
}

/// `key`, a host as [`key`] leaves it, and every domain it is a subdomain
/// of, the most specific first; an address is only itself.
pub fn covering(key: &str) -> impl Iterator<Item = &str> {
    let address = key.parse::<IpAddr>().is_ok();
    let mut next = Some(key);
    std::iter::from_fn(move || {
        let name = next?;
        next = if address {
            None
        } else {
            name.split_once('.').map(|(_, parent)| parent)
        };
        Some(name)
    })
}

/// A value for each listed host or domain, as `tls.trusted_cas`,
/// `tls.accept_invalid_certs` and `tls.client_certificates` list them:
/// each entry covers its host and that host's subdomains, keyed as [`key`]
/// keys a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedDomains<T>(BTreeMap<String, T>);

impl<T> Default for ListedDomains<T> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

/// A listed domain that names no host.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` names no host or domain")]
#[non_exhaustive]
pub struct NoHost(pub String);

impl<T> ListedDomains<T> {
    /// None listed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The value listed for `domain`, listed with `listed()` when it has
    /// none yet.
    ///
    /// # Errors
    ///
    /// [`NoHost`] when `domain` names no host.
    pub fn entry(&mut self, domain: &str, listed: impl FnOnce() -> T) -> Result<&mut T, NoHost> {
        let key = key(domain).ok_or_else(|| NoHost(domain.to_owned()))?;
        Ok(self.0.entry(key).or_insert_with(listed))
    }

    /// Lists `domain` with `value`, in place of any value it had.
    ///
    /// # Errors
    ///
    /// [`NoHost`] when `domain` names no host.
    pub fn insert(&mut self, domain: &str, value: T) -> Result<(), NoHost> {
        let key = key(domain).ok_or_else(|| NoHost(domain.to_owned()))?;
        self.0.insert(key, value);
        Ok(())
    }

    /// The listed domains that cover `host`, each with its value, the most
    /// specific first.
    pub fn covering(&self, host: &str) -> Vec<(&str, &T)> {
        let Some(host) = key(host) else {
            return Vec::new();
        };
        covering(&host)
            .filter_map(|name| self.listed(name))
            .collect()
    }

    /// The most specific listed domain that covers `host`, with its value.
    pub fn most_specific(&self, host: &str) -> Option<(&str, &T)> {
        let host = key(host)?;
        covering(&host).find_map(|name| self.listed(name))
    }

    /// Each listed domain with its value, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.0
            .iter()
            .map(|(domain, value)| (domain.as_str(), value))
    }

    fn listed(&self, name: &str) -> Option<(&str, &T)> {
        self.0
            .get_key_value(name)
            .map(|(listed, value)| (listed.as_str(), value))
    }
}
