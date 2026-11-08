//! The egress gate: one connection's whole check under its egress class
//! (ADR 0017), in the one order the rules need. Its URL, or the host it
//! connects to, is judged before any lookup ([`Gate::judge`],
//! [`Gate::judge_host`]); then every address its host resolves to is
//! judged before any connection ([`Judged::addresses`]), which only a
//! [`Judged`] can do, so no caller judges addresses it never judged the
//! URL of. [`Gate::pass`] runs all three steps, lookup included, in one
//! call, and answers the addresses the connection may try: only those, so
//! an answer that changes after the check (rebinding) reaches nothing.

use std::net::SocketAddr;

use goodfetch_protocol::egress::EgressRule;
use url::{Host, Url};

use super::{
    EndpointPolicy, Judgment, Policy, Refused, Request, RequestKind, judge, judge_endpoint,
    judge_endpoint_addresses, judge_reachable,
};
use crate::host_name;

/// Which egress class a connection is judged under (ADR 0017).
#[derive(Clone, Copy, Debug)]
pub enum Gate<'policy> {
    /// A request of a Session, of `kind`, under its `policy`: the domain
    /// policy, then the resolved-address check.
    Session {
        /// The Session's egress policy.
        policy: &'policy Policy,
        /// The kind of request, as the code that sends it sees it.
        kind: RequestKind,
    },
    /// An Operator-configured endpoint, the second class: its scheme, then
    /// `egress.no_private_endpoints`.
    Endpoint(EndpointPolicy),
}

/// A URL or host the gate allowed, whose addresses it judges next.
#[derive(Clone, Debug)]
pub struct Judged<'named, 'policy> {
    gate: Gate<'policy>,
    /// What a refusal names: the request's URL, or the endpoint's.
    named: &'named str,
    /// The URL as the parser leaves it, when a URL was judged.
    url: Option<Url>,
    /// The host the connection reaches: a name as [`host_name::key`]
    /// leaves it, an address without brackets.
    host: String,
    /// Whether `host` is a name rather than an address.
    name: bool,
}

/// Why a connection is not made.
#[derive(Debug)]
pub enum Stopped<E> {
    /// Its URL or host was refused before any lookup.
    Refused(Refused),
    /// Its host has no address: the resolver's failure.
    Unresolved(E),
    /// An address its host resolved to was refused before any connection.
    AddressRefused(Refused),
}

/// A connection the gate let through, with the only addresses it may try.
#[derive(Clone, Debug)]
pub struct Passed {
    /// The URL as the parser leaves it.
    pub url: Url,
    /// The judged addresses of its host, on its port.
    pub addresses: Vec<SocketAddr>,
}

impl<'policy> Gate<'policy> {
    /// Judges `url` before any lookup: under a Session's policy, its
    /// scheme and the domain policy; for an endpoint, its scheme alone.
    ///
    /// # Errors
    ///
    /// [`Refused`] as [`judge`] refuses it, for an endpoint a scheme other
    /// than HTTP's.
    pub fn judge<'named>(self, url: &'named str) -> Result<Judged<'named, 'policy>, Refused> {
        let judgment = match self {
            Self::Session { policy, kind } => judge(&Request { url, kind }, policy),
            Self::Endpoint(_) => judge_endpoint(url),
        };
        let allowed = match judgment {
            Judgment::Allowed(allowed) => allowed,
            Judgment::Refused(refused) => return Err(refused),
        };
        let (host, name) = match allowed.host() {
            Some(Host::Domain(domain)) => (host_name::without_root(domain).to_owned(), true),
            Some(Host::Ipv4(address)) => (address.to_string(), false),
            Some(Host::Ipv6(address)) => (address.to_string(), false),
            // A `file:` URL a navigation may load reaches no host.
            None => (String::new(), false),
        };
        Ok(Judged {
            gate: self,
            named: url,
            url: Some(allowed),
            host,
            name,
        })
    }

    /// Judges `host`, the host a connection reaches, before any lookup,
    /// for a caller that holds no URL of it, such as a transport's
    /// connector given only a name, or a proxy: under a Session's policy,
    /// the domain policy; for an endpoint, nothing. A refusal names
    /// `named`.
    ///
    /// # Errors
    ///
    /// [`Refused`] by the domain policy, or for a host that is none.
    pub fn judge_host<'named>(
        self,
        host: &str,
        named: &'named str,
    ) -> Result<Judged<'named, 'policy>, Refused> {
        let refused = || Refused {
            url: named.to_owned(),
            rule: EgressRule::DomainPolicy,
            address: None,
            setting: None,
        };
        let key = host_name::key(host).ok_or_else(refused)?;
        let name = key.parse::<std::net::IpAddr>().is_err();
        if let Self::Session { policy, .. } = self {
            policy.judge_host(&key, name).map_err(|setting| Refused {
                setting,
                ..refused()
            })?;
        }
        Ok(Judged {
            gate: self,
            named,
            url: None,
            host: key,
            name,
        })
    }

    /// Judges `url`, resolves its host with `resolve`, given the host
    /// (an IPv6 address without brackets) and the URL's port, and judges
    /// every address it answered: the whole check of a connection to
    /// `url`, in one call.
    ///
    /// # Errors
    ///
    /// [`Stopped`]: at the step that stopped it.
    ///
    /// # Cancel safety
    ///
    /// As `resolve`'s; nothing is kept.
    pub async fn pass<E>(
        self,
        url: &str,
        resolve: impl AsyncFnOnce(&str, u16) -> Result<Vec<SocketAddr>, E>,
    ) -> Result<Passed, Stopped<E>> {
        let judged = self.judge(url).map_err(Stopped::Refused)?;
        let port = judged.port().ok_or_else(|| {
            Stopped::Refused(Refused {
                url: url.to_owned(),
                rule: EgressRule::DomainPolicy,
                address: None,
                setting: None,
            })
        })?;
        let addresses = resolve(judged.lookup_name(), port)
            .await
            .map_err(Stopped::Unresolved)?;
        judged
            .addresses(&addresses)
            .map_err(Stopped::AddressRefused)?;
        Ok(Passed {
            url: judged
                .url
                .expect("invariant: a judged URL keeps its parsed URL"),
            addresses,
        })
    }
}

impl Judged<'_, '_> {
    /// The URL as the parser leaves it, when a URL was judged.
    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    /// The host the connection reaches: a name in lower case and punycode
    /// without its trailing dot, or an address without brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// What a lookup of the host asks the resolver for: [`Self::host`], but
    /// for a name a URL wrote fully qualified, which keeps its trailing dot,
    /// so no search list applies to a name written to avoid one.
    pub fn lookup_name(&self) -> &str {
        match self.url.as_ref().and_then(Url::host) {
            Some(Host::Domain(name)) => name,
            _ => &self.host,
        }
    }

    /// The judged URL's port, its scheme's default when it names none.
    pub fn port(&self) -> Option<u16> {
        self.url.as_ref().and_then(Url::port_or_known_default)
    }

    /// Judges `addresses`, every answer the host has, before the
    /// connection tries any (ADR 0017): one the class refuses refuses the
    /// connection, so a name with a public and a private answer is reached
    /// through neither (ADR 0067).
    ///
    /// # Errors
    ///
    /// [`Refused`] with [`EgressRule::PrivateAddress`] and the first
    /// address refused: under a Session's policy, one a page
    /// may not reach (none while `security.resolved_ip_check` is off); for
    /// an endpoint, one that is not globally reachable under
    /// `egress.no_private_endpoints`.
    pub fn addresses(&self, addresses: &[SocketAddr]) -> Result<(), Refused> {
        match self.gate {
            Gate::Session { policy, .. } => {
                if !policy.resolved_ip_check {
                    return Ok(());
                }
                let host = self.name.then_some(self.host.as_str());
                judge_reachable(self.named, host, addresses, policy)
            }
            Gate::Endpoint(endpoints) => judge_endpoint_addresses(self.named, addresses, endpoints),
        }
    }
}
