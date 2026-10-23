//! The per-Session loopback relay (ADR 0037 / SA-22; ADR 0021): the
//! browser connects to this loopback listener instead of the Session's
//! proxy, and the relay opens a connection to the proxy for each of the
//! browser's. It relays in one of two ways:
//!
//! - **SOCKS5 with credentials.** Chromium takes no SOCKS5 credentials, so
//!   the relay asks the browser for none and authenticates each connection
//!   upstream with the Session's username and password (RFC 1928, RFC
//!   1929). The password goes only to the proxy, over the connection the
//!   relay opens to it; it is never in the browser's arguments.
//! - **As it is.** An HTTP or credential-free SOCKS5 proxy's connections
//!   pass byte for byte; one the proxy does not accept is answered, for an
//!   HTTP proxy, with the relay's own `502` whose `Proxy-Status` names
//!   `connection_refused` (RFC 9209), so it stays the proxy's failure.
//!
//! A Session whose proxy a Proxy provider gave relays every connection, so
//! every byte on its connections to the proxy, the handshakes included, is
//! charged to its Run, under its proxy kind, before the browser or the
//! proxy is given it ([`ProxyUsage`]); once a Limit refuses a charge, the
//! connection closes. The listener binds port 0 of loopback and reports its
//! address (ADR 0059), and it closes, with every connection it relays, when
//! the Session's proxy is dropped or its Run stops.
#![expect(
    clippy::disallowed_types,
    reason = "the per-Session SOCKS5 relay opens its own loopback listener and its connections to the proxy, in the proxy module only, and its tasks set one flag, read with no lock order (ADR 0060)"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use goodfetch_engine::runs::Run;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::proxy_provider::ProxyUsage;

/// The most bytes relayed at once, in each direction.
const CHUNK: usize = 16 * 1024;

/// The relay's answer to the browser when an HTTP proxy cannot be reached.
const UNREACHABLE: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\nProxy-Status: goodfetch-relay; error=connection_refused\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// SOCKS's version 5.
const VERSION: u8 = 5;
/// The method that asks for no credentials.
const NO_AUTHENTICATION: u8 = 0;
/// The username and password method (RFC 1929).
const USERNAME_PASSWORD: u8 = 2;
/// No method the relay takes.
const NO_ACCEPTABLE_METHOD: u8 = 0xff;
/// RFC 1929's version.
const CREDENTIALS_VERSION: u8 = 1;
/// The `CONNECT` command, the only one the browser sends.
const CONNECT: u8 = 1;
/// Replies: a general failure, a refusal by the proxy's rules, and an
/// unsupported command.
const GENERAL_FAILURE: u8 = 1;
const NOT_ALLOWED: u8 = 2;
const COMMAND_NOT_SUPPORTED: u8 = 7;
/// The address types: IPv4, a domain name and IPv6.
const IPV4: u8 = 1;
const DOMAIN: u8 = 3;
const IPV6: u8 = 4;

/// How the relay relays a connection.
pub(crate) enum Mode {
    /// SOCKS5, authenticating upstream with these credentials.
    Socks5 {
        username: String,
        password: SecretString,
    },
    /// Byte for byte, to an HTTP proxy (`http`) or a SOCKS5 one.
    AsItIs { http: bool },
}

/// Where the relay sends each connection: the proxy, how, and what its
/// bytes are charged to.
struct Upstream {
    address: SocketAddr,
    mode: Mode,
    /// Whether the proxy refused the credentials, which a failed request
    /// then raises as `proxy_auth_failed`.
    refused: AtomicBool,
    /// What the bytes on the connections to the proxy are charged to, for
    /// a Proxy provider's proxy.
    usage: Option<Arc<ProxyUsage>>,
}

/// An open relay: its loopback address, closed when dropped.
pub(crate) struct Relay {
    address: SocketAddr,
    upstream: Arc<Upstream>,
    _closed_when_dropped: DropGuard,
}

impl Relay {
    /// Binds the relay on port 0 of loopback for the proxy at `proxy`, and
    /// relays every connection the browser opens to it as a task of `run`,
    /// as `mode` says, charging its bytes to `usage` when it has one.
    ///
    /// # Errors
    ///
    /// The bind's failure.
    pub(crate) async fn open(
        run: &Run,
        proxy: SocketAddr,
        mode: Mode,
        usage: Option<Arc<ProxyUsage>>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let upstream = Arc::new(Upstream {
            address: proxy,
            mode,
            refused: AtomicBool::new(false),
            usage,
        });
        let stop = CancellationToken::new();
        run.spawn(accept(
            listener,
            Arc::clone(&upstream),
            run.clone(),
            stop.clone(),
        ));
        Ok(Self {
            address,
            upstream,
            _closed_when_dropped: stop.drop_guard(),
        })
    }

    /// Its loopback address, which the browser takes as its proxy.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// Whether the proxy refused the Session's credentials.
    pub(crate) fn refused_credentials(&self) -> bool {
        self.upstream.refused.load(Ordering::Relaxed)
    }
}

/// Accepts the browser's connections until `stop` or the Run's end, each
/// relayed on a task of its own.
async fn accept(listener: TcpListener, upstream: Arc<Upstream>, run: Run, stop: CancellationToken) {
    let ending = run.cancellation().clone();
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => return,
            () = ending.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let Ok((browser, _)) = accepted else {
            // A connection that failed before it was accepted concerns
            // only the browser.
            continue;
        };
        let upstream = Arc::clone(&upstream);
        let stop = stop.clone();
        let ending = ending.clone();
        run.spawn(async move {
            tokio::select! {
                () = relay(browser, &upstream) => {}
                () = stop.cancelled() => {}
                () = ending.cancelled() => {}
            }
        });
    }
}

/// Relays one of the browser's connections through the proxy.
async fn relay(browser: TcpStream, upstream: &Upstream) {
    match &upstream.mode {
        Mode::Socks5 { username, password } => {
            socks5(browser, upstream, username, password).await;
        }
        Mode::AsItIs { http } => as_it_is(browser, upstream, *http).await,
    }
}

/// Relays a connection byte for byte; one the proxy does not accept is
/// answered with the relay's `502` when the proxy is an HTTP one.
async fn as_it_is(mut browser: TcpStream, upstream: &Upstream, http: bool) {
    let Ok(proxy) = TcpStream::connect(upstream.address).await else {
        if http {
            let _ = browser.write_all(UNREACHABLE).await;
        }
        return;
    };
    let mut proxy = Tallied::new(proxy, upstream.usage.as_deref());
    let _ = pass(&mut browser, &mut proxy).await;
}

/// Relays a SOCKS5 connection, authenticating upstream with `username` and
/// `password`.
async fn socks5(
    mut browser: TcpStream,
    upstream: &Upstream,
    username: &str,
    password: &SecretString,
) {
    if greet_browser(&mut browser).await.is_err() {
        return;
    }
    let Ok(request) = read_request(&mut browser).await else {
        return;
    };
    if request[1] != CONNECT {
        let _ = browser.write_all(&reply(COMMAND_NOT_SUPPORTED)).await;
        return;
    }
    let Ok(proxy) = TcpStream::connect(upstream.address).await else {
        let _ = browser.write_all(&reply(GENERAL_FAILURE)).await;
        return;
    };
    let mut proxy = Tallied::new(proxy, upstream.usage.as_deref());
    match authenticate(&mut proxy, username, password).await {
        Ok(true) => {}
        Ok(false) => {
            upstream.refused.store(true, Ordering::Relaxed);
            let _ = browser.write_all(&reply(NOT_ALLOWED)).await;
            return;
        }
        Err(_) => {
            let _ = browser.write_all(&reply(GENERAL_FAILURE)).await;
            return;
        }
    }
    // The browser's own request goes on as it sent it, and the proxy's
    // reply comes back as the proxy sent it.
    if proxy.write_all(&request).await.is_err() || !proxy.charged().await {
        return;
    }
    let Ok(answered) = read_request(&mut proxy).await else {
        let _ = browser.write_all(&reply(GENERAL_FAILURE)).await;
        return;
    };
    if !proxy.charged().await || browser.write_all(&answered).await.is_err() || answered[1] != 0 {
        return;
    }
    let _ = pass(&mut browser, &mut proxy).await;
}

/// The proxy's end of a relayed connection, which tallies every byte read
/// from it or written to it; its owner charges them at [`Tallied::charged`].
struct Tallied<'usage> {
    stream: TcpStream,
    usage: Option<&'usage ProxyUsage>,
    /// The bytes carried since the last charge.
    uncharged: u64,
}

impl<'usage> Tallied<'usage> {
    fn new(stream: TcpStream, usage: Option<&'usage ProxyUsage>) -> Self {
        Self {
            stream,
            usage,
            uncharged: 0,
        }
    }

    /// Charges the bytes carried since the last charge: whether the
    /// connection may carry more, false once a Limit refused them.
    async fn charged(&mut self) -> bool {
        let bytes = std::mem::take(&mut self.uncharged);
        match self.usage {
            Some(usage) => usage.charge(bytes).await,
            None => true,
        }
    }

    fn tally(&mut self, bytes: usize) {
        self.uncharged = self
            .uncharged
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }
}

impl AsyncRead for Tallied<'_> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        let polled = std::pin::Pin::new(&mut this.stream).poll_read(context, buffer);
        let read = buffer.filled().len() - before;
        this.tally(read);
        polled
    }
}

impl AsyncWrite for Tallied<'_> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let polled = std::pin::Pin::new(&mut this.stream).poll_write(context, bytes);
        if let std::task::Poll::Ready(Ok(written)) = &polled {
            this.tally(*written);
        }
        polled
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_flush(context)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_shutdown(context)
    }
}

/// Relays bytes both ways between the browser and the proxy until both
/// ends close, charging what the proxy sends before the browser is given
/// it and what the browser sends once the proxy took it; it stops at the
/// first refused charge.
async fn pass(browser: &mut TcpStream, proxy: &mut Tallied<'_>) -> std::io::Result<()> {
    let mut up = vec![0; CHUNK];
    let mut down = vec![0; CHUNK];
    let (mut browser_open, mut proxy_open) = (true, true);
    while browser_open || proxy_open {
        tokio::select! {
            read = browser.read(&mut up), if browser_open => {
                let read = read?;
                if read == 0 {
                    browser_open = false;
                    proxy.shutdown().await?;
                } else {
                    proxy.write_all(&up[..read]).await?;
                }
                if !proxy.charged().await {
                    return Err(std::io::Error::other("a Limit refused the proxy's bytes"));
                }
            }
            read = proxy.read(&mut down), if proxy_open => {
                let read = read?;
                if !proxy.charged().await {
                    return Err(std::io::Error::other("a Limit refused the proxy's bytes"));
                }
                if read == 0 {
                    proxy_open = false;
                    browser.shutdown().await?;
                } else {
                    browser.write_all(&down[..read]).await?;
                }
            }
        }
    }
    Ok(())
}

/// Reads the browser's greeting and asks it for no credentials.
async fn greet_browser(browser: &mut TcpStream) -> std::io::Result<()> {
    let mut head = [0; 2];
    browser.read_exact(&mut head).await?;
    let mut methods = vec![0; usize::from(head[1])];
    browser.read_exact(&mut methods).await?;
    if head[0] != VERSION || !methods.contains(&NO_AUTHENTICATION) {
        browser.write_all(&[VERSION, NO_ACCEPTABLE_METHOD]).await?;
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    browser.write_all(&[VERSION, NO_AUTHENTICATION]).await
}

/// Reads a request or a reply, which share their shape: version, command
/// or reply, a reserved byte, then an address and a port.
async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> std::io::Result<Vec<u8>> {
    let mut message = vec![0; 4];
    stream.read_exact(&mut message).await?;
    if message[0] != VERSION {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let address = match message[3] {
        IPV4 => 4,
        IPV6 => 16,
        DOMAIN => {
            let mut length = [0; 1];
            stream.read_exact(&mut length).await?;
            message.push(length[0]);
            usize::from(length[0])
        }
        _ => return Err(std::io::ErrorKind::InvalidData.into()),
    };
    let start = message.len();
    message.resize(start + address + 2, 0);
    stream.read_exact(&mut message[start..]).await?;
    Ok(message)
}

/// Authenticates with the proxy as `username`: whether it took the
/// credentials.
async fn authenticate(
    proxy: &mut Tallied<'_>,
    username: &str,
    password: &SecretString,
) -> std::io::Result<bool> {
    proxy.write_all(&[VERSION, 1, USERNAME_PASSWORD]).await?;
    let mut chosen = [0; 2];
    proxy.read_exact(&mut chosen).await?;
    match chosen {
        [VERSION, NO_AUTHENTICATION] => return Ok(true),
        [VERSION, USERNAME_PASSWORD] => {}
        _ => return Ok(false),
    }
    let username = username.as_bytes();
    let password = password.expose_secret().as_bytes();
    let (Ok(username_length), Ok(password_length)) =
        (u8::try_from(username.len()), u8::try_from(password.len()))
    else {
        return Ok(false);
    };
    let mut credentials = Vec::with_capacity(3 + username.len() + password.len());
    credentials.extend_from_slice(&[CREDENTIALS_VERSION, username_length]);
    credentials.extend_from_slice(username);
    credentials.push(password_length);
    credentials.extend_from_slice(password);
    let sent = proxy.write_all(&credentials).await;
    // The buffer that held the password is cleared before it is dropped.
    credentials.fill(0);
    sent?;
    let mut status = [0; 2];
    proxy.read_exact(&mut status).await?;
    Ok(status == [CREDENTIALS_VERSION, 0])
}

/// A reply to the browser carrying `code` and no address.
fn reply(code: u8) -> [u8; 10] {
    [VERSION, code, 0, IPV4, 0, 0, 0, 0, 0, 0]
}
