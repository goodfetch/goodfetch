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
//!   stays the proxy's failure. The relay records whether each connection
//!   to the proxy failed to open, by the authority it asked for, so neither
//!   that `502` nor a tunnel it could not open counts as the proxy refusing
//!   that authority's request: a proxy that is down teaches its limits
//!   nothing.
//!
//! The relay dials only the addresses the Session's open judged (ADR 0017),
//! each connection within its connect deadline, so a proxy that never
//! answers is the proxy's failure before the navigation's deadline; and a
//! client that sends no request head within [`HEAD_DEADLINE`] is dropped.
//!
//! A plain request's connection closes after one answer, so each carries
//! the one request the relay authenticated. A Session whose proxy a Proxy
//! provider gave relays every connection, so every byte on its connections
//! to the proxy, the handshakes included, is charged to its Run, under its
//! proxy kind, before it is sent to the proxy or read from it: each
//! direction of a connection reserves an allowance ahead ([`Ahead`]), and
//! reads and writes no more than it covers, so no byte passes uncharged,
//! and a Limit's refusal closes the connection before the bytes it refused
//! pass (the deep review's 599-7 and 599-12). What a connection carried of
//! its allowances is settled when it closes, as when its Run ends
//! mid-transfer: the provider bills every byte it carried. For a hosted
//! provider's HTTP proxy, the relay also keeps the proxy's own answer to
//! latest `CONNECT` of each authority it refused ([`ProviderAnswer`]), which the browser
//! reports only as a network error, so the provider's adapter can read its
//! failure (ADR 0127). The listener binds port 0 of loopback and reports
//! its address (ADR 0059), and it closes, with every connection it relays,
//! when the Session's proxy is dropped or its Run stops.
#![expect(
    clippy::disallowed_types,
    reason = "the per-Session proxy relay opens its own loopback listener and its connections to the proxy, in the proxy module only, and its tasks set flags and one answer, each read alone with no lock order (ADR 0060)"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::endpoint::{Listener, Refusals, new_token, same_token};
use goodfetch_engine::runs::Run;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

use super::party::{RELAY, relay_unreachable};
use crate::egress_relay::deadlines::{HEAD_DEADLINE, within, within_or_none};
use crate::egress_relay::head::{self, HEAD_LIMIT, Head, Target, Unread};
use crate::egress_relay::serve::{Met, socks5_request};
use crate::egress_relay::{ByAuthority, Outcomes, authority_of, connect_deadline};
use crate::proxy_provider::failure::ProviderAnswer;
use crate::proxy_provider::{Ahead, ProxyUsage};

/// The username of every relay's client credentials; the password is the
/// Session's relay's own.
pub(crate) const USERNAME: &str = "goodfetch";

/// How long the relay waits before it accepts again after an error that is
/// not one connection's own, such as too many open files.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(250);

/// The most bytes relayed at once, in each direction.
const CHUNK: usize = 16 * 1024;

/// The most a SOCKS5 proxy's answers to the greeting, the credentials and
/// the request take: two bytes, two bytes, and a reply naming a domain
/// of at most 255 bytes (RFC 1928, RFC 1929).
const SOCKS5_ANSWERS: u64 = 2 + 2 + 4 + 1 + 255 + 2;

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
    /// The proxy's addresses, which the Session's open judged.
    addresses: Vec<SocketAddr>,
    mode: Mode,
    /// The `Proxy-Authorization` value a client must give.
    client: String,
    /// Where it records a client's wrong credentials.
    refusals: Refusals,
    /// What each connection met, by the authority it asked for: whether it
    /// reached no proxy, a proxy that is down, whose failures refuse
    /// nothing; whether the proxy refused the credentials, which a failed
    /// request then raises as `proxy_auth_failed`; and whether the relay
    /// answered it itself, with its own `502` for a SOCKS5 proxy it could
    /// not use, a response that is the proxy's failure, never the site's
    /// answer.
    outcomes: Outcomes,
    /// What the bytes on the connections to the proxy are charged to, for
    /// a Proxy provider's proxy.
    usage: Option<Arc<ProxyUsage>>,
    /// For a hosted provider's HTTP proxy, its answer to the latest
    /// `CONNECT` of each authority, when it refused it.
    refusal: Option<ByAuthority<ProviderAnswer>>,
    /// The Run's clock, which its deadlines run on.
    clock: Clock,
    /// How long a connection to the proxy may take to open.
    connect_deadline: Duration,
}

impl Upstream {
    /// A connection to the first of the proxy's addresses that opens, each
    /// within the connect deadline; none when none opened.
    async fn connect(&self) -> Option<TcpStream> {
        for address in &self.addresses {
            let connecting = TcpStream::connect(*address);
            if let Ok(stream) = within(&self.clock, self.connect_deadline, connecting).await {
                return Some(stream);
            }
        }
        None
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
    /// Binds the relay on port 0 of loopback for the proxy at `proxy`, the
    /// addresses the Session's open judged, with new client credentials,
    /// and relays every connection that gives them
    /// as a task of `run`, as `mode` says, charging its bytes to `usage`
    /// when it has one, and keeping an HTTP proxy's refusals of a `CONNECT`
    /// when `keeps_refusals`.
    ///
    /// # Errors
    ///
    /// The bind's failure, or the OS generator's.
    pub(crate) async fn open(
        run: &Run,
        proxy: Vec<SocketAddr>,
        mode: Mode,
        usage: Option<Arc<ProxyUsage>>,
        keeps_refusals: bool,
    ) -> std::io::Result<Self> {
        let password = new_token().map_err(std::io::Error::other)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let keeps_refusals = keeps_refusals && matches!(mode, Mode::Http { .. });
        let upstream = Arc::new(Upstream {
            addresses: proxy,
            mode,
            client: basic(USERNAME, &password),
            refusals: run.refusals(),
            outcomes: Outcomes::default(),
            usage,
            refusal: keeps_refusals.then(ByAuthority::default),
            clock: run.clock(),
            connect_deadline: connect_deadline(run),
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

    /// Whether the proxy refused the Session's credentials on the latest
    /// connection for `url`.
    pub(crate) fn refused_credentials(&self, url: &str) -> bool {
        self.upstream.outcomes.refused(url)
    }

    /// Whether the relay's latest connection for `url` failed to open.
    pub(crate) fn unreachable(&self, url: &str) -> bool {
        self.upstream.outcomes.unreachable(url)
    }

    /// Whether the relay answered its latest client for `url` itself, with
    /// its own `502` for a SOCKS5 proxy it could not use.
    pub(crate) fn answered(&self, url: &str) -> bool {
        self.upstream.outcomes.answered(url)
    }

    /// The proxy's answer to the latest `CONNECT` for `url`, if it refused
    /// it and the relay keeps refusals.
    pub(crate) fn latest_refusal(&self, url: &str) -> Option<ProviderAnswer> {
        let authority = authority_of(url)?;
        self.upstream.refusal.as_ref()?.get(&authority)
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
/// client gave the relay's credentials; a client that sends no whole
/// request head within [`HEAD_DEADLINE`] is dropped.
async fn relay(mut client: TcpStream, upstream: &Upstream) {
    let reading = head::read(&mut client);
    let Some(read) = within_or_none(&upstream.clock, HEAD_DEADLINE, reading).await else {
        return;
    };
    let (head, rest) = match read {
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
                upstream
                    .refusals
                    .refused(Listener::ProxyRelay, "token_wrong")
                    .await;
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
    let authority = head.target.authority();
    let met = |met| upstream.outcomes.record(&authority, met);
    let Some(proxy) = upstream.connect().await else {
        met(Met::Unreachable);
        let _ = client.write_all(relay_unreachable().as_bytes()).await;
        return;
    };
    let mut proxy = Tallied::new(proxy, upstream.usage.as_ref());
    if let Target::Absolute { .. } = head.target {
        met(Met::Reached);
        let mut forwarded = closing(head, true, authorization);
        let sent = proxy.send(&forwarded).await;
        // The buffer that held the credentials is cleared before it is
        // dropped.
        forwarded.fill(0);
        if sent.is_err() {
            return;
        }
        let _ = carry(client, proxy, rest).await;
        return;
    }
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n").into_bytes();
    if let Some(authorization) = authorization {
        request.extend_from_slice(b"Proxy-Authorization: ");
        request.extend_from_slice(authorization.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    let sent = proxy.send(&request).await;
    request.fill(0);
    if sent.is_err() || !proxy.cover_down(HEAD_LIMIT as u64).await {
        return;
    }
    let Ok(answer) = head::answer(&mut proxy).await else {
        met(Met::Reached);
        let _ = client
            .write_all(failed("502 Bad Gateway", "http_response_incomplete").as_bytes())
            .await;
        return;
    };
    let took = (200..300).contains(&answer.status);
    met(if answer.status == 407 {
        Met::CredentialsRefused
    } else {
        Met::Reached
    });
    if let Some(refusal) = &upstream.refusal {
        match provider_answer(&answer.to_browser).filter(|_| !took) {
            Some(answer) => refusal.keep(&authority, answer),
            None => refusal.forget(&authority),
        }
    }
    if !took {
        let _ = client.write_all(&answer.to_browser).await;
        return;
    }
    if client.write_all(ESTABLISHED).await.is_err() {
        return;
    }
    let _ = carry(client, proxy, rest).await;
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
    // What the connection met is its authority's latest answer, never one
    // another connection met.
    let authority = head.target.authority();
    let met = |met| upstream.outcomes.record(&authority, met);
    let Some(proxy) = upstream.connect().await else {
        met(Met::Unreachable);
        let _ = client.write_all(relay_unreachable().as_bytes()).await;
        return;
    };
    let mut proxy = Tallied::new(proxy, upstream.usage.as_ref());
    if !proxy.cover_down(SOCKS5_ANSWERS).await {
        return;
    }
    let authenticated = authenticate(&mut proxy, credentials).await;
    if !matches!(authenticated, Ok(true)) {
        met(if matches!(authenticated, Ok(false)) {
            Met::CredentialsRefused
        } else {
            Met::Refused
        });
        let _ = client
            .write_all(failed("502 Bad Gateway", "proxy_internal_error").as_bytes())
            .await;
        return;
    }
    if proxy.send(&request).await.is_err() {
        return;
    }
    let opened = read_reply(&mut proxy).await;
    if !matches!(opened, Ok(true)) {
        met(Met::Refused);
        let _ = client
            .write_all(failed("502 Bad Gateway", "connection_refused").as_bytes())
            .await;
        return;
    }
    met(Met::Reached);
    let started = match head.target {
        Target::Tunnel { .. } => client.write_all(ESTABLISHED).await,
        Target::Absolute { .. } => proxy.send(&closing(head, false, None)).await,
    };
    if started.is_err() {
        return;
    }
    let _ = carry(client, proxy, rest).await;
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
async fn carry(client: TcpStream, mut proxy: Tallied, rest: &[u8]) -> std::io::Result<()> {
    if !rest.is_empty() {
        proxy.send(rest).await?;
    }
    pass(client, proxy).await
}

/// A plain request's head as the egress relay sends it on
/// ([`Head::as_sent`]), with `Connection: close` added: this relay judges
/// and tallies each connection's host once, at its start, so the
/// connection carries one request.
fn closing(head: &Head, to_proxy: bool, authorization: Option<&str>) -> Vec<u8> {
    let mut sent = head.as_sent(to_proxy, authorization);
    sent.truncate(sent.len().saturating_sub(2));
    sent.extend_from_slice(b"Connection: close\r\n\r\n");
    sent
}

/// The error of bytes a Limit refused, which closes the connection.
fn refused() -> std::io::Error {
    std::io::Error::other("a Limit refused the proxy's bytes")
}

/// The proxy's end of a relayed connection and, for a Proxy provider's
/// proxy, what each direction of it reserved ahead: its owner covers what
/// it sends with [`Tallied::send`] and what it reads with
/// [`Tallied::cover_down`], and it reads and writes no byte they did not
/// cover.
struct Tallied {
    stream: TcpStream,
    /// What the bytes sent to the proxy are charged ahead to.
    up: Option<Ahead>,
    /// What the bytes read from the proxy are charged ahead to.
    down: Option<Ahead>,
}

impl Tallied {
    fn new(stream: TcpStream, usage: Option<&Arc<ProxyUsage>>) -> Self {
        Self {
            stream,
            up: usage.map(ProxyUsage::ahead),
            down: usage.map(ProxyUsage::ahead),
        }
    }

    /// Sends `bytes` to the proxy once they are charged ahead.
    async fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(up) = &mut self.up
            && !up.cover(length(bytes.len())).await
        {
            return Err(refused());
        }
        self.write_all(bytes).await
    }

    /// Charges ahead up to `bytes` the proxy is about to answer, read one
    /// at a time: whether they may be read, false once a Limit refused
    /// them.
    async fn cover_down(&mut self, bytes: u64) -> bool {
        match &mut self.down {
            Some(down) => down.cover(bytes).await,
            None => true,
        }
    }
}

/// `bytes` as a count of bytes charged.
fn length(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// The bytes `ahead` still covers, all of them when nothing is charged.
fn covered(ahead: Option<&Ahead>) -> usize {
    ahead.map_or(usize::MAX, |ahead| {
        usize::try_from(ahead.left()).unwrap_or(usize::MAX)
    })
}

impl AsyncRead for Tallied {
    /// Reads no more than what is charged ahead: a read it does not cover
    /// fails, and nothing is read.
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if buffer.remaining() > covered(this.down.as_ref()) {
            return std::task::Poll::Ready(Err(refused()));
        }
        let before = buffer.filled().len();
        let polled = std::pin::Pin::new(&mut this.stream).poll_read(context, buffer);
        if let Some(down) = &mut this.down {
            down.carried(length(buffer.filled().len() - before));
        }
        polled
    }
}

impl AsyncWrite for Tallied {
    /// Writes no more than what is charged ahead.
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let allowed = bytes.len().min(covered(this.up.as_ref()));
        if allowed == 0 && !bytes.is_empty() {
            return std::task::Poll::Ready(Err(refused()));
        }
        let polled = std::pin::Pin::new(&mut this.stream).poll_write(context, &bytes[..allowed]);
        if let (std::task::Poll::Ready(Ok(written)), Some(up)) = (&polled, &mut this.up) {
            up.carried(length(*written));
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
/// ends close, each direction on its own, charging ahead what the browser
/// sends before the proxy is given it and what the proxy sends before it
/// is read; it stops at the first refused charge.
async fn pass(browser: TcpStream, proxy: Tallied) -> std::io::Result<()> {
    let Tallied {
        stream,
        mut up,
        mut down,
    } = proxy;
    let (mut from_proxy, mut to_proxy) = stream.into_split();
    let (mut from_browser, mut to_browser) = browser.into_split();
    let upload = async {
        let mut chunk = vec![0; CHUNK];
        loop {
            let read = from_browser.read(&mut chunk).await?;
            if read == 0 {
                return to_proxy.shutdown().await;
            }
            if let Some(up) = &mut up
                && !up.cover(length(read)).await
            {
                return Err(refused());
            }
            // Each write is counted as it is taken, so a connection its
            // Run's end cuts short mid-chunk counts what the proxy took.
            let mut unsent = &chunk[..read];
            while !unsent.is_empty() {
                let written = to_proxy.write(unsent).await?;
                if written == 0 {
                    return Err(std::io::ErrorKind::WriteZero.into());
                }
                if let Some(up) = &mut up {
                    up.carried(length(written));
                }
                unsent = &unsent[written..];
            }
        }
    };
    let download = async {
        let mut chunk = vec![0; CHUNK];
        loop {
            let room = match &mut down {
                Some(down) => {
                    let room = down.room(length(CHUNK)).await.ok_or_else(refused)?;
                    usize::try_from(room).unwrap_or(CHUNK)
                }
                None => CHUNK,
            };
            let read = from_proxy.read(&mut chunk[..room]).await?;
            if let Some(down) = &mut down {
                down.carried(length(read));
            }
            if read == 0 {
                return to_browser.shutdown().await;
            }
            to_browser.write_all(&chunk[..read]).await?;
        }
    };
    tokio::try_join!(upload, download).map(drop)
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
    proxy.send(&[VERSION, 1, method]).await?;
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
    let sent = proxy.send(&message).await;
    // The buffer that held the password is cleared before it is dropped.
    message.fill(0);
    sent?;
    let mut status = [0; 2];
    proxy.read_exact(&mut status).await?;
    Ok(status == [CREDENTIALS_VERSION, 0])
}

#[cfg(test)]
mod tests;
