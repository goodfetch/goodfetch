//! The engine's own HTTP client (ADRs 0017, 0043 and 0060): the one place
//! the engine opens an outbound connection on the OS's network, which the
//! Simulation never carries. It sends one HTTPS request over HTTP/1.1 and
//! streams the answer's body, the `resolve` setting applied before any
//! lookup (ADR 0059), Mozilla's roots trusted beside the certificate
//! authorities its caller adds. It adds no header of its own.
//!
//! Every connection it opens races the host's addresses under RFC 8305
//! ([`happy_eyeballs`], ADR 0118). [`connect`] is that step for the HTTP
//! transports that resolve through the engine's [`Dns`]: from the families
//! the Session's local addresses hold, to its proxy's addresses when it has
//! one, forgetting a name whose every address failed.
//!
//! ADR 0043 allows only two kinds of caller: `pinned_download`, the one path
//! for every connection nobody configured, and the module of each kind of
//! Operator-configured endpoint. Which modules call it is `modules.toml`'s
//! list, which a test holds to those kinds.

use std::collections::BTreeMap;
use std::future::ready;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use http_body_util::{BodyExt as _, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};

use crate::clock::Clock;
use crate::dns::{Dns, IpFamily, LookupError, Route, family_of};
use crate::happy_eyeballs::{Delays, Families, Race, RaceError};

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
    /// Certificate authorities trusted beside Mozilla's roots, such as those
    /// of `tls.trusted_cas`.
    pub(crate) trusted_certificates: &'a [CertificateDer<'static>],
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
}

/// Why a request got no answer.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The host is no name TLS can verify.
    ServerName,
    /// No answer, or a connection that broke.
    Network,
}

type Connection = hyper::client::conn::http1::Connection<TokioIo<TlsStream>, Empty<Bytes>>;

#[expect(
    clippy::disallowed_types,
    reason = "the engine's own HTTP client opens its outbound sockets, which the Simulation never carries (ADR 0060)"
)]
type TcpStream = tokio::net::TcpStream;

type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

/// An answer whose body is still arriving.
pub(crate) struct Response {
    /// Its status.
    pub(crate) status: StatusCode,
    /// Its `Content-Length`, when it gave a number.
    pub(crate) content_length: Option<u64>,
    body: Incoming,
    connection: Pin<Box<Connection>>,
    connection_done: bool,
}

impl Response {
    /// The body's next bytes; `None` once it ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no bytes already received.
    pub(crate) async fn chunk(&mut self) -> Result<Option<Bytes>, Failure> {
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

/// Sends `request` to `server` over HTTPS and answers its head; its body
/// arrives through [`Response::chunk`].
///
/// # Cancel safety
///
/// Dropping the future closes any connection it opened.
pub(crate) async fn send(
    server: &Server<'_>,
    request: Request<Empty<Bytes>>,
    network: &Network<'_>,
) -> Result<Response, Failure> {
    let server_name =
        ServerName::try_from(server.host.to_owned()).map_err(|_| Failure::ServerName)?;
    let tcp = race_to(server, network).await?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(tls_config(network)))
        .connect(server_name, tcp)
        .await
        .map_err(|_| Failure::Network)?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| Failure::Network)?;
    let mut connection = Box::pin(connection);
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

fn tls_config(network: &Network<'_>) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for certificate in network.trusted_certificates {
        // A certificate that is no CA certificate trusts nothing; it is the
        // setting's to refuse, not the request's.
        let _ = roots.add(certificate.clone());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("invariant: ring supports rustls's default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}
