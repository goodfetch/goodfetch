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
use hyper::header::{CONTENT_TYPE, HOST, HeaderValue};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::authority::Authority;
use crate::clock;
use crate::observation::Observations;
use crate::site::{Reply, Site};
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
    acceptor: TlsAcceptor,
    observations: Arc<Observations>,
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
        let hosts = Site::hosts();
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
                    let Ok((stream, _)) = accepted else {
                        // A connection that failed before it was accepted
                        // concerns only its client.
                        continue;
                    };
                    let connection = Connection {
                        site: Arc::clone(&self.site),
                        observations: Arc::clone(&self.observations),
                        acceptor: self.acceptor.clone(),
                        executor: Tracked(tracker.clone()),
                    };
                    let stop = stop.clone();
                    tracker.spawn(async move {
                        tokio::select! {
                            () = connection.serve(stream) => {}
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
}

impl Connection {
    /// Serves one connection to its end; a failed one concerns only its
    /// client.
    async fn serve(self, stream: TcpStream) {
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
        let observed = self.observations.connection();
        let tls = read == 1 && first[0] == TLS_HANDSHAKE;
        let wire = Wire::new(stream, Arc::clone(&observed), tls);
        if tls {
            let Ok(stream) = self.acceptor.accept(wire).await else {
                return;
            };
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
    let host = parts
        .headers
        .get(HOST)
        .and_then(|host| host.to_str().ok())
        .or_else(|| {
            parts
                .uri
                .authority()
                .map(hyper::http::uri::Authority::as_str)
        })
        .unwrap_or_default()
        .to_owned();
    let reply = match Limited::new(body, BODY_LIMIT).collect().await {
        Ok(body) => site.respond(&parts.method, &host, parts.uri.path(), &body.to_bytes()),
        Err(_) => Reply::error(StatusCode::PAYLOAD_TOO_LARGE, "body_unreadable"),
    };
    let mut response = Response::new(Full::new(reply.body));
    *response.status_mut() = reply.status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(reply.content_type));
    response
}
