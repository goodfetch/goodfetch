//! The engine's own HTTP client (ADRs 0017, 0043 and 0060): the one place
//! the engine opens an outbound connection on the OS's network, which the
//! Simulation never carries. It sends one HTTPS request over HTTP/1.1 and
//! streams the answer's body, the `resolve` setting applied before any
//! lookup (ADR 0059), Mozilla's roots trusted beside the certificate
//! authorities its caller adds. It adds no header of its own.
//!
//! ADR 0043 allows only two kinds of caller: `pinned_download`, the one path
//! for every connection nobody configured, and the module of each kind of
//! Operator-configured endpoint. Which modules call it is `modules.toml`'s
//! list, which a test holds to those kinds.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use http_body_util::{BodyExt as _, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};

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

/// What a request may reach and trust.
pub(crate) struct Network<'a> {
    /// The `resolve` setting.
    pub(crate) resolve: &'a Resolve,
    /// Certificate authorities trusted beside Mozilla's roots, such as those
    /// of `tls.trusted_cas`.
    pub(crate) trusted_certificates: &'a [CertificateDer<'static>],
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
type TlsStream = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

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
    let tcp = connect(&addresses(server, network.resolve).await?).await?;
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

/// Connects to the first of `addresses` that accepts, in order.
#[expect(
    clippy::disallowed_types,
    reason = "the engine's own HTTP client opens its outbound sockets, which the Simulation never carries (ADR 0060)"
)]
async fn connect(addresses: &[SocketAddr]) -> Result<tokio::net::TcpStream, Failure> {
    for address in addresses {
        if let Ok(stream) = tokio::net::TcpStream::connect(address).await {
            return Ok(stream);
        }
    }
    Err(Failure::Network)
}

async fn addresses(server: &Server<'_>, resolve: &Resolve) -> Result<Vec<SocketAddr>, Failure> {
    if let Some(mapped) = resolve.addresses(server.host) {
        return Ok(mapped
            .iter()
            .map(|address| SocketAddr::new(*address, server.port))
            .collect());
    }
    if let Ok(address) = server.host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(address, server.port)]);
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
