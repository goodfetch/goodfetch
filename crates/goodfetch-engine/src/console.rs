//! The Console's HTTP surface (ADRs 0031, 0043 and 0057): the files of a
//! Product's Console build, a value only the executable crates pass in
//! their composition, so an SDK crate's linked engine serves no Console,
//! answered from its listener as they are stored, brotli-compressed, with
//! `Content-Encoding: br`, and every answer under the production headers:
//! the strict Content Security Policy (own scripts and styles only, no
//! inline code or `eval`, Trusted Types, nothing third-party), `frame-
//! ancestors 'none'`, `Referrer-Policy: no-referrer`, COOP, CORP and a
//! `Cache-Control` per kind of file.
//!
//! The page swaps the sign-in link's token, which the URL fragment carried,
//! once for an `HttpOnly`, `SameSite=Strict` cookie at `POST /sign-in`,
//! through `people`'s sign-in driver (ADR 0024); `GET /session` answers
//! whether a request's cookie is a sign-in. The Console then reaches the
//! engine over a WebSocket on its own origin, at `/protocol`, whose
//! handshake must carry the Console's own `Host` and `Origin` (ADRs 0017
//! and 0067) and a sign-in's cookie; the Daemon serves each as a
//! Connection whose `hello` needs no token. Each refusal is logged under
//! the protocol's refusal target with the listener `console` and the
//! endpoint table's reason, never with the token or the cookie.

use std::convert::Infallible;
use std::sync::Arc;

use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::upgrade::OnUpgrade;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_util::sync::CancellationToken;

use crate::people::{SignInDriver, SignInFailure, SignInRefused};
use crate::rpc::{Authorities, CONSOLE_LISTENER, ConsoleSocket, REFUSAL_TARGET, Refusal};
use crate::tasks::TaskOwner;
use crate::transport::{ConsoleListener, ConsoleStream};

/// The production Content Security Policy (ADR 0031): the Console's own
/// scripts, styles, images, fonts and connections only; no inline code and
/// no `eval`, since no source allows them; every string sent to a script
/// sink refused (`require-trusted-types-for 'script'`) and no Trusted Types
/// policy allowed; no frame may embed it; violations reported to its own
/// address.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'; report-uri /csp-report";

/// Where browsers send the policy's violation reports.
const REPORTS: &str = "/csp-report";

/// A file whose name holds its content's hash: it never changes.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// A file at a fixed name, such as the page itself: checked on every use.
const REVALIDATED: &str = "no-cache";
/// Any other answer.
const UNSTORED: &str = "no-store";

/// One file of a Console build: its path from the build's root, such as
/// `/assets/index-4f2a.js`, and its bytes, brotli-compressed.
#[derive(Clone, Copy, Debug)]
pub struct ConsoleFile {
    path: &'static str,
    brotli: &'static [u8],
}

impl ConsoleFile {
    /// The file at `path`, such as `/index.html`, holding `brotli`.
    pub const fn new(path: &'static str, brotli: &'static [u8]) -> Self {
        Self { path, brotli }
    }
}

/// A Product's Console build, as its executable embeds it: brotli files
/// only, `xtask console build`'s output (ADR 0057).
#[derive(Debug)]
pub struct ConsoleBuild {
    files: &'static [ConsoleFile],
}

impl ConsoleBuild {
    /// The build made of `files`.
    pub const fn new(files: &'static [ConsoleFile]) -> Self {
        Self { files }
    }

    /// The file a request's path names: `/` names the page itself.
    fn file(&self, path: &str) -> Option<&ConsoleFile> {
        let path = if path == "/" { "/index.html" } else { path };
        self.files.iter().find(|file| file.path == path)
    }
}

/// Where the page swaps the sign-in link's token for its cookie.
const SIGN_IN: &str = "/sign-in";
/// Where the page asks whether its cookie is a sign-in.
const SESSION: &str = "/session";
/// Where the Console's WebSocket opens.
const PROTOCOL: &str = "/protocol";
/// The longest token `POST /sign-in` reads.
const MAX_TOKEN_BYTES: usize = 1024;

/// What the Daemon serves its Console with.
pub(crate) struct Served {
    /// The Product's Console build.
    pub(crate) build: &'static ConsoleBuild,
    /// The listener at `console.address`.
    pub(crate) listener: ConsoleListener,
    /// Who a request is.
    pub(crate) sign_ins: SignInDriver,
    /// Where each WebSocket opened goes, for the Daemon to serve.
    pub(crate) sockets: mpsc::Sender<ConsoleSocket>,
}

/// The Console's state that each of its connections reaches.
struct Site {
    build: &'static ConsoleBuild,
    sign_ins: SignInDriver,
    /// The names it is reached at: its listener's address and `localhost`.
    authorities: Authorities,
    /// The sign-in cookie's name, which holds the listener's port, since a
    /// browser sends a host's cookies to each of its ports and each major
    /// has its own Console (ADR 0030).
    cookie: String,
    sockets: mpsc::Sender<ConsoleSocket>,
}

/// Serves the Console `served` describes until `owner` stops, each
/// connection a task of its own.
///
/// # Cancel safety
///
/// Dropping the future stops accepting; the connections are `owner`'s,
/// which its stop ends.
pub(crate) async fn serve(served: Served, owner: TaskOwner) {
    let Served {
        build,
        listener,
        sign_ins,
        sockets,
    } = served;
    let address = match listener.local_address() {
        Ok(address) => address,
        Err(error) => {
            tracing::error!(%error, "the Console's listener has no address, so the Console serves nothing");
            return;
        }
    };
    let site = Arc::new(Site {
        build,
        sign_ins,
        authorities: Authorities::loopback(address),
        cookie: format!("goodfetch_console_{}", address.port()),
        sockets,
    });
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                let stopping = owner.token().clone();
                owner.spawn(Arc::clone(&site).connection(stream, stopping));
            }
            Err(error) => {
                tracing::warn!(%error, "the Console's listener cannot accept a connection");
            }
        }
    }
}

impl Site {
    /// Serves one browser connection until it ends or `stopping` is
    /// cancelled; a WebSocket it opens then goes to the Daemon.
    async fn connection(self: Arc<Self>, stream: ConsoleStream, stopping: CancellationToken) {
        let upgrade = Arc::new(Mutex::new(None));
        let answering = Arc::clone(&self);
        let upgrading = Arc::clone(&upgrade);
        let connection = hyper::server::conn::http1::Builder::new()
            .serve_connection(
                TokioIo::new(stream),
                hyper::service::service_fn(move |request: Request<Incoming>| {
                    let site = Arc::clone(&answering);
                    let upgrade = Arc::clone(&upgrading);
                    async move { Ok::<_, Infallible>(site.answer(request, &upgrade).await) }
                }),
            )
            .with_upgrades();
        tokio::pin!(connection);
        tokio::select! {
            ended = connection.as_mut() => {
                if let Err(error) = ended {
                    tracing::debug!(%error, "a Console connection ended with an error");
                }
            }
            () = stopping.cancelled() => {
                connection.as_mut().graceful_shutdown();
                return;
            }
        }
        let opened = upgrade.lock().await.take();
        if let Some(opened) = opened {
            self.open_socket(opened).await;
        }
    }

    /// What the Console answers `request`; a WebSocket handshake it accepts
    /// leaves its upgrade in `upgrade`.
    async fn answer(
        &self,
        mut request: Request<Incoming>,
        upgrade: &Mutex<Option<OnUpgrade>>,
    ) -> Response<Full<Bytes>> {
        match request.uri().path() {
            SIGN_IN => self.sign_in(request).await,
            SESSION => self.session(request.method(), request.headers()).await,
            PROTOCOL => {
                let answered = self.handshake(request.method(), request.headers()).await;
                if answered.status() == StatusCode::SWITCHING_PROTOCOLS {
                    *upgrade.lock().await = Some(hyper::upgrade::on(&mut request));
                }
                answered
            }
            path => answer(self.build, request.method(), path),
        }
    }

    /// `POST /sign-in`: swaps the link's token, the body, for a sign-in's
    /// cookie.
    async fn sign_in(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        if request.method() != Method::POST {
            return refused_method(&["POST"]);
        }
        if let Some(refusal) = self.foreign(request.headers()) {
            return refused(refusal.reason(), StatusCode::FORBIDDEN);
        }
        let Ok(body) = Limited::new(request.into_body(), MAX_TOKEN_BYTES)
            .collect()
            .await
        else {
            return plain(StatusCode::PAYLOAD_TOO_LARGE);
        };
        let body = body.to_bytes();
        let token = std::str::from_utf8(&body).unwrap_or_default().trim();
        match self.sign_ins.with_link(token).await {
            Ok(signed_in) => {
                let mut response = plain(StatusCode::NO_CONTENT);
                let cookie = format!(
                    "{}={}; HttpOnly; SameSite=Strict; Path=/",
                    self.cookie,
                    signed_in.cookie()
                );
                if let Ok(cookie) = HeaderValue::from_str(&cookie) {
                    response.headers_mut().insert(header::SET_COOKIE, cookie);
                }
                tracing::info!("the Console signed the OS user in through the sign-in link");
                response
            }
            Err(SignInFailure::Refused(refusal)) => {
                refused(refusal.reason(), StatusCode::UNAUTHORIZED)
            }
            Err(SignInFailure::Generator(error)) => {
                tracing::error!(%error, "the OS generator gave no cookie, so nobody signed in");
                plain(StatusCode::SERVICE_UNAVAILABLE)
            }
        }
    }

    /// `GET /session`: `204` for a signed-in request, `401` otherwise.
    async fn session(&self, method: &Method, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if method != Method::GET && method != Method::HEAD {
            return refused_method(&["GET", "HEAD"]);
        }
        match self.signed_in(headers).await {
            Ok(()) => plain(StatusCode::NO_CONTENT),
            Err(_) => plain(StatusCode::UNAUTHORIZED),
        }
    }

    /// The WebSocket handshake at `/protocol`: `101` when it carries the
    /// Console's own `Host` and `Origin` and a sign-in's cookie.
    async fn handshake(&self, method: &Method, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if method != Method::GET {
            return refused_method(&["GET"]);
        }
        if let Some(refusal) = self.foreign(headers) {
            return refused(refusal.reason(), StatusCode::FORBIDDEN);
        }
        if let Err(refusal) = self.signed_in(headers).await {
            return refused(refusal.reason(), StatusCode::UNAUTHORIZED);
        }
        let value = |name| headers.get(name).and_then(|value| value.to_str().ok());
        let upgrade =
            value(header::UPGRADE).is_some_and(|upgrade| upgrade.eq_ignore_ascii_case("websocket"));
        let version = value(header::SEC_WEBSOCKET_VERSION) == Some("13");
        let (Some(key), true, true) = (value(header::SEC_WEBSOCKET_KEY), upgrade, version) else {
            return plain(StatusCode::BAD_REQUEST);
        };
        let mut response = plain(StatusCode::SWITCHING_PROTOCOLS);
        let headers = response.headers_mut();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        if let Ok(accept) = HeaderValue::from_str(&derive_accept_key(key.as_bytes())) {
            headers.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
        }
        response
    }

    /// Hands the WebSocket on the connection `opened` upgrades to the
    /// Daemon.
    async fn open_socket(&self, opened: OnUpgrade) {
        match opened.await {
            Ok(upgraded) => {
                let socket = ConsoleSocket::opened(TokioIo::new(upgraded)).await;
                if self.sockets.send(socket).await.is_err() {
                    tracing::debug!("the Daemon stopped before it served a Console's WebSocket");
                }
            }
            Err(error) => tracing::debug!(%error, "a Console's WebSocket did not open"),
        }
    }

    /// The rule a request's `Host` or `Origin` breaks, if any.
    fn foreign(&self, headers: &HeaderMap) -> Option<Refusal> {
        let value = |name| headers.get(name).and_then(|value| value.to_str().ok());
        self.authorities
            .refusal(value(header::HOST), value(header::ORIGIN))
    }

    /// Whether the request's cookie is a sign-in, or why not.
    async fn signed_in(&self, headers: &HeaderMap) -> Result<(), SignInRefused> {
        let cookie = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|cookies| cookies.split(';'))
            .find_map(|pair| {
                let (name, value) = pair.trim().split_once('=')?;
                (name == self.cookie).then_some(value)
            });
        match cookie {
            None => Err(SignInRefused::TokenMissing),
            Some(cookie) => self
                .sign_ins
                .principal(cookie)
                .await
                .map(|_| ())
                .ok_or(SignInRefused::TokenWrong),
        }
    }
}

/// A refusal by the endpoint table's rule `reason`, logged, answered with
/// `status` and no body.
fn refused(reason: &'static str, status: StatusCode) -> Response<Full<Bytes>> {
    tracing::warn!(target: REFUSAL_TARGET, listener = CONSOLE_LISTENER, reason, "the Console refused a request");
    plain(status)
}

/// What the Console answers `method` on `path`: a file of `build` as it is
/// stored, the reports endpoint's acknowledgement, or an error, each under
/// the production headers.
fn answer(build: &ConsoleBuild, method: &Method, path: &str) -> Response<Full<Bytes>> {
    if path == REPORTS {
        return if method == Method::POST {
            tracing::debug!("the Console received a Content Security Policy report");
            plain(StatusCode::NO_CONTENT)
        } else {
            refused_method(&["POST"])
        };
    }
    let Some(file) = build.file(path) else {
        return plain(StatusCode::NOT_FOUND);
    };
    if method != Method::GET && method != Method::HEAD {
        return refused_method(&["GET", "HEAD"]);
    }
    let body = if method == Method::HEAD {
        Bytes::new()
    } else {
        Bytes::from_static(file.brotli)
    };
    let mut response = Response::new(Full::new(body));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(file.path)),
    );
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
    let cache = if file.path.starts_with("/assets/") {
        IMMUTABLE
    } else {
        REVALIDATED
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    secured(response)
}

/// An answer with no body.
fn plain(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(UNSTORED));
    secured(response)
}

/// A method the path does not take, naming those it does.
fn refused_method(allowed: &[&'static str]) -> Response<Full<Bytes>> {
    let mut response = plain(StatusCode::METHOD_NOT_ALLOWED);
    if let Ok(allow) = HeaderValue::from_str(&allowed.join(", ")) {
        response.headers_mut().insert(header::ALLOW, allow);
    }
    response
}

/// `response` with the headers every answer of the Console carries.
fn secured(mut response: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// The media type of a build's file, by its extension.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
