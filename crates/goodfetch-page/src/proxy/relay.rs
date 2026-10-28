//! The per-Session loopback relay (ADR 0037 / SA-22; ADR 0021): the
//! browser, or the Session's egress relay in front of it, connects to this
//! loopback HTTP proxy instead of the Session's proxy, and the relay opens
//! a connection to the proxy for each of its client's. Only a client giving
//! the relay's own per-Session credentials is relayed: the engine answers
//! the browser's `Fetch.authRequired` with them, and the egress relay sends
//! them. A client without them gets a `407` and no relayed byte, and one
//! giving wrong ones is refused in the Audit log too, so no other local
//! process tunnels through the user's proxy, its credentials and its
//! metered bytes (spec #577's review, M26). It relays in one of two ways:
//!
//! - **To a SOCKS5 proxy.** Chromium takes no SOCKS5 credentials, so the
//!   relay translates each `CONNECT`, and each plain request in absolute
//!   form, into a SOCKS5 tunnel to its host's name, authenticating it with
//!   the Session's username and password when it has them (RFC 1928, RFC
//!   1929). The password goes only to the proxy, over the connection the
//!   relay opens to it; it is never in the browser's arguments.
//! - **To an HTTP proxy.** Each `CONNECT` and plain request goes on with the
//!   proxy's own credentials in place of the relay's. A proxy that cannot
//!   be reached is answered with the relay's own `502` whose `Proxy-Status`
//!   member is the relay's and names `connection_refused` (RFC 9209), so it
//!   stays the proxy's failure. The relay records whether its latest
//!   connection to the proxy failed to open, so neither that `502` nor a
//!   tunnel it could not open counts as the proxy refusing a request: a
//!   proxy that is down teaches its limits nothing.
//!
//! A plain request's connection closes after one answer, so each carries
//! the one request the relay authenticated. A Session whose proxy a Proxy
//! provider gave relays every connection, so every byte on its connections
//! to the proxy, the handshakes included, is charged to its Run, under its
//! proxy kind, before the browser or the proxy is given it
//! ([`ProxyUsage`]); once a Limit refuses a charge, the connection closes.
//! A connection that closes while a charge is under way, or with bytes not
//! yet charged, as when its Run ends mid-transfer, still has them charged
//! in full: the provider bills every byte it carried. For a hosted
//! provider's HTTP proxy, the relay also keeps the proxy's own answer to
//! the latest `CONNECT` it refused ([`ProviderAnswer`]), which the browser
//! reports only as a network error, so the provider's adapter can read its
//! failure (ADR 0127). The listener binds port 0 of loopback and reports
//! its address (ADR 0059), and it closes, with every connection it relays,
//! when the Session's proxy is dropped or its Run stops.
#![expect(
    clippy::disallowed_types,
    reason = "the per-Session proxy relay opens its own loopback listener and its connections to the proxy, in the proxy module only, and its tasks set flags and one answer, each read alone with no lock order (ADR 0060)"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::endpoint::{Refusals, new_token, same_token};
use goodfetch_engine::runs::Run;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

use super::party::{RELAY, relay_unreachable};
use crate::egress_relay::head::{self, Head, Target, Unread};
use crate::egress_relay::serve::socks5_request;
use crate::proxy_provider::ProxyUsage;
use crate::proxy_provider::failure::ProviderAnswer;

/// The relay's name in the endpoint table and in its refusals' Audit log
/// entries.
pub(crate) const LISTENER: &str = "proxy_relay";

/// The username of every relay's client credentials; the password is the
/// Session's relay's own.
pub(crate) const USERNAME: &str = "goodfetch";

/// How long the relay waits before it accepts again after an error that is
/// not one connection's own, such as too many open files.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(250);

/// The most bytes relayed at once, in each direction.
const CHUNK: usize = 16 * 1024;

/// The answer to a client without the relay's credentials.
const AUTHENTICATE: &[u8] = b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"goodfetch\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// The answer to a tunnel that opened.
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

/// SOCKS's version 5.
const VERSION: u8 = 5;
/// The method that asks for no credentials.
const NO_AUTHENTICATION: u8 = 0;
/// The username and password method (RFC 1929).
const USERNAME_PASSWORD: u8 = 2;
/// RFC 1929's version.
const CREDENTIALS_VERSION: u8 = 1;
/// The address types: IPv4, a domain name and IPv6.
const IPV4: u8 = 1;
const DOMAIN: u8 = 3;
const IPV6: u8 = 4;

/// A SOCKS5 proxy's username and password.
pub(crate) struct Credentials {
    pub(crate) username: String,
    pub(crate) password: SecretString,
}

/// How the relay reaches the proxy.
pub(crate) enum Mode {
    /// A SOCKS5 proxy, authenticating with these credentials, if any.
    Socks5 { credentials: Option<Credentials> },
    /// An HTTP proxy, given this `Proxy-Authorization` value, if any.
    Http { authorization: Option<SecretString> },
}

/// The `Proxy-Authorization` value of `username` and `password`'s Basic
/// credentials (RFC 7617).
pub(crate) fn basic(username: &str, password: &str) -> String {
    format!(
        "Basic {}",
        BASE64.encode(format!("{username}:{password}").as_bytes())
    )
}

/// Where the relay sends each connection: the proxy, how, and what its
/// bytes are charged to.
struct Upstream {
    address: SocketAddr,
    mode: Mode,
    /// The `Proxy-Authorization` value a client must give.
    client: String,
    /// Where it records a client's wrong credentials.
    refusals: Refusals,
    /// Whether the proxy refused the credentials on the latest connection
    /// that reached it, which a failed request then raises as
    /// `proxy_auth_failed`.
    refused: AtomicBool,
    /// Whether the relay's latest connection to the proxy failed to open:
    /// a proxy that is down, whose failures refuse nothing.
    unreachable: AtomicBool,
    /// Whether the relay answered its latest client itself, with its own
    /// `502` for a SOCKS5 proxy it could not use: one that is down, that
    /// refused the credentials or that refused the tunnel. A response the
    /// relay wrote is the proxy's failure, never the site's answer.
    answered: AtomicBool,
    /// What the bytes on the connections to the proxy are charged to, for
    /// a Proxy provider's proxy.
    usage: Option<Arc<ProxyUsage>>,
    /// For a hosted provider's HTTP proxy, its answer to the latest
    /// `CONNECT`, when it refused it; none when it took it.
    refusal: Option<Mutex<Option<ProviderAnswer>>>,
}

impl Upstream {
    /// A connection to the proxy, recording whether it opened.
    async fn connect(&self) -> Option<TcpStream> {
        let connected = TcpStream::connect(self.address).await.ok();
        self.unreachable
            .store(connected.is_none(), Ordering::Relaxed);
        connected
    }
}

/// An open relay: its loopback address and its client credentials, closed
/// when dropped.
pub(crate) struct Relay {
    address: SocketAddr,
    password: SecretString,
    upstream: Arc<Upstream>,
    _closed_when_dropped: DropGuard,
}

impl Relay {
    /// Binds the relay on port 0 of loopback for the proxy at `proxy`, with
    /// new client credentials, and relays every connection that gives them
    /// as a task of `run`, as `mode` says, charging its bytes to `usage`
    /// when it has one, and keeping an HTTP proxy's refusals of a `CONNECT`
    /// when `keeps_refusals`.
    ///
    /// # Errors
    ///
    /// The bind's failure, or the OS generator's.
    pub(crate) async fn open(
        run: &Run,
        proxy: SocketAddr,
        mode: Mode,
        usage: Option<Arc<ProxyUsage>>,
        keeps_refusals: bool,
    ) -> std::io::Result<Self> {
        let password = new_token().map_err(std::io::Error::other)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let keeps_refusals = keeps_refusals && matches!(mode, Mode::Http { .. });
        let upstream = Arc::new(Upstream {
            address: proxy,
            mode,
            client: basic(USERNAME, &password),
            refusals: run.refusals(),
            refused: AtomicBool::new(false),
            unreachable: AtomicBool::new(false),
            answered: AtomicBool::new(false),
            usage,
            refusal: keeps_refusals.then(|| Mutex::new(None)),
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
            password: SecretString::from(password),
            upstream,
            _closed_when_dropped: stop.drop_guard(),
        })
    }

    /// Its loopback address, which the browser takes as its proxy.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// The credentials a client gives it: its username and password.
    pub(crate) fn credentials(&self) -> (&'static str, &SecretString) {
        (USERNAME, &self.password)
    }

    /// Whether the proxy refused the Session's credentials.
    pub(crate) fn refused_credentials(&self) -> bool {
        self.upstream.refused.load(Ordering::Relaxed)
    }

    /// Whether the relay's latest connection to the proxy failed to open.
    pub(crate) fn unreachable(&self) -> bool {
        self.upstream.unreachable.load(Ordering::Relaxed)
    }

    /// Whether the relay answered its latest client itself, with its own
    /// `502` for a SOCKS5 proxy it could not use.
    pub(crate) fn answered(&self) -> bool {
        self.upstream.answered.load(Ordering::Relaxed)
    }

    /// The proxy's answer to the latest `CONNECT`, if it refused it and
    /// the relay keeps refusals.
    pub(crate) fn latest_refusal(&self) -> Option<ProviderAnswer> {
        self.upstream
            .refusal
            .as_ref()?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Accepts the browser's connections until `stop` or the Run's end, each
/// relayed on a task of its own.
async fn accept(listener: TcpListener, upstream: Arc<Upstream>, run: Run, stop: CancellationToken) {
    let ending = run.cancellation().clone();
    let clock = run.clock();
    accepting(
        || listener.accept(),
        &clock,
        &stop,
        &ending,
        |(browser, _)| {
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
        },
    )
    .await;
}

/// Takes each connection `accept` gives to `serve` until `stop` or
/// `ending`. A connection that failed before it was accepted concerns only
/// the browser, and the next is accepted at once; any other error, such as
/// too many open files, would fail every accept for a while, so the next
/// waits [`ACCEPT_BACKOFF`] on `clock` rather than spin.
async fn accepting<T, A, F, S>(
    mut accept: A,
    clock: &Clock,
    stop: &CancellationToken,
    ending: &CancellationToken,
    mut serve: S,
) where
    A: FnMut() -> F,
    F: Future<Output = std::io::Result<T>>,
    S: FnMut(T),
{
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => return,
            () = ending.cancelled() => return,
            accepted = accept() => accepted,
        };
        match accepted {
            Ok(accepted) => serve(accepted),
            Err(error) if one_connections(&error) => {}
            Err(error) => {
                tracing::warn!(%error, "the Session's relay could not accept a connection: it waits before the next");
                tokio::select! {
                    () = stop.cancelled() => return,
                    () = ending.cancelled() => return,
                    () = clock.sleep(ACCEPT_BACKOFF) => {}
                }
            }
        }
    }
}

/// Whether `error`, an accept's, is one connection's own, which the next
/// accept does not meet.
fn one_connections(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::Interrupted
    )
}

/// The relay's own answer to a request it could not take on, with
/// `status` and the `Proxy-Status` member naming `error` (RFC 9209).
fn failed(status: &str, error: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nProxy-Status: {RELAY}; error={error}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// Relays one of its client's connections through the proxy, once the
/// client gave the relay's credentials.
async fn relay(mut client: TcpStream, upstream: &Upstream) {
    let (head, rest) = match head::read(&mut client).await {
        Ok(read) => read,
        Err(Unread::Closed) => return,
        Err(Unread::Malformed) => {
            let _ = client
                .write_all(failed("400 Bad Request", "http_request_error").as_bytes())
                .await;
            return;
        }
    };
    match head.authorization.as_deref() {
        Some(given) if same_token(given, &upstream.client) => {}
        given => {
            // A client's first request carries no credentials until the
            // relay asks for them, the browser's included: only wrong ones
            // are a refusal.
            if given.is_some() {
                upstream.refusals.refused(LISTENER, "token_wrong").await;
            }
            let _ = client.write_all(AUTHENTICATE).await;
            return;
        }
    }
    match &upstream.mode {
        Mode::Socks5 { credentials } => {
            socks5(client, &head, &rest, upstream, credentials.as_ref()).await;
        }
        Mode::Http { authorization } => {
            let authorization = authorization
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret);
            http(client, &head, &rest, upstream, authorization).await;
        }
    }
}

/// Relays `head`'s request, and `rest` after it, through an HTTP proxy
/// with `authorization`: a plain request in absolute form, a tunnel through
/// the proxy's own, its answer passed back as the proxy gave it, its body
/// left out, and kept when the proxy refused the tunnel.
async fn http(
    mut client: TcpStream,
    head: &Head,
    rest: &[u8],
    upstream: &Upstream,
    authorization: Option<&str>,
) {
    let Some(proxy) = upstream.connect().await else {
        let _ = client.write_all(relay_unreachable().as_bytes()).await;
        return;
    };
    let mut proxy = Tallied::new(proxy, upstream.usage.clone());
    if let Target::Absolute { .. } = head.target {
        let mut forwarded = head.forwarded(true, authorization);
        let sent = proxy.write_all(&forwarded).await;
        // The buffer that held the credentials is cleared before it is
        // dropped.
        forwarded.fill(0);
        if sent.is_err() || !proxy.charged().await {
            return;
        }
        let _ = carry(&mut client, &mut proxy, rest).await;
        return;
    }
    let authority = head.target.authority();
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n").into_bytes();
    if let Some(authorization) = authorization {
        request.extend_from_slice(b"Proxy-Authorization: ");
        request.extend_from_slice(authorization.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    let sent = proxy.write_all(&request).await;
    request.fill(0);
    if sent.is_err() || !proxy.charged().await {
        return;
    }
    let Ok(answer) = head::answer(&mut proxy).await else {
        let _ = client
            .write_all(failed("502 Bad Gateway", "http_response_incomplete").as_bytes())
            .await;
        return;
    };
    if !proxy.charged().await {
        return;
    }
    let took = (200..300).contains(&answer.status);
    upstream
        .refused
        .store(answer.status == 407, Ordering::Relaxed);
    if let Some(refusal) = &upstream.refusal {
        *refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = if took {
            None
        } else {
            provider_answer(&answer.to_browser)
        };
    }
    if !took {
        let _ = client.write_all(&answer.to_browser).await;
        return;
    }
    if client.write_all(ESTABLISHED).await.is_err() {
        return;
    }
    let _ = carry(&mut client, &mut proxy, rest).await;
}

/// The proxy's answer in `bytes`, its status and headers, if they start
/// with a whole response head.
fn provider_answer(bytes: &[u8]) -> Option<ProviderAnswer> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut headers);
    if !response.parse(bytes).ok()?.is_complete() {
        return None;
    }
    Some(ProviderAnswer {
        status: response.code?,
        headers: response
            .headers
            .iter()
            .map(|header| {
                (
                    header.name.to_ascii_lowercase(),
                    String::from_utf8_lossy(header.value).into_owned(),
                )
            })
            .collect(),
    })
}

/// Relays `head`'s request, and `rest` after it, through a SOCKS5 proxy, in
/// a tunnel to its host's name authenticated with `credentials`, if any: a
/// `CONNECT` answered as opened, a plain request sent on in origin form.
async fn socks5(
    mut client: TcpStream,
    head: &Head,
    rest: &[u8],
    upstream: &Upstream,
    credentials: Option<&Credentials>,
) {
    let Ok(request) = socks5_request(head.target.host(), head.target.port()) else {
        let _ = client
            .write_all(failed("400 Bad Request", "http_request_error").as_bytes())
            .await;
        return;
    };
    // Whether the proxy refused the credentials is this connection's
    // answer, the latest, never one a connection before met.
    let answered = |answered: bool| upstream.answered.store(answered, Ordering::Relaxed);
    let Some(proxy) = upstream.connect().await else {
        upstream.refused.store(false, Ordering::Relaxed);
        answered(true);
        let _ = client.write_all(relay_unreachable().as_bytes()).await;
        return;
    };
    let mut proxy = Tallied::new(proxy, upstream.usage.clone());
    let authenticated = authenticate(&mut proxy, credentials).await;
    upstream
        .refused
        .store(matches!(authenticated, Ok(false)), Ordering::Relaxed);
    if !matches!(authenticated, Ok(true)) || !proxy.charged().await {
        answered(true);
        let _ = client
            .write_all(failed("502 Bad Gateway", "proxy_internal_error").as_bytes())
            .await;
        return;
    }
    if proxy.write_all(&request).await.is_err() || !proxy.charged().await {
        return;
    }
    let opened = read_reply(&mut proxy).await;
    if !proxy.charged().await {
        return;
    }
    if !matches!(opened, Ok(true)) {
        answered(true);
        let _ = client
            .write_all(failed("502 Bad Gateway", "connection_refused").as_bytes())
            .await;
        return;
    }
    answered(false);
    let started = match head.target {
        Target::Tunnel { .. } => client.write_all(ESTABLISHED).await,
        Target::Absolute { .. } => proxy.write_all(&head.forwarded(false, None)).await,
    };
    if started.is_err() {
        return;
    }
    let _ = carry(&mut client, &mut proxy, rest).await;
}

/// Reads a SOCKS5 reply: whether the proxy opened the tunnel.
async fn read_reply(proxy: &mut (impl AsyncRead + Unpin)) -> std::io::Result<bool> {
    let mut reply = [0; 4];
    proxy.read_exact(&mut reply).await?;
    if reply[0] != VERSION {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let address = match reply[3] {
        IPV4 => 4,
        IPV6 => 16,
        DOMAIN => usize::from(proxy.read_u8().await?),
        _ => return Err(std::io::ErrorKind::InvalidData.into()),
    };
    let mut bound = vec![0; address + 2];
    proxy.read_exact(&mut bound).await?;
    Ok(reply[1] == 0)
}

/// Carries `rest`, the client's bytes read past its head, to the proxy,
/// then bytes both ways.
async fn carry(client: &mut TcpStream, proxy: &mut Tallied, rest: &[u8]) -> std::io::Result<()> {
    if !rest.is_empty() {
        proxy.write_all(rest).await?;
        if !proxy.charged().await {
            return Err(std::io::Error::other("a Limit refused the proxy's bytes"));
        }
    }
    pass(client, proxy).await
}

/// The proxy's end of a relayed connection, which tallies every byte read
/// from it or written to it; its owner charges them at [`Tallied::charged`],
/// and what is left uncharged when it is dropped is charged then.
struct Tallied {
    stream: TcpStream,
    usage: Option<Arc<ProxyUsage>>,
    /// The bytes carried since the last charge.
    uncharged: u64,
}

impl Tallied {
    fn new(stream: TcpStream, usage: Option<Arc<ProxyUsage>>) -> Self {
        Self {
            stream,
            usage,
            uncharged: 0,
        }
    }

    /// Charges the bytes carried since the last charge, to the end even if
    /// the relay stops waiting: whether the connection may carry more,
    /// false once a Limit refused them.
    async fn charged(&mut self) -> bool {
        let bytes = std::mem::take(&mut self.uncharged);
        match &self.usage {
            Some(usage) => usage.charge_to_the_end(bytes).await.unwrap_or(false),
            None => true,
        }
    }

    fn tally(&mut self, bytes: usize) {
        self.uncharged = self
            .uncharged
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }
}

impl Drop for Tallied {
    /// Charges what the connection carried since its last charge: a write
    /// its Run's end or a failure cut short.
    fn drop(&mut self) {
        if let Some(usage) = &self.usage {
            drop(usage.charge_to_the_end(self.uncharged));
        }
    }
}

impl AsyncRead for Tallied {
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

impl AsyncWrite for Tallied {
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
async fn pass(browser: &mut TcpStream, proxy: &mut Tallied) -> std::io::Result<()> {
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

/// Greets the proxy offering `credentials`' method, or none, and
/// authenticates with them: whether it took them.
async fn authenticate(
    proxy: &mut Tallied,
    credentials: Option<&Credentials>,
) -> std::io::Result<bool> {
    let method = if credentials.is_some() {
        USERNAME_PASSWORD
    } else {
        NO_AUTHENTICATION
    };
    proxy.write_all(&[VERSION, 1, method]).await?;
    let mut chosen = [0; 2];
    proxy.read_exact(&mut chosen).await?;
    let credentials = match (chosen, credentials) {
        ([VERSION, NO_AUTHENTICATION], _) => return Ok(true),
        ([VERSION, USERNAME_PASSWORD], Some(credentials)) => credentials,
        _ => return Ok(false),
    };
    let username = credentials.username.as_bytes();
    let password = credentials.password.expose_secret().as_bytes();
    // The Session refuses credentials longer than RFC 1929 carries when it
    // opens; none reaches here, and none would be the proxy's refusal.
    let (Ok(username_length), Ok(password_length)) =
        (u8::try_from(username.len()), u8::try_from(password.len()))
    else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
    let mut message = Vec::with_capacity(3 + username.len() + password.len());
    message.extend_from_slice(&[CREDENTIALS_VERSION, username_length]);
    message.extend_from_slice(username);
    message.push(password_length);
    message.extend_from_slice(password);
    let sent = proxy.write_all(&message).await;
    // The buffer that held the password is cleared before it is dropped.
    message.fill(0);
    sent?;
    let mut status = [0; 2];
    proxy.read_exact(&mut status).await?;
    Ok(status == [CREDENTIALS_VERSION, 0])
}

#[cfg(test)]
mod tests;
