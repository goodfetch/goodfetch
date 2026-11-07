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
//! endpoint table's reason, never with the token or the cookie. Where
//! a proxy refuses `WebSocket`s, the page carries its Connection over
//! Server-Sent Events plus `POST` instead (`event_stream`), each `POST`
//! with its sign-in's CSRF token.
//!
//! A Run's files are fetched by handle: `GET /bodies/<run_id>/<body>`
//! answers a signed-in request with the file's bytes, as an attachment no
//! browser shows on the Console's origin, and `GET /content-address/
//! <run_id>/<body>?type=<shown>` with the file's address on the content
//! origin (`content`), signed for that file and valid 5 minutes, shown in
//! a sandboxed frame; the Console's policy frames that origin alone.
//!
//! Size: one concern, the Console listener's one `Site` and the routes it
//! answers, kept whole so every answer passes the same sign-in check and
//! the same production headers.

use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;

use goodfetch_protocol::console::OFFLINE_CACHE;
use goodfetch_protocol::run::RunId;

use http_body_util::{BodyExt as _, Either, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::upgrade::OnUpgrade;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_util::sync::CancellationToken;

use crate::clock::Clock;
use crate::composition::ConsoleBuild;
use crate::content_origin::{BODIES, ContentFile, ContentKey, Shown};
use crate::daemon::ContentOrigin;
use crate::endpoint::{self, Authorities, Hosts, Listener, Origins, REFUSAL_TARGET, Refusal};
use crate::people::{SignInDriver, SignInFailure, SignInRefused, sign_in_id};
use crate::rpc::event_stream::EventStreamBody;
use crate::rpc::{ConsoleConnection, ConsoleSocket};
use crate::runs::Runs;
use crate::settings::{Settings, catalog_default};
use crate::tasks::TaskOwner;
use crate::transport::{ConsoleListener, ConsoleStream};

mod content;
mod event_stream;
pub mod links;
mod offline_key;

use content::{Asked, Content};
use event_stream::{EVENTS, MESSAGES, Streams};

pub use event_stream::fuzz_post_fallback;

/// What the Console answers: bytes, or the event stream of a Connection.
type Answered = Either<Full<Bytes>, EventStreamBody>;

/// The production Content Security Policy (ADR 0031): the Console's own
/// scripts, styles, images, fonts and connections only; no inline code and
/// no `eval`, since no source allows them; every string sent to a script
/// sink refused (`require-trusted-types-for 'script'`) and no Trusted Types
/// policy allowed; no frame may embed it; violations reported to its own
/// address. With a content origin at `content`, a bare origin, it may frame
/// that origin and nothing else's; its files, images included, show in the
/// origin's own `sandbox` frames, so no other directive widens.
pub fn content_security_policy(content: Option<&str>) -> String {
    let frames = content.map_or_else(String::new, |content| format!("frame-src {content}; "));
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; {frames}base-uri 'none'; form-action 'self'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'; report-uri /csp-report"
    )
}

/// Where browsers send the policy's violation reports.
const REPORTS: &str = "/csp-report";

/// A file whose name holds its content's hash: it never changes.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// A file at a fixed name, such as the page itself: checked on every use.
const REVALIDATED: &str = "no-cache";
/// Any other answer.
const UNSTORED: &str = "no-store";

/// Where the page swaps the sign-in link's token for its cookie.
const SIGN_IN: &str = "/sign-in";
/// Where the page asks whether its cookie is a sign-in.
const SESSION: &str = "/session";
/// Where the page ends its own sign-in.
const SIGN_OUT: &str = "/sign-out";
/// Where the page gets its offline cache key, new or unwrapped (ADR 0031).
const OFFLINE_KEY: &str = "/offline-key";
/// Where the Console's WebSocket opens.
const PROTOCOL: &str = "/protocol";
/// Where the page turns a file's handle into its signed content address:
/// `/content-address/<run_id>/<body>?type=<shown>`.
const CONTENT_ADDRESS: &str = "/content-address/";
/// The header every signed-in `POST` carries its sign-in's CSRF token in.
const CSRF_HEADER: &str = "x-csrf-token";
/// The endpoint table's reason for a signed-in `POST` without its CSRF
/// token.
const CSRF_TOKEN_MISSING: &str = "csrf_token_missing";
/// The endpoint table's reason for a signed-in `POST` with another CSRF
/// token than its sign-in's.
const CSRF_TOKEN_WRONG: &str = "csrf_token_wrong";
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
    /// The sign-in cookie's name, which names the Daemon's major: a
    /// browser sends a host's cookies to each of its ports, each major has
    /// its own Console and accounts store (ADR 0030), and a sign-in
    /// outlives a restart that moves the Console to another port.
    pub(crate) cookie: String,
    /// Where each Connection opened goes, a WebSocket or an event stream,
    /// for the Daemon to serve.
    pub(crate) connections: mpsc::Sender<ConsoleConnection>,
    /// Where a Run's files are.
    pub(crate) runs: Runs,
    /// The engine's settings, whose machine layers set its offline cache.
    pub(crate) settings: Settings,
    /// The wall time content addresses are signed and checked at.
    pub(crate) clock: Clock,
    /// The content origin, if it started.
    pub(crate) content: Option<ContentOrigin>,
}

/// Which of its origins a connection of the Console reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// The Console's listener, which a shared content origin's requests
    /// reach too, told apart by their `Host`.
    Console,
    /// The content origin's own listener.
    Content,
}

/// The Console's state that each of its connections reaches.
struct Site {
    build: &'static ConsoleBuild,
    sign_ins: SignInDriver,
    /// The names it is reached at: its listener's address and `localhost`.
    authorities: Authorities,
    /// The sign-in cookie's name.
    cookie: String,
    connections: mpsc::Sender<ConsoleConnection>,
    /// The event streams open, where each `POST` of the fallback goes.
    streams: Streams,
    runs: Runs,
    settings: Settings,
    clock: Clock,
    /// The content origin, if it started.
    content: Option<Content>,
    /// The Console's policy, which names the content origin.
    policy: HeaderValue,
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
        cookie,
        connections,
        runs,
        settings,
        clock,
        content,
    } = served;
    let address = match listener.local_address() {
        Ok(address) => address,
        Err(error) => {
            tracing::error!(%error, "the Console's listener has no address, so the Console serves nothing");
            return;
        }
    };
    let authorities = Authorities::loopback(address);
    let (content, content_listener) = match content.map(|origin| content_of(origin, &authorities)) {
        Some(Some((content, listener))) => (Some(content), listener),
        Some(None) | None => (None, None),
    };
    let policy = content_security_policy(content.as_ref().map(|content| content.origin.as_str()));
    let site = Arc::new(Site {
        build,
        sign_ins,
        authorities,
        cookie,
        connections,
        streams: Streams::default(),
        runs,
        settings,
        clock,
        content,
        policy: HeaderValue::from_str(&policy)
            .expect("invariant: the Console's policy is header text"),
    });
    if let Some(listener) = content_listener {
        owner.spawn(accept(
            Arc::clone(&site),
            listener,
            Side::Content,
            owner.child(),
        ));
    }
    accept(site, listener, Side::Console, owner).await;
}

/// The content origin `origin` describes, beside the Console reached at
/// `console`, with its own listener if it has one; none when the OS
/// generator gives no key or its listener has no address, which leaves
/// the Console serving without it.
fn content_of(
    origin: ContentOrigin,
    console: &Authorities,
) -> Option<(Content, Option<ConsoleListener>)> {
    let key = match ContentKey::generated() {
        Ok(key) => key,
        Err(error) => {
            tracing::error!(%error, "the OS generator gave no content key, so the Console shows no Run's files");
            return None;
        }
    };
    let ContentOrigin { listener, url } = origin;
    let origin = url.trim_end_matches('/').to_owned();
    let public = origin
        .split_once("://")
        .map_or(origin.as_str(), |(_, authority)| authority)
        .to_owned();
    let authorities = match &listener {
        Some(listener) => match listener.local_address() {
            Ok(address) => Authorities::loopback(address).with_name(public),
            Err(error) => {
                tracing::error!(%error, "the content origin's listener has no address, so the Console shows no Run's files");
                return None;
            }
        },
        None => Authorities::only(public),
    };
    let policy = HeaderValue::from_str(&Content::policy(console))
        .expect("invariant: the content origin's policy is header text");
    let shared = listener.is_none();
    Some((
        Content {
            key,
            origin,
            authorities,
            shared,
            policy,
        },
        listener,
    ))
}

/// Accepts connections on `listener`, the Console's `side`, until `owner`
/// stops, each a task of its own.
async fn accept(site: Arc<Site>, listener: ConsoleListener, reached: Side, owner: TaskOwner) {
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                let stopping = owner.token().clone();
                owner.spawn(Arc::clone(&site).connection(stream, stopping, reached));
            }
            Err(error) => {
                tracing::warn!(%error, ?reached, "the Console's listener cannot accept a connection");
            }
        }
    }
}

impl Site {
    /// Serves one browser connection until it ends or `stopping` is
    /// cancelled; a WebSocket it opens then goes to the Daemon.
    async fn connection(
        self: Arc<Self>,
        stream: ConsoleStream,
        stopping: CancellationToken,
        reached: Side,
    ) {
        let from = match stream.peer_address() {
            Ok(peer) => peer.ip(),
            Err(error) => {
                tracing::debug!(%error, "a Console connection has no peer address, so it closes");
                return;
            }
        };
        let upgrade = Arc::new(Mutex::new(None));
        let answering = Arc::clone(&self);
        let upgrading = Arc::clone(&upgrade);
        let connection = hyper::server::conn::http1::Builder::new()
            .serve_connection(
                TokioIo::new(stream),
                hyper::service::service_fn(move |request: Request<Incoming>| {
                    let site = Arc::clone(&answering);
                    let upgrade = Arc::clone(&upgrading);
                    async move {
                        Ok::<_, Infallible>(site.answer(request, from, reached, &upgrade).await)
                    }
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

    /// What the Console answers `request`: its event stream at
    /// `/protocol/events`, else bytes; a WebSocket handshake it accepts
    /// leaves its upgrade in `upgrade`.
    async fn answer(
        &self,
        request: Request<Incoming>,
        from: IpAddr,
        reached: Side,
        upgrade: &Mutex<Option<OnUpgrade>>,
    ) -> Response<Answered> {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        let content_takes = self
            .content
            .as_ref()
            .is_some_and(|content| content.takes(host));
        if reached == Side::Console && !content_takes && request.uri().path() == EVENTS {
            return self.event_stream(&request).await;
        }
        self.answer_bytes(request, from, reached, upgrade)
            .await
            .map(Either::Left)
    }

    /// What the Console answers `request` with bytes.
    async fn answer_bytes(
        &self,
        mut request: Request<Incoming>,
        from: IpAddr,
        reached: Side,
        upgrade: &Mutex<Option<OnUpgrade>>,
    ) -> Response<Full<Bytes>> {
        if let Some(content) = &self.content {
            let host = request
                .headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok());
            if reached == Side::Content || content.takes(host) {
                return self.content_answer(content, &request).await;
            }
        } else if reached == Side::Content {
            return self.plain(StatusCode::NOT_FOUND);
        }
        match request.uri().path() {
            SIGN_IN => self.sign_in(request, from).await,
            SESSION => self.session(request.method(), request.headers()).await,
            SIGN_OUT => self.sign_out(request.method(), request.headers()).await,
            OFFLINE_KEY => self.offline_key(request).await,
            PROTOCOL => {
                let answered = self.handshake(request.method(), request.headers()).await;
                if answered.status() == StatusCode::SWITCHING_PROTOCOLS {
                    *upgrade.lock().await = Some(hyper::upgrade::on(&mut request));
                }
                answered
            }
            path if path.starts_with(MESSAGES) => self.post_messages(request).await,
            path if path.starts_with(CONTENT_ADDRESS) => self.content_address(&request).await,
            path if path.starts_with(BODIES) => self.body(&request).await,
            path => build_file(self.build, request.method(), path, &self.policy),
        }
    }

    /// What the content origin answers `request`: the file its address is
    /// signed for, under the content origin's headers, or a refusal it
    /// records in the Audit log.
    async fn content_answer(
        &self,
        content: &Content,
        request: &Request<Incoming>,
    ) -> Response<Full<Bytes>> {
        let value = |name| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let target = request
            .uri()
            .path_and_query()
            .map_or("/", |target| target.as_str());
        match content.asked(
            request.method(),
            target,
            value(header::HOST),
            value(header::ORIGIN),
            &self.authorities,
            self.clock.wall(),
        ) {
            Asked::File { file, head } => match self.read(&file).await {
                Some(bytes) => content.file(&file, bytes, head),
                None => content.plain(StatusCode::NOT_FOUND),
            },
            Asked::Refused { reason, status } => {
                self.sign_ins.refused(Listener::ContentOrigin, reason).await;
                tracing::warn!(target: REFUSAL_TARGET, listener = Listener::ContentOrigin.name(), reason, "the content origin refused a request");
                content.plain(status)
            }
            Asked::Answered(response) => response,
        }
    }

    /// `GET /content-address/<run_id>/<body>?type=<shown>`: for a signed-in
    /// request, `200` with the file's content address, `{"url": "…"}`,
    /// signed for that one file and valid 5 minutes.
    async fn content_address(&self, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        if let Some(refused) = self.refused_read(request).await {
            return refused;
        }
        let Some(content) = &self.content else {
            return self.plain(StatusCode::SERVICE_UNAVAILABLE);
        };
        let path = request.uri().path().trim_start_matches(CONTENT_ADDRESS);
        let shown = request
            .uri()
            .query()
            .unwrap_or_default()
            .split('&')
            .find_map(|pair| pair.strip_prefix("type="))
            .map_or(Some(Shown::Html), Shown::named);
        let file = path
            .split_once('/')
            .zip(shown)
            .and_then(|((run_id, body), shown)| ContentFile::of(run_id, body, shown));
        let Some(file) = file else {
            return self.plain(StatusCode::BAD_REQUEST);
        };
        if self.file(&file.run_id, &file.body).await.is_none() {
            return self.plain(StatusCode::NOT_FOUND);
        }
        let url = format!(
            "{}{}",
            content.origin,
            content.key.sign(&file, self.clock.wall())
        );
        let mut response = self.plain(StatusCode::OK);
        *response.body_mut() =
            Full::new(Bytes::from(serde_json::json!({ "url": url }).to_string()));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response
    }

    /// `GET /bodies/<run_id>/<body>`: for a signed-in request, the file's
    /// bytes, as an attachment no browser shows on the Console's origin.
    async fn body(&self, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        if let Some(refused) = self.refused_read(request).await {
            return refused;
        }
        let file = request
            .uri()
            .path()
            .trim_start_matches(BODIES)
            .split_once('/')
            .and_then(|(run_id, body)| ContentFile::of(run_id, body, Shown::Html));
        let Some(file) = file else {
            return self.plain(StatusCode::NOT_FOUND);
        };
        let Some(bytes) = self.read(&file).await else {
            return self.plain(StatusCode::NOT_FOUND);
        };
        let head = request.method() == Method::HEAD;
        let length = bytes.len();
        let mut response = self.plain(StatusCode::OK);
        if !head {
            *response.body_mut() = Full::new(Bytes::from(bytes));
        }
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
        response
    }

    /// The answer that refuses `request`, recorded in the Audit log, unless
    /// it is a signed-in read of the Console's own: a `GET` or `HEAD`
    /// naming its `Host`, with no other origin's `Origin` (a same-origin
    /// read carries none), and a sign-in's cookie.
    async fn refused_read(&self, request: &Request<Incoming>) -> Option<Response<Full<Bytes>>> {
        let method = request.method();
        if method != Method::GET && method != Method::HEAD {
            return Some(self.refused_method(&["GET", "HEAD"]));
        }
        let headers = request.headers();
        let value = |name| headers.get(name).and_then(|value| value.to_str().ok());
        if let Some(refusal) = endpoint::refusal(
            value(header::HOST),
            value(header::ORIGIN),
            Hosts::Named(&self.authorities),
            Origins::Allowed(&[&self.authorities]),
        ) {
            return Some(self.refused(refusal.reason(), StatusCode::FORBIDDEN).await);
        }
        if let Err(refusal) = self.signed_in(headers).await {
            return Some(
                self.refused(refusal.reason(), StatusCode::UNAUTHORIZED)
                    .await,
            );
        }
        None
    }

    /// The bytes of `file`, if its Run holds it.
    async fn read(&self, file: &ContentFile) -> Option<Vec<u8>> {
        let path = self.file(&file.run_id, &file.body).await?;
        tokio::fs::read(path).await.ok()
    }

    /// The path of the body `body` of the Run `run_id`, if it holds one.
    async fn file(&self, run_id: &RunId, body: &str) -> Option<std::path::PathBuf> {
        let folder = self.runs.folder(run_id.clone()).await.ok()?;
        let path = crate::body::file(&folder, body)?;
        tokio::fs::try_exists(&path)
            .await
            .ok()
            .and_then(|exists| exists.then_some(path))
    }

    /// An answer with no body, under the Console's headers.
    fn plain(&self, status: StatusCode) -> Response<Full<Bytes>> {
        plain(status, &self.policy)
    }

    /// A method the path does not take, naming those it does.
    fn refused_method(&self, allowed: &[&'static str]) -> Response<Full<Bytes>> {
        refused_method(allowed, &self.policy)
    }

    /// A refusal by the endpoint table's rule `reason`, logged, answered
    /// with `status` and no body.
    fn refused_plain(&self, reason: &'static str, status: StatusCode) -> Response<Full<Bytes>> {
        refused(reason, status, &self.policy)
    }

    /// `POST /sign-in`: swaps the link's token, the body, for a sign-in's
    /// cookie.
    async fn sign_in(&self, request: Request<Incoming>, from: IpAddr) -> Response<Full<Bytes>> {
        if request.method() != Method::POST {
            return self.refused_method(&["POST"]);
        }
        if let Some(refusal) = self.foreign(request.headers()) {
            return self.refused(refusal.reason(), StatusCode::FORBIDDEN).await;
        }
        let Ok(body) = Limited::new(request.into_body(), MAX_TOKEN_BYTES)
            .collect()
            .await
        else {
            return self.plain(StatusCode::PAYLOAD_TOO_LARGE);
        };
        let body = body.to_bytes();
        match self.sign_ins.with_link(sign_in_token(&body), from).await {
            Ok(signed_in) => {
                let mut response = self.plain(StatusCode::NO_CONTENT);
                self.set_cookie(&mut response, signed_in.cookie(), self.max_age());
                tracing::info!("the Console signed the OS user in through the sign-in link");
                response
            }
            Err(SignInFailure::Refused(refusal)) => {
                let status = match refusal {
                    SignInRefused::RateExceeded => StatusCode::TOO_MANY_REQUESTS,
                    SignInRefused::TokenMissing | SignInRefused::TokenWrong => {
                        StatusCode::UNAUTHORIZED
                    }
                };
                self.refused(refusal.reason(), status).await
            }
            Err(SignInFailure::Generator(error)) => {
                tracing::error!(%error, "the OS generator gave no cookie, so nobody signed in");
                self.plain(StatusCode::SERVICE_UNAVAILABLE)
            }
            Err(SignInFailure::People(error)) => {
                tracing::error!(%error, "the accounts store, the Console key or the Audit log failed, so nobody signed in");
                self.plain(StatusCode::SERVICE_UNAVAILABLE)
            }
        }
    }

    /// `console.offline_cache` from the machine's layers as they are now,
    /// so that a change takes effect at the next request with no restart
    /// (ADR 0031); on, its built-in value, when they cannot be read, as the
    /// warning logged says.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future changes nothing.
    async fn offline_cache(&self) -> bool {
        let settings = &self.settings;
        let product = settings.product();
        let read = settings
            .machine()
            .await
            .and_then(|machine| machine.resolve(product));
        match read {
            Ok(resolved) => resolved.get(&OFFLINE_CACHE),
            Err(error) => {
                tracing::warn!(%error, "`console.offline_cache` cannot be read: the Console takes its built-in value, on");
                catalog_default(&OFFLINE_CACHE)
            }
        }
    }

    /// `GET /session`: for a signed-in request, `200` with the sign-in's
    /// CSRF token and whether `console.offline_cache` is on,
    /// `{"csrf_token": "…", "offline_cache": true}`, which only the
    /// Console's own origin can read, and its cookie's expiry set again;
    /// `401` otherwise.
    async fn session(&self, method: &Method, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if method != Method::GET && method != Method::HEAD {
            return self.refused_method(&["GET", "HEAD"]);
        }
        let Some(cookie) = self.cookie_of(headers) else {
            return self.plain(StatusCode::UNAUTHORIZED);
        };
        let Some(csrf_token) = self.sign_ins.csrf_token(cookie).await else {
            return self.plain(StatusCode::UNAUTHORIZED);
        };
        let mut response = self.plain(StatusCode::OK);
        if method == Method::GET {
            let body = serde_json::json!({
                "csrf_token": csrf_token,
                "offline_cache": self.offline_cache().await,
            })
            .to_string();
            *response.body_mut() = Full::new(Bytes::from(body));
        }
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        self.set_cookie(&mut response, cookie, self.max_age());
        response
    }

    /// `POST /sign-out`: ends the request's sign-in, when it carries the
    /// Console's own `Origin`, its cookie and its CSRF token, and clears
    /// the cookie.
    async fn sign_out(&self, method: &Method, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if method != Method::POST {
            return self.refused_method(&["POST"]);
        }
        if let Some(refusal) = self.foreign(headers) {
            return self.refused(refusal.reason(), StatusCode::FORBIDDEN).await;
        }
        let Some(cookie) = self.cookie_of(headers) else {
            return self
                .refused(
                    SignInRefused::TokenMissing.reason(),
                    StatusCode::UNAUTHORIZED,
                )
                .await;
        };
        if let Err(reason) = self.csrf_checked(headers, cookie).await {
            return self.refused(reason, StatusCode::FORBIDDEN).await;
        }
        self.sign_ins.sign_out(cookie).await;
        let mut response = self.plain(StatusCode::NO_CONTENT);
        self.set_cookie(&mut response, "", 0);
        response
    }

    /// Whether a signed-in `POST` carries the CSRF token of the sign-in
    /// `cookie`, or the endpoint table's reason it does not.
    async fn csrf_checked(&self, headers: &HeaderMap, cookie: &str) -> Result<(), &'static str> {
        let token = headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if token.is_empty() {
            Err(CSRF_TOKEN_MISSING)
        } else if self.sign_ins.csrf_checked(cookie, token).await {
            Ok(())
        } else {
            Err(CSRF_TOKEN_WRONG)
        }
    }

    /// Sets the sign-in cookie to `value` on `response`, `HttpOnly` and
    /// `SameSite=Strict`, lasting `max_age` seconds; 0 clears it.
    fn set_cookie(&self, response: &mut Response<Full<Bytes>>, value: &str, max_age: u64) {
        let cookie = format!(
            "{}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}",
            self.cookie
        );
        if let Ok(cookie) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, cookie);
        }
    }

    /// How long a sign-in's cookie lasts in the browser: its idle expiry,
    /// in seconds, which each use sets again.
    fn max_age(&self) -> u64 {
        self.sign_ins.idle_expiry().as_secs()
    }

    /// A refusal by the endpoint table's rule `reason`, logged and recorded
    /// in the Audit log, answered with `status` and no body.
    async fn refused(&self, reason: &'static str, status: StatusCode) -> Response<Full<Bytes>> {
        self.sign_ins.refused(Listener::Console, reason).await;
        self.refused_plain(reason, status)
    }

    /// The WebSocket handshake at `/protocol`: `101` when it carries the
    /// Console's own `Host` and `Origin` and a sign-in's cookie.
    async fn handshake(&self, method: &Method, headers: &HeaderMap) -> Response<Full<Bytes>> {
        if method != Method::GET {
            return self.refused_method(&["GET"]);
        }
        if let Some(refusal) = self.foreign(headers) {
            return self.refused(refusal.reason(), StatusCode::FORBIDDEN).await;
        }
        if let Err(refusal) = self.signed_in(headers).await {
            return self
                .refused(refusal.reason(), StatusCode::UNAUTHORIZED)
                .await;
        }
        let value = |name| headers.get(name).and_then(|value| value.to_str().ok());
        let upgrade =
            value(header::UPGRADE).is_some_and(|upgrade| upgrade.eq_ignore_ascii_case("websocket"));
        let version = value(header::SEC_WEBSOCKET_VERSION) == Some("13");
        let (Some(key), true, true) = (value(header::SEC_WEBSOCKET_KEY), upgrade, version) else {
            return self.plain(StatusCode::BAD_REQUEST);
        };
        let mut response = self.plain(StatusCode::SWITCHING_PROTOCOLS);
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
                let opened = ConsoleConnection::WebSocket(socket);
                if self.connections.send(opened).await.is_err() {
                    tracing::debug!("the Daemon stopped before it served a Console's WebSocket");
                }
            }
            Err(error) => tracing::debug!(%error, "a Console's WebSocket did not open"),
        }
    }

    /// The rule a request's `Host` or `Origin` breaks, if any.
    fn foreign(&self, headers: &HeaderMap) -> Option<Refusal> {
        let value = |name| headers.get(name).and_then(|value| value.to_str().ok());
        endpoint::refusal(
            value(header::HOST),
            value(header::ORIGIN),
            Hosts::Named(&self.authorities),
            Origins::Required(&[&self.authorities]),
        )
    }

    /// Whether the request's cookie is a sign-in, or why not.
    async fn signed_in(&self, headers: &HeaderMap) -> Result<(), SignInRefused> {
        match self.cookie_of(headers) {
            None => Err(SignInRefused::TokenMissing),
            Some(cookie) => self
                .sign_ins
                .principal(cookie)
                .await
                .map(|_| ())
                .ok_or(SignInRefused::TokenWrong),
        }
    }

    /// The sign-in cookie's value a request carries, if any.
    fn cookie_of<'headers>(&self, headers: &'headers HeaderMap) -> Option<&'headers str> {
        cookie_named(headers, &self.cookie)
    }
}

/// The value of the cookie `name` among a request's `Cookie` headers, if
/// any.
fn cookie_named<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .find_map(|pair| {
            let (found, value) = pair.trim().split_once('=')?;
            (found == name).then_some(value)
        })
}

/// The link's token a `POST /sign-in` body carries: its text, trimmed;
/// empty when it is no UTF-8.
fn sign_in_token(body: &[u8]) -> &str {
    std::str::from_utf8(body).unwrap_or_default().trim()
}

/// The fuzz target `console_sign_in`'s body (ADR 0067): the inputs of the
/// Console's sign-in endpoints, which any page or program may send, split
/// at NUL bytes into a `Cookie` header, a `POST /sign-in` or
/// `POST /offline-key` body and an `X-CSRF-Token` header. Each is read as
/// the endpoints read it: a cookie's value is one of the header's pairs and
/// its sign-in's identifier its first part, a body's token is trimmed text,
/// a body's wrapped key reads back as itself, and a CSRF token is header
/// text. It panics only on a bug.
pub fn fuzz_sign_in_endpoints(data: &[u8]) {
    const NAME: &str = "goodfetch_console_0.0";
    let mut parts = data.splitn(3, |byte| *byte == 0);
    let (cookies, body, csrf) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_bytes(cookies) {
        headers.insert(header::COOKIE, value);
    }
    if let Ok(value) = HeaderValue::from_bytes(csrf) {
        headers.insert(CSRF_HEADER, value);
    }
    if let Some(value) = cookie_named(&headers, NAME) {
        assert!(!value.contains(';'), "a cookie's value spans two pairs");
        let text = headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(
            text.contains(&format!("{NAME}={value}")),
            "a cookie's value is no pair of its header"
        );
        if let Some(id) = sign_in_id(value) {
            assert!(
                value.starts_with(id) && !id.contains('.'),
                "a sign-in's identifier is not its cookie's first part"
            );
        }
    }
    let token = sign_in_token(body);
    assert_eq!(token, token.trim(), "a sign-in token keeps its blanks");
    offline_key::fuzz_body(body);
    let _ = headers.get(CSRF_HEADER).map(HeaderValue::to_str);
}

/// A refusal by the endpoint table's rule `reason`, logged, answered with
/// `status` and no body.
fn refused(
    reason: &'static str,
    status: StatusCode,
    policy: &HeaderValue,
) -> Response<Full<Bytes>> {
    tracing::warn!(target: REFUSAL_TARGET, listener = Listener::Console.name(), reason, "the Console refused a request");
    plain(status, policy)
}

/// What the Console answers `method` on `path`: a file of `build` as it is
/// stored, the reports endpoint's acknowledgement, or an error, each under
/// the production headers.
/// What a Console build answers `method` on `path` with, under the
/// production headers and `policy`: the file the path names, brotli as it
/// is stored, or the page itself for `/` and each route of the build; `204`
/// to a policy violation report; `404` or `405` otherwise. The Stub gateway
/// serves Foxhound's build with it, as Goodfetch Cloud will (ADR 0096).
pub fn build_file(
    build: &ConsoleBuild,
    method: &Method,
    path: &str,
    policy: &HeaderValue,
) -> Response<Full<Bytes>> {
    if path == REPORTS {
        return if method == Method::POST {
            tracing::debug!("the Console received a Content Security Policy report");
            plain(StatusCode::NO_CONTENT, policy)
        } else {
            refused_method(&["POST"], policy)
        };
    }
    let Some(file) = build.file(path) else {
        return plain(StatusCode::NOT_FOUND, policy);
    };
    if method != Method::GET && method != Method::HEAD {
        return refused_method(&["GET", "HEAD"], policy);
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
    production_headers(response, policy)
}

/// An answer with no body.
fn plain(status: StatusCode, policy: &HeaderValue) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(UNSTORED));
    production_headers(response, policy)
}

/// A method the path does not take, naming those it does.
fn refused_method(allowed: &[&'static str], policy: &HeaderValue) -> Response<Full<Bytes>> {
    let mut response = plain(StatusCode::METHOD_NOT_ALLOWED, policy);
    if let Ok(allow) = HeaderValue::from_str(&allowed.join(", ")) {
        response.headers_mut().insert(header::ALLOW, allow);
    }
    response
}

/// `response` with the production headers every answer of the Console
/// carries: `policy`, the Content Security Policy, then
/// `Referrer-Policy: no-referrer`, COOP and CORP `same-origin` and
/// `X-Content-Type-Options: nosniff`.
pub fn production_headers<B>(mut response: Response<B>, policy: &HeaderValue) -> Response<B> {
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, policy.clone());
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

#[cfg(test)]
mod sign_in_endpoints {
    goodfetch_test_support::fuzz_replay!(
        crate::console::fuzz_sign_in_endpoints,
        "fuzz/console_sign_in"
    );
}
