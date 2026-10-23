//! The per-Session SOCKS5 relay (ADR 0037 / SA-22): Chromium takes no SOCKS5
//! credentials, so a Session whose SOCKS5 proxy needs them has the browser
//! connect to this loopback listener instead, which asks the browser for no
//! credentials and authenticates each connection upstream with the
//! Session's username and password (RFC 1928, RFC 1929). The password goes
//! only to the proxy, over the connection the relay opens to it; it is never
//! in the browser's arguments. The listener binds port 0 of loopback and
//! reports its address (ADR 0059), and it closes, with every connection it
//! relays, when the Session's proxy is dropped or its Run stops.
#![expect(
    clippy::disallowed_types,
    reason = "the per-Session SOCKS5 relay opens its own loopback listener and its connections to the proxy, in the proxy module only, and its tasks set one flag, read with no lock order (ADR 0060)"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use goodfetch_engine::runs::Run;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

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

/// Where the relay sends each connection: the proxy, and the credentials it
/// gives it.
struct Upstream {
    address: SocketAddr,
    username: String,
    password: SecretString,
    /// Whether the proxy refused the credentials, which a failed request
    /// then raises as `proxy_auth_failed`.
    refused: AtomicBool,
}

/// An open relay: its loopback address, closed when dropped.
pub(crate) struct Relay {
    address: SocketAddr,
    upstream: Arc<Upstream>,
    _closed_when_dropped: DropGuard,
}

impl Relay {
    /// Binds the relay on port 0 of loopback for the proxy at `proxy`, and
    /// relays every connection the browser opens to it as a task of `run`.
    ///
    /// # Errors
    ///
    /// The bind's failure.
    pub(crate) async fn open(
        run: &Run,
        proxy: SocketAddr,
        username: String,
        password: SecretString,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let upstream = Arc::new(Upstream {
            address: proxy,
            username,
            password,
            refused: AtomicBool::new(false),
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

    /// Its loopback address, which the browser takes as its SOCKS5 proxy.
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
async fn relay(mut browser: TcpStream, upstream: &Upstream) {
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
    let Ok(mut proxy) = TcpStream::connect(upstream.address).await else {
        let _ = browser.write_all(&reply(GENERAL_FAILURE)).await;
        return;
    };
    match authenticate(&mut proxy, upstream).await {
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
    if proxy.write_all(&request).await.is_err() {
        return;
    }
    let Ok(answered) = read_request(&mut proxy).await else {
        let _ = browser.write_all(&reply(GENERAL_FAILURE)).await;
        return;
    };
    if browser.write_all(&answered).await.is_err() || answered[1] != 0 {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut browser, &mut proxy).await;
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
async fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
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

/// Authenticates with the proxy: whether it took the credentials.
async fn authenticate(proxy: &mut TcpStream, upstream: &Upstream) -> std::io::Result<bool> {
    proxy.write_all(&[VERSION, 1, USERNAME_PASSWORD]).await?;
    let mut chosen = [0; 2];
    proxy.read_exact(&mut chosen).await?;
    match chosen {
        [VERSION, NO_AUTHENTICATION] => return Ok(true),
        [VERSION, USERNAME_PASSWORD] => {}
        _ => return Ok(false),
    }
    let username = upstream.username.as_bytes();
    let password = upstream.password.expose_secret().as_bytes();
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
