//! The egress policy (ADRs 0014, 0017, 0100 and 0120): whether a request
//! of the run may leave, judged before any lookup or connection. Its
//! interface is [`judge`], which answers [`Judgment::Allowed`] with the URL
//! as the WHATWG parser leaves it, or [`Judgment::Refused`] with the rule
//! and the setting that would allow it. It fails closed: a URL that does
//! not parse, a scheme no request may use and a kind of request the caller
//! cannot see are refused.
//!
//! The domain policy is `egress.allowed_domains` and
//! `egress.blocked_domains`. An entry matches its host and their
//! subdomains, as `agent.domains` does (ADR 0100); `*` matches every host,
//! only when written; a blocked entry wins. Hosts and entries alike are
//! read as the WHATWG URL parser reads a host, so `2130706433`,
//! `0x7f000001` and `127.0.0.1` are one address, and `bücher.test` and
//! `xn--bcher-kva.test` one name.

use std::collections::HashSet;
use std::net::IpAddr;

use goodfetch_protocol::egress::{
    EGRESS_ALLOWED_DOMAINS, EGRESS_BLOCKED_DOMAINS, EGRESS_REFUSED, EVERY_HOST,
    EgressRefusedDetails, EgressRule,
};
use url::{Host, Url};

use crate::composition::CallError;
use crate::settings::Resolved;

#[cfg(test)]
mod tests;

/// A Session's egress policy: its domain policy, which [`judge`] applies.
#[derive(Clone, Debug)]
pub struct Policy {
    allowed: Entries,
    blocked: Entries,
}

/// One domain list, each entry as the URL parser reads a host.
#[derive(Clone, Debug, Default)]
struct Entries {
    every_host: bool,
    domains: HashSet<Box<str>>,
    addresses: HashSet<IpAddr>,
}

/// An entry of a domain list that is no host.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{entry}` in `{setting}` is no host: write a host name, an IP address or `*`")]
pub struct InvalidEntry {
    /// The setting that holds it.
    pub setting: &'static str,
    /// The entry, as written.
    pub entry: String,
}

impl Policy {
    /// The policy allowing `allowed` and blocking `blocked`, the values of
    /// `egress.allowed_domains` and `egress.blocked_domains`.
    ///
    /// # Errors
    ///
    /// [`InvalidEntry`] for the first entry that is no host, such as one
    /// with a path, a port or a `*` label.
    pub fn new(
        allowed: &[impl AsRef<str>],
        blocked: &[impl AsRef<str>],
    ) -> Result<Self, InvalidEntry> {
        Ok(Self {
            allowed: Entries::read(allowed, EGRESS_ALLOWED_DOMAINS.name)?,
            blocked: Entries::read(blocked, EGRESS_BLOCKED_DOMAINS.name)?,
        })
    }

    /// The policy a Run's resolved settings give.
    ///
    /// # Errors
    ///
    /// [`InvalidEntry`] as [`Policy::new`].
    pub fn of(settings: &Resolved) -> Result<Self, InvalidEntry> {
        let list = |key: &str| -> Vec<String> {
            settings
                .value(key)
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.as_str().map(str::to_owned))
                .collect()
        };
        Self::new(
            &list(EGRESS_ALLOWED_DOMAINS.name),
            &list(EGRESS_BLOCKED_DOMAINS.name),
        )
    }
}

impl Entries {
    fn read(entries: &[impl AsRef<str>], setting: &'static str) -> Result<Self, InvalidEntry> {
        let mut read = Self::default();
        for entry in entries {
            let entry = entry.as_ref();
            if entry == EVERY_HOST {
                read.every_host = true;
                continue;
            }
            let invalid = || InvalidEntry {
                setting,
                entry: entry.to_owned(),
            };
            match entry_host(entry).ok_or_else(invalid)? {
                Host::Domain(domain) => {
                    let domain = without_root(&domain);
                    if domain.is_empty() || domain.contains('*') {
                        return Err(invalid());
                    }
                    read.domains.insert(domain.into());
                }
                Host::Ipv4(address) => {
                    read.addresses.insert(IpAddr::V4(address));
                }
                Host::Ipv6(address) => {
                    read.addresses.insert(IpAddr::V6(address));
                }
            }
        }
        Ok(read)
    }

    /// Whether an entry matches `host`: the host itself, or a domain it is
    /// a subdomain of.
    fn matches(&self, host: &Host<&str>) -> bool {
        if self.every_host {
            return true;
        }
        match host {
            Host::Domain(domain) => {
                let mut name = without_root(domain);
                loop {
                    if self.domains.contains(name) {
                        return true;
                    }
                    match name.split_once('.') {
                        Some((_, parent)) => name = parent,
                        None => return false,
                    }
                }
            }
            Host::Ipv4(address) => self.addresses.contains(&IpAddr::V4(*address)),
            Host::Ipv6(address) => self.addresses.contains(&IpAddr::V6(*address)),
        }
    }
}

/// An entry read as the URL parser reads a host: a name in punycode and
/// lower case, or an address in any spelling it accepts; an IPv6 address
/// with or without its brackets.
fn entry_host(entry: &str) -> Option<Host<String>> {
    if entry.contains(':') && !entry.starts_with('[') {
        return Host::parse(&format!("[{entry}]")).ok();
    }
    Host::parse(entry).ok()
}

/// A name without the one trailing dot that makes it fully qualified: the
/// same host either way.
fn without_root(name: &str) -> &str {
    name.strip_suffix('.').unwrap_or(name)
}

/// The kind of a request, as the code that sends it sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// A request the engine's own HTTP client sends for the run.
    Fetch,
    /// A request whose kind the interception that holds it cannot see.
    Unseen,
}

/// A request to judge.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Its URL, as the run gave it.
    pub url: &'a str,
    /// Its kind.
    pub kind: RequestKind,
}

/// What [`judge`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Judgment {
    /// The request may leave, to this URL as the parser leaves it.
    Allowed(Url),
    /// The request may not.
    Refused(Refused),
}

/// A refusal: the request, the rule and what would allow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The request's URL, as the run gave it.
    pub url: String,
    /// The rule that refused it.
    pub rule: EgressRule,
    /// The address the rule judged, when it judged one.
    pub address: Option<IpAddr>,
    /// The setting whose value would allow it, when one would.
    pub setting: Option<&'static str>,
}

impl Refused {
    /// What `egress_refused` details.
    pub fn details(&self) -> EgressRefusedDetails {
        EgressRefusedDetails {
            url: self.url.clone(),
            rule: self.rule,
            address: self.address.map(|address| address.to_string()),
            setting: self.setting.map(str::to_owned),
        }
    }

    /// The failure the call that waited for the request answers with.
    pub fn error(&self) -> CallError {
        CallError::new(&EGRESS_REFUSED, self.to_string(), &self.details())
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the egress policy refused a request to {} ({})",
            self.url,
            self.rule.as_str()
        )?;
        if let Some(setting) = self.setting {
            write!(formatter, "; `{setting}` would allow it")?;
        }
        Ok(())
    }
}

/// The schemes a request may use; every other is refused (ADR 0017).
const SCHEMES: [&str; 4] = ["http", "https", "ws", "wss"];

/// Judges `request` under `session`'s policy, before any lookup.
pub fn judge(request: &Request<'_>, session: &Policy) -> Judgment {
    let refused = |rule, setting| {
        Judgment::Refused(Refused {
            url: request.url.to_owned(),
            rule,
            address: None,
            setting,
        })
    };
    if request.kind == RequestKind::Unseen {
        return refused(EgressRule::DomainPolicy, None);
    }
    let Ok(url) = Url::parse(request.url) else {
        return refused(EgressRule::DomainPolicy, None);
    };
    if !SCHEMES.contains(&url.scheme()) {
        return refused(EgressRule::Scheme, None);
    }
    let Some(host) = url.host() else {
        return refused(EgressRule::DomainPolicy, None);
    };
    if session.blocked.matches(&host) {
        return refused(EgressRule::DomainPolicy, Some(EGRESS_BLOCKED_DOMAINS.name));
    }
    if !session.allowed.matches(&host) {
        return refused(EgressRule::DomainPolicy, Some(EGRESS_ALLOWED_DOMAINS.name));
    }
    Judgment::Allowed(url)
}
