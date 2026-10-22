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
//! lookup or connection (ADRs 0014 and 0017): a refusal ends the call that
//! waited for it with `egress_refused`, or, when no call waits, is a
//! `page.request_failed` event in the Run trace.
//!
//! ADR 0043 allows only two kinds of caller beside a run's own requests:
//! `pinned_download`, the one path for every connection nobody configured,
//! and the module of each kind of Operator-configured endpoint. Which
//! modules call it is `modules.toml`'s list, which a test holds to those
//! kinds.

use std::collections::BTreeMap;
use std::future::ready;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::page::RequestFailed;
use http_body_util::{BodyExt as _, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use url::Host;

use crate::clock::Clock;
use crate::dns::{Dns, IpFamily, LookupError, Route, family_of};
use crate::egress::{self, Judgment, Refused};
use crate::happy_eyeballs::{Delays, Families, Race, RaceError};
use crate::run_trace::RequestLog;
use crate::tls::{self, CertificateInvalid, Trust};

#[cfg(test)]
mod tests;

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
    /// What the race waits on.
    pub(crate) clock: Clock,
    /// The race's delays.
    pub(crate) delays: Delays,
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
    /// What each host's certificate may chain to beside Mozilla's roots:
    /// the certificate authorities `tls.trusted_cas` lists for its domain.
    pub trust: Trust,
    /// What the Happy Eyeballs race waits on (ADR 0118).
    pub clock: Clock,
    /// `resolver.resolution_delay` and `resolver.attempt_delay`.
    pub delays: Delays,
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
    /// its head; its body arrives through [`Response::chunk`]. A refusal
    /// opens no connection and makes no lookup; when nobody waits for the
    /// request, it is also recorded in the Run trace.
    ///
    /// # Errors
    ///
    /// [`FetchError::Refused`] when the policy refuses it, whose
    /// [`Refused::error`] is `egress_refused`;
    /// [`FetchError::UnsupportedScheme`] for an allowed scheme other than
    /// `http` and `https`; [`FetchError::Failed`] when it got no answer.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any connection it opened.
    pub async fn get(&self, url: &str, waiter: Waiter<'_>) -> Result<Response, FetchError> {
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
            clock: self.clock,
            delays: self.delays,
        };
        let server = Server {
            host: &host,
            port,
            plain,
        };
        send(&server, request, &network)
            .await
            .map_err(FetchError::Failed)
    }
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
pub(crate) async fn send(
    server: &Server<'_>,
    request: Request<Empty<Bytes>>,
    network: &Network<'_>,
) -> Result<Response, Failure> {
    let (mut sender, mut connection) = if server.plain {
        let tcp = race_to(server, network).await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|_| Failure::Network)?;
        (sender, Box::pin(connection) as Connection)
    } else {
        let server_name =
            ServerName::try_from(server.host.to_owned()).map_err(|_| Failure::ServerName)?;
        let tcp = race_to(server, network).await?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(network.trust.client_config()))
            .connect(server_name, tcp)
            .await
            .map_err(|error| {
                tls::certificate_invalid(&error, server.host)
                    .map_or(Failure::Network, Failure::CertificateInvalid)
            })?;
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
    Ok(Response {
        status: head.status(),
        content_length,
        body: head.into_body(),
        connection,
        connection_done,
    })
}

/// Connects to `server`'s addresses, both families answered at once,
/// raced under RFC 8305.
async fn race_to(server: &Server<'_>, network: &Network<'_>) -> Result<TcpStream, Failure> {
    let found = addresses(server, network.resolve).await?;
    let of = |family: IpFamily| -> Result<Vec<IpAddr>, LookupError> {
        Ok(found
            .iter()
            .copied()
            .filter(|address| family_of(*address) == family)
            .collect())
    };
    let race = Race {
        clock: network.clock,
        delays: network.delays,
        families: Families::Both,
    };
    race.connect(
        server.host,
        server.port,
        ready(of(IpFamily::Ipv6)),
        ready(of(IpFamily::Ipv4)),
        |address| attempt(address, None),
    )
    .await
    .map(|won| won.connection)
    .map_err(|_| Failure::Network)
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

/// `server`'s addresses: the `resolve` mapping's, its literal address, or
/// the system resolver's.
async fn addresses(server: &Server<'_>, resolve: &Resolve) -> Result<Vec<IpAddr>, Failure> {
    if let Some(mapped) = resolve.addresses(server.host) {
        return Ok(mapped.to_vec());
    }
    if let Ok(address) = server.host.parse::<IpAddr>() {
        return Ok(vec![address]);
    }
    let found: Vec<IpAddr> = tokio::net::lookup_host((server.host, server.port))
        .await
        .map_err(|_| Failure::Network)?
        .map(|address| address.ip())
        .collect();
    if found.is_empty() {
        return Err(Failure::Network);
    }
    Ok(found)
}
