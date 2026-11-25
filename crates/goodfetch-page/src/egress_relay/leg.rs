//! The relay's leg out: a TCP connection to a site or to the Session's
//! upstream proxy, inside TLS when the proxy is an `https://` one (issue
//! 1609), verified as an Operator-configured endpoint's under the Run's
//! `Trust` (ADR 0042).

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// A connection the relay opened.
pub(crate) enum Leg {
    /// Bytes as they go: to a site, or to an `http://` or SOCKS5 proxy.
    Plain(TcpStream),
    /// Inside TLS to an `https://` proxy.
    Tls(Box<TlsStream<TcpStream>>),
}

impl Leg {
    /// `tcp` inside TLS to `host` under `config`.
    ///
    /// # Errors
    ///
    /// `InvalidInput` for a host no TLS server name can be, or the
    /// handshake's failure, a certificate the Run does not trust among them.
    pub(crate) async fn tls(
        tcp: TcpStream,
        host: &str,
        config: Arc<rustls::ClientConfig>,
    ) -> io::Result<Self> {
        let name = ServerName::try_from(host.to_owned())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let stream = tokio_rustls::TlsConnector::from(config)
            .connect(name, tcp)
            .await?;
        Ok(Self::Tls(Box::new(stream)))
    }
}

impl AsyncRead for Leg {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_read(context, buffer),
            Self::Tls(stream) => Pin::new(stream).poll_read(context, buffer),
        }
    }
}

impl AsyncWrite for Leg {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_write(context, buffer),
            Self::Tls(stream) => Pin::new(stream).poll_write(context, buffer),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_write_vectored(context, buffers),
            Self::Tls(stream) => Pin::new(stream).poll_write_vectored(context, buffers),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Plain(stream) => stream.is_write_vectored(),
            Self::Tls(stream) => stream.is_write_vectored(),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_flush(context),
            Self::Tls(stream) => Pin::new(stream).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_shutdown(context),
            Self::Tls(stream) => Pin::new(stream).poll_shutdown(context),
        }
    }
}
