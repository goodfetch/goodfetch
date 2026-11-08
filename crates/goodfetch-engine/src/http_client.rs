//! The engine's own HTTP client (ADRs 0017, 0043 and 0060): the one place
//! the engine opens an outbound connection on the OS's network, which the
//! Simulation never carries. It sends one HTTP or HTTPS request over
//! HTTP/1.1 and streams the answer's body, the `resolve` setting applied before any
//! lookup (ADR 0059), every certificate verified by [`tls`]'s per-host
//! decision: Mozilla's roots, and the certificate authorities trusted for
//! the host's domain (ADR 0042). A certificate that fails ends the request
//! with `certificate_invalid` before anything is sent. It adds no header of
//! its own.
//!
//! Every connection it opens races the host's addresses under RFC 8305
//! ([`happy_eyeballs`], ADR 0118). [`connect`] is that step for the HTTP
//! transports that resolve through the engine's [`Dns`]: from the families
//! the Session's local addresses hold, to its proxy's addresses when it has
//! one, forgetting a name whose every address failed.
//!
//! A request of the run, an address a page can reach, goes through
//! [`PageTraffic`], which the Session's egress policy judges before any
//! lookup or connection (ADRs 0014 and 0017), and whose host's every
//! address it judges again before connecting, so the connection tries only
//! addresses the resolved-address check let through: a refusal ends the
//! call that waited for it with `egress_refused`, or, when no call waits,
//! is a `page.request_failed` event in the Run trace. Behind the
//! Session's upstream proxy the same check runs before anything is sent to
//! the proxy (ADR 0017): the engine resolves the name itself, the `resolve`
//! mapping first, and refuses a private answer; the proxy then resolves the
//! name again itself. [`tunnel`] is the translation point of the proxy's
//! answer to a tunnel request (ADR 0127).
//!
//! A Session with local addresses (ADR 0044) sends each connection from its
//! address of the family it connects over, to the site or its proxy, and
//! resolves every name through the engine's own resolver, its queries sent
//! from that address too ([`LocalBinding`]); a host with addresses only in
//! a family it holds none of fails with `address_family_unavailable`.
//!
//! ADR 0043 allows only two kinds of caller beside a run's own requests:
//! `pinned_download`, the one path for every connection nobody configured,
//! and the module of each kind of Operator-configured endpoint. Which
//! modules call it is `modules.toml`'s list, which a test holds to those
//! kinds, each a row of `egress`'s `CONFIGURED_KINDS`. An
//! Operator-configured endpoint goes through `ConfiguredEndpoint`, the
//! second egress class (ADR 0017): its URL and every address it resolves
//! to are judged under the Operator policy's `egress.no_private_endpoints`,
//! and it takes no certificate exception or client certificate of a site,
//! only the authorities trusted for its own domain (ADR 0042).
//!
//! Size: one client, whose lookup, egress judgement, connection race, TLS
//! and request all serve the same request on the same connection; the
//! proxy's tunnel is in its own module.

use std::collections::BTreeMap;
use std::future::ready;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::page::RequestFailed;
use goodfetch_protocol::settings::NETWORK_RESOLVE;
use goodfetch_protocol::tls::TLS_TRUSTED_CAS;
use http_body_util::Full;
use http_body_util::{BodyExt as _, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use url::Host;

use crate::clock::Clock;
use crate::dns::{Dns, IpFamily, LookupError, Resolution, Route, family_of};
use crate::egress::{self, ConfiguredKind, EndpointPolicy, Judgment, Refused};
use crate::happy_eyeballs::{Delays, Families, FamilyUnavailable, Race, RaceError};
use crate::local_addresses::SessionAddresses;
use crate::response_limits::{
    BodyFailure, Decoding, JsonFailure, LimitExceeded, Limits, parse_json,
};
use crate::run_trace::RequestLog;
use crate::settings::Resolved;
use crate::tls::{
    self, CertificateChanged, CertificateExceptions, CertificateInvalid, ClientCertificateFailure,
    ClientCertificates, ExceptionUsed, Trust,
};

#[cfg(test)]
mod tests;

mod tunnel;

pub use tunnel::{ProxyFailure, Unrecognized};

/// The `resolve` setting (ADR 0059): host names mapped to addresses before
/// any lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolve(BTreeMap<String, Vec<IpAddr>>);

impl Resolve {
    /// No mapping: every name goes to the OS's resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps `host` to `addresses`.
    #[must_use]
    pub fn with(mut self, host: impl Into<String>, addresses: Vec<IpAddr>) -> Self {
        self.0.insert(host.into().to_ascii_lowercase(), addresses);
        self
    }

    fn addresses(&self, host: &str) -> Option<&[IpAddr]> {
        self.0.get(&host.to_ascii_lowercase()).map(Vec::as_slice)
    }
}

/// What a request may reach and trust, and how it races a host's addresses.
pub(crate) struct Network<'a> {
    /// The `resolve` setting.
    pub(crate) resolve: &'a Resolve,
    /// What each host's certificate may chain to beside Mozilla's roots.
    pub(crate) trust: &'a Trust,
    /// The Session's certificate exceptions; none for an
    /// Operator-configured endpoint, which never takes one.
    pub(crate) exceptions: Option<&'a CertificateExceptions>,
    /// The Session's client certificates; none for an Operator-configured
    /// endpoint, whose own certificate is its module's.
    pub(crate) client_certificates: Option<&'a ClientCertificates>,
    /// What the race waits on.
    pub(crate) clock: Clock,
    /// The race's delays.
    pub(crate) delays: Delays,
    /// The Session's local addresses, which its connections leave from;
    /// none for an Operator-configured endpoint.
    pub(crate) local: Option<&'a LocalBinding>,
}

/// A Session's local addresses on the engine's own HTTP client (ADR
/// 0044): what its connections bind, and the engine's resolution whose
/// queries leave from them.
#[derive(Clone)]
pub struct LocalBinding {
    addresses: SessionAddresses,
    dns: Dns,
}

impl LocalBinding {
    /// The Session holding `addresses`, resolving through `dns` with its
    /// queries sent from them.
    pub fn new(addresses: SessionAddresses, dns: &Dns) -> Self {
        Self {
            addresses,
            dns: dns.from_local_addresses(addresses.ipv4, addresses.ipv6),
        }
    }

    fn families(&self) -> Families {
        match (self.addresses.ipv4, self.addresses.ipv6) {
            (Some(_), None) => Families::Only(IpFamily::Ipv4),
            (None, Some(_)) => Families::Only(IpFamily::Ipv6),
            _ => Families::Both,
        }
    }

    fn of(&self, family: IpFamily) -> Option<IpAddr> {
        match family {
            IpFamily::Ipv4 => self.addresses.ipv4.map(IpAddr::V4),
            IpFamily::Ipv6 => self.addresses.ipv6.map(IpAddr::V6),
        }
    }

    /// `server`'s addresses on its port: the `resolve` mapping's, its
    /// literal address, or the engine's own resolver's, asked from the
    /// Session's addresses.
    async fn addresses(
        &self,
        server: &Server<'_>,
        resolve: &Resolve,
    ) -> Result<Vec<SocketAddr>, Failure> {
        let on_port = |address: &IpAddr| SocketAddr::new(*address, server.port);
        if let Some(mapped) = resolve.addresses(server.host) {
            return Ok(mapped.iter().map(on_port).collect());
        }
        if let Ok(address) = server.host.parse::<IpAddr>() {
            return Ok(vec![on_port(&address)]);
        }
        match self.dns.lookup(server.host, Route::LocalAddress).await {
            Ok(Resolution::Addresses { addresses, .. }) => {
                Ok(addresses.iter().map(on_port).collect())
            }
            Ok(Resolution::ByProxy) | Err(_) => Err(Failure::Network),
        }
    }

    /// The addresses of `host` on `port`, as a Session holding these local
    /// addresses resolves it before it connects, such as the egress relay
    /// of a browser Session (ADR 0044): the `resolve` mapping's, its
    /// literal address, or the engine's own resolver's, whose queries leave
    /// from the Session's addresses.
    ///
    /// # Errors
    ///
    /// [`Failure::Network`] when the name has no address.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the lookup; nothing is kept.
    pub async fn lookup(
        &self,
        host: &str,
        port: u16,
        resolve: &Resolve,
    ) -> Result<Vec<SocketAddr>, Failure> {
        let server = Server {
            host,
            port,
            plain: true,
        };
        self.addresses(&server, resolve).await
    }

    /// A TCP connection to `address`, bound to the Session's address of its
    /// family.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::AddrNotAvailable`] when the Session holds no address
    /// of that family, so none is opened from another; else the
    /// connection's own failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes a connection still being opened.
    pub async fn connect(&self, address: SocketAddr) -> io::Result<TcpStream> {
        let Some(local) = self.of(family_of(address.ip())) else {
            return Err(io::ErrorKind::AddrNotAvailable.into());
        };
        attempt(address, Some(local)).await
    }
}

impl std::fmt::Debug for LocalBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalBinding")
            .field("addresses", &self.addresses)
            .finish_non_exhaustive()
    }
}

/// Where a Session's connections leave from (ADR 0044): the local address
/// of each family it holds, and its proxy, which its connections reach in
/// place of the site.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Egress {
    /// The IPv4 address its connections bind, when it holds one.
    pub local_ipv4: Option<Ipv4Addr>,
    /// The IPv6 address its connections bind, when it holds one.
    pub local_ipv6: Option<Ipv6Addr>,
    /// Its proxy's host and port.
    pub proxy: Option<(String, u16)>,
}

impl Egress {
    fn families(&self) -> Families {
        match (self.local_ipv4, self.local_ipv6) {
            (Some(_), None) => Families::Only(IpFamily::Ipv4),
            (None, Some(_)) => Families::Only(IpFamily::Ipv6),
            _ => Families::Both,
        }
    }

    fn route(&self) -> Route {
        if self.local_ipv4.is_some() || self.local_ipv6.is_some() {
            Route::LocalAddress
        } else {
            Route::Direct
        }
    }

    fn local_address(&self, family: IpFamily) -> Option<IpAddr> {
        match family {
            IpFamily::Ipv4 => self.local_ipv4.map(IpAddr::V4),
            IpFamily::Ipv6 => self.local_ipv6.map(IpAddr::V6),
        }
    }
}

/// How [`connect`] resolves and races.
#[derive(Clone)]
pub struct Connecting {
    /// The engine's name resolution.
    pub dns: Dns,
    /// What the race waits on.
    pub clock: Clock,
    /// The race's delays.
    pub delays: Delays,
}

/// A connection [`connect`] opened.
#[derive(Debug)]
pub struct Connected {
    /// The connection.
    pub stream: TcpStream,
    /// The address it reached: the site's, or its proxy's.
    pub peer: SocketAddr,
}

/// Opens a connection to `host` on `port` for a Session leaving through
/// `egress`: to its proxy instead when it has one, which resolves `host`
/// itself; else to the host's addresses that the Session's local addresses
/// can reach, raced under RFC 8305. When every address tried failed, the
/// resolver forgets the name, so the next connection asks again (RFC 8305
/// section 9).
///
/// # Errors
///
/// [`RaceError`]: the name has no address, none in a family the Session
/// holds, or none that connected.
///
/// # Cancel safety
///
/// Dropping the future closes every attempt in flight.
pub async fn connect(
    host: &str,
    port: u16,
    egress: &Egress,
    connecting: &Connecting,
) -> Result<Connected, RaceError> {
    let (name, port) = match &egress.proxy {
        Some((proxy, proxy_port)) => (proxy.as_str(), *proxy_port),
        None => (host, port),
    };
    let route = egress.route();
    let race = Race {
        clock: connecting.clock,
        delays: connecting.delays,
        families: egress.families(),
    };
    let dns = &connecting.dns;
    let raced = race
        .connect(
            name,
            port,
            dns.addresses(name, IpFamily::Ipv6, route),
            dns.addresses(name, IpFamily::Ipv4, route),
            |address| attempt(address, egress.local_address(family_of(address.ip()))),
        )
        .await;
    match raced {
        Ok(won) => Ok(Connected {
            stream: won.connection,
            peer: won.address,
        }),
        Err(error) => {
            if matches!(&error, RaceError::Unreachable { tried } if !tried.is_empty()) {
                dns.forget(name).await;
            }
            Err(error)
        }
    }
}

/// Where a request goes: the server it connects to and names in TLS.
pub(crate) struct Server<'a> {
    /// Its host name or IP address, without brackets.
    pub(crate) host: &'a str,
    /// Its port.
    pub(crate) port: u16,
    /// Whether it speaks plain HTTP rather than HTTPS.
    pub(crate) plain: bool,
}

/// What a request of the run may reach, from the engine's own HTTP client:
/// the Session's egress policy, which judges each request before any
/// lookup, and the network it then uses.
#[derive(Clone, Debug)]
pub struct PageTraffic {
    /// The Session's egress policy.
    pub egress: egress::Policy,
    /// The `resolve` setting (ADR 0059).
    pub resolve: Resolve,
    /// The Session's local addresses (ADR 0044), which its connections and
    /// DNS queries leave from when it holds some.
    pub local: Option<LocalBinding>,
    /// The Session's upstream proxy, which its requests go through when it
    /// has one.
    pub proxy: Option<UpstreamProxy>,
    /// What each host's certificate may chain to beside Mozilla's roots:
    /// the certificate authorities `tls.trusted_cas` lists for its domain.
    pub trust: Trust,
    /// The exceptions `tls.accept_invalid_certs` makes, and where they pin.
    pub exceptions: CertificateExceptions,
    /// The client certificates `tls.client_certificates` lists, each
    /// presented to its domain's hosts when they ask for one.
    pub client_certificates: ClientCertificates,
    /// The Session's cookies, which go to no host reached through an
    /// exception.
    pub cookies: Vec<SessionCookie>,
    /// What the Happy Eyeballs race waits on (ADR 0118).
    pub clock: Clock,
    /// `resolver.resolution_delay` and `resolver.attempt_delay`.
    pub delays: Delays,
}

/// A Session's upstream HTTP proxy, as the engine's HTTP client reaches
/// it: a plain `http://` request goes to it in absolute form, and an
/// `https://` one through the tunnel it opens to the site. It resolves the
/// site's name itself; the engine has judged that name's addresses before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpstreamProxy {
    /// Its host name or IP address, without brackets.
    pub host: String,
    /// Its port.
    pub port: u16,
    /// How its own addresses are judged: as an Operator-configured
    /// endpoint's, under `egress.no_private_endpoints` (ADR 0017).
    pub policy: EndpointPolicy,
}

impl std::fmt::Display for UpstreamProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.contains(':') {
            write!(formatter, "http://[{}]:{}", self.host, self.port)
        } else {
            write!(formatter, "http://{}:{}", self.host, self.port)
        }
    }
}

/// One cookie of a Session, sent to its domain and the domain's
/// subdomains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionCookie {
    /// The domain it is sent to, with its subdomains.
    pub domain: String,
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
}

/// Who waits for a request: a call, which a refusal fails, or nobody, when
/// the refusal is recorded in the Run trace instead.
#[derive(Clone, Copy, Debug)]
pub enum Waiter<'a> {
    /// A call, which answers the refusal as `egress_refused`.
    Call,
    /// No call: a refusal is a `page.request_failed` event in this Run's
    /// request log.
    Nobody(&'a RequestLog),
}

/// Why a request of the run got no answer.
#[derive(Debug)]
#[non_exhaustive]
pub enum FetchError {
    /// The egress policy refused it before any lookup or connection.
    Refused(Refused),
    /// The policy allows its scheme, but the engine's client speaks only
    /// HTTP and HTTPS, such as for `ws:`.
    UnsupportedScheme(String),
    /// It was sent and got no answer.
    Failed(Failure),
}

impl PageTraffic {
    /// Sends a GET for `url` once the egress policy allows it, and answers
    /// its head; its body arrives through [`Response::chunk`]. The policy
    /// judges the request before any lookup, then every address its host
    /// stands for (the `resolve` mapping's, its literal address or the
    /// resolver's answers) before any connection, which tries only those
    /// addresses (ADR 0017's resolved-address check). Behind an upstream
    /// proxy, the engine still resolves the name itself and judges its
    /// answers before anything is sent to the proxy, which then resolves
    /// the name again itself: the check refuses a private answer there, and
    /// cannot see one the proxy's own resolver gives. A refusal opens no
    /// connection; when nobody waits for the request, it is also recorded
    /// in the Run trace.
    ///
    /// # Errors
    ///
    /// [`FetchError::Refused`] when the policy refuses it, whose
    /// [`Refused::error`] is `egress_refused`;
    /// [`FetchError::UnsupportedScheme`] for an allowed scheme other than
    /// `http` and `https`; [`FetchError::Failed`] when it got no answer,
    /// with [`Failure::Proxy`] when the proxy carried nothing.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any connection it opened.
    pub async fn get(&self, url: &str, waiter: Waiter<'_>) -> Result<Response, FetchError> {
        let requested = url;
        let request = egress::Request {
            url,
            kind: egress::RequestKind::Fetch,
        };
        let url = match egress::judge(&request, &self.egress) {
            Judgment::Allowed(url) => url,
            Judgment::Refused(refused) => {
                if let Waiter::Nobody(log) = waiter {
                    record_unwaited(log, &refused).await;
                }
                return Err(FetchError::Refused(refused));
            }
        };
        let plain = match url.scheme() {
            "http" => true,
            "https" => false,
            scheme => return Err(FetchError::UnsupportedScheme(scheme.to_owned())),
        };
        let host = match url.host() {
            Some(Host::Ipv6(address)) => address.to_string(),
            Some(host) => host.to_string(),
            None => return Err(FetchError::Failed(Failure::ServerName)),
        };
        let port = url
            .port_or_known_default()
            .ok_or(FetchError::Failed(Failure::ServerName))?;
        let authority = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_owned(),
        };
        let path = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let request = Request::get(path)
            .header(hyper::header::HOST, authority)
            .body(Empty::<Bytes>::new())
            .map_err(|_| FetchError::Failed(Failure::ServerName))?;
        let network = Network {
            resolve: &self.resolve,
            trust: &self.trust,
            exceptions: Some(&self.exceptions),
            client_certificates: Some(&self.client_certificates),
            clock: self.clock,
            delays: self.delays,
            local: self.local.as_ref(),
        };
        let server = Server {
            host: &host,
            port,
            plain,
        };
        let found = self.addresses(&server).await.map_err(FetchError::Failed)?;
        if let Err(refused) = egress::judge_addresses(requested, &found, &self.egress) {
            if let Waiter::Nobody(log) = waiter {
                record_unwaited(log, &refused).await;
            }
            return Err(FetchError::Refused(refused));
        }
        let via = match &self.proxy {
            None => Via::Direct(&found),
            Some(proxy) => {
                let proxy_server = Server {
                    host: &proxy.host,
                    port: proxy.port,
                    plain: true,
                };
                let at = self
                    .addresses(&proxy_server)
                    .await
                    .map_err(|_| FetchError::Failed(Failure::Proxy(ProxyFailure::Unreachable)))?;
                if let Err(refused) =
                    egress::judge_endpoint_addresses(&proxy.to_string(), &at, proxy.policy)
                {
                    if let Waiter::Nobody(log) = waiter {
                        record_unwaited(log, &refused).await;
                    }
                    return Err(FetchError::Refused(refused));
                }
                Via::Proxy {
                    port: proxy_server.port,
                    host: &proxy.host,
                    at,
                }
            }
        };
        send_to(&server, &via, request, &self.cookies, &network)
            .await
            .map_err(FetchError::Failed)
    }

    /// `server`'s addresses as this Session resolves them: from its local
    /// addresses when it holds some.
    async fn addresses(&self, server: &Server<'_>) -> Result<Vec<SocketAddr>, Failure> {
        match &self.local {
            Some(local) => local.addresses(server, &self.resolve).await,
            None => addresses(server, &self.resolve).await,
        }
    }
}

/// An Operator-configured endpoint the engine connects to on its own
/// behalf, the second egress class (ADR 0017): no run's domain policy,
/// private ranges, certificate exception or client certificate applies to
/// it. Its kind is a row of `egress`'s `CONFIGURED_KINDS`.
pub(crate) struct ConfiguredEndpoint<'a> {
    /// Its kind.
    pub(crate) kind: ConfiguredKind,
    /// What the Operator policy lets it reach.
    pub(crate) policy: EndpointPolicy,
    /// The `resolve` setting.
    pub(crate) resolve: &'a Resolve,
    /// What its certificate may chain to beside Mozilla's roots: the
    /// authorities `tls.trusted_cas` lists for its own domain.
    pub(crate) trust: &'a Trust,
    /// What the race waits on.
    pub(crate) clock: Clock,
    /// The race's delays.
    pub(crate) delays: Delays,
}

/// Why an Operator-configured endpoint got no request.
#[derive(Debug)]
pub(crate) enum EndpointFailure {
    /// The egress policy refused it before any connection: its scheme, or
    /// an address `egress.no_private_endpoints` refuses.
    Refused(Refused),
    /// It was sent and got no answer.
    Failed(Failure),
}

impl ConfiguredEndpoint<'_> {
    /// Sends `request`, whose URI is the endpoint's absolute URL, once the
    /// egress policy allows it, and answers its head; its body arrives
    /// through [`Response::chunk`]. The URL is judged before any lookup,
    /// then every address its host stands for before any connection, which
    /// tries only those; an invalid certificate ends it with
    /// `certificate_invalid` whatever `tls.accept_invalid_certs` lists.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any connection it opened.
    pub(crate) async fn send<B: RequestBody>(
        &self,
        mut request: Request<B>,
    ) -> Result<Response, EndpointFailure> {
        let url = request.uri().to_string();
        let url = match egress::judge_endpoint(&url) {
            Judgment::Allowed(url) => url,
            Judgment::Refused(refused) => return Err(EndpointFailure::Refused(refused)),
        };
        let failed = |failure| EndpointFailure::Failed(failure);
        let plain = url.scheme() == "http";
        let host = match url.host() {
            Some(Host::Ipv6(address)) => address.to_string(),
            Some(host) => host.to_string(),
            None => return Err(failed(Failure::ServerName)),
        };
        let port = url
            .port_or_known_default()
            .ok_or(failed(Failure::ServerName))?;
        let authority = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_owned(),
        };
        let path = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        *request.uri_mut() = path.parse().map_err(|_| failed(Failure::ServerName))?;
        let authority = hyper::header::HeaderValue::from_str(&authority)
            .map_err(|_| failed(Failure::ServerName))?;
        request.headers_mut().insert(hyper::header::HOST, authority);
        let server = Server {
            host: &host,
            port,
            plain,
        };
        let found = addresses(&server, self.resolve).await.map_err(failed)?;
        egress::judge_endpoint_addresses(url.as_str(), &found, self.policy)
            .map_err(EndpointFailure::Refused)?;
        tracing::debug!(
            kind = self.kind.classed().name,
            "connecting to an Operator-configured endpoint"
        );
        let network = Network {
            resolve: self.resolve,
            trust: self.trust,
            exceptions: None,
            client_certificates: None,
            clock: self.clock,
            delays: self.delays,
            local: None,
        };
        send_to(&server, &Via::Direct(&found), request, &[], &network)
            .await
            .map_err(failed)
    }
}

/// What the machine's settings let every Operator-configured endpoint
/// reach and trust: `egress.no_private_endpoints`, `network.resolve`,
/// `tls.trusted_cas`, the race's delays and `network.max_body_size`, which
/// caps every reply as each endpoint's own limit does. Each endpoint's
/// module holds one and sends through it with its own kind.
#[derive(Clone, Debug)]
pub(crate) struct EndpointSettings {
    policy: EndpointPolicy,
    resolve: Resolve,
    authorities: Vec<(String, Vec<PathBuf>)>,
    clock: Clock,
    delays: Delays,
    max_body_size: u64,
}

/// Why an Operator-configured endpoint gave no whole answer.
#[derive(Debug)]
pub(crate) enum WholeFailure {
    /// The egress policy refused it before any connection.
    Refused(Refused),
    /// Its certificate failed verification, so nothing was sent.
    CertificateInvalid(CertificateInvalid),
    /// No answer, or a connection that broke.
    Network,
    /// The answer was longer than the caller's limit.
    TooLong,
}

impl EndpointSettings {
    /// The settings `settings`, the machine's resolved ones, give.
    pub(crate) fn of(settings: &Resolved, clock: Clock) -> Self {
        let resolve = settings
            .hosts(&NETWORK_RESOLVE)
            .into_iter()
            .fold(Resolve::new(), |mapping, (host, addresses)| {
                mapping.with(host, addresses)
            });
        let authorities = settings
            .value(&TLS_TRUSTED_CAS)
            .as_object()
            .into_iter()
            .flatten()
            .map(|(domain, files)| {
                let files = files
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|file| file.as_str().map(PathBuf::from))
                    .collect();
                (domain.clone(), files)
            })
            .collect();
        Self {
            policy: EndpointPolicy::of(settings),
            resolve,
            authorities,
            clock,
            delays: Delays::of(settings),
            max_body_size: Limits::of(settings).max_body_size,
        }
    }

    /// Sends `request`, whose URI is the endpoint's absolute URL, to an
    /// endpoint of `kind`, and answers its whole answer, at most `limit`
    /// bytes of body and at most `network.max_body_size`.
    ///
    /// # Errors
    ///
    /// [`WholeFailure`] when the policy refused the endpoint, its
    /// certificate failed, or it gave no whole answer within both limits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub(crate) async fn send_whole(
        &self,
        kind: ConfiguredKind,
        request: Request<Bytes>,
        limit: usize,
    ) -> Result<hyper::Response<Bytes>, WholeFailure> {
        let trust = self.trust().await;
        let endpoint = ConfiguredEndpoint {
            kind,
            policy: self.policy,
            resolve: &self.resolve,
            trust: &trust,
            clock: self.clock,
            delays: self.delays,
        };
        let mut answer = endpoint
            .send(request.map(Full::new))
            .await
            .map_err(|failure| match failure {
                EndpointFailure::Refused(refused) => WholeFailure::Refused(refused),
                EndpointFailure::Failed(Failure::CertificateInvalid(invalid)) => {
                    WholeFailure::CertificateInvalid(invalid)
                }
                EndpointFailure::Failed(_) => WholeFailure::Network,
            })?;
        let limit = limit.min(usize::try_from(self.max_body_size).unwrap_or(usize::MAX));
        let mut body = Vec::new();
        while let Some(bytes) = answer.chunk().await.map_err(|_| WholeFailure::Network)? {
            if body.len() + bytes.len() > limit {
                return Err(WholeFailure::TooLong);
            }
            body.extend_from_slice(&bytes);
        }
        let mut response = hyper::Response::new(Bytes::from(body));
        *response.status_mut() = answer.status;
        *response.headers_mut() = answer.headers;
        Ok(response)
    }

    /// The trust of the endpoint's certificate: the authorities
    /// `tls.trusted_cas` lists for each domain, read now; a file that
    /// cannot be read is logged and trusts nothing.
    async fn trust(&self) -> Trust {
        let mut trust = Trust::default_roots();
        for (domain, files) in &self.authorities {
            let mut authorities = Vec::new();
            for file in files {
                let read = tokio::fs::read(file)
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        tls::read_certificates(&bytes).map_err(|error| error.to_string())
                    });
                match read {
                    Ok(read) => authorities.extend(read),
                    Err(error) => tracing::warn!(
                        %error,
                        file = %file.display(),
                        "a certificate authority of `tls.trusted_cas` cannot be read"
                    ),
                }
            }
            match trust.clone().with_trusted_cas(domain, authorities) {
                Ok(with) => trust = with,
                Err(error) => tracing::warn!(%error, "an entry of `tls.trusted_cas` names no host"),
            }
        }
        trust
    }
}

/// How a request reaches its server.
enum Via<'a> {
    /// Straight, to one of the server's addresses.
    Direct(&'a [SocketAddr]),
    /// Through the Session's upstream proxy, at one of its addresses: in
    /// absolute form for plain HTTP, else through the tunnel it opens.
    Proxy {
        /// The proxy's host.
        host: &'a str,
        /// The proxy's port.
        port: u16,
        /// The proxy's addresses.
        at: Vec<SocketAddr>,
    },
}

/// Records `refused` as a `page.request_failed` event in the Run trace,
/// for a request no call waited for.
async fn record_unwaited(log: &RequestLog, refused: &Refused) {
    let failed = RequestFailed {
        url: refused.url.clone(),
        method: "GET".to_owned(),
        failure: refused.failure(),
    };
    if let Err(error) = log.request_failed(&failed).await {
        tracing::warn!(%error, "the Run trace cannot record a refused request");
    }
}

/// Why a request got no answer.
#[derive(Debug)]
pub enum Failure {
    /// The host is no name TLS can verify.
    ServerName,
    /// No answer, or a connection that broke.
    Network,
    /// The host's certificate failed verification, so nothing was sent:
    /// `certificate_invalid`.
    CertificateInvalid(CertificateInvalid),
    /// The host is pinned to another key than it presented, or its pin row
    /// cannot be read, so nothing was sent (ADR 0042).
    CertificateChanged(CertificateChanged),
    /// The client certificate listed for the host cannot be presented, or
    /// may not go to it, so nothing was sent.
    ClientCertificate(ClientCertificateFailure),
    /// The Session's upstream proxy carried nothing to the site: its own
    /// failure, never the site's.
    Proxy(ProxyFailure),
    /// The host has addresses only in a family the Session holds no local
    /// address of, so nothing was sent: `address_family_unavailable`.
    AddressFamily(FamilyUnavailable),
}

/// Why a body was not read whole.
#[derive(Debug)]
#[non_exhaustive]
pub enum BodyError {
    /// It passed one of the limits on what the engine reads:
    /// `response_limit_exceeded`.
    Limit(LimitExceeded),
    /// It is no stream of the content codings its `Content-Encoding` lists.
    Decoding,
    /// It is no JSON.
    Json,
    /// The connection broke before its end.
    Failed(Failure),
}

/// A request's body: none for a GET, its bytes for an endpoint's POST.
pub(crate) trait RequestBody:
    hyper::body::Body<Data = Bytes, Error: Into<Box<dyn std::error::Error + Send + Sync>>>
    + Send
    + 'static
{
}

impl<B> RequestBody for B where
    B: hyper::body::Body<Data = Bytes, Error: Into<Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + 'static
{
}

/// The task that drives one connection, plain or over TLS, until it ends.
type Connection = Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>;

#[expect(
    clippy::disallowed_types,
    reason = "the engine's own HTTP client opens its outbound sockets, which the Simulation never carries (ADR 0060)"
)]
type TcpStream = tokio::net::TcpStream;

/// An answer whose body is still arriving.
pub struct Response {
    /// Its status.
    pub(crate) status: StatusCode,
    /// Its `Content-Length`, when it gave a number.
    pub(crate) content_length: Option<u64>,
    /// Its headers.
    pub(crate) headers: hyper::HeaderMap,
    body: Incoming,
    connection: Connection,
    connection_done: bool,
    exception: Option<ExceptionUsed>,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("content_length", &self.content_length)
            .finish_non_exhaustive()
    }
}

impl Response {
    /// Its status code.
    pub fn status(&self) -> u16 {
        self.status.as_u16()
    }

    /// The invalid-certificate exception it was fetched under, which its
    /// Provenance carries: the kind and the certificate's hash.
    pub fn exception(&self) -> Option<&ExceptionUsed> {
        self.exception.as_ref()
    }

    /// The whole body, decoded under its `Content-Encoding` and `limits`
    /// as it arrives (SC-05): it stops reading once a limit is passed, so
    /// the engine holds no more of it than the limit. `url` is the URL it
    /// answers, which a refusal names.
    ///
    /// # Errors
    ///
    /// [`BodyError::Limit`], `response_limit_exceeded`, past a limit;
    /// [`BodyError::Decoding`] for a body that is no stream of its codings;
    /// [`BodyError::Failed`] when the connection broke.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses what it read.
    pub async fn body(&mut self, url: &str, limits: &Limits) -> Result<Vec<u8>, BodyError> {
        let codings: Vec<&str> = self
            .headers
            .get_all(hyper::header::CONTENT_ENCODING)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect();
        let mut decoding = Decoding::new(url, codings.into_iter(), *limits);
        let refused = |failure| match failure {
            BodyFailure::Limit(exceeded) => BodyError::Limit(exceeded),
            BodyFailure::Decoding => BodyError::Decoding,
        };
        while let Some(bytes) = self.chunk().await.map_err(BodyError::Failed)? {
            decoding.push(&bytes).map_err(refused)?;
        }
        decoding.finish().map_err(refused)
    }

    /// The body read as [`Self::body`] reads it, then parsed as JSON once
    /// found within `network.max_json_depth` ([`parse_json`]).
    ///
    /// # Errors
    ///
    /// As [`Self::body`] says, [`BodyError::Limit`] for JSON nested past
    /// the depth, and [`BodyError::Json`] for a body that is no JSON.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses what it read.
    pub async fn json(
        &mut self,
        url: &str,
        limits: &Limits,
    ) -> Result<serde_json::Value, BodyError> {
        let body = self.body(url, limits).await?;
        parse_json(url, &body, limits).map_err(|failure| match failure {
            JsonFailure::Limit(exceeded) => BodyError::Limit(exceeded),
            JsonFailure::Malformed => BodyError::Json,
        })
    }

    /// The body's next bytes; `None` once it ended.
    ///
    /// # Errors
    ///
    /// [`Failure::Network`] when the connection broke.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no bytes already received.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, Failure> {
        loop {
            tokio::select! {
                frame = self.body.frame() => {
                    match frame {
                        None => return Ok(None),
                        Some(Err(_)) => return Err(Failure::Network),
                        Some(Ok(frame)) => {
                            if let Ok(bytes) = frame.into_data() {
                                return Ok(Some(bytes));
                            }
                        }
                    }
                }
                ended = &mut self.connection, if !self.connection_done => {
                    self.connection_done = true;
                    if ended.is_err() {
                        return Err(Failure::Network);
                    }
                }
            }
        }
    }
}

/// Sends `request` to `server`, over HTTPS unless the server is plain, and
/// answers its head; its body arrives through [`Response::chunk`].
///
/// # Cancel safety
///
/// Dropping the future closes any connection it opened.
pub(crate) async fn send<B: RequestBody>(
    server: &Server<'_>,
    request: Request<B>,
    network: &Network<'_>,
) -> Result<Response, Failure> {
    let found = addresses(server, network.resolve).await?;
    send_to(server, &Via::Direct(&found), request, &[], network).await
}

/// The `Cookie` header of `cookies` for `host`: those whose domain is the
/// host or one it is a subdomain of; none when no cookie is for it.
fn cookie_header(cookies: &[SessionCookie], host: &str) -> Option<hyper::header::HeaderValue> {
    let host = host.to_ascii_lowercase();
    let line = cookies
        .iter()
        .filter(|cookie| {
            let domain = cookie.domain.to_ascii_lowercase();
            host == domain
                || host
                    .strip_suffix(domain.as_str())
                    .is_some_and(|start| start.ends_with('.'))
        })
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ");
    if line.is_empty() {
        return None;
    }
    hyper::header::HeaderValue::from_str(&line).ok()
}

/// Sends `request` to `server` `via` one of its addresses or its proxy,
/// over HTTPS ([`open_tls`]) unless the server is plain, and answers its
/// head. `cookies` go to the host unless it was reached through an
/// exception.
///
/// # Cancel safety
///
/// Dropping the future closes any connection it opened.
async fn send_to<B: RequestBody>(
    server: &Server<'_>,
    via: &Via<'_>,
    mut request: Request<B>,
    cookies: &[SessionCookie],
    network: &Network<'_>,
) -> Result<Response, Failure> {
    let mut exception = None;
    if server.plain && matches!(via, Via::Proxy { .. }) {
        absolute_form(&mut request)?;
    }
    let (mut sender, mut connection) = if server.plain {
        let tcp = open(server, via, network).await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|_| Failure::Network)?;
        (sender, Box::pin(connection) as Connection)
    } else {
        let (tls, used) = open_tls(server, via, network).await?;
        exception = used;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|_| Failure::Network)?;
        (sender, Box::pin(connection) as Connection)
    };
    if exception.is_none()
        && let Some(cookie) = cookie_header(cookies, server.host)
    {
        request.headers_mut().insert(hyper::header::COOKIE, cookie);
    }
    let mut connection_done = false;
    let head = {
        let sent = sender.send_request(request);
        tokio::pin!(sent);
        loop {
            tokio::select! {
                answered = &mut sent => break answered.map_err(|_| Failure::Network)?,
                ended = &mut connection, if !connection_done => {
                    connection_done = true;
                    if ended.is_err() {
                        return Err(Failure::Network);
                    }
                }
            }
        }
    };
    let content_length = head
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse().ok());
    let headers = head.headers().clone();
    Ok(Response {
        status: head.status(),
        content_length,
        headers,
        body: head.into_body(),
        connection,
        connection_done,
        exception,
    })
}

/// A TLS connection to `server` `via` its addresses or its proxy, and the
/// invalid-certificate exception it was let through under, if any. A host
/// listed for trust on first use is let through or stopped by the
/// Session's exceptions once its handshake is done; the client certificate
/// listed for its domain, its key resolved now, is presented when it asks
/// for one, and the handshake stops before it is sent unless the host's
/// certificate verifies and it has no pin.
///
/// # Cancel safety
///
/// Dropping the future closes any connection it opened.
async fn open_tls(
    server: &Server<'_>,
    via: &Via<'_>,
    network: &Network<'_>,
) -> Result<
    (
        tokio_rustls::client::TlsStream<TcpStream>,
        Option<ExceptionUsed>,
    ),
    Failure,
> {
    let server_name =
        ServerName::try_from(server.host.to_owned()).map_err(|_| Failure::ServerName)?;
    let listed = network
        .exceptions
        .filter(|exceptions| exceptions.covers(server.host));
    if let Some(exceptions) = listed
        && let Some(stopped) = exceptions.stopped(server.host).await
    {
        return Err(Failure::CertificateChanged(stopped));
    }
    let tcp = open(server, via, network).await?;
    let client = match network.client_certificates {
        Some(certificates) => certificates
            .for_host(server.host)
            .await
            .map_err(Failure::ClientCertificate)?,
        None => None,
    };
    let seen = match (listed, &client) {
        (Some(exceptions), Some(_)) => Some(Arc::new(tls::Seen::before_client_certificate(
            exceptions.pinned(server.host).await,
        ))),
        (Some(_), None) => Some(Arc::new(tls::Seen::default())),
        (None, _) => None,
    };
    let config = network
        .trust
        .connection_config(seen.clone(), client, network.clock);
    let tls = match tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(server_name, tcp)
        .await
    {
        Ok(tls) => tls,
        Err(error) => {
            let presented = seen.as_ref().and_then(|seen| seen.take());
            return Err(match (listed, presented) {
                (Some(exceptions), Some(presented)) if tls::withheld(&error) => {
                    withheld(exceptions, network, server.host, presented).await
                }
                _ => tls::certificate_invalid(&error, server.host)
                    .map_or(Failure::Network, Failure::CertificateInvalid),
            });
        }
    };
    let mut exception = None;
    if let (Some(exceptions), Some(seen)) = (listed, &seen) {
        let presented = seen.take().ok_or(Failure::Network)?;
        exception = exceptions
            .admit(server.host, presented)
            .await
            .map_err(Failure::CertificateChanged)?;
    }
    Ok((tls, exception))
}

/// Why a handshake with `host` stopped before its client certificate was
/// sent, once it showed `presented`: the exception's decision, a changed
/// key's `certificate_changed`, or, for a host its exception lets through,
/// the refusal of a client certificate to a host reached that way.
async fn withheld(
    exceptions: &CertificateExceptions,
    network: &Network<'_>,
    host: &str,
    presented: tls::Presented,
) -> Failure {
    match exceptions.admit(host, presented).await {
        Err(changed) => Failure::CertificateChanged(changed),
        Ok(Some(_)) => network
            .client_certificates
            .and_then(|certificates| certificates.refused(host))
            .map_or(Failure::Network, |refused| {
                Failure::ClientCertificate(ClientCertificateFailure::Secret(refused))
            }),
        // The host's pin went between the lookup and the handshake: the
        // next connection presents the certificate.
        Ok(None) => Failure::Network,
    }
}

/// `request`'s target in absolute form, as a proxy takes a plain HTTP
/// request: its scheme and its `Host` before its path.
fn absolute_form<B>(request: &mut Request<B>) -> Result<(), Failure> {
    let host = request
        .headers()
        .get(hyper::header::HOST)
        .and_then(|host| host.to_str().ok())
        .ok_or(Failure::ServerName)?;
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", hyper::http::uri::PathAndQuery::as_str);
    *request.uri_mut() = format!("http://{host}{path}")
        .parse()
        .map_err(|_| Failure::ServerName)?;
    Ok(())
}

/// A connection to `server` `via` its addresses, or through its proxy: a
/// tunnel to it for HTTPS, the proxy's answer read by [`tunnel`].
async fn open(
    server: &Server<'_>,
    via: &Via<'_>,
    network: &Network<'_>,
) -> Result<TcpStream, Failure> {
    match via {
        Via::Direct(found) => race_to(server.host, server.port, found, network)
            .await
            .map_err(|error| match error {
                RaceError::FamilyUnavailable(unavailable) => Failure::AddressFamily(unavailable),
                _ => Failure::Network,
            }),
        Via::Proxy { host, port, at } => {
            let mut tcp = race_to(host, *port, at, network)
                .await
                .map_err(|error| match error {
                    RaceError::FamilyUnavailable(unavailable) => {
                        Failure::AddressFamily(unavailable)
                    }
                    _ => Failure::Proxy(ProxyFailure::Unreachable),
                })?;
            if !server.plain {
                let authority = match server.host.parse::<IpAddr>() {
                    Ok(IpAddr::V6(address)) => format!("[{address}]:{}", server.port),
                    _ => format!("{}:{}", server.host, server.port),
                };
                tunnel::open(&mut tcp, &authority)
                    .await
                    .map_err(Failure::Proxy)?;
            }
            Ok(tcp)
        }
    }
}

/// Connects to one of `found`, the addresses of `name` on `port`, both
/// families answered at once, raced under RFC 8305: it tries no other
/// address, and only those of a family the Session's local addresses hold,
/// each connection bound to its address of that family.
async fn race_to(
    name: &str,
    port: u16,
    found: &[SocketAddr],
    network: &Network<'_>,
) -> Result<TcpStream, RaceError> {
    let of = |family: IpFamily| -> Result<Vec<IpAddr>, LookupError> {
        Ok(found
            .iter()
            .map(SocketAddr::ip)
            .filter(|address| family_of(*address) == family)
            .collect())
    };
    let race = Race {
        clock: network.clock,
        delays: network.delays,
        families: network.local.map_or(Families::Both, LocalBinding::families),
    };
    let local = network.local;
    race.connect(
        name,
        port,
        ready(of(IpFamily::Ipv6)),
        ready(of(IpFamily::Ipv4)),
        |address| {
            attempt(
                address,
                local.and_then(|local| local.of(family_of(address.ip()))),
            )
        },
    )
    .await
    .map(|won| won.connection)
}

/// One connection attempt to `address`, bound to `local` when the Session
/// holds a local address of its family.
#[expect(
    clippy::disallowed_types,
    reason = "the engine's own HTTP client opens its outbound sockets, which the Simulation never carries (ADR 0060)"
)]
async fn attempt(address: SocketAddr, local: Option<IpAddr>) -> io::Result<TcpStream> {
    let socket = if address.is_ipv4() {
        tokio::net::TcpSocket::new_v4()?
    } else {
        tokio::net::TcpSocket::new_v6()?
    };
    if let Some(local) = local {
        socket.bind(SocketAddr::new(local, 0))?;
    }
    socket.connect(address).await
}

/// The addresses of `host` on `port`, as the engine itself resolves it
/// before connecting: the `resolve` mapping's, its literal address (an
/// IPv6 one without brackets), or the system resolver's, its zone ID
/// included. A Borrowed browser resolves names itself, so the engine
/// judges these for each of its requests at interception (ADR 0056), and a
/// `cdp_url` before connecting to it.
///
/// # Errors
///
/// [`Failure::Network`] when the name has no address.
///
/// # Cancel safety
///
/// Dropping the future abandons the lookup; nothing is kept.
pub async fn lookup(host: &str, port: u16, resolve: &Resolve) -> Result<Vec<SocketAddr>, Failure> {
    let server = Server {
        host,
        port,
        plain: true,
    };
    addresses(&server, resolve).await
}

/// `server`'s addresses on its port: the `resolve` mapping's, its literal
/// address, or the system resolver's, each as the resolver gives it, its
/// zone ID included.
async fn addresses(server: &Server<'_>, resolve: &Resolve) -> Result<Vec<SocketAddr>, Failure> {
    let on_port = |address: &IpAddr| SocketAddr::new(*address, server.port);
    if let Some(mapped) = resolve.addresses(server.host) {
        return Ok(mapped.iter().map(on_port).collect());
    }
    if let Ok(address) = server.host.parse::<IpAddr>() {
        return Ok(vec![on_port(&address)]);
    }
    let found: Vec<SocketAddr> = tokio::net::lookup_host((server.host, server.port))
        .await
        .map_err(|_| Failure::Network)?
        .collect();
    if found.is_empty() {
        return Err(Failure::Network);
    }
    Ok(found)
}
