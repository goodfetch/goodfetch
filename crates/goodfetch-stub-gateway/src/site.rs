//! The gateway's two origins (ADRs 0031 and 0096): the Console's, which
//! serves Foxhound's build through the engine's Console module under the
//! production headers, its policy naming the content origin, and the
//! content origin's own. A browser signs in at `POST /sign-in` with the
//! configured token, swapped for an `HttpOnly`, `SameSite=Strict` cookie;
//! `GET /session` answers whether a request's cookie is a sign-in, and a
//! browser's WebSocket at `/protocol` opens only with one, on the Console's
//! own `Host` and `Origin`. A program's WebSocket, which carries no
//! `Origin`, opens there too, and signs in with the token its `hello`
//! presents.

use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::io;
use std::sync::Arc;

use goodfetch_engine::ConsoleBuild;
use goodfetch_engine::console::{build_file, content_security_policy, production_headers};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport::ConsoleListener;
use goodfetch_protocol::audit::{Listener, RefusalReason};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::sync::Mutex;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::relay::{Door, Gateway};

/// Where the page swaps the token for its cookie.
const SIGN_IN: &str = "/sign-in";
/// Where the page asks whether its cookie is a sign-in.
const SESSION: &str = "/session";
/// Where the page ends its sign-in.
const SIGN_OUT: &str = "/sign-out";
/// Where the Console's WebSocket opens.
const PROTOCOL: &str = "/protocol";
/// The sign-in cookie's name.
const COOKIE: &str = "goodfetch_stub_gateway";
/// The longest token `POST /sign-in` reads.
const MAX_TOKEN_BYTES: usize = 1024;
/// What the content origin's answers carry: nothing runs or loads there.
const CONTENT_POLICY: &str = "default-src 'none'; sandbox";

/// The gateway's origins and sign-ins.
pub(crate) struct Site {
    build: &'static ConsoleBuild,
    token: String,
    gateway: Arc<Gateway>,
    /// The Console's `host:port` names: its address and `localhost`.
    authorities: [String; 2],
    console: String,
    content: String,
    policy: HeaderValue,
    /// Each sign-in's cookie value and CSRF token.
    sign_ins: Mutex<SignIns>,
}

/// The sign-ins the gateway gave.
struct SignIns {
    cookies: HashSet<String>,
    entropy: Entropy,
}

impl SignIns {
    /// A new unguessable value: 256 bits of the gateway's generator.
    fn draw(&mut self) -> String {
        (0..4).fold(String::new(), |mut drawn, _| {
            let _ = write!(drawn, "{:016x}", self.entropy.next_u64());
            drawn
        })
    }
}

impl Site {
    /// The site of `build` behind `gateway`, reached at `console`'s address
    /// and showing files from `content`'s.
    ///
    /// # Errors
    ///
    /// When a listener has no address.
    pub(crate) fn new(
        build: &'static ConsoleBuild,
        token: String,
        gateway: Arc<Gateway>,
        console: &ConsoleListener,
        content: &ConsoleListener,
        entropy: Entropy,
    ) -> io::Result<Self> {
        let address = console.local_address()?;
        let content = format!("http://{}", content.local_address()?);
        let policy = HeaderValue::from_str(&content_security_policy(Some(&content)))
            .expect("invariant: the Console's policy is header text");
        Ok(Self {
            build,
            token,
            gateway,
            authorities: [address.to_string(), format!("localhost:{}", address.port())],
            console: format!("http://{address}"),
            content,
            policy,
            sign_ins: Mutex::new(SignIns {
                cookies: HashSet::new(),
                entropy,
            }),
        })
    }

    /// The Console's origin.
    pub(crate) fn console_origin(&self) -> &str {
        &self.console
    }

    /// The content origin.
    pub(crate) fn content_origin(&self) -> &str {
        &self.content
    }

    /// Serves the content origin until `owner` stops. It shows no Run's
    /// file yet: each request answers `404` under its own policy.
    pub(crate) async fn serve_content(
        self: Arc<Self>,
        listener: ConsoleListener,
        owner: Arc<TaskOwner>,
    ) {
        loop {
            let accepted = tokio::select! {
                () = owner.token().cancelled() => return,
                accepted = listener.accept() => accepted,
            };
            let Ok(stream) = accepted else { continue };
            owner.spawn(async move {
                let service = hyper::service::service_fn(|_: Request<Incoming>| async {
                    let mut response = Response::new(Full::new(Bytes::new()));
                    *response.status_mut() = StatusCode::NOT_FOUND;
                    Ok::<_, Infallible>(production_headers(
                        response,
                        &HeaderValue::from_static(CONTENT_POLICY),
                    ))
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    }

    /// Serves the Console's origin until `owner` stops.
    pub(crate) async fn serve_console(
        self: Arc<Self>,
        listener: ConsoleListener,
        owner: Arc<TaskOwner>,
    ) {
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
                    async move { Ok::<_, Infallible>(site.answer(request, &connections).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    }

    /// What the Console's origin answers `request`.
    async fn answer(
        &self,
        request: Request<Incoming>,
        owner: &Arc<TaskOwner>,
    ) -> Response<Full<Bytes>> {
        let path = request.uri().path().to_owned();
        match path.as_str() {
            SIGN_IN => self.sign_in(request).await,
            SESSION => self.session(request.headers()).await,
            SIGN_OUT => self.sign_out(request.headers()).await,
            PROTOCOL => self.protocol(request, owner).await,
            path => build_file(self.build, request.method(), path, &self.policy),
        }
    }

    /// `POST /sign-in`: the configured token swapped for a cookie.
    async fn sign_in(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        if request.method() != Method::POST {
            return self.plain(StatusCode::METHOD_NOT_ALLOWED);
        }
        if !self.own(request.headers()) {
            return self.plain(StatusCode::FORBIDDEN);
        }
        let Ok(body) = Limited::new(request.into_body(), MAX_TOKEN_BYTES)
            .collect()
            .await
        else {
            return self.plain(StatusCode::PAYLOAD_TOO_LARGE);
        };
        let body = body.to_bytes();
        if std::str::from_utf8(&body).unwrap_or_default().trim() != self.token {
            tracing::warn!(
                listener = Listener::Console.as_str(),
                reason = RefusalReason::TokenWrong.as_str(),
                "the gateway refused a sign-in"
            );
            return self.plain(StatusCode::UNAUTHORIZED);
        }
        let cookie = {
            let mut sign_ins = self.sign_ins.lock().await;
            let cookie = sign_ins.draw();
            sign_ins.cookies.insert(cookie.clone());
            cookie
        };
        let mut response = self.plain(StatusCode::NO_CONTENT);
        if let Ok(value) = HeaderValue::from_str(&format!(
            "{COOKIE}={cookie}; Path=/; HttpOnly; SameSite=Strict"
        )) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
        response
    }

    /// `GET /session`: `200` with a CSRF token for a sign-in's cookie,
    /// `401` otherwise.
    async fn session(&self, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if !self.signed_in(headers).await {
            return self.plain(StatusCode::UNAUTHORIZED);
        }
        let csrf_token = self.sign_ins.lock().await.draw();
        let mut response = self.plain(StatusCode::OK);
        *response.body_mut() = Full::new(Bytes::from(
            serde_json::json!({"csrf_token": csrf_token, "offline_cache": false}).to_string(),
        ));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response
    }

    /// `POST /sign-out`: the request's sign-in ended.
    async fn sign_out(&self, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if let Some(cookie) = cookie_of(headers) {
            self.sign_ins.lock().await.cookies.remove(cookie);
        }
        self.plain(StatusCode::NO_CONTENT)
    }

    /// `GET /protocol`: a Client's WebSocket, served by the gateway. A
    /// browser's, which carries an `Origin`, opens on the Console's own
    /// origin with a sign-in's cookie; a program's, which carries none,
    /// signs in with the token its `hello` presents (spec #586's duty 8).
    async fn protocol(
        &self,
        mut request: Request<Incoming>,
        owner: &Arc<TaskOwner>,
    ) -> Response<Full<Bytes>> {
        let headers = request.headers();
        if request.method() != Method::GET {
            return self.plain(StatusCode::METHOD_NOT_ALLOWED);
        }
        let door = if headers.get(header::ORIGIN).is_some() {
            if !self.own(headers) {
                return self.plain(StatusCode::FORBIDDEN);
            }
            if !self.signed_in(headers).await {
                return self.plain(StatusCode::UNAUTHORIZED);
            }
            Door::Cookie
        } else {
            Door::Hello
        };
        let Some(key) = headers
            .get(header::SEC_WEBSOCKET_KEY)
            .map(|key| derive_accept_key(key.as_bytes()))
        else {
            return self.plain(StatusCode::BAD_REQUEST);
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
                    gateway.serve(socket, door, connection).await;
                }
                Err(error) => tracing::debug!(%error, "a Console's WebSocket did not open"),
            }
        });
        let mut response = self.plain(StatusCode::SWITCHING_PROTOCOLS);
        let headers = response.headers_mut();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        if let Ok(accept) = HeaderValue::from_str(&key) {
            headers.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
        }
        response
    }

    /// Whether the request's `Host`, and its `Origin` when it has one, are
    /// the Console's own.
    fn own(&self, headers: &HeaderMap) -> bool {
        let value = |name| headers.get(name).and_then(|value| value.to_str().ok());
        let host =
            value(header::HOST).is_some_and(|host| self.authorities.iter().any(|own| own == host));
        let origin = value(header::ORIGIN).is_none_or(|origin| {
            self.authorities
                .iter()
                .any(|own| origin == format!("http://{own}"))
        });
        host && origin
    }

    /// Whether the request carries a sign-in's cookie.
    async fn signed_in(&self, headers: &HeaderMap) -> bool {
        match cookie_of(headers) {
            Some(cookie) => self.sign_ins.lock().await.cookies.contains(cookie),
            None => false,
        }
    }

    /// An answer with no body, under the production headers.
    fn plain(&self, status: StatusCode) -> Response<Full<Bytes>> {
        let mut response = Response::new(Full::new(Bytes::new()));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        production_headers(response, &self.policy)
    }
}

/// The gateway's sign-in cookie among a request's `Cookie` headers.
fn cookie_of(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE).then_some(value)
        })
}
