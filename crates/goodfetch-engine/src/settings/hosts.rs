//! Lists of hosts, as the domain policy's keys write them (ADRs 0014 and
//! 0100): each entry a host and its subdomains, read as the WHATWG URL
//! parser reads a host, or `*` for every host. The egress policy matches
//! requests against them, and the resolver decides whether one list
//! narrows another (spec #563, G5).

use std::net::IpAddr;

use goodfetch_protocol::catalog::HostList;
use goodfetch_protocol::egress::EVERY_HOST;
use serde_json::Value;
use url::Host;

/// One entry of a list of hosts, as the URL parser reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEntry {
    /// `*`: every host.
    Every,
    /// A name, in punycode and lower case, without a trailing dot, and its
    /// subdomains.
    Domain(String),
    /// An address, in any spelling the URL parser accepts.
    Address(IpAddr),
}

impl HostEntry {
    /// `entry` read as the URL parser reads a host, an IPv6 address with
    /// or without its brackets, or `None` when it is no host, such as one
    /// with a path, a port or a `*` label.
    pub fn read(entry: &str) -> Option<Self> {
        if entry == EVERY_HOST {
            return Some(Self::Every);
        }
        let host = if entry.contains(':') && !entry.starts_with('[') {
            Host::parse(&format!("[{entry}]")).ok()?
        } else {
            Host::parse(entry).ok()?
        };
        match host {
            Host::Domain(domain) => {
                let domain = without_root(&domain);
                (!domain.is_empty() && !domain.contains('*'))
                    .then(|| Self::Domain(domain.to_owned()))
            }
            Host::Ipv4(address) => Some(Self::Address(IpAddr::V4(address))),
            Host::Ipv6(address) => Some(Self::Address(IpAddr::V6(address))),
        }
    }

    /// Whether every host `other` matches is one this matches.
    fn covers(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Every, _) => true,
            (Self::Domain(name), Self::Domain(other)) => {
                other == name
                    || other
                        .strip_suffix(name.as_str())
                        .is_some_and(|label| label.ends_with('.'))
            }
            (Self::Address(address), Self::Address(other)) => address == other,
            _ => false,
        }
    }
}

/// A name without the one trailing dot that makes it fully qualified: the
/// same host either way.
pub fn without_root(name: &str) -> &str {
    name.strip_suffix('.').unwrap_or(name)
}

/// Whether `higher`, a list of hosts a Recipe or a Client acting for a
/// model gave, allows no more than `lower`, the value the layers below
/// give: a list that allows covers no host `lower` does not, and one that
/// blocks covers every host `lower` lists. An entry that is no host covers
/// nothing and is covered by nothing, so it never narrows.
pub(super) fn narrows(list: HostList, higher: &Value, lower: &Value) -> bool {
    let (Some(higher), Some(lower)) = (entries(higher), entries(lower)) else {
        return false;
    };
    let (inner, outer) = match list {
        HostList::Allows => (higher, lower),
        HostList::Blocks => (lower, higher),
    };
    inner.iter().all(|entry| {
        entry
            .as_ref()
            .is_some_and(|entry| outer.iter().flatten().any(|cover| cover.covers(entry)))
    })
}

/// A list's entries, each `None` where it is no host; `None` when `value`
/// is no list of text.
fn entries(value: &Value) -> Option<Vec<Option<HostEntry>>> {
    value
        .as_array()?
        .iter()
        .map(|entry| entry.as_str().map(HostEntry::read))
        .collect()
}
