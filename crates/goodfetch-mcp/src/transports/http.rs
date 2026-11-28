//! Streamable HTTP transport for [`McpServer`] (ADRs 0011, 0017, 0041, 0067,
//! 0070; spec #608, ticket #1365).
//!
//! Enforces loopback binding by default, strict `Host` and `Origin` checks,
//! constant-time per-launch bearer token authentication, query-string token
//! refusal, and per-session engine Connection isolation via `rmcp`'s
//! [`StreamableHttpService`].

use std::convert::Infallible;
use std::fmt::Write as _;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use goodfetch_protocol::audit::{Listener, RefusalReason};
use http::{HeaderMap, HeaderValue, Request, Response, StatusCode, header};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::TransportError;
use crate::engine::EngineConnector;
use crate::server::{McpServer, ServerOptions};

/// The Streamable HTTP endpoint path for MCP requests.
pub const MCP_HTTP_PATH: &str = "/mcp";

/// The `tracing` target used for refused endpoint attempts (matches
/// `goodfetch_engine::endpoint::REFUSAL_TARGET`).
pub const REFUSAL_TARGET: &str = "goodfetch::rpc::refusal";

/// Default loopback bind address when `--http` is passed without an address.
pub const DEFAULT_HTTP_BIND: &str = "127.0.0.1:0";

/// Length in bytes of a per-launch random token (256 bits, ADR 0017).
const TOKEN_BYTES: usize = 32;

/// Maximum time to wait for an HTTP request head before closing the connection.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of concurrent HTTP connections accepted at once.
const OPEN_AT_MOST: usize = 64;

/// Grace period for in-flight HTTP responses when shutting down.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Generates a fresh 256-bit per-launch token from the OS generator (never
/// from `Entropy`, ADR 0059), formatted as 64 lowercase hex characters.
///
/// # Errors
///
/// Returns [`getrandom::Error`] if the OS random source fails.
pub fn new_launch_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().fold(String::new(), |mut token, byte| {
        let _ = write!(token, "{byte:02x}");
        token
    }))
}

/// Constant-time comparison of a presented token against the expected token.
#[must_use]
pub fn same_token(given: &str, expected: &str) -> bool {
    bool::from(given.as_bytes().ct_eq(expected.as_bytes()))
}

/// Extracts the bearer token from an `Authorization: Bearer <token>` header.
#[must_use]
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let trimmed = token.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Returns `true` if the query string carries a `token` or `access_token`
/// parameter (including percent-encoded names), which ADR 0017 refuses.
#[must_use]
pub fn query_carries_token(query: Option<&str>) -> bool {
    let Some(query) = query else {
        return false;
    };
    url::form_urlencoded::parse(query.as_bytes()).any(|(key, _)| {
        key.eq_ignore_ascii_case("access_token") || key.eq_ignore_ascii_case("token")
    })
}

/// Admitted `Host` authorities and `Origin` scheme for a Streamable HTTP
/// listener (mirrors `goodfetch_engine::endpoint::Authorities`, ADR 0017).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpAuthorities {
    names: Vec<String>,
    origin_scheme: &'static str,
}

impl HttpAuthorities {
    /// Authorities admitted on a plain loopback listener bound at `bound`:
    /// `bound` itself and `localhost:<port>`, with `http://` origins.
    #[must_use]
    pub fn loopback(bound: SocketAddr) -> Self {
        let port = bound.port();
        Self {
            names: vec![bound.to_string(), format!("localhost:{port}")],
            origin_scheme: "http://",
        }
    }

    /// Whether `host` is an admitted `Host` header value.
    #[must_use]
    pub fn admits_host(&self, host: &str) -> bool {
        self.names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(host))
    }

    /// Whether `origin` is an admitted `Origin` header value.
    #[must_use]
    pub fn admits_origin(&self, origin: &str) -> bool {
        origin
            .strip_prefix(self.origin_scheme)
            .is_some_and(|authority| self.admits_host(authority))
    }

    /// Validates `Host` (required) and `Origin` (optional, must match when
    /// present) on an incoming HTTP request.
    ///
    /// # Errors
    ///
    /// Returns [`RefusalReason::HostForeign`] or
    /// [`RefusalReason::OriginForeign`].
    pub fn check_headers(
        &self,
        host: Option<&str>,
        origin: Option<&str>,
    ) -> Result<(), RefusalReason> {
        if !host.is_some_and(|host| self.admits_host(host)) {
            return Err(RefusalReason::HostForeign);
        }
        if let Some(origin) = origin
            && !self.admits_origin(origin)
        {
            return Err(RefusalReason::OriginForeign);
        }
        Ok(())
    }
}

/// Callback invoked whenever the Streamable HTTP listener refuses an attempt
/// so the executable can record it in the Audit log (`Listener::McpHttp`).
pub trait HttpRefusalRecorder: Send + Sync + 'static {
    /// Records one refusal for `reason`.
    fn record(&self, reason: RefusalReason) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Authentication policy for the Streamable HTTP listener.
#[derive(Clone)]
pub enum HttpAuth {
    /// Local loopback server protected by a per-launch random bearer token
    /// (ADR 0017, ticket #1365).
    LaunchToken(SecretString),
}

/// Configuration for binding a Streamable HTTP MCP server.
pub struct HttpServerConfig {
    /// Socket address to bind (`127.0.0.1:0` by default).
    pub bind: SocketAddr,
    /// Authentication mode (per-launch bearer token on loopback).
    pub auth: HttpAuth,
    /// Optional Audit log refusal recorder.
    pub refusals: Option<Arc<dyn HttpRefusalRecorder>>,
    /// Cancellation token that gracefully shuts the listener down.
    pub cancellation: CancellationToken,
}

/// A bound Streamable HTTP MCP listener ready to announce its `{ address, token }`
/// startup record and begin serving requests.
pub struct BoundHttpServer {
    #[expect(
        clippy::disallowed_types,
        reason = "the MCP transports module is the one place that opens the Streamable HTTP listener (ADR 0060)"
    )]
    listener: tokio::net::TcpListener,
    local_addr: SocketAddr,
    url: String,
    state: Arc<HttpServingState>,
}

struct HttpServingState {
    authorities: HttpAuthorities,
    auth: HttpAuth,
    refusals: Option<Arc<dyn HttpRefusalRecorder>>,
    cancellation: CancellationToken,
    mcp_service: StreamableHttpService<McpServer, LocalSessionManager>,
}

impl BoundHttpServer {
    /// Binds a TCP socket at `config.bind` and prepares the Streamable HTTP
    /// MCP server without starting the accept loop yet.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Serve`] if binding the socket fails.
    pub async fn bind(
        config: HttpServerConfig,
        options: ServerOptions,
        connector: Arc<dyn EngineConnector>,
    ) -> Result<Self, TransportError> {
        #[expect(
            clippy::disallowed_types,
            reason = "the MCP transports module is the one place that opens the Streamable HTTP listener (ADR 0060)"
        )]
        let listener = tokio::net::TcpListener::bind(config.bind)
            .await
            .map_err(|err| {
                TransportError::Serve(format!("failed to bind {}: {err}", config.bind))
            })?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| TransportError::Serve(format!("failed to read bound address: {err}")))?;
        let url = format_endpoint_url("http", local_addr);
        let authorities = HttpAuthorities::loopback(local_addr);
        let session_manager = Arc::new(LocalSessionManager::default());
        let rmcp_config = StreamableHttpServerConfig::default()
            .disable_allowed_hosts()
            .disable_allowed_origins()
            .with_cancellation_token(config.cancellation.child_token());
        let mcp_service = StreamableHttpService::new(
            move || {
                Ok(McpServer::with_connector(
                    options.clone(),
                    Arc::clone(&connector),
                ))
            },
            session_manager,
            rmcp_config,
        );
        let state = Arc::new(HttpServingState {
            authorities,
            auth: config.auth,
            refusals: config.refusals,
            cancellation: config.cancellation,
            mcp_service,
        });
        Ok(Self {
            listener,
            local_addr,
            url,
            state,
        })
    }

    /// The OS-assigned local socket address.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The full Streamable HTTP endpoint URL (`http://<addr>/mcp`).
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Runs the accept loop until `cancellation` fires, then drains active
    /// connections cleanly.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Serve`] on fatal listener errors.
    ///
    /// # Cancel safety
    ///
    /// Cancelling the server via its `CancellationToken` drains active
    /// connections gracefully.
    pub async fn serve(self) -> Result<(), TransportError> {
        let tasks = TaskTracker::new();
        let open = Arc::new(Semaphore::new(OPEN_AT_MOST));
        let cancellation = self.state.cancellation.clone();

        loop {
            let accepted = tokio::select! {
                () = cancellation.cancelled() => break,
                accepted = self.listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, _peer)) => {
                    let Ok(permit) = Arc::clone(&open).try_acquire_owned() else {
                        tracing::debug!(
                            "an MCP HTTP connection past the concurrency limit was closed"
                        );
                        drop(stream);
                        continue;
                    };
                    let state = Arc::clone(&self.state);
                    let stopping = cancellation.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        serve_http_connection(stream, state, stopping).await;
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "MCP Streamable HTTP accept failed");
                    tokio::select! {
                        () = cancellation.cancelled() => break,
                        () = tokio::time::sleep(Duration::from_millis(200)) => {}
                    }
                }
            }
        }

        tasks.close();
        tasks.wait().await;
        Ok(())
    }
}

async fn serve_http_connection<S>(
    stream: S,
    state: Arc<HttpServingState>,
    stopping: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let connection = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT)
        .serve_connection(
            TokioIo::new(stream),
            hyper::service::service_fn(move |request: Request<Incoming>| {
                let state = Arc::clone(&state);
                async move { Ok::<_, Infallible>(state.answer(request).await) }
            }),
        );
    tokio::pin!(connection);
    tokio::select! {
        ended = connection.as_mut() => {
            if let Err(error) = ended {
                tracing::debug!(%error, "MCP HTTP connection ended");
            }
        }
        () = stopping.cancelled() => {
            connection.as_mut().graceful_shutdown();
            tokio::select! {
                ended = connection.as_mut() => {
                    if let Err(error) = ended {
                        tracing::debug!(%error, "MCP HTTP connection ended during shutdown");
                    }
                }
                () = tokio::time::sleep(SHUTDOWN_GRACE) => {}
            }
        }
    }
}

impl HttpServingState {
    async fn answer(&self, request: Request<Incoming>) -> Response<BoxBody<Bytes, Infallible>> {
        let headers = request.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        // Any `Origin` header is checked even when it is not valid UTF-8.
        let origin = headers
            .get(header::ORIGIN)
            .map(|value| value.to_str().unwrap_or_default());

        if let Err(reason) = self.authorities.check_headers(host, origin) {
            self.record_refusal(reason).await;
            let message = match reason {
                RefusalReason::HostForeign => "Forbidden: Host header is not admitted",
                _ => "Forbidden: Origin header is not admitted",
            };
            return plain_error(StatusCode::FORBIDDEN, message);
        }

        if request.uri().path() != MCP_HTTP_PATH {
            return plain_error(StatusCode::NOT_FOUND, "Not Found");
        }

        if let Some(unauthorized) = self.check_auth(request.uri().query(), headers).await {
            return unauthorized;
        }

        self.mcp_service.handle(request).await
    }

    async fn check_auth(
        &self,
        query: Option<&str>,
        headers: &HeaderMap,
    ) -> Option<Response<BoxBody<Bytes, Infallible>>> {
        let query_token = query_carries_token(query);
        match &self.auth {
            HttpAuth::LaunchToken(expected) => match bearer_token(headers) {
                None => {
                    self.record_refusal(RefusalReason::TokenMissing).await;
                    Some(unauthorized_bearer(
                        "a bearer token is required in `Authorization: Bearer`",
                    ))
                }
                Some(_) if query_token => {
                    self.record_refusal(RefusalReason::TokenWrong).await;
                    Some(unauthorized_bearer(
                        "tokens in the query string are refused",
                    ))
                }
                Some(given) if !same_token(given, expected.expose_secret()) => {
                    self.record_refusal(RefusalReason::TokenWrong).await;
                    Some(unauthorized_bearer("the bearer token was refused"))
                }
                Some(_) => None,
            },
        }
    }

    async fn record_refusal(&self, reason: RefusalReason) {
        if let Some(recorder) = &self.refusals {
            recorder.record(reason).await;
        } else {
            tracing::warn!(
                target: REFUSAL_TARGET,
                listener = Listener::McpHttp.as_str(),
                reason = reason.as_str(),
                "a listener refused an attempt"
            );
        }
    }
}

fn format_endpoint_url(scheme: &str, addr: SocketAddr) -> String {
    format!("{scheme}://{addr}{MCP_HTTP_PATH}")
}

fn unauthorized_bearer(message: &str) -> Response<BoxBody<Bytes, Infallible>> {
    let mut response = plain_error(StatusCode::UNAUTHORIZED, message);
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn plain_error(status: StatusCode, message: &str) -> Response<BoxBody<Bytes, Infallible>> {
    let mut response = Response::new(Full::new(Bytes::from(message.to_owned())).boxed());
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}
