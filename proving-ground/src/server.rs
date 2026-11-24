//! The page server: binds the address, answers each connection on a task the
//! server's tracker owns, and hands each request to the site until the
//! process is told to stop. One port serves TLS and plain text: a
//! connection whose first byte starts a TLS handshake gets TLS from the
//! seed's certificate authority, then HTTP/2 or HTTP/1.1 as its first bytes
//! say, whatever ALPN chose. The first request head of an HTTP/1.1
//! connection is read before hyper reads it, so a page whose head hyper
//! cannot write, such as one with a NUL in a header value, is answered with
//! its own bytes.
#![expect(
    clippy::disallowed_types,
    reason = "the Proving ground opens its own listening socket, outside the engine's transport (ADR 0060)"
)]

use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result};
use http_body_util::{BodyExt as _, Either, Full, Limited};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{
    AUTHORIZATION, CACHE_CONTROL, CONNECTION, CONTENT_TYPE, COOKIE, HOST, HeaderValue,
    SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE,
};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::LazyConfigAcceptor;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::authority::Authority;
use crate::blackhole::Blackhole;
use crate::browser_builds::BrowserBuilds;
use crate::certificates;
use crate::client_hello::ClientHello;
use crate::clock;
use crate::exchange::{self, Reply};
use crate::model::{self, Ending};
use crate::observation::{self, Observations};
use crate::online::Online;
use crate::push::{self, EventStream, Push};
use crate::raw;
use crate::raw_site::{self, RawSite};
use crate::site::{Pems, Site};
use crate::tap::{Tap, Wire};

/// The largest request body the server reads, a submission to the oracle
/// included; a larger one is refused (STYLE.md: every buffer is bounded).
const BODY_LIMIT: usize = 16 * 1024 * 1024;

/// The first byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 0x16;

/// The most bytes read of a connection's first request head before hyper
/// reads it; a longer head goes to hyper as it is.
const FIRST_HEAD_LIMIT: usize = 64 * 1024;

/// The most header fields of a first request head that is parsed.
const FIRST_HEAD_FIELDS: usize = 64;

/// A bound listener and the site it serves.
pub(crate) struct Server {
    listener: TcpListener,
    site: Arc<Site>,
    tls: Tls,
    observations: Arc<Observations>,
    /// The pages the raw server answers.
    raw_site: Arc<RawSite>,
    /// The IPv6 path that drops packets, kept while the server runs.
    _blackhole: Option<Blackhole>,
}

/// The TLS configurations: one for most host names, one for those that
/// offer HTTP/1.1 alone, and one for those that require a client
/// certificate.
#[derive(Clone)]
struct Tls {
    any: Arc<ServerConfig>,
    http1: Arc<ServerConfig>,
    client_certificate: Arc<ServerConfig>,
}

impl Server {
    /// Binds the address; port 0 binds a free port. Clients reach it on
    /// `reached_on` when a latency proxy stands in front of it, else on the
    /// port it bound.
    pub(crate) async fn bind(
        address: SocketAddr,
        reached_on: Option<u16>,
        seed: u64,
        builds: BrowserBuilds,
        online: Option<Online>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(address)
            .await
            .with_context(|| format!("binding {address}"))?;
        let port = match reached_on {
            Some(port) => port,
            None => listener
                .local_addr()
                .context("reading the bound address")?
                .port(),
        };
        let hosts = Site::hosts(seed);
        let authority = Authority::new(seed, hosts.keys().map(String::as_str), clock::wall_now())?;
        let tls = Tls {
            any: authority.server_config(),
            http1: authority.http1_server_config(),
            client_certificate: authority.client_certificate_server_config(),
        };
        let observations = Arc::new(Observations::new(hosts));
        let blackhole = if address.is_ipv4() {
            Blackhole::bind(port).await
        } else {
            None
        };
        Ok(Self {
            listener,
            site: Arc::new(Site::new(
                seed,
                port,
                Pems {
                    certificate_authority: authority.certificate_pem().to_owned(),
                    client_certificate: authority.client_certificate_pem().to_owned(),
                    client_key: authority.client_key_pem().to_owned(),
                },
                Arc::clone(&observations),
                builds,
                online,
            )),
            tls,
            observations,
            raw_site: Arc::new(RawSite::new(seed)),
            _blackhole: blackhole,
        })
    }

    /// The site it serves.
    pub(crate) fn site(&self) -> &Site {
        &self.site
    }

    /// What the server observes, which the network fixtures record into
    /// too.
    pub(crate) fn observations(&self) -> Arc<Observations> {
        Arc::clone(&self.observations)
    }

    /// The bound address.
    pub(crate) fn address(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .expect("invariant: a bound listener has an address")
    }

    /// Serves until `stop_signal` is received, then closes every connection
    /// and waits for their tasks.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops the listener; connection tasks already
    /// started run on until their connections close.
    pub(crate) async fn run(self, mut stop_signal: StopSignal) {
        let tracker = TaskTracker::new();
        let stop = CancellationToken::new();
        loop {
            tokio::select! {
                () = stop_signal.received() => break,
                accepted = self.listener.accept() => {
                    let Ok((stream, source)) = accepted else {
                        // A connection that failed before it was accepted
                        // concerns only its client.
                        continue;
                    };
                    let connection = Connection {
                        site: Arc::clone(&self.site),
                        observations: Arc::clone(&self.observations),
                        tls: self.tls.clone(),
                        raw_site: Arc::clone(&self.raw_site),
                        executor: Tracked(tracker.clone()),
                        peer: source.ip(),
                        tunnels: Tunnels {
                            address: self.address(),
                            tracker: tracker.clone(),
                            stop: stop.clone(),
                        },
                    };
                    let stop = stop.clone();
                    tracker.spawn(async move {
                        tokio::select! {
                            () = connection.serve(stream, source) => {}
                            () = stop.cancelled() => {}
                        }
                    });
                }
            }
        }
        tracker.close();
        stop.cancel();
        tracker.wait().await;
    }
}

/// What one connection needs.
struct Connection {
    site: Arc<Site>,
    observations: Arc<Observations>,
    tls: Tls,
    raw_site: Arc<RawSite>,
    executor: Tracked,
    /// The address the connection came from.
    peer: IpAddr,
    tunnels: Tunnels,
}

impl Connection {
    /// Serves one connection to its end; a failed one concerns only its
    /// client. A TLS connection to a host the raw server answers is its.
    async fn serve(self, stream: TcpStream, source: SocketAddr) {
        let Ok(destination) = stream.local_addr() else {
            return;
        };
        let mut first = [0; 1];
        let Ok(read) = stream.peek(&mut first).await else {
            return;
        };
        let (site, tunnels, peer) = (self.site, self.tunnels, self.peer);
        let heads = Arc::clone(&site);
        let observed = self.observations.connection(source, destination);
        let connection = Arc::clone(&observed);
        let service = service_fn(move |request| {
            let (site, tunnels) = (Arc::clone(&site), tunnels.clone());
            // The handshake ended before the first request arrived.
            let tls = connection.tls();
            async move {
                Ok::<_, hyper::Error>(answer(&site, &tunnels, request, peer, tls.as_deref()).await)
            }
        });
        let builder = auto::Builder::new(self.executor);
        let tls = read == 1 && first[0] == TLS_HANDSHAKE;
        let wire = Wire::new(stream, Arc::clone(&observed), tls);
        if tls {
            let Ok(start) =
                LazyConfigAcceptor::new(rustls::server::Acceptor::default(), wire).await
            else {
                return;
            };
            let name = start
                .client_hello()
                .server_name()
                .map(observation::host_name);
            let config = if name
                .as_deref()
                .is_some_and(certificates::requires_client_certificate)
            {
                Arc::clone(&self.tls.client_certificate)
            } else if name
                .as_deref()
                .is_some_and(|name| raw_site::http1_only(name) || Site::http1_only(name))
            {
                Arc::clone(&self.tls.http1)
            } else {
                Arc::clone(&self.tls.any)
            };
            let stream = match start.into_stream(config).into_fallible().await {
                Ok(stream) => stream,
                // Any alert has had its one write attempt; the client's
                // request may follow its last flight unread.
                Err((_, mut wire)) => {
                    crate::linger::linger(&mut wire).await;
                    return;
                }
            };
            if let Some(name) = &name
                && stream.get_ref().1.peer_certificates().is_some()
            {
                self.observations.client_certificate(name);
            }
            if let Some(name) = name.filter(|name| raw_site::serves(name)) {
                let http2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");
                let tapped = Tap::new(stream, Arc::clone(&observed));
                Box::pin(raw::serve(
                    tapped,
                    http2,
                    Some(name),
                    observed,
                    &*self.raw_site,
                ))
                .await;
                return;
            }
            let Some((stream, _)) = first_head(&heads, Tap::new(stream, observed)).await else {
                return;
            };
            let _ = builder
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        } else {
            let Some((stream, host)) =
                first_head(&heads, Tap::new(wire, Arc::clone(&observed))).await
            else {
                return;
            };
            // A raw host's pages over plain HTTP/1.1, for a client that
            // cannot trust the Proving ground's authority.
            if host.is_some_and(|host| raw_site::serves(&observation::host_name(&host))) {
                Box::pin(raw::serve(stream, false, None, observed, &*self.raw_site)).await;
                return;
            }
            let _ = builder
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        }
    }
}

/// Reads a connection's first request head before hyper does. When the
/// site answers it with a whole response of its own bytes, writes them and
/// closes the connection, returning `None`; otherwise returns the stream,
/// which gives what was read first to hyper or the raw server, and the
/// head's `Host`, which tells them apart. Only a first request is answered
/// so: a page whose head hyper cannot write is on its own host name, which
/// a client reaches on a new connection.
async fn first_head<S: AsyncRead + AsyncWrite + Unpin>(
    site: &Site,
    mut stream: S,
) -> Option<(Rewind<S>, Option<String>)> {
    let mut read = Vec::new();
    let mut chunk = [0; 4096];
    while read.len() < FIRST_HEAD_LIMIT && !read.windows(4).any(|end| end == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => read.extend_from_slice(&chunk[..count]),
            Err(_) => return None,
        }
    }
    if let Some(response) = raw_response(site, &read) {
        // A failed write concerns only the client.
        if stream.write_all(&response).await.is_ok() {
            let _ = stream.shutdown().await;
        }
        return None;
    }
    let host = head(&read).map(|(host, _)| host);
    Some((
        Rewind {
            read,
            at: 0,
            inner: stream,
        },
        host,
    ))
}

/// The site's own bytes for the request head `read` begins with, if it
/// answers it so.
fn raw_response(site: &Site, read: &[u8]) -> Option<Vec<u8>> {
    let (host, path) = head(read)?;
    site.raw_response(&host, &path)
}

/// The `Host` and the path of the whole request head `read` begins with.
fn head(read: &[u8]) -> Option<(String, String)> {
    let mut fields = [httparse::EMPTY_HEADER; FIRST_HEAD_FIELDS];
    let mut request = httparse::Request::new(&mut fields);
    if !request.parse(read).ok()?.is_complete() {
        return None;
    }
    let host = request
        .headers
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case("host"))
        .and_then(|field| std::str::from_utf8(field.value).ok())?;
    let target = request.path?;
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    Some((host.to_owned(), path.to_owned()))
}

/// A stream that gives back the bytes read from it first, then reads on.
struct Rewind<S> {
    read: Vec<u8>,
    /// How many of `read` were given back.
    at: usize,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for Rewind<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.at < this.read.len() {
            let count = buffer.remaining().min(this.read.len() - this.at);
            buffer.put_slice(&this.read[this.at..this.at + count]);
            this.at += count;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(context, buffer)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Rewind<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

/// hyper's executor for HTTP/2 streams, which spawns each on the server's
/// tracker (STYLE.md).
#[derive(Clone)]
struct Tracked(TaskTracker);

impl<F> hyper::rt::Executor<F> for Tracked
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn(future);
    }
}

/// The signals that tell the process to stop: Ctrl-C and, on Unix,
/// `SIGTERM`. It listens from the moment it is made, so a signal that
/// arrives before [`StopSignal::received`] is first awaited waits for it
/// instead of meeting the default action, which ends the process at once.
pub(crate) struct StopSignal {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    ctrl_c: tokio::signal::windows::CtrlC,
}

impl StopSignal {
    /// Starts listening, inside the runtime. `serve` calls it before it
    /// reports its address, since a caller may stop it as soon as it reads
    /// the report.
    pub(crate) fn listen() -> Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt()).context("listening for Ctrl-C")?,
                terminate: signal(SignalKind::terminate()).context("listening for SIGTERM")?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                ctrl_c: tokio::signal::windows::ctrl_c().context("listening for Ctrl-C")?,
            })
        }
    }

    /// Resolves when the process is told to stop.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: a signal that arrives while nothing awaits it is kept
    /// for the next call.
    async fn received(&mut self) {
        #[cfg(unix)]
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
        #[cfg(windows)]
        self.ctrl_c.recv().await;
    }
}

/// Reads one request, asks the site, and writes its reply, once the time a
/// held reply asks for has passed. The host is the `Host` header, or an
/// HTTP/2 request's `:authority`; `peer` is the address the request came
/// from, and `tls` its connection's `ClientHello`, `None` over plain text.
async fn answer(
    site: &Site,
    tunnels: &Tunnels,
    request: Request<Incoming>,
    peer: IpAddr,
    tls: Option<&ClientHello>,
) -> Response<Body> {
    if request.method() == Method::CONNECT {
        return tunnels.open(site, request);
    }
    if let Some(push) = pushed(&request) {
        return tunnels.hold_open(push, request);
    }
    let (parts, body) = request.into_parts();
    let header = |name| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let host = header(HOST)
        .or_else(|| {
            parts
                .uri
                .authority()
                .map(hyper::http::uri::Authority::as_str)
        })
        .unwrap_or_default()
        .to_owned();
    // HTTP/2 may split the `Cookie` header into one field per cookie
    // (RFC 9113, section 8.2.3); they are joined back as HTTP/1.1 sends them.
    let cookies = parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join("; ");
    let (reply, held) = match Limited::new(body, BODY_LIMIT).collect().await {
        Ok(body) => {
            let body = body.to_bytes();
            let target = parts
                .uri
                .path_and_query()
                .map_or("/", hyper::http::uri::PathAndQuery::as_str);
            site.sight_canaries(
                &host,
                target,
                parts
                    .headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_bytes())),
                &body,
            );
            if let Some(answer) = site.model(&parts.method, &host, target, &body) {
                return model_response(answer).await;
            }
            if let Some(reply) = site
                .challenge(&parts.method, &host, parts.uri.path(), &body)
                .await
            {
                (reply, None)
            } else if parts.uri.scheme().is_some() && parts.version <= Version::HTTP_11 {
                // A request in absolute form is one to the replay proxy;
                // HTTP/2's requests always carry their scheme.
                (site.proxy(&parts.method, &parts.uri), None)
            } else {
                site.respond_held(
                    &host,
                    &exchange::Request {
                        method: &parts.method,
                        path: parts.uri.path(),
                        query: parts.uri.query().unwrap_or_default(),
                        cookies: &cookies,
                        authorization: header(AUTHORIZATION).unwrap_or_default(),
                        body: &body,
                        peer,
                        tls,
                    },
                )
            }
        }
        Err(_) => (
            Reply::error(StatusCode::PAYLOAD_TOO_LARGE, "body_unreadable"),
            None,
        ),
    };
    if let Some(held) = held {
        held.release().await;
    }
    let mut response = Response::new(Either::Left(Full::new(reply.body)));
    *response.status_mut() = reply.status;
    let headers = response.headers_mut();
    // An empty media type is a reply sent without `Content-Type`.
    if !reply.content_type.is_empty() {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(reply.content_type));
    }
    for (name, value) in reply.headers {
        let value = HeaderValue::try_from(value)
            .expect("invariant: the Proving ground's own header values are visible ASCII");
        headers.append(name, value);
    }
    response
}

/// What a `CONNECT` to the replay proxy needs: the server's own address,
/// which each tunnel connects to so its TLS is the Proving ground's own, and
/// the tracker and stop signal its task runs under.
#[derive(Clone)]
struct Tunnels {
    address: SocketAddr,
    tracker: TaskTracker,
    stop: CancellationToken,
}

impl Tunnels {
    /// Answers a `CONNECT`: `200`, then the tunnel to the server's own
    /// address, when it names an archived host's port 443; `403` otherwise.
    fn open(&self, site: &Site, request: Request<Incoming>) -> Response<Body> {
        let authority = request
            .uri()
            .authority()
            .map(hyper::http::uri::Authority::as_str)
            .unwrap_or_default();
        let mut response = Response::new(Either::Left(Full::new(Bytes::new())));
        if !site.tunnel(authority) {
            *response.status_mut() = StatusCode::FORBIDDEN;
            return response;
        }
        let (address, stop) = (self.address, self.stop.clone());
        self.tracker.spawn(async move {
            tokio::select! {
                () = tunnel(request, address) => {}
                () = stop.cancelled() => {}
            }
        });
        response
    }
}

/// The connection `request` asks the server to hold open, if the site
/// holds one open for it.
fn pushed(request: &Request<Incoming>) -> Option<Push> {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(hyper::http::uri::Authority::as_str)
        })?;
    Site::push(host, request.method(), request.uri().path())
}

impl Tunnels {
    /// Answers a request for a connection the site holds open: a
    /// server-sent events stream, or a WebSocket's `101`, then the socket
    /// on the server's tracker until it closes or the server stops. A
    /// WebSocket request without its handshake headers is answered `400`.
    fn hold_open(&self, push: Push, mut request: Request<Incoming>) -> Response<Body> {
        let mut response = Response::new(Either::Left(Full::new(Bytes::new())));
        match push {
            Push::Events => {
                *response.body_mut() = Either::Right(Either::Right(EventStream::new()));
                let headers = response.headers_mut();
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
                headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            }
            Push::Socket => {
                let header = |name| {
                    request
                        .headers()
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                };
                let upgrade =
                    header(UPGRADE).is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
                let Some(key) = header(SEC_WEBSOCKET_KEY).filter(|_| upgrade) else {
                    *response.status_mut() = StatusCode::BAD_REQUEST;
                    return response;
                };
                let accept = HeaderValue::try_from(push::accept(key))
                    .expect("invariant: an accept value is base64");
                let upgraded = hyper::upgrade::on(&mut request);
                let stop = self.stop.clone();
                self.tracker.spawn(async move {
                    let Ok(upgraded) = upgraded.await else {
                        return;
                    };
                    tokio::select! {
                        () = push::socket(TokioIo::new(upgraded)) => {}
                        () = stop.cancelled() => {}
                    }
                });
                *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
                let headers = response.headers_mut();
                headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
                headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
                headers.insert(SEC_WEBSOCKET_ACCEPT, accept);
            }
        }
        response
    }
}

/// Carries a tunnel's bytes to and from a new connection to `address`
/// until either side closes; a failed one concerns only its client.
async fn tunnel(request: Request<Incoming>, address: SocketAddr) {
    let Ok(upgraded) = hyper::upgrade::on(request).await else {
        return;
    };
    let Ok(mut inner) = TcpStream::connect(address).await else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(upgraded), &mut inner).await;
}

/// The response body: whole, cut before its end, or a server-sent events
/// stream that never ends.
type Body = Either<Full<Bytes>, Either<Cut, EventStream>>;

/// Writes the Fake model's answer: whole, cut before its end, or never.
async fn model_response(answer: model::Answer) -> Response<Body> {
    if answer.ending == Ending::Hold {
        // Holds the request until the client gives up or the server stops.
        let never: Infallible = std::future::pending().await;
        match never {}
    }
    let body = if answer.ending == Ending::Cut {
        Either::Right(Either::Left(Cut::new(answer.body)))
    } else {
        Either::Left(Full::new(answer.body))
    };
    let mut response = Response::new(body);
    *response.status_mut() = answer.status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(answer.content_type));
    for (name, value) in answer.headers {
        let value = HeaderValue::try_from(value)
            .expect("invariant: the Fake model's header values are visible ASCII");
        headers.append(name, value);
    }
    response
}

/// A body that sends its bytes, lets the connection flush them, then fails,
/// so the connection closes before the body's end: the Fake model's cut
/// stream.
pub(crate) struct Cut {
    bytes: Option<Bytes>,
    flushed: bool,
}

impl Cut {
    fn new(bytes: Bytes) -> Self {
        Self {
            bytes: Some(bytes),
            flushed: false,
        }
    }
}

impl hyper::body::Body for Cut {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
        let cut = self.get_mut();
        if let Some(bytes) = cut.bytes.take() {
            return Poll::Ready(Some(Ok(Frame::data(bytes))));
        }
        if !cut.flushed {
            // Pending once, so the connection writes what it holds first.
            cut.flushed = true;
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Some(Err(std::io::Error::other(
            "the Fake model cuts its answer",
        ))))
    }
}
