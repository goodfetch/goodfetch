//! The page server: binds the address, answers each connection over HTTP/1.1
//! on a task the server's tracker owns, and hands each request to the site
//! until the process is told to stop.
#![expect(
    clippy::disallowed_types,
    reason = "the Proving ground opens its own listening socket, outside the engine's transport (ADR 0060)"
)]

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_TYPE, COOKIE, HOST, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::exchange::{self, Reply};
use crate::site::Site;

/// The largest request body the server reads, a submission to the oracle
/// included; a larger one is refused (STYLE.md: every buffer is bounded).
const BODY_LIMIT: usize = 16 * 1024 * 1024;

/// A bound listener and the site it serves.
pub(crate) struct Server {
    listener: TcpListener,
    site: Arc<Site>,
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
        Ok(Self {
            listener,
            site: Arc::new(Site::new(seed, port)),
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
                    let site = Arc::clone(&self.site);
                    let stop = stop.clone();
                    tracker.spawn(async move {
                        let service = service_fn(move |request| {
                            let site = Arc::clone(&site);
                            async move { Ok::<_, hyper::Error>(answer(&site, request).await) }
                        });
                        let connection =
                            http1::Builder::new().serve_connection(TokioIo::new(stream), service);
                        tokio::select! {
                            // A failed connection concerns only its client.
                            _ = connection => {}
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

/// Reads one request, asks the site, and writes its reply.
async fn answer(site: &Site, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let (parts, body) = request.into_parts();
    let header = |name| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    };
    let reply = match Limited::new(body, BODY_LIMIT).collect().await {
        Ok(body) => site.respond(
            header(HOST),
            &exchange::Request {
                method: &parts.method,
                path: parts.uri.path(),
                cookies: header(COOKIE),
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
