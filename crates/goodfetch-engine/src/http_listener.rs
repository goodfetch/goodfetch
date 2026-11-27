//! The Daemon's HTTP JSON-RPC listener (`POST /v1/rpc`, ADRs 0027, 0030, 0049,
//! 0067, 0099 and 0132): owns the listener's routes, bearer token verification,
//! `Host` and `Origin` checks (reusing `endpoint::Authorities` and
//! `endpoint::refusal`), and TLS termination (reusing the Exposed Daemon's
//! keys). It decodes, dispatches and encodes nothing itself: every valid
//! `POST /v1/rpc` hands its body and header-derived `hello` fields straight to
//! `rpc::serve_http_request` as one more carriage of a Connection (ADR 0140's
//! E1).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::connection::{EngineInstance, Form, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::events::{LastRead, Position};
use goodfetch_protocol::http_listener::{
    EVENTS_PATH, HEADER_FORM, HEADER_PRODUCT, HEADER_PROTOCOL, RPC_PATH, parse_form, parse_product,
};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, TraceContext};
use goodfetch_protocol::run::RunId;
use http_body_util::{BodyExt as _, Full, LengthLimitError, Limited};
use hyper::body::{Body, Bytes, Frame, Incoming, SizeHint};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Map;
use tokio::sync::Semaphore;

use crate::call_error::CallError;
use crate::composition::DaemonSide;
use crate::endpoint::{self, Authorities, Exposure, Hosts, Origins, Termination, same_token};
use crate::engine_context::EngineContext;
use crate::events::Events;
use crate::framing::MAX_MESSAGE_BYTES;
use crate::rpc::{
    Dispatch, HttpAnswer, HttpContext, HttpRequest, HttpStatus, SseStream, check_http_hello,
    http_error_bytes, open_sse_stream, serve_http_request, terminate_tls,
};
use crate::runs::Runs;
use crate::sync::{AtomicU64, Ordering};
use crate::tasks::{OwnerKind, TaskOwner};
use crate::transport::{Connection, DaemonListener, TransportError};

/// How long a connection has to send a request head before the listener closes it.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How many HTTP listener connections may be open at once.
const OPEN_AT_MOST: usize = 64;

/// How long the listener pauses after a process-wide accept error (`EMFILE`,
/// `ENFILE`) before accepting again.
const ACCEPT_PAUSE: Duration = Duration::from_secs(1);

/// The `Content-Type` of every JSON-RPC response body.
const APPLICATION_JSON: &str = "application/json";

/// The `Content-Type` of Server-Sent Events (`GET /v1/events`).
const TEXT_EVENT_STREAM: &str = "text/event-stream; charset=utf-8";

/// The HTTP response body of the listener: a single JSON-RPC payload or a
/// Server-Sent Events stream backed by `rpc::stream::SseStream`.
enum HttpBody {
    Full(Full<Bytes>),
    Events(SseStream),
}

impl Body for HttpBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Self::Full(full) => Pin::new(full).poll_frame(cx),
            Self::Events(events) => Pin::new(events).poll_frame(cx),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Full(full) => full.is_end_stream(),
            Self::Events(_) => false,
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Full(full) => full.size_hint(),
            Self::Events(events) => events.size_hint(),
        }
    }
}

/// Configuration for the Daemon's HTTP JSON-RPC listener, built by the
/// Daemon's starter once `http_listener.enabled` is on.
pub struct HttpListenerOptions {
    /// The listening socket or in-memory network opened through `transport`.
    pub listener: DaemonListener,
    /// The address bound by `listener` (including the OS-assigned port when
    /// configured with port `0`).
    pub bound: SocketAddr,
    /// The bearer token (`http-listener.token`), compared in constant time.
    pub token: SecretString,
    /// Allowed `Host`/`Origin` authorities and TLS termination (`daemon.tls.*`).
    pub exposure: Exposure,
}

/// What the Daemon hands the HTTP listener to serve requests against the same
/// dispatch, Runs, Events and idle holds as its socket listener.
pub(crate) struct DaemonContext {
    pub(crate) dispatch: Arc<Dispatch>,
    pub(crate) runs: Runs,
    pub(crate) events: Events,
    pub(crate) engine: EngineContext,
    pub(crate) daemon: Arc<dyn DaemonSide>,
    pub(crate) next_connection_id: Arc<AtomicU64>,
    pub(crate) hold_outliving: Arc<dyn Fn() -> Box<dyn Send + Sync> + Send + Sync>,
}

/// Shared state across accepted HTTP listener connections.
struct Serving {
    token: SecretString,
    exposure: Exposure,
    exposed: bool,
    daemon: DaemonContext,
    owner: TaskOwner,
}

/// Serves the Daemon's HTTP JSON-RPC listener until `owner` stops or the
/// listener closes.
///
/// # Cancel safety
///
/// Dropping the future stops accepting; active connections belong to `owner`.
pub(crate) async fn serve(options: HttpListenerOptions, daemon: DaemonContext, owner: TaskOwner) {
    let HttpListenerOptions {
        mut listener,
        bound,
        token,
        exposure,
    } = options;
    let clock = daemon.engine.clock();
    let exposed = !matches!(exposure.termination, Termination::Plain) || !bound.ip().is_loopback();
    let serving = Arc::new(Serving {
        token,
        exposure,
        exposed,
        daemon,
        owner: owner.child(),
    });
    let open = Arc::new(Semaphore::new(OPEN_AT_MOST));
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(connection) => {
                let Ok(permit) = Arc::clone(&open).try_acquire_owned() else {
                    tracing::debug!(
                        "an HTTP listener connection past the concurrency limit is closed"
                    );
                    drop(connection);
                    continue;
                };
                let stopping = owner.token().clone();
                let serving = Arc::clone(&serving);
                owner.spawn(async move {
                    let _permit = permit;
                    serve_connection(connection, serving, stopping).await;
                });
            }
            Err(TransportError::DialerClosed | TransportError::EngineGone) => return,
            Err(TransportError::ListenerClosed(error)) => {
                tracing::warn!(%error, "the HTTP listener is closed and stops accepting");
                return;
            }
            Err(TransportError::Stream(error)) => {
                tracing::debug!(%error, "an accepted HTTP listener stream was dropped");
            }
            Err(TransportError::Accept(error)) => {
                tracing::warn!(%error, "the HTTP listener cannot accept a connection");
                tokio::select! {
                    () = owner.token().cancelled() => return,
                    () = clock.sleep(ACCEPT_PAUSE) => {}
                }
            }
        }
    }
}

/// Terminates TLS when configured, then serves HTTP/1.1 requests on `connection`.
async fn serve_connection(
    connection: Connection,
    serving: Arc<Serving>,
    stopping: tokio_util::sync::CancellationToken,
) {
    let peer = connection.addresses().map(|addresses| addresses.peer);
    let (connection, authorities) = match &serving.exposure.termination {
        Termination::Plain | Termination::Upstream(_) => {
            (connection, serving.exposure.authorities.clone())
        }
        Termination::Tls(certificate) => {
            let Some(terminated) = terminate_tls(
                connection,
                certificate,
                &serving.exposure.authorities,
                |_| Listener::HttpListener,
                &serving.daemon.engine,
            )
            .await
            else {
                return;
            };
            terminated
        }
    };
    let authorities = Arc::new(authorities);
    let (reader, writer) = connection.split();
    let stream = tokio::io::join(reader, writer);
    let connection = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT)
        .serve_connection(
            TokioIo::new(stream),
            hyper::service::service_fn(move |request: Request<Incoming>| {
                let serving = Arc::clone(&serving);
                let authorities = Arc::clone(&authorities);
                async move { Ok::<_, Infallible>(serving.answer(request, &authorities, peer).await) }
            }),
        );
    tokio::pin!(connection);
    tokio::select! {
        ended = connection.as_mut() => {
            if let Err(error) = ended {
                tracing::debug!(%error, "an HTTP listener connection ended with an error");
            }
        }
        () = stopping.cancelled() => connection.as_mut().graceful_shutdown(),
    }
}

impl Serving {
    /// Answers one HTTP request.
    async fn answer(
        &self,
        request: Request<Incoming>,
        authorities: &Authorities,
        peer: Option<SocketAddr>,
    ) -> Response<HttpBody> {
        let headers = request.headers();
        let effective = if self.exposure.termination.trusts(peer) {
            authorities.forwarded(
                header_str(headers, "x-forwarded-host"),
                header_str(headers, "x-forwarded-proto"),
            )
        } else {
            authorities.clone()
        };
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        // Any `Origin` header is checked even when it is not valid UTF-8.
        let origin = headers
            .get(header::ORIGIN)
            .map(|value| value.to_str().unwrap_or_default());
        if let Some(refusal) = endpoint::refusal(
            host,
            origin,
            Hosts::Named(&effective),
            Origins::Optional(&[&effective]),
        ) {
            self.daemon
                .engine
                .refusals()
                .refused(Listener::HttpListener, refusal.reason())
                .await;
            let message = match refusal.reason() {
                RefusalReason::HostForeign => "the request's `Host` header is not admitted",
                _ => "the request's `Origin` header is not admitted",
            };
            return self.json_error(
                StatusCode::FORBIDDEN,
                &CallError::protocol(ProtocolFailure::InvalidRequest, message),
                Form::Catalog,
            );
        }
        match request.uri().path() {
            RPC_PATH => self.answer_rpc(request).await,
            EVENTS_PATH => self.answer_events(&request).await,
            _ => self.json_error(
                StatusCode::NOT_FOUND,
                &CallError::protocol(ProtocolFailure::InvalidRequest, "route not found"),
                Form::Catalog,
            ),
        }
    }

    async fn answer_rpc(&self, request: Request<Incoming>) -> Response<HttpBody> {
        if request.method() != Method::POST {
            return self.method_not_allowed("`/v1/rpc` accepts `POST` only", "POST");
        }
        let headers = request.headers();
        if let Some(unauthorized) = self.check_token(request.uri().query(), headers).await {
            return unauthorized;
        }
        let hello =
            match parse_hello_headers(headers, &self.daemon.dispatch.identity.product_version) {
                Ok(hello) => hello,
                Err(error) => {
                    return self.json_error(StatusCode::BAD_REQUEST, &error, Form::Catalog);
                }
            };
        let form = hello.form.unwrap_or(Form::Catalog);
        let fallback_trace = extract_trace_headers(headers);
        let body = match read_body(request.into_body()).await {
            Ok(body) => body,
            Err((status, error)) => return self.json_error(status, &error, form),
        };
        let answer = self
            .dispatch_rpc(HttpRequest {
                hello,
                body,
                fallback_trace,
            })
            .await;
        http_response(answer)
    }

    async fn answer_events(&self, request: &Request<Incoming>) -> Response<HttpBody> {
        if request.method() != Method::GET {
            return self.method_not_allowed("`/v1/events` accepts `GET` only", "GET");
        }
        let headers = request.headers();
        if let Some(unauthorized) = self.check_token(request.uri().query(), headers).await {
            return unauthorized;
        }
        if headers.contains_key(HEADER_PROTOCOL) || headers.contains_key(HEADER_PRODUCT) {
            let hello = match parse_hello_headers(
                headers,
                &self.daemon.dispatch.identity.product_version,
            ) {
                Ok(hello) => hello,
                Err(error) => {
                    return self.json_error(StatusCode::BAD_REQUEST, &error, Form::Catalog);
                }
            };
            let form = hello.form.unwrap_or(Form::Catalog);
            if let Err(error) = check_http_hello(
                &self.daemon.dispatch,
                &self.daemon.events,
                self.daemon.daemon.as_ref(),
                &hello,
            )
            .await
            {
                return self.json_error(StatusCode::BAD_REQUEST, &error, form);
            }
        }
        let (run, after) = match parse_events_request(
            request.uri().query(),
            headers,
            self.daemon.events.instance(),
        ) {
            Ok(parsed) => parsed,
            Err(error) => {
                return self.json_error(StatusCode::BAD_REQUEST, &error, Form::Catalog);
            }
        };
        let Some(stream) = open_sse_stream(
            &self.daemon.events,
            after,
            run,
            self.daemon.dispatch.masks.clone(),
            &self.owner,
        )
        .await
        else {
            return self.json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                &CallError::protocol(
                    ProtocolFailure::InternalError,
                    "the Daemon's event stream is closed",
                ),
                Form::Catalog,
            );
        };
        sse_response(stream)
    }

    fn method_not_allowed(&self, message: &str, allow: &'static str) -> Response<HttpBody> {
        let mut response = self.json_error(
            StatusCode::METHOD_NOT_ALLOWED,
            &CallError::protocol(ProtocolFailure::InvalidRequest, message),
            Form::Catalog,
        );
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static(allow));
        response
    }

    /// Checks `Authorization: Bearer` and refuses tokens carried in the query
    /// string, logging and auditing each refusal.
    async fn check_token(
        &self,
        query: Option<&str>,
        headers: &HeaderMap,
    ) -> Option<Response<HttpBody>> {
        let query_token = query_carries_token(query);
        match endpoint::bearer(headers) {
            None => {
                self.daemon
                    .engine
                    .refusals()
                    .refused(Listener::HttpListener, RefusalReason::TokenMissing)
                    .await;
                Some(self.unauthorized("a bearer token is required in `Authorization: Bearer`"))
            }
            Some(_) if query_token => {
                self.daemon
                    .engine
                    .refusals()
                    .refused(Listener::HttpListener, RefusalReason::TokenWrong)
                    .await;
                Some(self.unauthorized("tokens in the query string are refused"))
            }
            Some(given) if !same_token(given, self.token.expose_secret()) => {
                self.daemon
                    .engine
                    .refusals()
                    .refused(Listener::HttpListener, RefusalReason::TokenWrong)
                    .await;
                Some(self.unauthorized("the bearer token was refused"))
            }
            Some(_) => None,
        }
    }

    /// Hands one validated `POST /v1/rpc` request to `rpc` as a single-request
    /// Connection carriage, keeping any Run it starts alive after the HTTP
    /// response completes.
    async fn dispatch_rpc(&self, request: HttpRequest) -> HttpAnswer {
        let id = self
            .daemon
            .next_connection_id
            .fetch_add(1, Ordering::Relaxed);
        let runs = self.daemon.runs.on_connection(id);
        let request_owner = self.owner.child_for(OwnerKind::Connection);
        let context = HttpContext {
            dispatch: Arc::clone(&self.daemon.dispatch),
            runs: runs.clone(),
            events: self.daemon.events.clone(),
            daemon: Arc::clone(&self.daemon.daemon),
            exposed: self.exposed,
            engine: self.daemon.engine.clone(),
        };
        let answer = serve_http_request(context, request, &request_owner).await;
        request_owner.stop().await;
        let outliving = runs.stop_connection(id).await;
        if !outliving.is_empty() {
            let held = (self.daemon.hold_outliving)();
            let stopping = self.owner.token().clone();
            self.owner.spawn(async move {
                tokio::select! {
                    biased;
                    () = stopping.cancelled() => {}
                    () = outliving.ended() => {}
                }
                drop(held);
            });
        }
        answer
    }

    fn unauthorized(&self, message: &str) -> Response<HttpBody> {
        let mut response = self.json_error(
            StatusCode::UNAUTHORIZED,
            &CallError::protocol(ProtocolFailure::InvalidRequest, message),
            Form::Catalog,
        );
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        response
    }

    fn json_error(&self, status: StatusCode, error: &CallError, form: Form) -> Response<HttpBody> {
        let body = http_error_bytes(
            error,
            form,
            &self.daemon.dispatch.masks,
            self.daemon.dispatch.work(),
        );
        json_response(status, Some(body))
    }
}

/// Parses `run=<id>`, `after=<position>` and `Last-Event-ID` for
/// `GET /v1/events`. `Last-Event-ID` takes precedence over `after` so an SSE
/// client reconnecting to its initial URL resumes from its last received ID;
/// both parse identically through [`parse_position`].
fn parse_events_request(
    query: Option<&str>,
    headers: &HeaderMap,
    instance: &EngineInstance,
) -> Result<(Option<RunId>, Option<LastRead>), CallError> {
    let mut run: Option<RunId> = None;
    let mut query_after: Option<String> = None;
    if let Some(query) = query {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "run" => {
                    run = Some(RunId::parse(value.as_ref()).map_err(|error| {
                        CallError::protocol(
                            ProtocolFailure::InvalidParams,
                            format!("invalid `run` query parameter: {error}"),
                        )
                    })?);
                }
                "after" => {
                    query_after = Some(value.into_owned());
                }
                _ => {}
            }
        }
    }
    let last_event_id = headers
        .get("last-event-id")
        .map(|value| {
            value.to_str().map(str::trim).map_err(|_| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "`Last-Event-ID` header must be UTF-8",
                )
            })
        })
        .transpose()?
        .filter(|value| !value.is_empty());
    let raw_position = last_event_id.or(query_after.as_deref());
    let after = raw_position
        .map(|value| parse_position(value, instance))
        .transpose()?;
    Ok((run, after))
}

/// Parses an event position (`<u64>` on the current engine instance, or
/// `<engine_instance>:<u64>`).
fn parse_position(raw: &str, current: &EngineInstance) -> Result<LastRead, CallError> {
    let trimmed = raw.trim();
    if let Some((instance, position)) = trimmed.split_once(':') {
        let position = position.parse::<u64>().map_err(|_| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("`{trimmed}` is not a valid event position"),
            )
        })?;
        return Ok(LastRead {
            engine_instance: EngineInstance::new(instance),
            position: Position(position),
        });
    }
    let position = trimmed.parse::<u64>().map_err(|_| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{trimmed}` is not a valid event position"),
        )
    })?;
    Ok(LastRead {
        engine_instance: current.clone(),
        position: Position(position),
    })
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// Whether a request's query string carries a token parameter (`access_token`
/// or `token`), which ADR 0017 forbids.
fn query_carries_token(query: Option<&str>) -> bool {
    let Some(query) = query else {
        return false;
    };
    query.split('&').any(|pair| {
        let key = pair.split_once('=').map_or(pair, |(key, _)| key);
        key.eq_ignore_ascii_case("access_token") || key.eq_ignore_ascii_case("token")
    })
}

/// Builds `HelloParams` from `Goodfetch-Protocol`, `Goodfetch-Product` and
/// optional `Goodfetch-Form` headers (ADRs 0027 and 0099).
fn parse_hello_headers(
    headers: &HeaderMap,
    product_version: &ProductVersion,
) -> Result<HelloParams, CallError> {
    let protocol_text = header_str(headers, HEADER_PROTOCOL)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                "missing required `Goodfetch-Protocol` header",
            )
        })?;
    let protocol_version = protocol_text.parse::<ProtocolVersion>().map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("invalid `Goodfetch-Protocol` header: {error}"),
        )
    })?;
    let product_text = header_str(headers, HEADER_PRODUCT)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                "missing required `Goodfetch-Product` header",
            )
        })?;
    let product = parse_product(product_text).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("invalid `Goodfetch-Product` header `{product_text}`"),
        )
    })?;
    let form = match header_str(headers, HEADER_FORM).map(str::trim) {
        None | Some("") => Form::Catalog,
        Some(text) => parse_form(text).ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("invalid `Goodfetch-Form` header `{text}`"),
            )
        })?,
    };
    Ok(HelloParams {
        protocol_version,
        surface: Surface::HttpListener,
        client_surface: None,
        needs: Some(product),
        product_version: product_version.clone(),
        token: None,
        form: Some(form),
        log_level: None,
        settings: Map::new(),
        handlers: BTreeMap::new(),
        keep_daemon_alive: false,
    })
}

/// Extracts optional W3C Trace Context headers (`traceparent` and `tracestate`)
/// for fallback when `_meta` carries none (ADR 0132). Non-UTF-8 values are
/// dropped rather than refused.
fn extract_trace_headers(headers: &HeaderMap) -> Option<goodfetch_protocol::jsonrpc::TraceContext> {
    let traceparent = header_str(headers, "traceparent").map(str::to_owned);
    let tracestate = header_str(headers, "tracestate").map(str::to_owned);
    if traceparent.is_none() && tracestate.is_none() {
        None
    } else {
        Some(TraceContext {
            traceparent,
            tracestate,
        })
    }
}

/// Collects the HTTP request body bounded by [`MAX_MESSAGE_BYTES`].
async fn read_body(body: Incoming) -> Result<Vec<u8>, (StatusCode, CallError)> {
    match Limited::new(body, MAX_MESSAGE_BYTES).collect().await {
        Ok(collected) => Ok(collected.to_bytes().into()),
        Err(error) => {
            if error.downcast_ref::<LengthLimitError>().is_some() {
                Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    CallError::protocol(
                        ProtocolFailure::MessageTooLarge,
                        format!("the request body exceeds {MAX_MESSAGE_BYTES} bytes"),
                    ),
                ))
            } else {
                Err((
                    StatusCode::BAD_REQUEST,
                    CallError::protocol(
                        ProtocolFailure::ParseError,
                        "the request body could not be read",
                    ),
                ))
            }
        }
    }
}

fn http_response(answer: HttpAnswer) -> Response<HttpBody> {
    let status = match answer.status {
        HttpStatus::Ok => StatusCode::OK,
        HttpStatus::Accepted => StatusCode::ACCEPTED,
        HttpStatus::BadRequest => StatusCode::BAD_REQUEST,
    };
    json_response(status, answer.body)
}

fn json_response(status: StatusCode, body: Option<Vec<u8>>) -> Response<HttpBody> {
    let has_body = body.is_some();
    let payload = body.map_or_else(Bytes::new, Bytes::from);
    let mut response = Response::new(HttpBody::Full(Full::new(payload)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if has_body {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(APPLICATION_JSON),
        );
    }
    response
}

fn sse_response(stream: SseStream) -> Response<HttpBody> {
    let mut response = Response::new(HttpBody::Events(stream));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(TEXT_EVENT_STREAM),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[cfg(test)]
mod tests;
