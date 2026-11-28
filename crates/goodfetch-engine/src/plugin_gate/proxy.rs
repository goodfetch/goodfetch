//! Per-instance `proxy_tcp` HTTP CONNECT / SOCKS5 proxy (ADR 0033, issue 833).
//!
//! Each `proxy_tcp` Plugin instance gets fresh OS-random credentials so no
//! other local process can use its proxy, and every requested destination is
//! checked through [`PluginFetch::connect`] (the exact same endpoint/target
//! allowlist and SSRF egress checks as `net.connect`). Each stream's bytes and
//! duration are accounted once when the stream closes via [`StreamAccountant`].
#![expect(
    clippy::disallowed_types,
    reason = "the `proxy_tcp` proxy accepts loopback TCP and Linux SCM_RIGHTS Unix streams behind the Plugin egress gate (ADRs 0033 and 0060)"
)]

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use url::{Origin, Url};

use crate::call_error::CallError;
use crate::plugin_gate::PluginFetch;
use crate::tasks::{TaskOwner, TaskStarter};

/// Maximum bytes read for an HTTP proxy request header block.
const MAX_HTTP_HEAD: usize = 16 * 1024;

/// Which transport carried a Plugin's opaque stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamTransport {
    /// Opened directly via `net.connect` / `net.stream.*` on stdio.
    NetConnect,
    /// Opened through the per-instance `proxy_tcp` HTTP/SOCKS5 proxy.
    ProxyTcp,
}

/// Completed stream record reported once when a stream closes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamRecord {
    /// The Plugin's name.
    pub plugin: String,
    /// Destination formatted as `host:port`.
    pub destination: String,
    /// Which transport carried the stream.
    pub transport: StreamTransport,
    /// Total bytes sent and received over the stream.
    pub bytes: u64,
    /// Total time the stream stayed open, in milliseconds.
    pub duration_ms: u64,
}

/// Records completed Plugin stream Usage (`plugin_stream_bytes` and
/// `plugin_stream_time`) and Run trace entries without coupling `plugin_gate`
/// or `plugin_host` to `ledger` or `run_trace` (ADR 0140).
pub trait StreamAccountant: fmt::Debug + Send + Sync {
    /// Records one completed stream once it closes.
    fn record(&self, record: StreamRecord) -> BoxFuture<'static, ()>;
}

/// Fresh per-instance proxy credentials generated from OS randomness.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyCredentials {
    /// Per-instance username (hex).
    pub username: String,
    /// Per-instance password (hex).
    pub password: String,
}

impl fmt::Debug for ProxyCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProxyCredentials")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl ProxyCredentials {
    /// Generates fresh random credentials for one Plugin instance from the OS
    /// generator (ADR 0033).
    ///
    /// # Errors
    ///
    /// When the OS random generator fails.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut raw = [0_u8; 32];
        getrandom::fill(&mut raw)?;
        Ok(Self {
            username: data_encoding::HEXLOWER.encode(&raw[..16]),
            password: data_encoding::HEXLOWER.encode(&raw[16..]),
        })
    }

    /// Formats an `http://user:pass@host:port` proxy URL.
    #[must_use]
    pub fn http_url(&self, addr: SocketAddr) -> String {
        format!("http://{}:{}@{addr}", self.username, self.password)
    }

    /// Formats a `socks5h://user:pass@host:port` proxy URL.
    #[must_use]
    pub fn socks5_url(&self, addr: SocketAddr) -> String {
        format!("socks5h://{}:{}@{addr}", self.username, self.password)
    }

    /// Constant-time credential verification.
    fn verify(&self, user: &str, pass: &str) -> bool {
        constant_time_eq(self.username.as_bytes(), user.as_bytes())
            && constant_time_eq(self.password.as_bytes(), pass.as_bytes())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Optional sink for observing the exact [`CallError`] when the proxy refuses
/// a destination (used by the SSRF table runner to assert the exact
/// `egress_refused` rule through the `proxy_tcp` path).
pub trait RefusalObserver: fmt::Debug + Send + Sync {
    /// Called whenever a proxy connect destination is refused.
    fn observed(&self, error: &CallError);
}

/// Per-instance proxy state shared across accepted client streams.
#[derive(Clone, Debug)]
struct Shared {
    plugin: String,
    fetch: Arc<PluginFetch>,
    credentials: ProxyCredentials,
    accountant: Option<Arc<dyn StreamAccountant>>,
    observer: Option<Arc<dyn RefusalObserver>>,
    open_streams: Arc<AtomicUsize>,
    stopping: CancellationToken,
}

/// Per-instance `proxy_tcp` proxy running on the engine side (ADR 0033).
#[derive(Debug)]
pub struct PluginProxy {
    shared: Arc<Shared>,
    local_addr: Option<SocketAddr>,
}

impl PluginProxy {
    /// Starts a loopback TCP proxy for `plugin` on `127.0.0.1:0`, running on
    /// `owner`. Used by tests and non-Linux hosts where the confined process
    /// connects directly to the engine's loopback port.
    ///
    /// # Errors
    ///
    /// When generating credentials or binding `127.0.0.1:0` fails.
    pub async fn start_loopback(
        owner: &TaskOwner,
        plugin: impl Into<String>,
        fetch: Arc<PluginFetch>,
        accountant: Option<Arc<dyn StreamAccountant>>,
        observer: Option<Arc<dyn RefusalObserver>>,
    ) -> io::Result<Self> {
        let credentials = ProxyCredentials::generate().map_err(io::Error::other)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let local_addr = listener.local_addr()?;
        let stopping = owner.token().child_token();
        let shared = Arc::new(Shared {
            plugin: plugin.into(),
            fetch,
            credentials,
            accountant,
            observer,
            open_streams: Arc::new(AtomicUsize::new(0)),
            stopping: stopping.clone(),
        });
        let task_shared = Arc::clone(&shared);
        let task_owner = TaskStarter::clone(owner);
        owner.spawn(async move {
            loop {
                let accepted = stopping.run_until_cancelled(listener.accept()).await;
                let Some(Ok((stream, _))) = accepted else {
                    break;
                };
                let conn_shared = Arc::clone(&task_shared);
                task_owner.spawn(async move {
                    serve_stream(conn_shared, stream, None).await;
                });
            }
        });
        Ok(Self {
            shared,
            local_addr: Some(local_addr),
        })
    }

    /// Starts the Linux `SCM_RIGHTS` server side of the `proxy_tcp` relay
    /// (ADR 0033): returns the proxy handle and the child-side `OwnedFd` to
    /// pass to `plugin-shim` as FD 3.
    ///
    /// # Errors
    ///
    /// When generating credentials or creating the `AF_UNIX` control socket
    /// pair fails.
    #[cfg(target_os = "linux")]
    pub fn start_linux_relay(
        owner: &TaskOwner,
        plugin: impl Into<String>,
        fetch: Arc<PluginFetch>,
        accountant: Option<Arc<dyn StreamAccountant>>,
    ) -> io::Result<(Self, std::os::unix::io::OwnedFd)> {
        use std::os::fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _, OwnedFd};
        use std::os::unix::net::UnixStream as StdUnixStream;

        let credentials = ProxyCredentials::generate().map_err(io::Error::other)?;
        let (engine_ctrl, child_ctrl) = StdUnixStream::pair()?;
        engine_ctrl.set_nonblocking(true)?;
        let engine_ctrl = tokio::net::UnixStream::from_std(engine_ctrl)?;
        let stopping = owner.token().child_token();
        let shared = Arc::new(Shared {
            plugin: plugin.into(),
            fetch,
            credentials,
            accountant,
            observer: None,
            open_streams: Arc::new(AtomicUsize::new(0)),
            stopping: stopping.clone(),
        });
        let task_shared = Arc::clone(&shared);
        let task_owner = TaskStarter::clone(owner);
        owner.spawn(async move {
            let mut ctrl = engine_ctrl;
            let mut req = [0_u8; 1];
            loop {
                let read = stopping
                    .run_until_cancelled(ctrl.read_exact(&mut req))
                    .await;
                let Some(Ok(_)) = read else {
                    break;
                };
                if req[0] != goodfetch_confinement::linux::relay::CONNECT_REQUEST {
                    break;
                }
                let Ok((engine_side, relay_side)) = StdUnixStream::pair() else {
                    break;
                };
                if engine_side.set_nonblocking(true).is_err() {
                    break;
                }
                // Wait until the control socket is writable, then send the
                // relay half's descriptor via SCM_RIGHTS.
                let writable = stopping.run_until_cancelled(ctrl.writable()).await;
                let Some(Ok(())) = writable else {
                    break;
                };
                let sent = ctrl.try_io(tokio::io::Interest::WRITABLE, || {
                    goodfetch_confinement::linux::relay::send_fd(
                        ctrl.as_raw_fd(),
                        relay_side.as_raw_fd(),
                    )
                });
                drop(relay_side);
                if sent.is_err() {
                    break;
                }
                let Ok(stream) = tokio::net::UnixStream::from_std(engine_side) else {
                    break;
                };
                let conn_shared = Arc::clone(&task_shared);
                task_owner.spawn(async move {
                    serve_stream(conn_shared, stream, None).await;
                });
            }
        });
        #[expect(
            unsafe_code,
            reason = "converting a fresh UnixStream into OwnedFd to inherit across exec"
        )]
        // SAFETY: `child_ctrl` is a freshly created valid socket owned here.
        let child_fd = unsafe { OwnedFd::from_raw_fd(child_ctrl.into_raw_fd()) };
        Ok((
            Self {
                shared,
                local_addr: None,
            },
            child_fd,
        ))
    }

    /// The per-instance credentials required by this proxy.
    #[must_use]
    pub fn credentials(&self) -> &ProxyCredentials {
        &self.shared.credentials
    }

    /// The bound loopback address when started with [`Self::start_loopback`].
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// Current number of active open proxied streams on this instance.
    #[must_use]
    pub fn open_streams(&self) -> usize {
        self.shared.open_streams.load(Ordering::Relaxed)
    }

    /// Stops accepting new connections and closes active proxied streams.
    pub fn stop(&self) {
        self.shared.stopping.cancel();
    }
}

impl Drop for PluginProxy {
    fn drop(&mut self) {
        self.shared.stopping.cancel();
    }
}

/// Guard decrementing `open_streams` when a proxied stream finishes.
struct OpenStreamGuard(Arc<AtomicUsize>);

impl OpenStreamGuard {
    fn enter(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(counter))
    }
}

impl Drop for OpenStreamGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Auto-detects SOCKS5 (`0x05`) vs HTTP proxy protocol on the first byte and
/// serves one client stream.
async fn serve_stream<S>(shared: Arc<Shared>, mut stream: S, fetch_target: Option<Origin>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut first = [0_u8; 1];
    if stream.read_exact(&mut first).await.is_err() {
        return;
    }
    if first[0] == 0x05 {
        serve_socks5(&shared, stream, fetch_target.as_ref()).await;
    } else {
        serve_http(&shared, first[0], stream, fetch_target.as_ref()).await;
    }
}

/// Performs the SOCKS5 greeting and RFC 1929 username/password handshake.
async fn authenticate_socks5<S: AsyncRead + AsyncWrite + Unpin>(
    credentials: &ProxyCredentials,
    stream: &mut S,
) -> bool {
    let mut nmethods = [0_u8; 1];
    if stream.read_exact(&mut nmethods).await.is_err() {
        return false;
    }
    let mut methods = vec![0_u8; usize::from(nmethods[0])];
    if stream.read_exact(&mut methods).await.is_err() {
        return false;
    }
    // Require username/password auth (method 0x02); never allow no-auth (0x00).
    if !methods.contains(&0x02) {
        let _ = stream.write_all(&[0x05, 0xff]).await;
        return false;
    }
    if stream.write_all(&[0x05, 0x02]).await.is_err() {
        return false;
    }
    // RFC 1929 subnegotiation: VER(1) ULEN(1) UNAME PLEN(1) PASSWD
    let mut ver_ulen = [0_u8; 2];
    if stream.read_exact(&mut ver_ulen).await.is_err() || ver_ulen[0] != 0x01 {
        let _ = stream.write_all(&[0x01, 0x01]).await;
        return false;
    }
    let mut user = vec![0_u8; usize::from(ver_ulen[1])];
    if stream.read_exact(&mut user).await.is_err() {
        return false;
    }
    let mut plen = [0_u8; 1];
    if stream.read_exact(&mut plen).await.is_err() {
        return false;
    }
    let mut pass = vec![0_u8; usize::from(plen[0])];
    if stream.read_exact(&mut pass).await.is_err() {
        return false;
    }
    let auth_ok = std::str::from_utf8(&user)
        .ok()
        .zip(std::str::from_utf8(&pass).ok())
        .is_some_and(|(u, p)| credentials.verify(u, p));
    if !auth_ok {
        let _ = stream.write_all(&[0x01, 0x01]).await;
        return false;
    }
    stream.write_all(&[0x01, 0x00]).await.is_ok()
}

/// Reads the SOCKS5 `CONNECT` request target `(host, port)`.
async fn read_socks5_target<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Option<(String, u16)> {
    // Request: VER(1) CMD(1) RSV(1) ATYP(1) DST.ADDR DST.PORT(2)
    let mut head = [0_u8; 4];
    if stream.read_exact(&mut head).await.is_err() || head[0] != 0x05 || head[1] != 0x01 {
        let _ = stream
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await;
        return None;
    }
    let host = match head[3] {
        0x01 => {
            let mut octets = [0_u8; 4];
            stream.read_exact(&mut octets).await.ok()?;
            std::net::Ipv4Addr::from(octets).to_string()
        }
        0x03 => {
            let mut len = [0_u8; 1];
            stream.read_exact(&mut len).await.ok()?;
            let mut name = vec![0_u8; usize::from(len[0])];
            stream.read_exact(&mut name).await.ok()?;
            let Ok(name) = String::from_utf8(name) else {
                let _ = stream
                    .write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await;
                return None;
            };
            name
        }
        0x04 => {
            let mut octets = [0_u8; 16];
            stream.read_exact(&mut octets).await.ok()?;
            std::net::Ipv6Addr::from(octets).to_string()
        }
        _ => {
            let _ = stream
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return None;
        }
    };
    let mut port_bytes = [0_u8; 2];
    stream.read_exact(&mut port_bytes).await.ok()?;
    Some((host, u16::from_be_bytes(port_bytes)))
}

/// Serves one SOCKS5 connection (RFC 1928 + RFC 1929 username/password auth).
async fn serve_socks5<S>(shared: &Shared, mut stream: S, fetch_target: Option<&Origin>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    if !authenticate_socks5(&shared.credentials, &mut stream).await {
        return;
    }
    let Some((host, port)) = read_socks5_target(&mut stream).await else {
        return;
    };
    let upstream = match shared.fetch.connect(&host, port, fetch_target).await {
        Ok(upstream) => upstream,
        Err(error) => {
            if let Some(observer) = &shared.observer {
                observer.observed(&error);
            }
            // 0x02 = connection not allowed by ruleset; 0x05 = connection refused.
            let rep = if error.code() == "plugin_fetch_failed" {
                0x05
            } else {
                0x02
            };
            let _ = stream
                .write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return;
        }
    };
    if stream
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .is_err()
    {
        return;
    }
    relay_and_account(shared, &host, port, stream, upstream, &[]).await;
}

/// Reads an HTTP proxy request head up to `\r\n\r\n`, returning the full
/// buffer and the offset immediately after the header terminator.
async fn read_http_head<S: AsyncRead + AsyncWrite + Unpin>(
    first_byte: u8,
    stream: &mut S,
) -> Option<(Vec<u8>, usize)> {
    let mut buf = Vec::with_capacity(1024);
    buf.push(first_byte);
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(pos) = find_head_end(&buf) {
            return Some((buf, pos));
        }
        if buf.len() >= MAX_HTTP_HEAD {
            let _ = stream
                .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\n\r\n")
                .await;
            return None;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Serves one HTTP proxy connection (`CONNECT host:port` or plain absolute-URL
/// HTTP request) requiring `Proxy-Authorization: Basic`.
async fn serve_http<S>(
    shared: &Shared,
    first_byte: u8,
    mut stream: S,
    fetch_target: Option<&Origin>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let Some((buf, head_end)) = read_http_head(first_byte, &mut stream).await else {
        return;
    };
    let head_bytes = &buf[..head_end];
    let leftover = &buf[head_end..];
    let Ok(head_text) = std::str::from_utf8(head_bytes) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };
    let mut lines = head_text.split("\r\n");
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };

    let mut authenticated = false;
    let mut forwarded_headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("proxy-authorization") {
                authenticated = verify_basic_auth(&shared.credentials, value.trim());
                continue;
            }
            if name.trim().eq_ignore_ascii_case("proxy-connection") {
                continue;
            }
        }
        forwarded_headers.push(line);
    }
    if !authenticated {
        let _ = stream
            .write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"goodfetch\"\r\nContent-Length: 0\r\n\r\n",
            )
            .await;
        return;
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = parse_authority(target) else {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return;
        };
        let Some(upstream) = connect_http(shared, &mut stream, &host, port, fetch_target).await
        else {
            return;
        };
        if stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .is_err()
        {
            return;
        }
        relay_and_account(shared, &host, port, stream, upstream, leftover).await;
        return;
    }

    // Plain HTTP proxy request with absolute-form URL (`GET http://host:port/path HTTP/1.1`).
    let Ok(url) = Url::parse(target) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };
    let Some(host) = url.host_str().map(str::to_owned) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };
    let Some(port) = url.port_or_known_default() else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };
    let Some(upstream) = connect_http(shared, &mut stream, &host, port, fetch_target).await else {
        return;
    };
    let mut origin_form = url.path().to_owned();
    if let Some(query) = url.query() {
        origin_form.push('?');
        origin_form.push_str(query);
    }
    let mut rewritten = format!("{method} {origin_form} {version}\r\n").into_bytes();
    for header in forwarded_headers {
        rewritten.extend_from_slice(header.as_bytes());
        rewritten.extend_from_slice(b"\r\n");
    }
    rewritten.extend_from_slice(b"\r\n");
    rewritten.extend_from_slice(leftover);
    relay_and_account(shared, &host, port, stream, upstream, &rewritten).await;
}

/// Dials `(host, port)` through `shared.fetch` for an HTTP proxy request,
/// observing refusals and writing `502` or `403` to `stream` on error.
async fn connect_http<S: AsyncWrite + Unpin>(
    shared: &Shared,
    stream: &mut S,
    host: &str,
    port: u16,
    fetch_target: Option<&Origin>,
) -> Option<TcpStream> {
    match shared.fetch.connect(host, port, fetch_target).await {
        Ok(upstream) => Some(upstream),
        Err(error) => {
            if let Some(observer) = &shared.observer {
                observer.observed(&error);
            }
            let status = if error.code() == "plugin_fetch_failed" {
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n".as_slice()
            } else {
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".as_slice()
            };
            let _ = stream.write_all(status).await;
            None
        }
    }
}

/// Copies bytes bidirectionally between `client` and `upstream`, cancelling on
/// `shared.stopping`, and records bytes and duration once when the stream ends.
async fn relay_and_account<S>(
    shared: &Shared,
    host: &str,
    port: u16,
    mut client: S,
    mut upstream: TcpStream,
    initial_upstream: &[u8],
) where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let _guard = OpenStreamGuard::enter(&shared.open_streams);
    let started = shared.fetch.clock().now();
    let mut total_bytes: u64 = 0;

    if !initial_upstream.is_empty() {
        if upstream.write_all(initial_upstream).await.is_err() {
            record_stream(shared, host, port, 0, Duration::ZERO).await;
            return;
        }
        total_bytes = total_bytes.saturating_add(initial_upstream.len() as u64);
    }

    let copied = shared
        .stopping
        .run_until_cancelled(tokio::io::copy_bidirectional(&mut client, &mut upstream))
        .await;
    if let Some(Ok((from_client, from_upstream))) = copied {
        total_bytes = total_bytes
            .saturating_add(from_client)
            .saturating_add(from_upstream);
    }
    let elapsed = shared
        .fetch
        .clock()
        .now()
        .saturating_duration_since(started);
    record_stream(shared, host, port, total_bytes, elapsed).await;
}

async fn record_stream(shared: &Shared, host: &str, port: u16, bytes: u64, elapsed: Duration) {
    let Some(accountant) = &shared.accountant else {
        return;
    };
    let destination = format_destination(host, port);
    let duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    accountant
        .record(StreamRecord {
            plugin: shared.plugin.clone(),
            destination,
            transport: StreamTransport::ProxyTcp,
            bytes,
            duration_ms,
        })
        .await;
}

/// Formats `(host, port)` consistently for Usage parts and Run trace entries.
#[must_use]
pub fn format_destination(host: &str, port: u16) -> String {
    let stripped = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if stripped.contains(':') {
        format!("[{stripped}]:{port}")
    } else {
        format!("{stripped}:{port}")
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn verify_basic_auth(credentials: &ProxyCredentials, header_value: &str) -> bool {
    let Some((scheme, encoded)) = header_value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(decoded) = data_encoding::BASE64.decode(encoded.trim().as_bytes()) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some((user, pass)) = text.split_once(':') else {
        return false;
    };
    credentials.verify(user, pass)
}

fn parse_authority(authority: &str) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, port_str) = rest.split_once("]:")?;
        let port: u16 = port_str.parse().ok()?;
        return Some((host.to_owned(), port));
    }
    let (host, port_str) = authority.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    Some((host.to_owned(), port))
}
