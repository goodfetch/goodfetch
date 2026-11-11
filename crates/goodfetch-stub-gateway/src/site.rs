//! The gateway's listener (ADR 0096): a Client's WebSocket opens at
//! `/protocol` and signs in with the token its `hello` presents (spec
//! #586's duty 8). Every other path answers `404`; the gateway serves no
//! file and sets no cookie.

use std::convert::Infallible;
use std::io;
use std::sync::Arc;

use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport::ConsoleListener;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::relay::Gateway;

/// Where a Client's WebSocket opens.
const PROTOCOL: &str = "/protocol";

/// The gateway's listener.
pub(crate) struct Site {
    gateway: Arc<Gateway>,
    address: String,
}

impl Site {
    /// The site behind `gateway`, reached at `listener`'s address.
    ///
    /// # Errors
    ///
    /// When the listener has no address.
    pub(crate) fn new(gateway: Arc<Gateway>, listener: &ConsoleListener) -> io::Result<Self> {
        Ok(Self {
            gateway,
            address: format!("http://{}", listener.local_address()?),
        })
    }

    /// Where the gateway listens, as `http://host:port`.
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// Serves the listener until `owner` stops.
    pub(crate) async fn serve(self: Arc<Self>, listener: ConsoleListener, owner: Arc<TaskOwner>) {
        loop {
            let accepted = tokio::select! {
                () = owner.token().cancelled() => return,
                accepted = listener.accept() => accepted,
            };
            let Ok(stream) = accepted else { continue };
            let (site, connections) = (Arc::clone(&self), Arc::clone(&owner));
            owner.spawn(async move {
                let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                    let (site, connections) = (Arc::clone(&site), Arc::clone(&connections));
                    async move { Ok::<_, Infallible>(site.answer(request, &connections)) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    }

    /// What the gateway answers `request`.
    fn answer(&self, request: Request<Incoming>, owner: &Arc<TaskOwner>) -> Response<Full<Bytes>> {
        if request.uri().path() == PROTOCOL {
            self.protocol(request, owner)
        } else {
            plain(StatusCode::NOT_FOUND)
        }
    }

    /// `GET /protocol`: a Client's WebSocket, served by the gateway, which
    /// signs it in with the token its `hello` presents.
    fn protocol(
        &self,
        mut request: Request<Incoming>,
        owner: &Arc<TaskOwner>,
    ) -> Response<Full<Bytes>> {
        if request.method() != Method::GET {
            return plain(StatusCode::METHOD_NOT_ALLOWED);
        }
        let Some(key) = request
            .headers()
            .get(header::SEC_WEBSOCKET_KEY)
            .map(|key| derive_accept_key(key.as_bytes()))
        else {
            return plain(StatusCode::BAD_REQUEST);
        };
        let upgrade = hyper::upgrade::on(&mut request);
        let (gateway, connection) = (Arc::clone(&self.gateway), owner.child());
        owner.spawn(async move {
            match upgrade.await {
                Ok(upgraded) => {
                    let socket = WebSocketStream::from_raw_socket(
                        TokioIo::new(upgraded),
                        Role::Server,
                        None,
                    )
                    .await;
                    gateway.serve(socket, connection).await;
                }
                Err(error) => tracing::debug!(%error, "a Client's WebSocket did not open"),
            }
        });
        let mut response = plain(StatusCode::SWITCHING_PROTOCOLS);
        let headers = response.headers_mut();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        if let Ok(accept) = HeaderValue::from_str(&key) {
            headers.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
        }
        response
    }
}

/// An answer with no body, which nothing stores.
fn plain(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
