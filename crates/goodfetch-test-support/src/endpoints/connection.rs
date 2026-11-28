//! The runner's connections to the listener under test: TCP, over TLS when
//! the listener terminates it, or a Unix socket. They are blocking, with a
//! deadline on every read, since the runner tries one attempt at a time and
//! reads no clock.
#![expect(
    clippy::disallowed_types,
    reason = "the endpoint table's runner is a test helper that connects to a product's listener from outside it, never inside the Simulation (ADRs 0060 and 0067)"
)]

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

/// How long the runner waits to connect, and for each read of an answer,
/// before it records that the listener stayed silent.
pub(super) const DEADLINE: Duration = Duration::from_secs(5);

/// Where a listener listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerAddress {
    /// A TCP address, loopback for every local listener.
    Tcp(SocketAddr),
    /// A Unix socket's path.
    #[cfg(unix)]
    Unix(PathBuf),
}

/// The TLS a listener terminates, as its Clients reach it: the name they
/// reach it by, which its certificate is for, the certificate or authority
/// they trust it with, and optionally the client certificate and PKCS #8 key
/// a mutual-TLS peer presents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerTls {
    server_name: String,
    trusted: CertificateDer<'static>,
    client: Option<(CertificateDer<'static>, Vec<u8>)>,
}

impl ListenerTls {
    /// A listener reached as `server_name`, its certificate trusted as
    /// `trusted`, in DER.
    pub fn new(server_name: impl Into<String>, trusted: impl Into<Vec<u8>>) -> Self {
        Self {
            server_name: server_name.into(),
            trusted: CertificateDer::from(trusted.into()),
            client: None,
        }
    }

    /// Presents `certificate` and PKCS #8 private `key` (both DER) during the
    /// TLS handshake, for mutual-TLS listeners such as the peer link.
    #[must_use]
    pub fn with_client_certificate(
        mut self,
        certificate: impl Into<Vec<u8>>,
        key: impl Into<Vec<u8>>,
    ) -> Self {
        self.client = Some((CertificateDer::from(certificate.into()), key.into()));
        self
    }

    /// The name its Clients reach it by.
    pub(super) fn server_name(&self) -> &str {
        &self.server_name
    }

    /// A Client's TLS connection to it, named by its server name, which
    /// trusts only its certificate.
    fn client(&self) -> io::Result<ClientConnection> {
        let invalid = |error: &dyn std::fmt::Display| {
            io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
        };
        let mut roots = RootCertStore::empty();
        roots
            .add(self.trusted.clone())
            .map_err(|error| invalid(&error))?;
        let builder =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|error| invalid(&error))?
                .with_root_certificates(roots);
        let config = match &self.client {
            None => builder.with_no_client_auth(),
            Some((certificate, key)) => builder
                .with_client_auth_cert(
                    vec![certificate.clone()],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.clone())),
                )
                .map_err(|error| invalid(&error))?,
        };
        let name =
            ServerName::try_from(self.server_name.clone()).map_err(|error| invalid(&error))?;
        ClientConnection::new(Arc::new(config), name).map_err(|error| invalid(&error))
    }
}

/// One open connection.
pub(super) enum Connection {
    Tcp(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Connection {
    /// Connects to `address`, over `tls` if any, with `DEADLINE` on every
    /// read.
    pub(super) fn open(address: &ListenerAddress, tls: Option<&ListenerTls>) -> io::Result<Self> {
        match address {
            ListenerAddress::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, DEADLINE)?;
                stream.set_read_timeout(Some(DEADLINE))?;
                match tls {
                    None => Ok(Self::Tcp(stream)),
                    Some(tls) => Ok(Self::Tls(Box::new(StreamOwned::new(tls.client()?, stream)))),
                }
            }
            #[cfg(unix)]
            ListenerAddress::Unix(path) => {
                if tls.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a Unix socket's listener terminates no TLS",
                    ));
                }
                let stream = UnixStream::connect(path)?;
                stream.set_read_timeout(Some(DEADLINE))?;
                Ok(Self::Unix(stream))
            }
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
        }
    }
}

/// Whether `error` means the listener closed, reset, or rejected the
/// connection with a TLS alert.
pub(super) fn is_closed(error: &io::Error) -> bool {
    let tls_alert = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .is_some_and(|inner| matches!(inner, rustls::Error::AlertReceived(_)));
    tls_alert
        || matches!(
            error.kind(),
            io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
        )
}

/// Whether `error` means the deadline passed with no data.
pub(super) fn is_silent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
