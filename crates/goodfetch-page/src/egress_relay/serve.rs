//! One browser connection through the egress relay, over any streams and
//! any [`Network`], so it is tested and measured in process: the browser's
//! credentials checked first, then the egress policy on the host and on
//! every address the connection may use, which the relay resolves itself,
//! then the connection opened to those addresses alone, or through the
//! Session's upstream proxy, and a tunnel's bytes passed both ways
//! unchanged, its ClientHello noted as it passes. A plain connection
//! carries one request after another, each judged on its own and each
//! answer passed back as the site gave it ([`plain`]). A client that sends
//! no request head within [`HEAD_DEADLINE`] is dropped, and each
//! connection the relay opens has its connect deadline, so a proxy that
//! never answers is the proxy's failure.
//!
//! Size: one concern, one browser connection from its first head to its
//! close: a tunnel and the plain requests share the credential check, the
//! judgment and the legs opened, and a plain connection may become a tunnel.

use goodfetch_cdp::NetError;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::{Gate, Policy, Refused, RequestKind, Stopped};
use goodfetch_engine::endpoint::same_token;
use goodfetch_engine::tls::ExceptionUsed;
use tokio::io::{
    AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadHalf, WriteHalf,
};

use super::answers::{AUTHENTICATE, ESTABLISHED, failed};
use super::client_hello::{ClientHello, Noting};
use super::deadlines::{HEAD_DEADLINE, IDLE_DEADLINE, within, within_or_none};
use super::head::{self, Head, Target, Unread};
use super::message::{Incoming, widened};
use super::terminate::{self, Stop, Terminated, Terminating};

/// What a name resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// The addresses a connection may try, in order, each judged as
    /// any answer is, a `network.resolve` mapping's too (ADR 0059).
    pub(crate) addresses: Vec<SocketAddr>,
}

/// Where the relay's connections go out, and how names resolve.
pub(crate) trait Network: Sync {
    /// A connection it opens.
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;

    /// The addresses of `host` on `port`: an address as it is, a mapped
    /// name's, else the resolver's; none when it has none.
    fn resolve(&self, host: &str, port: u16) -> impl Future<Output = Option<Resolved>> + Send;

    /// A connection to `address`.
    fn connect(&self, address: SocketAddr)
    -> impl Future<Output = io::Result<Self::Stream>> + Send;

    /// A connection to the Session's upstream proxy.
    fn connect_upstream(&self) -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

/// The egress policy's refusal of the Session's upstream proxy's own
/// address (ADR 0017), which the error [`Network::connect_upstream`] fails
/// with carries.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ProxyRefused(pub(crate) Refused);

/// The refusal `error`, a failed connection to the upstream proxy, carries,
/// if the egress policy refused the proxy's address.
fn proxy_refused(error: &io::Error) -> Option<&Refused> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ProxyRefused>())
        .map(|ProxyRefused(refused)| refused)
}

/// Where the Session's connections go after the relay.
#[derive(Clone, Debug)]
pub(crate) enum Upstream {
    /// To the site.
    Direct,
    /// Through an HTTP proxy, with the `Proxy-Authorization` value of its
    /// credentials if it has any.
    Http { authorization: Option<String> },
    /// Through a SOCKS5 proxy that asks for no credentials: the proxy
    /// itself, or the Session's loopback relay that authenticates to it.
    Socks5,
}

/// What a connection met beyond the relay, which the Session's proxy's
/// failures are read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Met {
    /// The upstream proxy answered.
    Reached,
    /// No connection to the upstream proxy opened.
    Unreachable,
    /// The upstream proxy refused the Session's credentials.
    CredentialsRefused,
    /// A SOCKS5 proxy took the connection but refused the tunnel, which
    /// the relay answered with its own `502`.
    Refused,
}

/// Why the relay could not reach a site itself, as the browser names the
/// same failure when it connects on its own: it sees only the relay's
/// answer, so the engine reports the site's failure by this name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unreached {
    /// The name has no address.
    NameNotResolved,
    /// Every address refused the connection.
    ConnectionRefused,
    /// The connection timed out.
    TimedOut,
    /// Any other failure to connect.
    ConnectionFailed,
}

impl Unreached {
    /// The browser's own network error for the failure.
    pub(crate) fn net_error(self) -> NetError {
        match self {
            Self::NameNotResolved => NetError::NameNotResolved,
            Self::ConnectionRefused => NetError::ConnectionRefused,
            Self::TimedOut => NetError::ConnectionTimedOut,
            Self::ConnectionFailed => NetError::ConnectionFailed,
        }
    }

    fn of(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::ConnectionRefused => Self::ConnectionRefused,
            io::ErrorKind::TimedOut => Self::TimedOut,
            _ => Self::ConnectionFailed,
        }
    }
}

/// What the relay tells the Session of its connections.
pub(crate) trait Observe: Sync {
    /// The relay could not reach the site at `authority` itself.
    fn unreached(&self, authority: &str, unreached: Unreached);
    /// A connection reached the site at `authority`, or the upstream
    /// proxy for it.
    fn reached(&self, authority: &str);
    /// The relay refused a connection `method` asked for, before any byte
    /// left for its target.
    fn refused(&self, refused: &Refused, method: &str);
    /// A connection for `authority` met the upstream proxy as `met`.
    fn upstream(&self, authority: &str, met: Met);
    /// A client gave no credentials, or wrong ones when `given`.
    fn unauthenticated(&self, given: bool);
    /// The relay terminated a connection to `host` and relayed it, through
    /// `used` when an exception let it pass (ADR 0042).
    fn terminated(&self, host: &str, used: Option<&ExceptionUsed>);
    /// The relay stopped a terminated connection to `url` that `method`
    /// asked for, before any byte of the browser's went up.
    fn stopped(&self, url: &str, method: &str, stop: &Stop);
    /// The browser opened TLS with `hello` on a tunnel to `authority` that
    /// the relay passes as it is.
    fn client_hello(&self, authority: &str, hello: ClientHello);
}

/// How the relay judges and routes a Session's connections.
pub(crate) struct Relaying {
    /// The Run's egress policy.
    pub(crate) policy: Policy,
    /// The `Proxy-Authorization` value the browser must give.
    pub(crate) authorization: String,
    /// Where connections go after the relay.
    pub(crate) upstream: Upstream,
    /// Whether a URL goes around the upstream proxy, as `proxy.bypass`
    /// says.
    pub(crate) bypasses: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// The hosts it terminates, and how, when the Run lists any.
    pub(crate) terminating: Option<Arc<Terminating>>,
    /// The Run's clock, which its deadlines run on.
    pub(crate) clock: Clock,
    /// How long a connection it opens, to a site or to the upstream
    /// proxy, may take to open.
    pub(crate) connect_deadline: Duration,
}

impl Relaying {
    /// A connection to `address` on `network`, within the connect deadline.
    async fn connect<N: Network>(&self, network: &N, address: SocketAddr) -> io::Result<N::Stream> {
        within(&self.clock, self.connect_deadline, network.connect(address)).await
    }

    /// A connection to the upstream proxy on `network`, within the connect
    /// deadline.
    async fn connect_upstream<N: Network>(&self, network: &N) -> io::Result<N::Stream> {
        within(
            &self.clock,
            self.connect_deadline,
            network.connect_upstream(),
        )
        .await
    }
}

/// How one connection ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Served {
    /// Its bytes went both ways: these many from the browser and to it.
    Relayed { up: u64, down: u64 },
    /// The relay refused it: no credentials, or wrong ones.
    Unauthenticated,
    /// The egress policy refused it.
    Refused,
    /// It ended before anything was relayed.
    Failed,
}

/// Serves one connection of the browser's.
///
/// # Cancel safety
///
/// Dropping the future closes both connections.
pub(crate) async fn serve<B, N, O>(
    mut browser: B,
    relaying: &Relaying,
    network: &N,
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    N: Network,
    O: Observe,
{
    let reading = head::read(&mut browser);
    let Some(read) = within_or_none(&relaying.clock, HEAD_DEADLINE, reading).await else {
        return Served::Failed;
    };
    let (head, rest) = match read {
        Ok(read) => read,
        Err(Unread::Closed) => return Served::Failed,
        Err(Unread::Malformed) => {
            let _ = browser
                .write_all(&failed("400 Bad Request", "http_request_error"))
                .await;
            return Served::Failed;
        }
    };
    let given = head.authorization.as_deref().unwrap_or_default();
    if !same_token(given, &relaying.authorization) {
        observe.unauthenticated(head.authorization.is_some());
        let _ = browser.write_all(AUTHENTICATE).await;
        return Served::Unauthenticated;
    }
    if let Target::Absolute { .. } = head.target {
        return plain(browser, head, rest, (relaying, network), observe).await;
    }
    tunnel(browser, &head, rest, (relaying, network), observe).await
}

/// Opens `head`'s tunnel, `rest` the browser's bytes past its head, once
/// the egress policy passed it, and relays it.
async fn tunnel<B, N, O>(
    mut browser: B,
    head: &Head,
    rest: Vec<u8>,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    N: Network,
    O: Observe,
{
    let resolved = match judged(&mut browser, head, (relaying, network), observe).await {
        Ok(resolved) => resolved,
        Err(served) => return served,
    };
    match upstream_of(relaying, &head.target.url()) {
        Upstream::Direct => {
            direct(
                browser,
                head,
                rest,
                &resolved.addresses,
                (relaying, network),
                observe,
            )
            .await
        }
        Upstream::Http { authorization } => {
            through_http(
                browser,
                head,
                rest,
                authorization.as_deref(),
                (relaying, network),
                observe,
            )
            .await
        }
        Upstream::Socks5 => through_socks5(browser, head, rest, (relaying, network), observe).await,
    }
}

/// Where a connection to `url` goes after the relay: around the upstream
/// proxy when `proxy.bypass` says so.
fn upstream_of<'r>(relaying: &'r Relaying, url: &str) -> &'r Upstream {
    if (relaying.bypasses)(url) {
        &Upstream::Direct
    } else {
        &relaying.upstream
    }
}

/// The addresses `head` may try, once the egress gate passed it
/// ([`passed`]); else how its connection ended, its refusal told the
/// Session and answered.
async fn judged<B, N, O>(
    browser: &mut B,
    head: &Head,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Result<Resolved, Served>
where
    B: AsyncWrite + Unpin,
    N: Network,
    O: Observe,
{
    match passed(&head.target.url(), relaying, network).await {
        Ok(resolved) => Ok(resolved),
        Err(Stopped::Refused(refused)) => {
            Err(refuse(browser, head, &refused, observe, "http_request_denied").await)
        }
        Err(Stopped::AddressRefused(refused)) => Err(refuse(
            browser,
            head,
            &refused,
            observe,
            "destination_ip_prohibited",
        )
        .await),
        // Nothing else stops it with a refusal to name: its name has no
        // address it may reach.
        Err(_) => Err(unreached(
            browser,
            head,
            observe,
            Unreached::NameNotResolved,
            "dns_error",
        )
        .await),
    }
}

/// The addresses a request to `url` may try, through the Session's egress
/// gate: the URL judged, its name resolved by the relay itself and each
/// answer judged before any connection, so an answer that changes after the
/// browser's own check reaches nothing (ADR 0017); behind a proxy, which
/// resolves the name again, as the engine's own client does before sending
/// to one.
async fn passed<N: Network>(
    url: &str,
    relaying: &Relaying,
    network: &N,
) -> Result<Resolved, Stopped<()>> {
    let gate = Gate::Session {
        policy: &relaying.policy,
        kind: RequestKind::Page,
    };
    let passed = gate
        .pass(url, async |host, port| {
            network
                .resolve(host, port)
                .await
                .map(|resolved| resolved.addresses)
                .ok_or(())
        })
        .await?;
    Ok(Resolved {
        addresses: passed.addresses,
    })
}

/// Refuses `head`'s connection as `refused` says: a tunnel is answered
/// with the relay's `403` naming `error`, and a plain request's connection
/// closes with no answer, so no answer of the site's is made up.
async fn refuse<B, O>(
    browser: &mut B,
    head: &Head,
    refused: &Refused,
    observe: &O,
    error: &str,
) -> Served
where
    B: AsyncWrite + Unpin,
    O: Observe,
{
    observe.refused(refused, &head.method);
    if matches!(head.target, Target::Tunnel { .. }) {
        let _ = browser.write_all(&failed("403 Forbidden", error)).await;
    }
    Served::Refused
}

/// Answers `head`'s connection, which the relay could not take to its
/// site itself: a tunnel with the relay's `502` naming `error`, a plain
/// request by closing with no answer, so the browser fails the request as
/// it fails one the site never answered, and the Session learns why.
async fn unreached<B, O>(
    browser: &mut B,
    head: &Head,
    observe: &O,
    unreached: Unreached,
    error: &str,
) -> Served
where
    B: AsyncWrite + Unpin,
    O: Observe,
{
    observe.unreached(&head.target.authority(), unreached);
    if matches!(head.target, Target::Tunnel { .. }) {
        let _ = browser.write_all(&failed("502 Bad Gateway", error)).await;
    }
    Served::Failed
}

/// Opens a tunnel to the first of `addresses` that answers, and relays.
async fn direct<B, N, O>(
    mut browser: B,
    head: &Head,
    rest: Vec<u8>,
    addresses: &[SocketAddr],
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    N: Network,
    O: Observe,
{
    let site = match connect_site(&mut browser, head, addresses, (relaying, network), observe).await
    {
        Ok((site, _)) => site,
        Err(served) => return served,
    };
    if browser.write_all(ESTABLISHED).await.is_err() {
        return Served::Failed;
    }
    forward(browser, site, rest, head, relaying, observe).await
}

/// A connection to the first of `addresses` that answers `head`'s, and
/// that address; else how the browser's connection ended, as
/// [`unreached`] answers it.
async fn connect_site<B, N, O>(
    browser: &mut B,
    head: &Head,
    addresses: &[SocketAddr],
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Result<(N::Stream, SocketAddr), Served>
where
    B: AsyncWrite + Unpin,
    N: Network,
    O: Observe,
{
    let mut last = Unreached::ConnectionFailed;
    for address in addresses {
        match relaying.connect(network, *address).await {
            Ok(stream) => {
                observe.reached(&head.target.authority());
                return Ok((stream, *address));
            }
            Err(error) => last = Unreached::of(&error),
        }
    }
    let error = match last {
        Unreached::TimedOut => "connection_timeout",
        _ => "connection_refused",
    };
    Err(unreached(browser, head, observe, last, error).await)
}

/// A connection to the upstream proxy for `head`; else how the browser's
/// connection ended, as [`unconnected`] answers it.
async fn connect_proxy<B, N, O>(
    browser: &mut B,
    head: &Head,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Result<N::Stream, Served>
where
    B: AsyncWrite + Unpin,
    N: Network,
    O: Observe,
{
    match relaying.connect_upstream(network).await {
        Ok(proxy) => Ok(proxy),
        Err(error) => Err(unconnected(browser, head, &error, observe).await),
    }
}

/// Relays a tunnel through an HTTP proxy's own tunnel to the host's name;
/// the proxy's refusal reaches the browser as it gave it, its body left
/// out.
async fn through_http<B, N, O>(
    mut browser: B,
    head: &Head,
    rest: Vec<u8>,
    authorization: Option<&str>,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    N: Network,
    O: Observe,
{
    let mut proxy = match connect_proxy(&mut browser, head, (relaying, network), observe).await {
        Ok(proxy) => proxy,
        Err(served) => return served,
    };
    let authority = head.target.authority();
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(authorization) = authorization {
        request.push_str("Proxy-Authorization: ");
        request.push_str(authorization);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    if proxy.write_all(request.as_bytes()).await.is_err() {
        observe.upstream(&head.target.authority(), Met::Unreachable);
        let _ = browser
            .write_all(&failed("502 Bad Gateway", "connection_refused"))
            .await;
        return Served::Failed;
    }
    let Ok(answer) = head::answer(&mut proxy).await else {
        observe.upstream(&head.target.authority(), Met::Reached);
        let _ = browser
            .write_all(&failed("502 Bad Gateway", "http_response_incomplete"))
            .await;
        return Served::Failed;
    };
    observe.upstream(
        &head.target.authority(),
        if answer.status == 407 {
            Met::CredentialsRefused
        } else {
            Met::Reached
        },
    );
    if !(200..300).contains(&answer.status) {
        let _ = browser.write_all(&answer.to_browser).await;
        return Served::Failed;
    }
    if browser.write_all(ESTABLISHED).await.is_err() {
        return Served::Failed;
    }
    forward(browser, proxy, rest, head, relaying, observe).await
}

/// Relays a tunnel through a SOCKS5 proxy that asks for no credentials.
async fn through_socks5<B, N, O>(
    mut browser: B,
    head: &Head,
    rest: Vec<u8>,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    N: Network,
    O: Observe,
{
    let proxy = match connect_socks5(&mut browser, head, (relaying, network), observe).await {
        Ok(proxy) => proxy,
        Err(served) => return served,
    };
    if browser.write_all(ESTABLISHED).await.is_err() {
        return Served::Failed;
    }
    forward(browser, proxy, rest, head, relaying, observe).await
}

/// A SOCKS5 tunnel for `head` through the upstream proxy, which asks for
/// no credentials, given the host's name (RFC 1928); else how the
/// browser's connection ended: the relay's `502` when the proxy refused.
async fn connect_socks5<B, N, O>(
    browser: &mut B,
    head: &Head,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Result<N::Stream, Served>
where
    B: AsyncWrite + Unpin,
    N: Network,
    O: Observe,
{
    let mut proxy = connect_proxy(browser, head, (relaying, network), observe).await?;
    if let Err(error) = socks5_connect(&mut proxy, head.target.host(), head.target.port()).await {
        // A proxy that asks for a method other than none asks for
        // credentials the Session does not give.
        observe.upstream(
            &head.target.authority(),
            if error.kind() == io::ErrorKind::PermissionDenied {
                Met::CredentialsRefused
            } else {
                Met::Refused
            },
        );
        let _ = browser
            .write_all(&failed("502 Bad Gateway", "proxy_internal_error"))
            .await;
        return Err(Served::Failed);
    }
    observe.upstream(&head.target.authority(), Met::Reached);
    Ok(proxy)
}

/// Answers `head`'s connection, which no connection to the upstream proxy
/// carries for `error`: refused as the egress policy refused the proxy's
/// own address, else the relay's `502` for a proxy it could not reach.
async fn unconnected<B, O>(browser: &mut B, head: &Head, error: &io::Error, observe: &O) -> Served
where
    B: AsyncWrite + Unpin,
    O: Observe,
{
    if let Some(refused) = proxy_refused(error) {
        let refused = Refused {
            url: head.target.url(),
            ..refused.clone()
        };
        return refuse(
            browser,
            head,
            &refused,
            observe,
            "destination_ip_prohibited",
        )
        .await;
    }
    observe.upstream(&head.target.authority(), Met::Unreachable);
    let named = if error.kind() == io::ErrorKind::TimedOut {
        "connection_timeout"
    } else {
        "connection_refused"
    };
    let _ = browser.write_all(&failed("502 Bad Gateway", named)).await;
    Served::Failed
}

/// Opens a SOCKS5 tunnel to `host` on `port` over `proxy`, asking for no
/// credentials.
async fn socks5_connect(
    proxy: &mut (impl AsyncRead + AsyncWrite + Unpin),
    host: &str,
    port: u16,
) -> io::Result<()> {
    const VERSION: u8 = 5;
    proxy.write_all(&[VERSION, 1, 0]).await?;
    let mut chosen = [0; 2];
    proxy.read_exact(&mut chosen).await?;
    if chosen != [VERSION, 0] {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    proxy.write_all(&socks5_request(host, port)?).await?;
    let mut reply = [0; 4];
    proxy.read_exact(&mut reply).await?;
    if reply[0] != VERSION || reply[1] != 0 {
        return Err(io::ErrorKind::ConnectionRefused.into());
    }
    let address = match reply[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(proxy.read_u8().await?),
        _ => return Err(io::ErrorKind::InvalidData.into()),
    };
    let mut bound = vec![0; address + 2];
    proxy.read_exact(&mut bound).await?;
    Ok(())
}

/// A SOCKS5 `CONNECT` request to `host`, an address or a name, on `port`
/// (RFC 1928, section 4).
///
/// # Errors
///
/// `InvalidInput` for a name longer than SOCKS5 carries.
pub(crate) fn socks5_request(host: &str, port: u16) -> io::Result<Vec<u8>> {
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(std::net::IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            let name = u8::try_from(host.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
            request.extend_from_slice(&[3, name]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    Ok(request)
}

/// Passes the bytes of `head`'s tunnel both ways, `rest` first, as they
/// are, its ClientHello noted as it passes, or through the terminating
/// mode for a host the Run lists for a certificate exception.
async fn forward<B, S, O>(
    browser: B,
    site: S,
    rest: Vec<u8>,
    head: &Head,
    relaying: &Relaying,
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
    O: Observe,
{
    let host = head.target.host();
    let terminating = relaying
        .terminating
        .as_deref()
        .filter(|terminating| terminating.terminates(host));
    let Some(terminating) = terminating else {
        let authority = head.target.authority();
        let noted = |hello| observe.client_hello(&authority, hello);
        return pass(Noting::new(browser, &rest, noted), site, &rest).await;
    };
    let passed = |used: Option<&ExceptionUsed>| observe.terminated(host, used);
    let stopped = |stop: &Stop| observe.stopped(&head.target.url(), &head.method, stop);
    match terminate::terminate(browser, site, (rest, host), terminating, passed, stopped).await {
        Terminated::Relayed { up, down } => Served::Relayed { up, down },
        Terminated::Stopped => Served::Refused,
        Terminated::Failed => Served::Failed,
    }
}

/// Where a plain request goes after the relay, which a later request on the
/// same browser connection reuses when it goes the same way.
#[derive(Debug, PartialEq, Eq)]
enum Way {
    /// To the site at `authority`, connected to `address`.
    Site {
        authority: String,
        address: SocketAddr,
    },
    /// To the upstream HTTP proxy, which takes any host's request.
    Proxy,
    /// Through a SOCKS5 tunnel to `authority`.
    Socks5 { authority: String },
}

/// The connection a plain request went on, read and written apart.
struct Onward<S> {
    way: Way,
    from: Incoming<ReadHalf<S>>,
    to: WriteHalf<S>,
}

impl<S> Onward<S> {
    /// Whether `head`, going on to `upstream`, may go on this connection:
    /// to the site, only while one of `addresses`, those its own judgment
    /// passed, is the one connected to.
    fn takes(&self, head: &Head, upstream: &Upstream, addresses: &[SocketAddr]) -> bool {
        match (&self.way, upstream) {
            (Way::Site { authority, address }, Upstream::Direct) => {
                *authority == head.target.authority() && addresses.contains(address)
            }
            (Way::Proxy, Upstream::Http { .. }) => true,
            (Way::Socks5 { authority }, Upstream::Socks5) => *authority == head.target.authority(),
            _ => false,
        }
    }
}

/// How a plain request's exchange ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// Its answer ended where its framing says, and the browser's
    /// connection carries the next request.
    Kept,
    /// The browser's connection closes after its answer: the site's
    /// answer ended at its close or asked to close, or the request's body
    /// was still coming when the answer ended.
    Closing,
    /// The site switched protocols (`101`): the bytes pass both ways from
    /// now on.
    Switched,
    /// Either end failed before the exchange ended.
    Failed,
}

/// Relays the plain requests a browser's connection carries, `first` the
/// first and `rest` its bytes past that head (issue 2102): each request is
/// judged on its own by the egress gate ([`judged`]), goes on as the
/// browser sent it ([`Head::as_sent`]), and its answer comes back as the
/// site gave it, ended as its framing says ([`exchange`]). A request goes
/// on the connection the one before went on when it goes the same way, to
/// an address its own judgment passed ([`Onward::takes`]), else on a new
/// one. A request the relay refuses or cannot take on closes the
/// browser's connection, as the first one's does; so does one with other
/// credentials than the Session's, while a later one with none is the
/// connection's own. The connection also closes after an answer that
/// ends it ([`End::Closing`]), when the browser sends no request within
/// [`IDLE_DEADLINE`], or when the site's connection closes first, so the
/// browser's next request opens a new one.
async fn plain<B, N, O>(
    browser: B,
    first: Head,
    rest: Vec<u8>,
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    N: Network,
    O: Observe,
{
    let (from_browser, mut to_browser) = tokio::io::split(browser);
    let mut from_browser = Incoming::new(from_browser, rest);
    let mut onward: Option<Onward<N::Stream>> = None;
    let mut head = first;
    let (mut up, mut down) = (0_u64, 0_u64);
    loop {
        let resolved = match judged(&mut to_browser, &head, (relaying, network), observe).await {
            Ok(resolved) => resolved,
            Err(served) => return served,
        };
        let upstream = upstream_of(relaying, &head.target.url());
        let kept = onward
            .take()
            .filter(|onward| onward.takes(&head, upstream, &resolved.addresses));
        let mut current = if let Some(kept) = kept {
            kept
        } else {
            let opened = open(
                &mut to_browser,
                &head,
                upstream,
                &resolved.addresses,
                (relaying, network),
                observe,
            );
            match opened.await {
                Ok(opened) => opened,
                Err(served) => return served,
            }
        };
        let authorization = match (&current.way, upstream) {
            (Way::Proxy, Upstream::Http { authorization }) => {
                observe.upstream(&head.target.authority(), Met::Reached);
                Some(authorization.as_deref())
            }
            _ => None,
        };
        let sent = head.as_sent(authorization.is_some(), authorization.flatten());
        let (sent_up, sent_down, end) = exchange(
            &head,
            &sent,
            (&mut from_browser, &mut to_browser),
            &mut current,
        )
        .await;
        up = up.saturating_add(sent_up);
        down = down.saturating_add(sent_down);
        match end {
            End::Kept if !head.closes => {}
            End::Kept | End::Closing => {
                let _ = to_browser.shutdown().await;
                return Served::Relayed { up, down };
            }
            End::Switched => {
                let browser = from_browser.into_parts();
                return hand_over(browser, to_browser, current, (up, down)).await;
            }
            End::Failed => return Served::Failed,
        }
        onward = Some(current);
        // A request the browser already sent goes before the site's close.
        let next = within_or_none(&relaying.clock, IDLE_DEADLINE, async {
            tokio::select! {
                biased;
                next = from_browser.request() => Some(next),
                () = closed(&mut onward) => None,
            }
        });
        head = match next.await.flatten() {
            Some(Ok(next)) => next,
            Some(Err(Unread::Malformed)) => {
                let _ = to_browser
                    .write_all(&failed("400 Bad Request", "http_request_error"))
                    .await;
                return Served::Failed;
            }
            Some(Err(Unread::Closed)) | None => {
                let _ = to_browser.shutdown().await;
                return Served::Relayed { up, down };
            }
        };
        let other = head
            .authorization
            .as_deref()
            .is_some_and(|given| !same_token(given, &relaying.authorization));
        if other {
            observe.unauthenticated(true);
            let _ = to_browser.write_all(AUTHENTICATE).await;
            return Served::Unauthenticated;
        }
        if let Target::Tunnel { .. } = head.target {
            drop(onward);
            let (from_browser, rest) = from_browser.into_parts();
            let browser = from_browser.unsplit(to_browser);
            return tunnel(browser, &head, rest, (relaying, network), observe).await;
        }
    }
}

/// Opens the connection `head` goes on to `upstream`: to the first of
/// `addresses` that answers, to the HTTP proxy, or through a SOCKS5
/// tunnel; else how the browser's connection ended.
async fn open<B, N, O>(
    browser: &mut B,
    head: &Head,
    upstream: &Upstream,
    addresses: &[SocketAddr],
    (relaying, network): (&Relaying, &N),
    observe: &O,
) -> Result<Onward<N::Stream>, Served>
where
    B: AsyncWrite + Unpin,
    N: Network,
    O: Observe,
{
    let (way, stream) = match upstream {
        Upstream::Direct => {
            let (stream, address) =
                connect_site(browser, head, addresses, (relaying, network), observe).await?;
            let authority = head.target.authority();
            (Way::Site { authority, address }, stream)
        }
        Upstream::Http { .. } => (
            Way::Proxy,
            connect_proxy(browser, head, (relaying, network), observe).await?,
        ),
        Upstream::Socks5 => (
            Way::Socks5 {
                authority: head.target.authority(),
            },
            connect_socks5(browser, head, (relaying, network), observe).await?,
        ),
    };
    let (from, to) = tokio::io::split(stream);
    Ok(Onward {
        way,
        from: Incoming::new(from, Vec::new()),
        to,
    })
}

/// Sends `head`, as `sent`, and its body on `onward`, and passes its
/// answer back: the bytes that went up and down, and how it ended. The
/// browser's body goes on while the relay reads the answer, so a body sent
/// after an interim answer reaches the site before it answers.
async fn exchange<BR, BW, S>(
    head: &Head,
    sent: &[u8],
    (from_browser, to_browser): (&mut Incoming<BR>, &mut BW),
    onward: &mut Onward<S>,
) -> (u64, u64, End)
where
    BR: AsyncRead + Unpin,
    BW: AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite,
{
    let Onward {
        from: from_site,
        to: to_site,
        ..
    } = onward;
    if to_site.write_all(sent).await.is_err() {
        return (0, 0, End::Failed);
    }
    let sending = async {
        let body = from_browser.pass_body(head.body, to_site).await?;
        to_site.flush().await?;
        Ok::<_, io::Error>(body)
    };
    let answering = answer(from_site, to_browser, head.method == "HEAD");
    let (mut sending, mut answering) = (std::pin::pin!(sending), std::pin::pin!(answering));
    let sent_first = tokio::select! {
        biased;
        body = &mut sending => Some(body),
        answered = &mut answering => {
            // The answer ended while the request's body was still coming.
            return match answered {
                Ok((down, End::Kept | End::Closing | End::Switched)) => {
                    (widened(sent.len()), down, End::Closing)
                }
                _ => (widened(sent.len()), 0, End::Failed),
            };
        }
    };
    let answered = answering.await;
    match (sent_first, answered) {
        (Some(Ok(body)), Ok((down, end))) => (widened(sent.len()).saturating_add(body), down, end),
        (_, Ok((down, End::Kept | End::Closing))) => (widened(sent.len()), down, End::Closing),
        _ => (widened(sent.len()), 0, End::Failed),
    }
}

/// Passes the site's answer to a request, a `HEAD` one when `to_head`, to
/// the browser as it came: its interim answers (`1xx`), then its final
/// one, ended as its framing says; or a protocol switch (`101`), the last
/// head. The bytes that went, and how the exchange ends.
///
/// # Errors
///
/// Either connection's, and `InvalidData` or `UnexpectedEof` for an
/// answer the relay cannot read whole.
async fn answer<R, W>(
    from_site: &mut Incoming<R>,
    to_browser: &mut W,
    to_head: bool,
) -> io::Result<(u64, End)>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut down = 0_u64;
    loop {
        let (bytes, answered) = from_site.answer(to_head).await.map_err(|unread| {
            io::Error::from(match unread {
                Unread::Closed => io::ErrorKind::UnexpectedEof,
                Unread::Malformed => io::ErrorKind::InvalidData,
            })
        })?;
        to_browser.write_all(&bytes).await?;
        to_browser.flush().await?;
        down = down.saturating_add(widened(bytes.len()));
        if answered.status == 101 {
            return Ok((down, End::Switched));
        }
        if (100..200).contains(&answered.status) {
            continue;
        }
        let body = from_site.pass_body(answered.body, to_browser).await?;
        to_browser.flush().await?;
        let end = if answered.closes {
            End::Closing
        } else {
            End::Kept
        };
        return Ok((down.saturating_add(body), end));
    }
}

/// Resolves when the connection `onward` holds closes, or sends a byte
/// that no request asked for, either of which ends it.
async fn closed<S: AsyncRead>(onward: &mut Option<Onward<S>>) {
    match onward {
        Some(onward) => {
            let _ = onward.from.fill().await;
        }
        None => std::future::pending().await,
    }
}

/// Passes the bytes of a connection whose protocol the site switched both
/// ways, each end's bytes read past the switch first, `relayed` the bytes
/// that went before.
async fn hand_over<B, S>(
    (from_browser, early_up): (ReadHalf<B>, Vec<u8>),
    to_browser: WriteHalf<B>,
    onward: Onward<S>,
    (up, down): (u64, u64),
) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut browser = from_browser.unsplit(to_browser);
    let (from_site, early_down) = onward.from.into_parts();
    let site = from_site.unsplit(onward.to);
    if browser.write_all(&early_down).await.is_err() {
        return Served::Failed;
    }
    match pass(browser, site, &early_up).await {
        Served::Relayed {
            up: passed_up,
            down: passed_down,
        } => Served::Relayed {
            up: up.saturating_add(passed_up),
            down: down
                .saturating_add(widened(early_down.len()))
                .saturating_add(passed_down),
        },
        other => other,
    }
}

/// Passes bytes both ways until both ends close, `rest`, the browser's
/// bytes read past its head, first.
async fn pass<B, S>(mut browser: B, mut site: S, rest: &[u8]) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    if !rest.is_empty() && site.write_all(rest).await.is_err() {
        return Served::Failed;
    }
    let early = u64::try_from(rest.len()).unwrap_or(u64::MAX);
    match tokio::io::copy_bidirectional(&mut browser, &mut site).await {
        Ok((up, down)) => Served::Relayed {
            up: up.saturating_add(early),
            down,
        },
        Err(_) => Served::Failed,
    }
}
