//! The engine's own HTTP client (ADRs 0017, 0043 and 0060): the one place
//! the engine opens an outbound connection on the OS's network, which the
//! Simulation never carries. It sends one HTTP or HTTPS request over
//! HTTP/1.1 and streams the answer's body, the `resolve` setting applied
//! before any lookup (ADR 0059), every certificate verified by [`tls`]'s
//! per-host decision: Mozilla's roots, and the certificate authorities
//! trusted for the host's domain (ADR 0042). A certificate that fails ends
//! the request with `certificate_invalid` before anything is sent. It adds
//! no header of its own.
//!
//! Every connection it opens races the host's addresses under RFC 8305
//! ([`happy_eyeballs`], ADR 0118). [`connect`] is that step for an HTTP
//! transport that resolves through the engine's [`Dns`]: from the families
//! the Session's local addresses hold, to its proxy's addresses when it has
//! one, forgetting a name whose every address failed. No Product's
//! transport takes it yet: Foxhound's HTTP tier hands the addresses its
//! resolver judged to its own connectors.
//!
//! A Session with local addresses (ADR 0044) resolves names and connects
//! through [`LocalBinding`], as the egress relay of a browser Session does:
//! its lookups go through the engine's own resolver, their queries sent
//! from the Session's address, and each connection leaves from its address
//! of the family it connects over.
//!
//! ADR 0043 allows only two kinds of caller:
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
//! and request all serve the same request on the same connection.

use std::collections::BTreeMap;
use std::future::ready;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use http_body_util::BodyExt as _;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use url::Host;

use crate::clock::Clock;
use crate::dns::{Dns, HostMapping, IpFamily, LookupError, Resolution, Route, family_of};
use crate::egress::{ConfiguredKind, EndpointPolicy, Gate, Refused, Stopped};
use crate::happy_eyeballs::{Delays, Families, Race, RaceError};
use crate::local_addresses::SessionAddresses;
use crate::response_limits::Limits;
use crate::settings::Resolved;
use crate::tls::{self, CertificateInvalid, Trust, TrustedCas};

#[cfg(test)]
mod tests;

/// What a request may reach and trust, and how it races a host's addresses.
pub(crate) struct Network<'a> {
    /// The `resolve` setting.
    pub(crate) resolve: &'a HostMapping,
    /// What each host's certificate may chain to beside Mozilla's roots.
    pub(crate) trust: &'a Trust,
    /// What the race waits on.
    pub(crate) clock: Clock,
    /// The race's delays.
    pub(crate) delays: Delays,
}

/// A Session's local addresses (ADR 0044): what its connections bind, and
/// the engine's resolution whose queries leave from them.
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

    fn of(&self, family: IpFamily) -> Option<IpAddr> {
        match family {
            IpFamily::Ipv4 => self.addresses.ipv4.map(IpAddr::V4),
            IpFamily::Ipv6 => self.addresses.ipv6.map(IpAddr::V6),
        }
    }

    /// `server`'s addresses on its port: its literal address, the
    /// `resolve` mapping's, or the engine's own resolver's, asked from the
    /// Session's addresses.
    async fn addresses(
        &self,
        server: &Server<'_>,
        resolve: &HostMapping,
    ) -> Result<Vec<SocketAddr>, Failure> {
        let on_port = |address: &IpAddr| SocketAddr::new(*address, server.port);
        if let Some(fixed) = resolve.fixed(server.host) {
            return Ok(fixed.iter().map(on_port).collect());
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
    /// of a browser Session (ADR 0044): its literal address, the `resolve`
    /// mapping's, or the engine's own resolver's, whose queries leave
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
        resolve: &HostMapping,
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
    pub(crate) resolve: &'a HostMapping,
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
    /// through [`Response::chunk`]. Its [`Gate`] judges the URL before any
    /// lookup, then every address its host stands for before any
    /// connection, which tries only those; an invalid certificate ends it
    /// with `certificate_invalid` whatever `tls.accept_invalid_certs`
    /// lists.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any connection it opened.
    pub(crate) async fn send<B: RequestBody>(
        &self,
        mut request: Request<B>,
    ) -> Result<Response, EndpointFailure> {
        let failed = |failure| EndpointFailure::Failed(failure);
        let url = request.uri().to_string();
        let passed = Gate::Endpoint(self.policy)
            .pass(&url, async |host, port| {
                let server = Server {
                    host,
                    port,
                    plain: true,
                };
                addresses(&server, self.resolve).await
            })
            .await
            .map_err(|stopped| match stopped {
                Stopped::Refused(refused) | Stopped::AddressRefused(refused) => {
                    EndpointFailure::Refused(refused)
                }
                Stopped::Unresolved(failure) => failed(failure),
            })?;
        let url = &passed.url;
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
        tracing::debug!(
            kind = self.kind.classed().name,
            "connecting to an Operator-configured endpoint"
        );
        let network = Network {
            resolve: self.resolve,
            trust: self.trust,
            clock: self.clock,
            delays: self.delays,
        };
        send_to(&server, &passed.addresses, request, &network)
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
    resolve: HostMapping,
    trusted_cas: TrustedCas,
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
        Self {
            policy: EndpointPolicy::of(settings),
            resolve: HostMapping::of(settings),
            trusted_cas: TrustedCas::of(settings),
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
    /// `tls.trusted_cas` lists for each domain, read now, as
    /// [`Trust::of`] reads them for a site; a file that cannot be read is
    /// logged and trusts nothing.
    async fn trust(&self) -> Trust {
        let mut read = BTreeMap::new();
        for file in self.trusted_cas.files() {
            read.insert(file.to_owned(), tokio::fs::read(file).await);
        }
        self.trusted_cas.trust(|file| match read.get(file) {
            Some(Ok(bytes)) => Ok(bytes.clone()),
            Some(Err(error)) => Err(io::Error::new(error.kind(), error.to_string())),
            None => Err(io::ErrorKind::NotFound.into()),
        })
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
    send_to(server, &found, request, network).await
}

/// Sends `request` to `server` at one of `found`, its addresses, over
/// HTTPS ([`open_tls`]) unless the server is plain, and answers its head.
///
/// # Cancel safety
///
/// Dropping the future closes any connection it opened.
async fn send_to<B: RequestBody>(
    server: &Server<'_>,
    found: &[SocketAddr],
    request: Request<B>,
    network: &Network<'_>,
) -> Result<Response, Failure> {
    let (mut sender, mut connection) = if server.plain {
        let tcp = open(server, found, network).await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|_| Failure::Network)?;
        (sender, Box::pin(connection) as Connection)
    } else {
        let tls = open_tls(server, found, network).await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|_| Failure::Network)?;
        (sender, Box::pin(connection) as Connection)
    };
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
    })
}

/// A TLS connection to `server` at one of `found`, its certificate
/// verified under the network's trust before anything is sent.
///
/// # Cancel safety
///
/// Dropping the future closes any connection it opened.
async fn open_tls(
    server: &Server<'_>,
    found: &[SocketAddr],
    network: &Network<'_>,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Failure> {
    let server_name =
        ServerName::try_from(server.host.to_owned()).map_err(|_| Failure::ServerName)?;
    let tcp = open(server, found, network).await?;
    let config = network.trust.client_config(network.clock);
    tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(server_name, tcp)
        .await
        .map_err(|error| {
            tls::certificate_invalid(&error, server.host)
                .map_or(Failure::Network, Failure::CertificateInvalid)
        })
}

/// A connection to `server` at one of `found`, its addresses.
async fn open(
    server: &Server<'_>,
    found: &[SocketAddr],
    network: &Network<'_>,
) -> Result<TcpStream, Failure> {
    race_to(server.host, server.port, found, network)
        .await
        .map_err(|_| Failure::Network)
}

/// Connects to one of `found`, the addresses of `name` on `port`, both
/// families answered at once, raced under RFC 8305: it tries no other
/// address.
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
        families: Families::Both,
    };
    race.connect(
        name,
        port,
        ready(of(IpFamily::Ipv6)),
        ready(of(IpFamily::Ipv4)),
        |address| attempt(address, None),
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
/// before connecting: its literal address (an IPv6 one without brackets),
/// the `resolve` mapping's, or the system resolver's, its zone ID
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
pub async fn lookup(
    host: &str,
    port: u16,
    resolve: &HostMapping,
) -> Result<Vec<SocketAddr>, Failure> {
    let server = Server {
        host,
        port,
        plain: true,
    };
    addresses(&server, resolve).await
}

/// `server`'s addresses on its port: its literal address, the `resolve`
/// mapping's, or the system resolver's, each as the resolver gives it, its
/// zone ID included.
async fn addresses(server: &Server<'_>, resolve: &HostMapping) -> Result<Vec<SocketAddr>, Failure> {
    let on_port = |address: &IpAddr| SocketAddr::new(*address, server.port);
    if let Some(fixed) = resolve.fixed(server.host) {
        return Ok(fixed.iter().map(on_port).collect());
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
