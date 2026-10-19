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
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HOST, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::authority::Authority;
use crate::clock;
use crate::exchange::{self, Reply};
use crate::model::{self, Ending};
use crate::observation::Observations;
use crate::site::Site;
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
    acceptor: TlsAcceptor,
    observations: Arc<Observations>,
}

impl Server {
    /// Binds the address; port 0 binds a free port. Clients reach it on
    /// `reached_on` when a latency proxy stands in front of it, else on the
    /// port it bound.
    pub(crate) async fn bind(
        address: SocketAddr,
        reached_on: Option<u16>,
        seed: u64,
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
        let acceptor = TlsAcceptor::from(authority.server_config());
        let observations = Arc::new(Observations::new(hosts));
        Ok(Self {
            listener,
            site: Arc::new(Site::new(
                seed,
                port,
                authority.certificate_pem().to_owned(),
                Arc::clone(&observations),
            )),
            acceptor,
            observations,
        })
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

    /// Serves until the process receives Ctrl-C (or `SIGTERM` on Unix), then
    /// closes every connection and waits for their tasks.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops the listener; connection tasks already
    /// started run on until their connections close.
    pub(crate) async fn run(self) -> Result<()> {
        let tracker = TaskTracker::new();
        let stop = CancellationToken::new();
        let stopped = stop_signal();
        tokio::pin!(stopped);
        loop {
            tokio::select! {
                result = &mut stopped => {
                    result?;
                    break;
                }
                accepted = self.listener.accept() => {
                    let Ok((stream, source)) = accepted else {
                        // A connection that failed before it was accepted
                        // concerns only its client.
                        continue;
                    };
                    let connection = Connection {
                        site: Arc::clone(&self.site),
                        observations: Arc::clone(&self.observations),
                        acceptor: self.acceptor.clone(),
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
        Ok(())
    }
}

/// What one connection needs.
struct Connection {
    site: Arc<Site>,
    observations: Arc<Observations>,
    acceptor: TlsAcceptor,
    executor: Tracked,
    /// The address the connection came from.
    peer: IpAddr,
    tunnels: Tunnels,
}

impl Connection {
    /// Serves one connection to its end; a failed one concerns only its
    /// client.
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
        let service = service_fn(move |request| {
            let (site, tunnels) = (Arc::clone(&site), tunnels.clone());
            async move { Ok::<_, hyper::Error>(answer(&site, &tunnels, request, peer).await) }
        });
        let builder = auto::Builder::new(self.executor);
        let observed = self.observations.connection(source, destination);
        let tls = read == 1 && first[0] == TLS_HANDSHAKE;
        let wire = Wire::new(stream, Arc::clone(&observed), tls);
        if tls {
            let Ok(stream) = self.acceptor.accept(wire).await else {
                return;
            };
            let Some(stream) = first_head(&heads, Tap::new(stream, observed)).await else {
                return;
            };
            let _ = builder
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        } else {
            let Some(stream) = first_head(&heads, Tap::new(wire, observed)).await else {
                return;
            };
            let _ = builder
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        }
    }
}

/// Reads a connection's first request head before hyper does. When the
/// site answers it with a whole response of its own bytes, writes them and
/// closes the connection, returning `None`; otherwise returns the stream,
/// which gives hyper what was read first. Only a first request is answered
/// so: a page whose head hyper cannot write is on its own host name, which
/// a client reaches on a new connection.
async fn first_head<S: AsyncRead + AsyncWrite + Unpin>(
    site: &Site,
    mut stream: S,
) -> Option<Rewind<S>> {
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
    Some(Rewind {
        read,
        at: 0,
        inner: stream,
    })
}

/// The site's own bytes for the request head `read` begins with, if it
/// answers it so.
fn raw_response(site: &Site, read: &[u8]) -> Option<Vec<u8>> {
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
    site.raw_response(host, path)
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

/// Resolves when the process is told to stop.
async fn stop_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("listening for SIGTERM")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("listening for Ctrl-C"),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("listening for Ctrl-C")
    }
}

/// Reads one request, asks the site, and writes its reply, once the time a
/// held reply asks for has passed. The host is the `Host` header, or an
/// HTTP/2 request's `:authority`; `peer` is the address the request came
/// from.
async fn answer(
    site: &Site,
    tunnels: &Tunnels,
    request: Request<Incoming>,
    peer: IpAddr,
) -> Response<Body> {
    if request.method() == Method::CONNECT {
        return tunnels.open(site, request);
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
            // A request in absolute form is one to the replay proxy;
            // HTTP/2's requests always carry their scheme.
            if parts.uri.scheme().is_some() && parts.version <= Version::HTTP_11 {
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

/// The response body: whole, or cut before its end.
type Body = Either<Full<Bytes>, Cut>;

/// Writes the Fake model's answer: whole, cut before its end, or never.
async fn model_response(answer: model::Answer) -> Response<Body> {
    if answer.ending == Ending::Hold {
        // Holds the request until the client gives up or the server stops.
        let never: Infallible = std::future::pending().await;
        match never {}
    }
    let body = if answer.ending == Ending::Cut {
        Either::Right(Cut::new(answer.body))
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
