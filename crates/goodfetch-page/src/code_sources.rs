//! Two-factor authentication `code_sources` (ADRs 0006, 0060 and 0092;
//! ticket #734): reads one-time codes from configured email or SMS sources
//! (`imap`, `gmail`, `microsoft_graph`, `webhook`, `endpoint`, `twilio`),
//! matches by sender, pattern and time window around the login attempt, marks
//! each matched message used once, and joins the Run's `Mask` as
//! `Reach::OneTimeCode`.
//!
//! Size: one concern — resolving a single `code_source`-bound placeholder
//! through its six built-in transports (`IMAP4rev1`, Gmail, Microsoft Graph,
//! local webhook receiver, generic HTTP endpoint and Twilio) sharing one
//! HTTP/1.1 client, timestamp parser, sender/pattern matcher and one-time-use
//! table.
#![expect(
    clippy::disallowed_types,
    reason = "ADR 0060: code_source connects to external endpoints outside the Simulation and locks short-lived in-memory tables never held across `.await`"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0060: code_source opens external sockets and reads wall-clock timestamps for message windows"
)]

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use data_encoding::{BASE64, BASE64URL_NOPAD};
use goodfetch_engine::CallError;
use goodfetch_engine::run::secret_error;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::secrets::{Purpose, RunSecrets};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::code_sources::{CODE_SOURCES, CodeSource, CodeSourceKind};
use goodfetch_protocol::secrets::{SECRET_UNAVAILABLE, SecretFailure, SecretUnavailableDetails};
use goodfetch_protocol::settings::SettingsLayer;
use jiff::fmt::rfc2822;
use jiff::{SignedDuration, Timestamp};
use regex::Regex;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::time::Instant;
use url::Url;

/// Default code extraction pattern when `code_sources.<name>.pattern` is omitted.
const DEFAULT_CODE_PATTERN: &str = r"\b(\d{4,8})\b";

/// Maximum HTTP response body accepted from a code source endpoint (1 MiB).
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;

/// Maximum webhook request body accepted (256 KiB).
const MAX_WEBHOOK_BODY_BYTES: usize = 256 * 1024;

/// Shared state for all `code_sources` in an engine instance: tracks used
/// message IDs (one-time use) and active loopback webhook listeners.
#[derive(Clone, Default)]
pub(crate) struct CodeSources(Arc<CodeSourcesInner>);

#[derive(Default)]
struct CodeSourcesInner {
    used: Mutex<HashSet<String>>,
    webhooks: Mutex<BTreeMap<String, Arc<WebhookInbox>>>,
}

#[derive(Default)]
struct WebhookInbox {
    messages: Mutex<Vec<WebhookMessage>>,
    notify: Notify,
}

#[derive(Clone)]
struct WebhookMessage {
    path: String,
    id: String,
    from: Option<String>,
    text: String,
    received_at: Timestamp,
}

/// One candidate message fetched from a code source.
struct CandidateMessage {
    id: String,
    from: Option<String>,
    text: String,
    received_at: Option<Timestamp>,
}

/// Everything needed to resolve one `code_source`-bound Secret placeholder.
pub(crate) struct CodeSourceRequest<'a> {
    pub(crate) placeholder: &'a str,
    pub(crate) source_name: &'a str,
    pub(crate) source: &'a CodeSource,
    pub(crate) secrets: &'a RunSecrets,
    pub(crate) run: &'a RunIdentity,
    pub(crate) layer: Option<SettingsLayer>,
}

impl CodeSources {
    /// Ensures any `webhook` code sources with an explicit `listen` address in
    /// `settings` are bound and ready to accept POSTs before a login triggers
    /// a code delivery.
    pub(crate) async fn ensure_webhooks(&self, run: &RunIdentity, settings: &Resolved) {
        let Some(map) = settings.value(&CODE_SOURCES).as_object() else {
            return;
        };
        for value in map.values() {
            if let Ok(source) = CodeSource::read(value)
                && let CodeSourceKind::Webhook {
                    listen: Some(addr), ..
                } = source.kind
            {
                let _ = self.ensure_webhook_listener(run, &addr).await;
            }
        }
    }

    /// Fetches one matching, unused code from `request.source` within its
    /// configured `window`. Returns `Ok(Some(code))` when found (joining the
    /// Run's Mask as `Reach::OneTimeCode`), `Ok(None)` when the window expires
    /// without a matching unused code, or `Err(CallError)` on configuration or
    /// credential resolution failures.
    pub(crate) async fn fetch_code(
        &self,
        request: &CodeSourceRequest<'_>,
    ) -> Result<Option<SecretString>, CallError> {
        let pattern_str = request
            .source
            .pattern
            .as_deref()
            .unwrap_or(DEFAULT_CODE_PATTERN);
        let regex = Regex::new(pattern_str).map_err(|error| {
            code_source_unavailable(
                request.placeholder,
                request.source_name,
                kind_name(&request.source.kind),
                SecretFailure::NotConfigured,
                format!(
                    "fix `code_sources.{}.pattern`: {error}",
                    request.source_name
                ),
            )
        })?;

        let webhook_inbox = if let CodeSourceKind::Webhook {
            listen: Some(addr), ..
        } = &request.source.kind
        {
            Some(
                self.ensure_webhook_listener(request.run, addr)
                    .await
                    .map_err(|error| {
                        code_source_unavailable(
                            request.placeholder,
                            request.source_name,
                            "webhook",
                            SecretFailure::ProviderFailed,
                            format!("webhook listener on `{addr}` failed to bind: {error}"),
                        )
                    })?,
            )
        } else {
            None
        };

        let attempt_started = now_timestamp();
        let window_duration = Duration::from_millis(request.source.window);
        let deadline = Instant::now() + window_duration;
        let poll_interval = Duration::from_millis((request.source.window / 4).clamp(25, 250));

        loop {
            let candidates = self.poll_once(request, webhook_inbox.as_deref()).await?;

            if let Some(code) = self.select_code(
                request.source_name,
                request.source,
                &regex,
                attempt_started,
                candidates,
            ) {
                request
                    .secrets
                    .join_one_time_code(request.placeholder, &code);
                return Ok(Some(code));
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }

            let wait = remaining.min(poll_interval);
            if let Some(inbox) = &webhook_inbox {
                let _ = tokio::time::timeout(wait, inbox.notify.notified()).await;
            } else {
                tokio::time::sleep(wait).await;
            }
        }
    }

    async fn poll_once(
        &self,
        request: &CodeSourceRequest<'_>,
        webhook_inbox: Option<&WebhookInbox>,
    ) -> Result<Vec<CandidateMessage>, CallError> {
        let provider = kind_name(&request.source.kind);
        let result = match &request.source.kind {
            CodeSourceKind::Imap {
                host,
                port,
                username,
                password,
                mailbox,
                tls,
            } => {
                let user = resolve_ref(request, username).await?;
                let pass = resolve_ref(request, password).await?;
                let use_tls = tls.unwrap_or_else(|| !is_loopback_host(host) || *port == 993);
                fetch_imap(
                    host,
                    *port,
                    user.expose_secret(),
                    pass.expose_secret(),
                    mailbox,
                    use_tls,
                )
                .await
            }
            CodeSourceKind::Gmail {
                credentials,
                endpoint,
            } => {
                let creds = resolve_ref(request, credentials).await?;
                let token = extract_bearer_token(creds.expose_secret());
                let base = endpoint
                    .as_deref()
                    .unwrap_or("https://gmail.googleapis.com");
                fetch_gmail(base, &token).await
            }
            CodeSourceKind::MicrosoftGraph {
                tenant,
                client_id,
                client_secret,
                mailbox,
                endpoint,
                authority,
            } => {
                let cid = resolve_ref(request, client_id).await?;
                let csec = resolve_ref(request, client_secret).await?;
                let auth_base = authority
                    .as_deref()
                    .unwrap_or("https://login.microsoftonline.com");
                let graph_base = endpoint.as_deref().unwrap_or("https://graph.microsoft.com");
                fetch_microsoft_graph(
                    auth_base,
                    graph_base,
                    tenant,
                    cid.expose_secret(),
                    csec.expose_secret(),
                    mailbox,
                )
                .await
            }
            CodeSourceKind::Webhook { path, .. } => Ok(read_webhook_inbox(webhook_inbox, path)),
            CodeSourceKind::Endpoint { url, headers } => {
                let mut resolved_headers = Vec::with_capacity(headers.len());
                for (name, reference) in headers {
                    let value = resolve_ref(request, reference).await?;
                    resolved_headers.push((name.clone(), value.expose_secret().to_owned()));
                }
                fetch_endpoint(url, &resolved_headers).await
            }
            CodeSourceKind::Twilio {
                account_sid,
                auth_token,
                to,
                endpoint,
            } => {
                let sid = resolve_ref(request, account_sid).await?;
                let tok = resolve_ref(request, auth_token).await?;
                let base = endpoint.as_deref().unwrap_or("https://api.twilio.com");
                fetch_twilio(base, sid.expose_secret(), tok.expose_secret(), to).await
            }
        };
        result.map_err(|error| {
            code_source_unavailable(
                request.placeholder,
                request.source_name,
                provider,
                SecretFailure::ProviderFailed,
                format!("{provider} fetch failed: {error}"),
            )
        })
    }

    fn select_code(
        &self,
        source_name: &str,
        source: &CodeSource,
        regex: &Regex,
        attempt_started: Timestamp,
        candidates: Vec<CandidateMessage>,
    ) -> Option<SecretString> {
        let window_span =
            SignedDuration::from_millis(i64::try_from(source.window).unwrap_or(i64::MAX));
        let earliest = attempt_started.checked_sub(window_span).ok();
        let latest = attempt_started.checked_add(window_span).ok();

        let mut used = self.0.used.lock().unwrap_or_else(PoisonError::into_inner);
        for candidate in candidates {
            if let Some(received_at) = candidate.received_at {
                if let Some(earliest) = earliest
                    && received_at < earliest
                {
                    continue;
                }
                if let Some(latest) = latest
                    && received_at > latest
                {
                    continue;
                }
            }
            if let Some(expected_from) = &source.from {
                let Some(actual_from) = &candidate.from else {
                    continue;
                };
                if !sender_matches(actual_from, expected_from) {
                    continue;
                }
            }
            let Some(captures) = regex.captures(&candidate.text) else {
                continue;
            };
            let matched = captures
                .get(1)
                .or_else(|| captures.get(0))
                .map(|m| m.as_str())?;
            if matched.is_empty() {
                continue;
            }
            let dedup_key = format!("{source_name}:{}", candidate.id);
            if !used.insert(dedup_key) {
                continue;
            }
            return Some(SecretString::from(matched.to_owned()));
        }
        None
    }

    async fn ensure_webhook_listener(
        &self,
        run: &RunIdentity,
        addr: &str,
    ) -> Result<Arc<WebhookInbox>, String> {
        if let Some(existing) = self
            .0
            .webhooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(addr)
            .cloned()
        {
            return Ok(existing);
        }

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|error| format!("bind `{addr}`: {error}"))?;
        let inbox = Arc::new(WebhookInbox::default());
        {
            let mut map = self
                .0
                .webhooks
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(existing) = map.get(addr).cloned() {
                return Ok(existing);
            }
            map.insert(addr.to_owned(), Arc::clone(&inbox));
        }

        let inbox_task = Arc::clone(&inbox);
        let cancel = run.owner().child_token();
        let inner = Arc::clone(&self.0);
        let addr_owned = addr.to_owned();
        run.spawn(async move {
            let mut seq: u64 = 0;
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else {
                            break;
                        };
                        seq = seq.saturating_add(1);
                        let _ = handle_webhook_connection(stream, &inbox_task, seq).await;
                    }
                }
            }
            inner
                .webhooks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&addr_owned);
        });
        Ok(inbox)
    }
}

fn read_webhook_inbox(webhook_inbox: Option<&WebhookInbox>, path: &str) -> Vec<CandidateMessage> {
    let Some(inbox) = webhook_inbox else {
        return Vec::new();
    };
    let messages = inbox
        .messages
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    messages
        .iter()
        .filter(|msg| msg.path == path)
        .map(|msg| CandidateMessage {
            id: msg.id.clone(),
            from: msg.from.clone(),
            text: msg.text.clone(),
            received_at: Some(msg.received_at),
        })
        .collect()
}

/// Builds a `secret_unavailable` [`CallError`] for a `code_source` failure.
pub(crate) fn code_source_unavailable(
    placeholder: &str,
    source_name: &str,
    provider: &str,
    failure: SecretFailure,
    fix: impl Into<String>,
) -> CallError {
    CallError::new(
        &SECRET_UNAVAILABLE,
        format!(
            "no one-time code for Secret placeholder `{placeholder}` from code source `{source_name}`"
        ),
        &SecretUnavailableDetails {
            placeholder: placeholder.to_owned(),
            reference: format!("code_source:{source_name}"),
            provider: provider.to_owned(),
            failure,
            fix: fix.into(),
        },
    )
}

pub(crate) const fn kind_name(kind: &CodeSourceKind) -> &'static str {
    match kind {
        CodeSourceKind::Imap { .. } => "imap",
        CodeSourceKind::Gmail { .. } => "gmail",
        CodeSourceKind::MicrosoftGraph { .. } => "microsoft_graph",
        CodeSourceKind::Webhook { .. } => "webhook",
        CodeSourceKind::Endpoint { .. } => "endpoint",
        CodeSourceKind::Twilio { .. } => "twilio",
    }
}

async fn resolve_ref(
    request: &CodeSourceRequest<'_>,
    reference_text: &str,
) -> Result<SecretString, CallError> {
    let reference = SecretReference::parse(reference_text).map_err(|error| {
        code_source_unavailable(
            request.placeholder,
            request.source_name,
            kind_name(&request.source.kind),
            SecretFailure::NotConfigured,
            format!("invalid Secret reference `{reference_text}`: {error}"),
        )
    })?;
    let purpose = Purpose {
        placeholder: request.placeholder,
        domain: None,
        domains: &[],
        layer: request.layer,
        run_id: Some(request.run.id()),
    };
    request
        .secrets
        .resolve(&reference, &purpose)
        .await
        .map_err(|error| secret_error(&error))
}

fn now_timestamp() -> Timestamp {
    Timestamp::try_from(SystemTime::now()).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn sender_matches(actual: &str, expected: &str) -> bool {
    actual
        .to_ascii_lowercase()
        .contains(&expected.to_ascii_lowercase())
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

fn extract_bearer_token(raw: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw)
        && let Some(token) = map.get("access_token").and_then(Value::as_str)
    {
        return token.to_owned();
    }
    raw.trim().to_owned()
}

// ---------------------------------------------------------------------------
// Webhook HTTP receiver
// ---------------------------------------------------------------------------

async fn handle_webhook_connection(
    mut stream: TcpStream,
    inbox: &WebhookInbox,
    seq: u64,
) -> Result<(), String> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read webhook: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > MAX_WEBHOOK_BODY_BYTES {
            return Ok(());
        }
    };

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    let _ = req
        .parse(&buf[..header_end])
        .map_err(|e| format!("parse webhook request: {e}"))?;
    let raw_path = req.path.unwrap_or("/");
    let path = raw_path.split('?').next().unwrap_or(raw_path).to_owned();

    let mut content_length = 0usize;
    let mut header_from: Option<String> = None;
    let mut header_id: Option<String> = None;
    for h in req.headers {
        if h.name.eq_ignore_ascii_case("content-length") {
            if let Ok(s) = std::str::from_utf8(h.value) {
                content_length = s.trim().parse().unwrap_or(0);
            }
        } else if h.name.eq_ignore_ascii_case("x-from") || h.name.eq_ignore_ascii_case("from") {
            if let Ok(s) = std::str::from_utf8(h.value) {
                header_from = Some(s.trim().to_owned());
            }
        } else if h.name.eq_ignore_ascii_case("x-message-id")
            && let Ok(s) = std::str::from_utf8(h.value)
        {
            header_id = Some(s.trim().to_owned());
        }
    }

    let content_length = content_length.min(MAX_WEBHOOK_BODY_BYTES);
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read webhook body: {e}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    let body_text = String::from_utf8_lossy(&body).into_owned();
    let msg = parse_webhook_message(&path, &body_text, header_from, header_id, seq);
    {
        let mut guard = inbox
            .messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.push(msg);
    }
    inbox.notify.notify_waiters();

    let _ = stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
        .await;
    Ok(())
}

fn parse_webhook_message(
    path: &str,
    body_text: &str,
    header_from: Option<String>,
    header_id: Option<String>,
    seq: u64,
) -> WebhookMessage {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body_text) {
        let id = map
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(header_id)
            .unwrap_or_else(|| format!("webhook-{seq}"));
        let from = map
            .get("from")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(header_from);
        let text = collect_text_fields(&map);
        let received_at = map
            .get("received_at")
            .or_else(|| map.get("timestamp"))
            .and_then(parse_json_timestamp)
            .unwrap_or_else(now_timestamp);
        return WebhookMessage {
            path: path.to_owned(),
            id,
            from,
            text,
            received_at,
        };
    }
    WebhookMessage {
        path: path.to_owned(),
        id: header_id.unwrap_or_else(|| format!("webhook-{seq}")),
        from: header_from,
        text: body_text.to_owned(),
        received_at: now_timestamp(),
    }
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 client for Gmail, Microsoft Graph, Endpoint, Twilio
// ---------------------------------------------------------------------------

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

async fn http_request(
    method: &str,
    raw_url: &str,
    headers: &[(String, String)],
    body: Option<(&str, &[u8])>,
) -> Result<HttpResponse, String> {
    let url = Url::parse(raw_url).map_err(|e| format!("invalid URL `{raw_url}`: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("URL `{raw_url}` has no host"))?;
    let is_https = match url.scheme() {
        "https" => true,
        "http" => false,
        other => return Err(format!("unsupported URL scheme `{other}`")),
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| format!("URL `{raw_url}` has no port"))?;

    let mut path_and_query = url.path().to_owned();
    if let Some(query) = url.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    let mut req_head = String::new();
    let _ = write!(req_head, "{method} {path_and_query} HTTP/1.1\r\n");
    if url.port().is_some() {
        let _ = write!(req_head, "Host: {host}:{port}\r\n");
    } else {
        let _ = write!(req_head, "Host: {host}\r\n");
    }
    req_head.push_str("Connection: close\r\n");
    for (name, value) in headers {
        let _ = write!(req_head, "{name}: {value}\r\n");
    }
    if let Some((content_type, bytes)) = body {
        let _ = write!(req_head, "Content-Type: {content_type}\r\n");
        let _ = write!(req_head, "Content-Length: {}\r\n", bytes.len());
    }
    req_head.push_str("\r\n");

    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("connect `{host}:{port}`: {e}"))?;

    if is_https {
        let tls_stream = wrap_tls(tcp, host).await?;
        send_and_read_http(tls_stream, req_head.as_bytes(), body.map(|(_, b)| b)).await
    } else {
        send_and_read_http(tcp, req_head.as_bytes(), body.map(|(_, b)| b)).await
    }
}

async fn wrap_tls(
    tcp: TcpStream,
    host: &str,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|e| format!("invalid TLS server name `{host}`: {e}"))?;
    connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| format!("TLS handshake with `{host}`: {e}"))
}

async fn send_and_read_http<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    head: &[u8],
    body: Option<&[u8]>,
) -> Result<HttpResponse, String> {
    stream
        .write_all(head)
        .await
        .map_err(|e| format!("write request head: {e}"))?;
    if let Some(bytes) = body {
        stream
            .write_all(bytes)
            .await
            .map_err(|e| format!("write request body: {e}"))?;
    }
    stream
        .flush()
        .await
        .map_err(|e| format!("flush request: {e}"))?;

    let mut raw = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("read response: {e}"))?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);
        if raw.len() > MAX_HTTP_BODY_BYTES + 16_384 {
            return Err("HTTP response exceeded maximum size".to_owned());
        }
    }

    let header_end = find_header_end(&raw)
        .ok_or_else(|| "malformed HTTP response (no header end)".to_owned())?;
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut headers);
    resp.parse(&raw[..header_end])
        .map_err(|e| format!("parse HTTP response: {e}"))?;
    let status = resp.code.unwrap_or(0);
    let is_chunked = resp.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("transfer-encoding")
            && std::str::from_utf8(h.value)
                .is_ok_and(|v| v.to_ascii_lowercase().contains("chunked"))
    });
    let raw_body = &raw[header_end..];
    let decoded = if is_chunked {
        decode_chunked(raw_body)?
    } else {
        raw_body.to_vec()
    };
    Ok(HttpResponse {
        status,
        body: decoded,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while let Some(pos) = input.windows(2).position(|w| w == b"\r\n") {
        let size_str = std::str::from_utf8(&input[..pos])
            .map_err(|e| format!("invalid chunk size: {e}"))?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|e| format!("invalid hex chunk size `{size_str}`: {e}"))?;
        input = &input[pos + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size {
            return Err("truncated chunked body".to_owned());
        }
        out.extend_from_slice(&input[..size]);
        input = &input[size..];
        if input.starts_with(b"\r\n") {
            input = &input[2..];
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 1. IMAP4rev1 fetcher
// ---------------------------------------------------------------------------

async fn fetch_imap(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    mailbox: &str,
    use_tls: bool,
) -> Result<Vec<CandidateMessage>, String> {
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("connect: {e}"))?;
    if use_tls {
        let tls = wrap_tls(tcp, host).await?;
        run_imap_session(tls, username, password, mailbox).await
    } else {
        run_imap_session(tcp, username, password, mailbox).await
    }
}

async fn run_imap_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    username: &str,
    password: &str,
    mailbox: &str,
) -> Result<Vec<CandidateMessage>, String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // Read greeting (* OK ...)
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("read greeting: {e}"))?;

    imap_command(
        &mut reader,
        "A1",
        &format!(
            "LOGIN \"{}\" \"{}\"",
            imap_escape(username),
            imap_escape(password)
        ),
    )
    .await?;
    imap_command(
        &mut reader,
        "A2",
        &format!("EXAMINE \"{}\"", imap_escape(mailbox)),
    )
    .await?;
    let search_lines = imap_command(&mut reader, "A3", "UID SEARCH ALL").await?;

    let mut uids: Vec<String> = Vec::new();
    for resp in &search_lines {
        if let Some(rest) = resp.strip_prefix("* SEARCH") {
            for token in rest.split_whitespace() {
                if !token.is_empty() {
                    uids.push(token.to_owned());
                }
            }
        }
    }
    // Check newest messages first, bounded to the 20 most recent.
    uids.reverse();
    uids.truncate(20);

    let mut messages = Vec::new();
    for (idx, uid) in uids.iter().enumerate() {
        let tag = format!("F{}", idx + 1);
        let raw_lines = imap_command(
            &mut reader,
            &tag,
            &format!("UID FETCH {uid} (INTERNALDATE BODY.PEEK[])"),
        )
        .await?;
        let joined = raw_lines.join("\n");
        let from = extract_email_header(&joined, "From:");
        let date = extract_email_header(&joined, "Date:")
            .and_then(|d| parse_timestamp_str(&d))
            .or_else(|| extract_imap_internaldate(&joined));
        messages.push(CandidateMessage {
            id: format!("imap-{mailbox}-{uid}"),
            from,
            text: joined,
            received_at: date,
        });
    }

    let _ = imap_command(&mut reader, "A9", "LOGOUT").await;
    Ok(messages)
}

async fn imap_command<S: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<S>,
    tag: &str,
    command: &str,
) -> Result<Vec<String>, String> {
    let wire = format!("{tag} {command}\r\n");
    reader
        .get_mut()
        .write_all(wire.as_bytes())
        .await
        .map_err(|e| format!("write `{tag}`: {e}"))?;
    reader
        .get_mut()
        .flush()
        .await
        .map_err(|e| format!("flush `{tag}`: {e}"))?;

    let ok_prefix = format!("{tag} OK");
    let no_prefix = format!("{tag} NO");
    let bad_prefix = format!("{tag} BAD");
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read `{tag}`: {e}"))?;
        if n == 0 {
            return Err(format!("unexpected EOF waiting for `{tag}`"));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']).to_owned();
        if trimmed.starts_with(&ok_prefix) {
            return Ok(lines);
        }
        if trimmed.starts_with(&no_prefix) || trimmed.starts_with(&bad_prefix) {
            return Err(format!("IMAP command rejected: {trimmed}"));
        }
        lines.push(trimmed);
    }
}

fn imap_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn extract_email_header(text: &str, prefix: &str) -> Option<String> {
    let lower = prefix.to_ascii_lowercase();
    for line in text.lines() {
        if line.len() >= prefix.len() && line[..prefix.len()].to_ascii_lowercase() == lower {
            return Some(line[prefix.len()..].trim().to_owned());
        }
    }
    None
}

fn extract_imap_internaldate(text: &str) -> Option<Timestamp> {
    let marker = "INTERNALDATE \"";
    let start = text.find(marker)? + marker.len();
    let end = text[start..].find('"')? + start;
    parse_timestamp_str(&text[start..end])
}

// ---------------------------------------------------------------------------
// 2. Gmail API fetcher
// ---------------------------------------------------------------------------

async fn fetch_gmail(base: &str, token: &str) -> Result<Vec<CandidateMessage>, String> {
    let base = base.trim_end_matches('/');
    let auth = vec![("Authorization".to_owned(), format!("Bearer {token}"))];
    let list_url = format!("{base}/gmail/v1/users/me/messages?maxResults=10");
    let list_resp = http_request("GET", &list_url, &auth, None).await?;
    if list_resp.status != 200 {
        return Err(format!("list messages returned HTTP {}", list_resp.status));
    }
    let list_json: Value =
        serde_json::from_slice(&list_resp.body).map_err(|e| format!("parse list JSON: {e}"))?;
    let Some(items) = list_json.get("messages").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for item in items.iter().take(10) {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let msg_url = format!("{base}/gmail/v1/users/me/messages/{id}?format=full");
        let msg_resp = http_request("GET", &msg_url, &auth, None).await?;
        if msg_resp.status != 200 {
            continue;
        }
        let Ok(msg_json) = serde_json::from_slice::<Value>(&msg_resp.body) else {
            continue;
        };
        let from = msg_json
            .pointer("/payload/headers")
            .and_then(Value::as_array)
            .and_then(|headers| {
                headers.iter().find_map(|h| {
                    let name = h.get("name").and_then(Value::as_str)?;
                    if name.eq_ignore_ascii_case("from") {
                        h.get("value").and_then(Value::as_str).map(str::to_owned)
                    } else {
                        None
                    }
                })
            });
        let received_at = msg_json
            .get("internalDate")
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| v.as_i64())
            })
            .and_then(|ms| Timestamp::from_millisecond(ms).ok());
        let mut text = String::new();
        if let Some(snippet) = msg_json.get("snippet").and_then(Value::as_str) {
            text.push_str(snippet);
            text.push('\n');
        }
        if let Some(data) = msg_json
            .pointer("/payload/body/data")
            .and_then(Value::as_str)
            && let Ok(bytes) = BASE64URL_NOPAD.decode(data.as_bytes())
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
        out.push(CandidateMessage {
            id: id.to_owned(),
            from,
            text,
            received_at,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 3. Microsoft Graph API fetcher
// ---------------------------------------------------------------------------

async fn fetch_microsoft_graph(
    authority: &str,
    endpoint: &str,
    tenant: &str,
    client_id: &str,
    client_secret: &str,
    mailbox: &str,
) -> Result<Vec<CandidateMessage>, String> {
    let authority = authority.trim_end_matches('/');
    let token_url = format!("{authority}/{tenant}/oauth2/v2.0/token");
    let form_body = format!(
        "grant_type=client_credentials&client_id={}&client_secret={}&scope=https%3A%2F%2Fgraph.microsoft.com%2F.default",
        percent_encode(client_id),
        percent_encode(client_secret)
    );
    let token_resp = http_request(
        "POST",
        &token_url,
        &[],
        Some(("application/x-www-form-urlencoded", form_body.as_bytes())),
    )
    .await?;
    if token_resp.status != 200 {
        return Err(format!(
            "token endpoint returned HTTP {}",
            token_resp.status
        ));
    }
    let token_json: Value =
        serde_json::from_slice(&token_resp.body).map_err(|e| format!("parse token JSON: {e}"))?;
    let access_token = token_json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "token response missing `access_token`".to_owned())?;

    let endpoint = endpoint.trim_end_matches('/');
    let messages_url = format!(
        "{endpoint}/v1.0/users/{}/messages?$top=10&$orderby=receivedDateTime%20desc",
        percent_encode(mailbox)
    );
    let auth = vec![("Authorization".to_owned(), format!("Bearer {access_token}"))];
    let resp = http_request("GET", &messages_url, &auth, None).await?;
    if resp.status != 200 {
        return Err(format!("messages endpoint returned HTTP {}", resp.status));
    }
    let json: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("parse Graph JSON: {e}"))?;
    let Some(items) = json.get("value").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let from = item
            .pointer("/from/emailAddress/address")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let received_at = item
            .get("receivedDateTime")
            .and_then(Value::as_str)
            .and_then(parse_timestamp_str);
        let mut text = String::new();
        if let Some(subject) = item.get("subject").and_then(Value::as_str) {
            text.push_str(subject);
            text.push('\n');
        }
        if let Some(preview) = item.get("bodyPreview").and_then(Value::as_str) {
            text.push_str(preview);
            text.push('\n');
        }
        if let Some(content) = item.pointer("/body/content").and_then(Value::as_str) {
            text.push_str(content);
        }
        out.push(CandidateMessage {
            id: id.to_owned(),
            from,
            text,
            received_at,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 4. Generic HTTP Endpoint fetcher
// ---------------------------------------------------------------------------

async fn fetch_endpoint(
    url: &str,
    headers: &[(String, String)],
) -> Result<Vec<CandidateMessage>, String> {
    let resp = http_request("GET", url, headers, None).await?;
    if resp.status != 200 {
        return Err(format!("HTTP {}", resp.status));
    }
    let json: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("parse endpoint JSON: {e}"))?;
    let items: Vec<&Value> = if let Some(arr) = json.get("messages").and_then(Value::as_array) {
        arr.iter().collect()
    } else if let Some(arr) = json.as_array() {
        arr.iter().collect()
    } else if json.is_object() {
        vec![&json]
    } else {
        Vec::new()
    };

    let mut out = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let Some(map) = item.as_object() else {
            continue;
        };
        let from = map.get("from").and_then(Value::as_str).map(str::to_owned);
        let text = collect_text_fields(map);
        let received_at = map
            .get("received_at")
            .or_else(|| map.get("timestamp"))
            .and_then(parse_json_timestamp);
        let id = map
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| format!("endpoint-{index}-{text}"), str::to_owned);
        out.push(CandidateMessage {
            id,
            from,
            text,
            received_at,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 5. Twilio SMS Messages API fetcher
// ---------------------------------------------------------------------------

async fn fetch_twilio(
    base: &str,
    account_sid: &str,
    auth_token: &str,
    to: &str,
) -> Result<Vec<CandidateMessage>, String> {
    let base = base.trim_end_matches('/');
    let url = format!(
        "{base}/2010-04-01/Accounts/{}/Messages.json?To={}&PageSize=10",
        percent_encode(account_sid),
        percent_encode(to)
    );
    let basic = BASE64.encode(format!("{account_sid}:{auth_token}").as_bytes());
    let headers = vec![("Authorization".to_owned(), format!("Basic {basic}"))];
    let resp = http_request("GET", &url, &headers, None).await?;
    if resp.status != 200 {
        return Err(format!("Twilio returned HTTP {}", resp.status));
    }
    let json: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("parse Twilio JSON: {e}"))?;
    let Some(items) = json.get("messages").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for item in items {
        let Some(sid) = item.get("sid").and_then(Value::as_str) else {
            continue;
        };
        let from = item.get("from").and_then(Value::as_str).map(str::to_owned);
        let body = item
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let received_at = item
            .get("date_sent")
            .or_else(|| item.get("date_created"))
            .and_then(Value::as_str)
            .and_then(parse_timestamp_str);
        out.push(CandidateMessage {
            id: sid.to_owned(),
            from,
            text: body,
            received_at,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Shared parsing helpers
// ---------------------------------------------------------------------------

fn collect_text_fields(map: &serde_json::Map<String, Value>) -> String {
    let mut text = String::new();
    for key in ["subject", "text", "body", "code"] {
        if let Some(s) = map.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(s);
        }
    }
    text
}

fn parse_json_timestamp(value: &Value) -> Option<Timestamp> {
    if let Some(s) = value.as_str() {
        return parse_timestamp_str(s);
    }
    if let Some(n) = value.as_i64() {
        if n > 10_000_000_000 {
            return Timestamp::from_millisecond(n).ok();
        }
        return Timestamp::from_second(n).ok();
    }
    None
}

fn parse_timestamp_str(s: &str) -> Option<Timestamp> {
    let s = s.trim();
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Some(ts);
    }
    if let Ok(zdt) = rfc2822::parse(s) {
        return Some(zdt.timestamp());
    }
    None
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}
