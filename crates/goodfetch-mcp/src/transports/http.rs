//! Streamable HTTP transport for [`McpServer`] (ADRs 0011, 0017, 0041, 0049,
//! 0067, 0070; spec #608, tickets #1365 and #1367).
//!
//! Enforces loopback binding by default, strict `Host` and `Origin` checks,
//! constant-time per-launch bearer token authentication on loopback, TLS
//! termination (`daemon.tls.*`) plus OAuth 2.1 RFC 9728 protected-resource
//! metadata and JWT bearer validation on remote addresses (stripping the
//! incoming `Authorization` header before dispatching to `McpServer` so no
//! token ever passes through), query-string token refusal, and per-session
//! engine Connection isolation via `rmcp`'s [`StreamableHttpService`].

use std::convert::Infallible;
use std::fmt::Write as _;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::daemon::TrustedProxy;
use http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, header};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use tokio::sync::Semaphore;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use super::TransportError;
use crate::engine::EngineConnector;
use crate::oauth::{OAuthMetadataFetcher, OAuthResourceServer};
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

/// Maximum time to wait for an HTTP request head or TLS handshake before closing.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of concurrent HTTP connections accepted at once.
const OPEN_AT_MOST: usize = 64;

/// Grace period for in-flight HTTP responses when shutting down.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Timeout for fetching OAuth 2.1 authorization server metadata or JWKS JSON.
const OAUTH_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum response body size accepted from an OAuth metadata or JWKS endpoint.
const MAX_OAUTH_METADATA_BYTES: usize = 256 * 1024;

/// First byte of a TLS handshake record (RFC 8446 Section 5.1).
const TLS_HANDSHAKE_RECORD_TYPE: u8 = 0x16;

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
/// listener (ADRs 0017, 0049).
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

    /// Authorities admitted on a listener bound at `bound` with optional TLS
    /// and an optional public authority (e.g. from `mcp.oauth.audience`).
    #[must_use]
    pub fn for_listener(bound: SocketAddr, tls: &HttpTls, public_authority: Option<&str>) -> Self {
        let port = bound.port();
        let mut names = vec![bound.to_string(), format!("localhost:{port}")];
        if bound.ip().is_unspecified() {
            names.push(format!("127.0.0.1:{port}"));
            if bound.is_ipv6() {
                names.push(format!("[::1]:{port}"));
            }
        }
        if let Some(authority) = public_authority
            && !authority.is_empty()
        {
            names.push(authority.to_owned());
        }
        let origin_scheme = match tls {
            HttpTls::Direct(_) => "https://",
            HttpTls::Plain | HttpTls::Upstream(_) => "http://",
        };
        Self {
            names,
            origin_scheme,
        }
    }

    /// Extends these authorities for one direct-TLS connection naming `sni`.
    #[must_use]
    pub fn with_tls_session(&self, sni: Option<&str>, port: u16) -> Self {
        let mut names = self.names.clone();
        if let Some(host) = sni {
            let entry = if port == 443 {
                host.to_owned()
            } else {
                format!("{host}:{port}")
            };
            names.push(entry);
        }
        Self {
            names,
            origin_scheme: "https://",
        }
    }

    /// Extends these authorities for a request received from a trusted
    /// upstream TLS-terminating proxy (`X-Forwarded-Proto: https` +
    /// `X-Forwarded-Host`).
    #[must_use]
    pub fn with_trusted_forwarded(&self, headers: &HeaderMap) -> Self {
        let first_token = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        if !first_token("x-forwarded-proto")
            .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
        {
            return self.clone();
        }
        let mut names = self.names.clone();
        if let Some(fwd_host) = first_token("x-forwarded-host") {
            names.push(fwd_host.to_owned());
        }
        Self {
            names,
            origin_scheme: "https://",
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

/// Supplies the current [`TlsAcceptor`] for each incoming TLS connection so
/// certificate renewals from `daemon.tls.*` take effect without restarting.
pub trait HttpTlsAcceptor: Send + Sync + 'static {
    /// Returns the active [`TlsAcceptor`].
    fn acceptor(&self) -> Pin<Box<dyn Future<Output = TlsAcceptor> + Send + '_>>;
}

/// Static [`TlsAcceptor`] wrapper implementing [`HttpTlsAcceptor`].
#[derive(Clone)]
pub struct StaticTlsAcceptor(pub TlsAcceptor);

impl HttpTlsAcceptor for StaticTlsAcceptor {
    fn acceptor(&self) -> Pin<Box<dyn Future<Output = TlsAcceptor> + Send + '_>> {
        let acceptor = self.0.clone();
        Box::pin(async move { acceptor })
    }
}

/// Where TLS is terminated for the Streamable HTTP MCP listener (ADR 0049).
#[derive(Clone, Default)]
pub enum HttpTls {
    /// Plain HTTP (loopback only).
    #[default]
    Plain,
    /// Terminated directly by the MCP server (`daemon.tls.cert_file` and
    /// `daemon.tls.key_file`). Non-TLS TCP connections are closed and recorded
    /// as `tls_required`.
    Direct(Arc<dyn HttpTlsAcceptor>),
    /// Terminated upstream (`daemon.tls.upstream`) with trusted proxy ranges
    /// (`daemon.trusted_proxies`).
    Upstream(Vec<TrustedProxy>),
}

impl HttpTls {
    fn trusts_proxy(&self, peer: Option<SocketAddr>) -> bool {
        match self {
            Self::Upstream(proxies) => {
                peer.is_some_and(|addr| proxies.iter().any(|proxy| proxy.holds(addr.ip())))
            }
            Self::Plain | Self::Direct(_) => false,
        }
    }
}

/// Authentication policy for the Streamable HTTP listener.
#[derive(Clone)]
pub enum HttpAuth {
    /// Local loopback server protected by a per-launch random bearer token
    /// (ADR 0017, ticket #1365).
    LaunchToken(SecretString),
    /// Remote resource server protected by OAuth 2.1 (RFC 9728 metadata +
    /// JWKS JWT validation, ADRs 0017 and 0067, ticket #1367).
    OAuth(Arc<OAuthResourceServer>),
}

/// Configuration for binding a Streamable HTTP MCP server.
pub struct HttpServerConfig {
    /// Socket address to bind (`127.0.0.1:0` by default).
    pub bind: SocketAddr,
    /// Authentication mode (per-launch bearer token on loopback, or OAuth 2.1).
    pub auth: HttpAuth,
    /// TLS termination mode (`daemon.tls.*`).
    pub tls: HttpTls,
    /// Optional Audit log refusal recorder.
    pub refusals: Option<Arc<dyn HttpRefusalRecorder>>,
    /// Cancellation token that gracefully shuts the listener down.
    pub cancellation: CancellationToken,
}

/// A bound Streamable HTTP MCP listener ready to announce its startup record
/// and begin serving requests.
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
    tls: HttpTls,
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
        let scheme = match config.tls {
            HttpTls::Direct(_) => "https",
            HttpTls::Plain | HttpTls::Upstream(_) => "http",
        };
        let url = format_endpoint_url(scheme, local_addr);
        let public_authority = match &config.auth {
            HttpAuth::LaunchToken(_) => None,
            HttpAuth::OAuth(oauth) => oauth.config().public_authority(),
        };
        let authorities = HttpAuthorities::for_listener(local_addr, &config.tls, public_authority);
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
            tls: config.tls,
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

    /// The full Streamable HTTP endpoint URL (`http://<addr>/mcp` or `https://<addr>/mcp`).
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
        let local_port = self.local_addr.port();

        loop {
            let accepted = tokio::select! {
                () = cancellation.cancelled() => break,
                accepted = self.listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, peer)) => {
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
                        handle_accepted_stream(stream, peer, local_port, state, stopping).await;
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

#[expect(
    clippy::disallowed_types,
    reason = "the MCP transports module accepts TCP streams from its own listener (ADR 0060)"
)]
async fn handle_accepted_stream(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    local_port: u16,
    state: Arc<HttpServingState>,
    stopping: CancellationToken,
) {
    match &state.tls {
        HttpTls::Plain | HttpTls::Upstream(_) => {
            let authorities = Arc::new(state.authorities.clone());
            serve_http_connection(stream, state, authorities, Some(peer), stopping).await;
        }
        HttpTls::Direct(provider) => {
            let mut first = [0u8; 1];
            match tokio::time::timeout(HEADER_READ_TIMEOUT, stream.peek(&mut first)).await {
                Ok(Ok(0)) => return,
                Ok(Ok(_)) if first[0] != TLS_HANDSHAKE_RECORD_TYPE => {
                    state.record_refusal(RefusalReason::TlsRequired).await;
                    return;
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::debug!(%error, "MCP TLS peek failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!("MCP TLS connection timed out before handshake");
                    return;
                }
            }
            let acceptor = provider.acceptor().await;
            let tls_stream =
                match tokio::time::timeout(HEADER_READ_TIMEOUT, acceptor.accept(stream)).await {
                    Ok(Ok(tls_stream)) => tls_stream,
                    Ok(Err(error)) => {
                        tracing::debug!(%error, "MCP TLS handshake failed");
                        return;
                    }
                    Err(_) => {
                        tracing::debug!("MCP TLS handshake timed out");
                        return;
                    }
                };
            let sni = tls_stream.get_ref().1.server_name().map(str::to_owned);
            let authorities = Arc::new(
                state
                    .authorities
                    .with_tls_session(sni.as_deref(), local_port),
            );
            serve_http_connection(tls_stream, state, authorities, None, stopping).await;
        }
    }
}

async fn serve_http_connection<S>(
    stream: S,
    state: Arc<HttpServingState>,
    authorities: Arc<HttpAuthorities>,
    peer: Option<SocketAddr>,
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
                let authorities = Arc::clone(&authorities);
                async move { Ok::<_, Infallible>(state.answer(request, &authorities, peer).await) }
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
    async fn answer(
        &self,
        mut request: Request<Incoming>,
        authorities: &HttpAuthorities,
        peer: Option<SocketAddr>,
    ) -> Response<BoxBody<Bytes, Infallible>> {
        let effective = if self.tls.trusts_proxy(peer) {
            authorities.with_trusted_forwarded(request.headers())
        } else {
            authorities.clone()
        };

        let headers = request.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        // Any `Origin` header is checked even when it is not valid UTF-8.
        let origin = headers
            .get(header::ORIGIN)
            .map(|value| value.to_str().unwrap_or_default());

        if let Err(reason) = effective.check_headers(host, origin) {
            self.record_refusal(reason).await;
            let message = match reason {
                RefusalReason::HostForeign => "Forbidden: Host header is not admitted",
                _ => "Forbidden: Origin header is not admitted",
            };
            return plain_error(StatusCode::FORBIDDEN, message);
        }

        // RFC 9728 Protected Resource Metadata is served unauthenticated on GET
        // so MCP clients can discover `authorization_servers` before obtaining a token.
        if let HttpAuth::OAuth(oauth) = &self.auth
            && *request.method() == Method::GET
            && oauth.matches_metadata_path(request.uri().path())
        {
            return json_ok(oauth.metadata_json());
        }

        if request.uri().path() != MCP_HTTP_PATH {
            return plain_error(StatusCode::NOT_FOUND, "Not Found");
        }

        if let Some(unauthorized) = self
            .check_auth(request.uri().query(), request.headers())
            .await
        {
            return unauthorized;
        }

        // Enforce no token passthrough (ADRs 0017, 0067; ticket #1367): strip
        // `Authorization` at the HTTP boundary before dispatching to `McpServer`.
        request.headers_mut().remove(header::AUTHORIZATION);

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
                        "Bearer",
                        "a bearer token is required in `Authorization: Bearer`",
                    ))
                }
                Some(_) if query_token => {
                    self.record_refusal(RefusalReason::TokenWrong).await;
                    Some(unauthorized_bearer(
                        "Bearer",
                        "tokens in the query string are refused",
                    ))
                }
                Some(given) if !same_token(given, expected.expose_secret()) => {
                    self.record_refusal(RefusalReason::TokenWrong).await;
                    Some(unauthorized_bearer(
                        "Bearer",
                        "the bearer token was refused",
                    ))
                }
                Some(_) => None,
            },
            HttpAuth::OAuth(oauth) => {
                if query_token {
                    self.record_refusal(RefusalReason::TokenWrong).await;
                    let challenge = oauth.www_authenticate(None);
                    return Some(unauthorized_bearer(
                        &challenge,
                        "tokens in the query string are refused",
                    ));
                }
                let Some(token) = bearer_token(headers) else {
                    self.record_refusal(RefusalReason::TokenMissing).await;
                    let challenge = oauth.www_authenticate(None);
                    return Some(unauthorized_bearer(
                        &challenge,
                        "an OAuth 2.1 bearer token is required in `Authorization: Bearer`",
                    ));
                };
                match oauth.verify_bearer(token).await {
                    Ok(_verified) => None,
                    Err(rejection) => {
                        self.record_refusal(RefusalReason::TokenWrong).await;
                        let challenge = oauth.www_authenticate(Some(&rejection));
                        Some(unauthorized_bearer(&challenge, rejection.description()))
                    }
                }
            }
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

/// Built-in HTTP/1.1 + rustls JSON fetcher for OAuth 2.1 authorization server
/// metadata and JWKS endpoints (no redirects, bounded size and timeout).
#[derive(Clone)]
pub struct HttpOAuthFetcher {
    tls: Arc<ClientConfig>,
}

impl HttpOAuthFetcher {
    /// Creates a fetcher trusting Mozilla's root certificates plus any
    /// `extra_roots` (for Operator-trusted CAs or test authorization server stubs).
    #[must_use]
    pub fn new(extra_roots: &[CertificateDer<'static>]) -> Self {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for cert in extra_roots {
            let _ = roots.add(cert.clone());
        }
        let tls =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("invariant: ring provider supports safe TLS protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
        Self { tls: Arc::new(tls) }
    }

    async fn get_json_inner(&self, raw_url: &str) -> Result<serde_json::Value, String> {
        let parsed =
            Url::parse(raw_url).map_err(|err| format!("invalid URL `{raw_url}`: {err}"))?;
        let use_tls = match parsed.scheme() {
            "https" => true,
            "http" => false,
            other => return Err(format!("unsupported URL scheme `{other}`")),
        };
        let host = parsed
            .host_str()
            .ok_or_else(|| format!("URL `{raw_url}` has no host"))?
            .to_owned();
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| format!("URL `{raw_url}` has no port"))?;
        let path_and_query = match parsed.query() {
            Some(q) => format!("{}?{q}", parsed.path()),
            None => parsed.path().to_owned(),
        };
        let host_header = match parsed.port() {
            Some(explicit) => format!("{host}:{explicit}"),
            None => host.clone(),
        };

        #[expect(
            clippy::disallowed_types,
            reason = "the MCP transports module opens outbound TCP sockets to fetch OAuth 2.1 discovery/JWKS JSON (ADR 0060)"
        )]
        let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|err| format!("connect {host}:{port}: {err}"))?;

        let response = if use_tls {
            let server_name = ServerName::try_from(host.clone())
                .map_err(|err| format!("invalid TLS server name `{host}`: {err}"))?;
            let connector = TlsConnector::from(Arc::clone(&self.tls));
            let tls_stream = connector
                .connect(server_name, tcp)
                .await
                .map_err(|err| format!("TLS handshake with {host}:{port}: {err}"))?;
            send_http1_get(tls_stream, &host_header, &path_and_query).await?
        } else {
            send_http1_get(tcp, &host_header, &path_and_query).await?
        };

        if response.status() != StatusCode::OK {
            return Err(format!("HTTP {} from `{raw_url}`", response.status()));
        }
        let collected = Limited::new(response.into_body(), MAX_OAUTH_METADATA_BYTES)
            .collect()
            .await
            .map_err(|err| format!("reading body from `{raw_url}`: {err}"))?
            .to_bytes();
        serde_json::from_slice(&collected)
            .map_err(|err| format!("parsing JSON from `{raw_url}`: {err}"))
    }
}

impl OAuthMetadataFetcher for HttpOAuthFetcher {
    fn fetch_json(
        &self,
        url: String,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + '_>> {
        Box::pin(async move {
            tokio::time::timeout(OAUTH_FETCH_TIMEOUT, self.get_json_inner(&url))
                .await
                .map_err(|_| format!("timed out fetching `{url}`"))?
        })
    }
}

async fn send_http1_get<S>(
    stream: S,
    host_header: &str,
    path_and_query: &str,
) -> Result<Response<Incoming>, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|err| format!("HTTP/1.1 handshake failed: {err}"))?;
    let request = Request::builder()
        .method(Method::GET)
        .uri(path_and_query)
        .header(header::HOST, host_header)
        .header(header::ACCEPT, "application/json")
        .header(header::CONNECTION, "close")
        .body(Full::new(Bytes::new()))
        .map_err(|err| format!("building GET request: {err}"))?;
    let (res, conn_res) = tokio::join!(sender.send_request(request), conn);
    if let Err(err) = conn_res {
        tracing::debug!(%err, "OAuth discovery HTTP connection closed");
    }
    res.map_err(|err| format!("HTTP GET failed: {err}"))
}

fn format_endpoint_url(scheme: &str, addr: SocketAddr) -> String {
    format!("{scheme}://{addr}{MCP_HTTP_PATH}")
}

fn json_ok(body: Bytes) -> Response<BoxBody<Bytes, Infallible>> {
    let mut response = Response::new(Full::new(body).boxed());
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn unauthorized_bearer(
    www_authenticate: &str,
    message: &str,
) -> Response<BoxBody<Bytes, Infallible>> {
    let mut response = plain_error(StatusCode::UNAUTHORIZED, message);
    let header_value = HeaderValue::from_str(www_authenticate)
        .unwrap_or_else(|_| HeaderValue::from_static("Bearer"));
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, header_value);
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
