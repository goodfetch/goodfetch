//! The page server: binds the address, answers each connection on a task the
//! server's tracker owns, and hands each request to the site until the
//! process is told to stop. One port serves TLS and plain text: a
//! connection whose first byte starts a TLS handshake gets TLS from the
//! seed's certificate authority, then HTTP/2 or HTTP/1.1 as its first bytes
//! say, whatever ALPN chose.
#![expect(
    clippy::disallowed_types,
    reason = "the Proving ground opens its own listening socket, outside the engine's transport (ADR 0060)"
)]

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_TYPE, COOKIE, HOST, HeaderValue};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use rustls::ServerConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::LazyConfigAcceptor;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::authority::Authority;
use crate::blackhole::Blackhole;
use crate::clock;
use crate::exchange::{self, Reply};
use crate::observation::{self, Observations};
use crate::raw;
use crate::raw_site::{self, RawSite};
use crate::site::Site;
use crate::tap::{Tap, Wire};

/// The largest request body the server reads, a submission to the oracle
/// included; a larger one is refused (STYLE.md: every buffer is bounded).
const BODY_LIMIT: usize = 16 * 1024 * 1024;

/// The first byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 0x16;

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
/// offer HTTP/1.1 alone.
#[derive(Clone)]
struct Tls {
    any: Arc<ServerConfig>,
    http1: Arc<ServerConfig>,
}

impl Server {
    /// Binds the address; port 0 binds a free port.
    pub(crate) async fn bind(address: SocketAddr, seed: u64) -> Result<Self> {
        let listener = TcpListener::bind(address)
            .await
            .with_context(|| format!("binding {address}"))?;
        let port = listener
            .local_addr()
            .context("reading the bound address")?
            .port();
        let hosts = Site::hosts(seed);
        let authority = Authority::new(seed, hosts.keys().map(String::as_str), clock::wall_now())?;
        let tls = Tls {
            any: authority.server_config(),
            http1: authority.http1_server_config(),
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
                authority.certificate_pem().to_owned(),
                Arc::clone(&observations),
            )),
            tls,
            observations,
            raw_site: Arc::new(RawSite::new(seed)),
            _blackhole: blackhole,
        })
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
                    let Ok((stream, peer)) = accepted else {
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
                    };
                    let stop = stop.clone();
                    tracker.spawn(async move {
                        tokio::select! {
                            () = connection.serve(stream, peer) => {}
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
    tls: Tls,
    raw_site: Arc<RawSite>,
    executor: Tracked,
}

impl Connection {
    /// Serves one connection to its end; a failed one concerns only its
    /// client. A TLS connection to a host the raw server answers is its.
    async fn serve(self, stream: TcpStream, peer: SocketAddr) {
        let mut first = [0; 1];
        let Ok(read) = stream.peek(&mut first).await else {
            return;
        };
        let site = self.site;
        let service = service_fn(move |request| {
            let site = Arc::clone(&site);
            async move { Ok::<_, hyper::Error>(answer(&site, request).await) }
        });
        let builder = auto::Builder::new(self.executor);
        let observed = self.observations.connection_from(peer);
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
            let config = if name.as_deref().is_some_and(raw_site::http1_only) {
                Arc::clone(&self.tls.http1)
            } else {
                Arc::clone(&self.tls.any)
            };
            let Ok(stream) = start.into_stream(config).await else {
                return;
            };
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
            let _ = builder
                .serve_connection(TokioIo::new(Tap::new(stream, observed)), service)
                .await;
        } else {
            let _ = builder
                .serve_connection(TokioIo::new(Tap::new(wire, observed)), service)
                .await;
        }
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

/// Reads one request, asks the site, and writes its reply. The host is the
/// `Host` header, or an HTTP/2 request's `:authority`.
async fn answer(site: &Site, request: Request<Incoming>) -> Response<Full<Bytes>> {
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
    let reply = match Limited::new(body, BODY_LIMIT).collect().await {
        Ok(body) => site.respond(
            &host,
            &exchange::Request {
                method: &parts.method,
                path: parts.uri.path(),
                query: parts.uri.query().unwrap_or_default(),
                cookies: &cookies,
                body: &body.to_bytes(),
            },
        ),
        Err(_) => Reply::error(StatusCode::PAYLOAD_TOO_LARGE, "body_unreadable"),
    };
    let mut response = Response::new(Full::new(reply.body));
    *response.status_mut() = reply.status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(reply.content_type));
    for (name, value) in reply.headers {
        let value = HeaderValue::try_from(value)
            .expect("invariant: the Proving ground's own header values are visible ASCII");
        headers.append(name, value);
    }
    response
}
