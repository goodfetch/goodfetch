//! One browser connection through the egress relay, over any streams and
//! any [`Network`], so it is tested and measured in process: the browser's
//! credentials checked first, then the egress policy on the host and on
//! every address the connection may use, which the relay resolves itself,
//! then the connection opened to those addresses alone, or through the
//! Session's upstream proxy, and the bytes passed both ways unchanged, a
//! tunnel's ClientHello noted as it passes. A client that sends no request
//! head within [`HEAD_DEADLINE`] is dropped, and each connection the relay
//! opens has its connect deadline, so a proxy that never answers is the
//! proxy's failure.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};
use goodfetch_engine::endpoint::same_token;
use goodfetch_engine::tls::ExceptionUsed;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::client_hello::{ClientHello, Noting};
use super::deadlines::{HEAD_DEADLINE, within, within_or_none};
use super::head::{self, Head, Target, Unread};
use super::terminate::{self, Stop, Terminated, Terminating};

/// The name the relay gives its own member of a `Proxy-Status` field (RFC
/// 9209).
pub(crate) const RELAY: &str = "goodfetch-relay";

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
    /// The browser's own name of the failure.
    pub(crate) fn net_error(self) -> &'static str {
        match self {
            Self::NameNotResolved => "net::ERR_NAME_NOT_RESOLVED",
            Self::ConnectionRefused => "net::ERR_CONNECTION_REFUSED",
            Self::TimedOut => "net::ERR_CONNECTION_TIMED_OUT",
            Self::ConnectionFailed => "net::ERR_CONNECTION_FAILED",
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

/// The answer to a client without the Session's credentials.
const AUTHENTICATE: &[u8] = b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"goodfetch\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// The answer to a tunnel that opened.
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

/// The relay's own answer to a tunnel that failed with `error` (RFC 9209),
/// and `status`.
fn failed(status: &str, error: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nProxy-Status: {RELAY}; error={error}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
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
    let url = head.target.url();
    let request = egress::Request {
        url: &url,
        kind: RequestKind::Page,
    };
    if let Judgment::Refused(refused) = egress::judge(&request, &relaying.policy) {
        return refuse(
            &mut browser,
            &head,
            &refused,
            observe,
            "http_request_denied",
        )
        .await;
    }
    // The relay resolves the name itself and judges each answer before
    // any connection, so an answer that changes after the browser's own
    // check reaches nothing (ADR 0017); behind a proxy, which resolves the
    // name again, as the engine's own client does before sending to one.
    let Some(resolved) = network
        .resolve(head.target.host(), head.target.port())
        .await
    else {
        return unreached(
            &mut browser,
            &head,
            observe,
            Unreached::NameNotResolved,
            "dns_error",
        )
        .await;
    };
    if let Err(refused) = egress::judge_addresses(&url, &resolved.addresses, &relaying.policy) {
        return refuse(
            &mut browser,
            &head,
            &refused,
            observe,
            "destination_ip_prohibited",
        )
        .await;
    }
    let upstream = if (relaying.bypasses)(&url) {
        &Upstream::Direct
    } else {
        &relaying.upstream
    };
    match upstream {
        Upstream::Direct => {
            direct(
                browser,
                &head,
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
                &head,
                rest,
                authorization.as_deref(),
                (relaying, network),
                observe,
            )
            .await
        }
        Upstream::Socks5 => {
            through_socks5(browser, &head, rest, (relaying, network), observe).await
        }
    }
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

/// Connects to the first of `addresses` that answers, and relays.
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
    let mut site = None;
    let mut last = Unreached::ConnectionFailed;
    for address in addresses {
        match relaying.connect(network, *address).await {
            Ok(stream) => {
                site = Some(stream);
                break;
            }
            Err(error) => last = Unreached::of(&error),
        }
    }
    let Some(mut site) = site else {
        let error = match last {
            Unreached::TimedOut => "connection_timeout",
            _ => "connection_refused",
        };
        return unreached(&mut browser, head, observe, last, error).await;
    };
    observe.reached(&head.target.authority());
    let opened = match head.target {
        Target::Tunnel { .. } => browser.write_all(ESTABLISHED).await,
        Target::Absolute { .. } => site.write_all(&head.as_sent(false, None)).await,
    };
    if opened.is_err() {
        return Served::Failed;
    }
    forward(browser, site, rest, head, relaying, observe).await
}

/// Relays through an HTTP proxy: a tunnel through its own tunnel to the
/// host's name, a plain request in absolute form; its answers reach the
/// browser as it gave them, its body left out.
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
    let mut proxy = match relaying.connect_upstream(network).await {
        Ok(proxy) => proxy,
        Err(error) => return unconnected(&mut browser, head, &error, observe).await,
    };
    if let Target::Absolute { .. } = head.target {
        observe.upstream(&head.target.authority(), Met::Reached);
        if proxy
            .write_all(&head.as_sent(true, authorization))
            .await
            .is_err()
        {
            return Served::Failed;
        }
        return answered_once(browser, proxy, &rest).await;
    }
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

/// Relays through a SOCKS5 proxy that asks for no credentials, giving it
/// the host's name (RFC 1928).
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
    let mut proxy = match relaying.connect_upstream(network).await {
        Ok(proxy) => proxy,
        Err(error) => return unconnected(&mut browser, head, &error, observe).await,
    };
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
        return Served::Failed;
    }
    observe.upstream(&head.target.authority(), Met::Reached);
    let opened = match head.target {
        Target::Tunnel { .. } => browser.write_all(ESTABLISHED).await,
        Target::Absolute { .. } => proxy.write_all(&head.as_sent(false, None)).await,
    };
    if opened.is_err() {
        return Served::Failed;
    }
    forward(browser, proxy, rest, head, relaying, observe).await
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

/// Passes the bytes of `head`'s connection both ways, `rest` first, as they
/// are, a tunnel's ClientHello noted as it passes, or through the
/// terminating mode for a tunnel to a host the Run lists for a certificate
/// exception.
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
    let (Target::Tunnel { .. }, Some(terminating)) = (&head.target, terminating) else {
        if let Target::Absolute { .. } = head.target {
            return answered_once(browser, site, &rest).await;
        }
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

/// Passes a plain request's bytes both ways, as [`pass`] does, with the
/// site's answer head marked `Connection: close` on its way to the
/// browser ([`head::closing`]), so the browser's connection carries that
/// one request. The browser's bytes go on to the site while the relay
/// reads that head, so a request body longer than the first read, or one
/// sent after an interim answer, reaches the site before it answers. An
/// answer head the relay cannot read fails the request, as a site that
/// never answered does.
async fn answered_once<B, S>(browser: B, site: S, rest: &[u8]) -> Served
where
    B: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut from_browser, mut to_browser) = tokio::io::split(browser);
    let (mut from_site, mut to_site) = tokio::io::split(site);
    let up = async {
        to_site.write_all(rest).await?;
        let up = tokio::io::copy(&mut from_browser, &mut to_site).await?;
        to_site.shutdown().await?;
        Ok::<_, io::Error>(up.saturating_add(u64::try_from(rest.len()).unwrap_or(u64::MAX)))
    };
    let down = async {
        let (answer, after) = head::closing(&mut from_site)
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        to_browser.write_all(&answer).await?;
        to_browser.write_all(&after).await?;
        let down = tokio::io::copy(&mut from_site, &mut to_browser).await?;
        to_browser.shutdown().await?;
        let early = answer.len().saturating_add(after.len());
        Ok(down.saturating_add(u64::try_from(early).unwrap_or(u64::MAX)))
    };
    match tokio::try_join!(up, down) {
        Ok((up, down)) => Served::Relayed { up, down },
        Err(_) => Served::Failed,
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
