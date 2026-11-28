//! Shared loopback HTTP/1.1 JSON server helper for the local chain and x402
//! facilitator stand-ins (#1537).
#![expect(
    clippy::disallowed_types,
    reason = "the payment stand-ins open loopback listeners outside the engine transport (ADR 0060)"
)]

use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};

use anyhow::{Context as _, Result};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// Binds a loopback listener on `(host, 0)`.
pub(super) async fn bind(host: IpAddr, label: &str) -> Result<TcpListener> {
    TcpListener::bind((host, 0))
        .await
        .with_context(|| format!("binding {label}"))
}

/// Returns the bound address of `listener`.
pub(super) fn address(listener: &TcpListener, label: &str) -> Result<SocketAddr> {
    listener
        .local_addr()
        .with_context(|| format!("reading {label}'s address"))
}

/// Serves HTTP/1.1 requests on `listener` until `stop` is cancelled, cancelling
/// any in-flight keep-alive connections on shutdown.
pub(super) async fn serve<S, F, Fut>(
    listener: TcpListener,
    state: S,
    handler: F,
    tracker: TaskTracker,
    stop: CancellationToken,
) where
    S: Clone + Send + Sync + 'static,
    F: Fn(S, Request<Incoming>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response<Full<Bytes>>> + Send + 'static,
{
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _)) = accepted else {
            continue;
        };
        let state = state.clone();
        let handler = handler.clone();
        let conn_stop = stop.clone();
        tracker.spawn(async move {
            let service = service_fn(move |request| {
                let state = state.clone();
                let handler = handler.clone();
                async move { Ok::<_, Infallible>(handler(state, request).await) }
            });
            let conn = http1::Builder::new().serve_connection(TokioIo::new(stream), service);
            tokio::select! {
                _ = conn => {}
                () = conn_stop.cancelled() => {}
            }
        });
    }
}

/// Builds a JSON HTTP response with `application/json` content type.
pub(super) fn json_response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(value).expect("invariant: JSON value serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("invariant: JSON response builds")
}
