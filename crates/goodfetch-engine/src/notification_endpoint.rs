//! A Notification channel's endpoint (ADRs 0017, 0027, 0043 and 0088;
//! issues 1385 and 1388): the server a channel reaches, an endpoint of the
//! Notification channel kind, which each delivery reaches through the
//! engine's own HTTP client, never a Session's, so no Session cookie goes
//! with it: a webhook's URL or an email provider's API, posted to
//! ([`NotificationEndpoint::send`]), or an SMTP server, a mail connection
//! ([`NotificationEndpoint::open_mail`]). The egress policy judges it as its
//! class: an endpoint on loopback or a private network is reached unless
//! `egress.no_private_endpoints` refuses it, each delivery's addresses
//! judged again before it connects. Its certificate chains to Mozilla's
//! roots or to the authorities `tls.trusted_cas` lists for its own domain.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::Request;
use hyper::body::Bytes;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use url::Url;

use crate::clock::Clock;
use crate::egress::{ConfiguredKind, Refused};
use crate::http_client::{EndpointSettings, TcpStream, WholeFailure};
use crate::settings::Resolved;
use crate::tasks::BlockingThreads;

/// The most bytes of a channel's answer a delivery reads: a receiver's
/// acknowledgement, or a provider's, whose body the channel reads at most
/// the start of.
pub const ANSWER_LIMIT: usize = 64 * 1024;

/// A Notification channel's endpoint, as the machine's settings let it be
/// reached.
#[derive(Clone, Debug)]
pub struct NotificationEndpoint {
    endpoint: EndpointSettings,
}

impl NotificationEndpoint {
    /// The endpoint the machine's `settings` let a channel reach: their
    /// `egress.no_private_endpoints`, `network.resolve` and
    /// `tls.trusted_cas`, whose files each delivery reads on `blocking`.
    pub fn of(settings: &Resolved, clock: Clock, blocking: BlockingThreads) -> Self {
        Self {
            endpoint: EndpointSettings::of(settings, clock, blocking),
        }
    }

    /// Judges `url`, a channel's destination, as its configuration is
    /// checked (ADR 0017): its scheme, and a host written as an address
    /// under `egress.no_private_endpoints`, so a refused destination fails
    /// the channel rather than each delivery. A host written as a name is
    /// judged again with its addresses at each delivery.
    ///
    /// # Errors
    ///
    /// [`Refused`] for a scheme other than HTTP's or an address the policy
    /// refuses.
    pub fn judge(&self, url: &Url) -> Result<(), Refused> {
        self.endpoint.judge(url.as_str())
    }

    /// Sends `request`, whose URI is the channel's absolute URL, and
    /// answers the whole answer.
    ///
    /// # Errors
    ///
    /// [`WholeFailure`] when the policy refused the endpoint, its
    /// certificate failed, or it gave no whole answer; the channel bounds
    /// how long it waits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn send(
        &self,
        request: Request<Bytes>,
    ) -> Result<hyper::Response<Bytes>, WholeFailure> {
        self.endpoint
            .send_whole(ConfiguredKind::NotificationChannel, request, ANSWER_LIMIT)
            .await
    }

    /// A connection to the SMTP server `host` on `port`, over TLS from its
    /// first byte when `implicit_tls`, else plain until
    /// [`MailConnection::start_tls`].
    ///
    /// # Errors
    ///
    /// [`WholeFailure`] when the policy refused the server, it has no
    /// address or none connected, or the TLS handshake failed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn open_mail(
        &self,
        host: &str,
        port: u16,
        implicit_tls: bool,
    ) -> Result<MailConnection, WholeFailure> {
        let named = if host.contains(':') {
            format!("smtp://[{host}]:{port}")
        } else {
            format!("smtp://{host}:{port}")
        };
        let tcp = self
            .endpoint
            .connect(ConfiguredKind::NotificationChannel, &named, host, port)
            .await?;
        let connection = MailConnection {
            stream: MailStream::Plain(tcp),
            host: host.to_owned(),
            endpoint: self.endpoint.clone(),
        };
        if implicit_tls {
            connection.start_tls().await
        } else {
            Ok(connection)
        }
    }
}

/// A connection to a Notification channel's SMTP server, plain or over TLS.
#[derive(Debug)]
pub struct MailConnection {
    stream: MailStream,
    host: String,
    endpoint: EndpointSettings,
}

/// A mail connection's stream.
#[derive(Debug)]
enum MailStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl MailConnection {
    /// The same connection over TLS, the server's certificate verified as an
    /// HTTPS endpoint's is: after the server answered `STARTTLS`, or at once
    /// for implicit TLS. A connection already over TLS stays as it is.
    ///
    /// # Errors
    ///
    /// [`WholeFailure`] when the certificate failed, the client certificate
    /// listed for the server cannot be presented, or the handshake broke.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection.
    pub async fn start_tls(self) -> Result<Self, WholeFailure> {
        let Self {
            stream,
            host,
            endpoint,
        } = self;
        let tcp = match stream {
            MailStream::Plain(tcp) => tcp,
            tls @ MailStream::Tls(_) => {
                return Ok(Self {
                    stream: tls,
                    host,
                    endpoint,
                });
            }
        };
        let tls = endpoint.secure(&host, tcp).await?;
        Ok(Self {
            stream: MailStream::Tls(Box::new(tls)),
            host,
            endpoint,
        })
    }

    /// Whether the connection is over TLS.
    pub fn is_secure(&self) -> bool {
        matches!(self.stream, MailStream::Tls(_))
    }

    /// The name the client gives in `EHLO`: its own address as an address
    /// literal (RFC 5321 section 4.1.3), `[192.0.2.1]` or `[IPv6:2001:db8::1]`,
    /// since the engine has no name of its own a server could check.
    pub fn local_name(&self) -> String {
        let tcp = match &self.stream {
            MailStream::Plain(tcp) => tcp,
            MailStream::Tls(tls) => tls.get_ref().0,
        };
        match tcp.local_addr() {
            Ok(SocketAddr::V4(address)) => format!("[{}]", address.ip()),
            Ok(SocketAddr::V6(address)) => format!("[IPv6:{}]", address.ip()),
            Err(_) => "[127.0.0.1]".to_owned(),
        }
    }
}

impl AsyncRead for MailConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.get_mut().stream {
            MailStream::Plain(tcp) => Pin::new(tcp).poll_read(context, buffer),
            MailStream::Tls(tls) => Pin::new(tls.as_mut()).poll_read(context, buffer),
        }
    }
}

impl AsyncWrite for MailConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().stream {
            MailStream::Plain(tcp) => Pin::new(tcp).poll_write(context, bytes),
            MailStream::Tls(tls) => Pin::new(tls.as_mut()).poll_write(context, bytes),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().stream {
            MailStream::Plain(tcp) => Pin::new(tcp).poll_flush(context),
            MailStream::Tls(tls) => Pin::new(tls.as_mut()).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().stream {
            MailStream::Plain(tcp) => Pin::new(tcp).poll_shutdown(context),
            MailStream::Tls(tls) => Pin::new(tls.as_mut()).poll_shutdown(context),
        }
    }
}
