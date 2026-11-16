//! A Borrowed browser's connection through its `cdp_url` (ADRs 0056 and
//! 0115): the address is resolved by the engine, through the Run's
//! `network.resolve` mapping, and judged by ADR 0056's address rules before
//! any connection, which reaches only the addresses judged; then the
//! browser's CDP WebSocket carries the driver's messages, each a message
//! of its own, as the debugging pipe carries them NUL-terminated. The
//! connection ends with the Run, and only the engine's own connection
//! closes: a Borrowed browser is never closed.
//!
//! This engine reaches a browser's own `ws://…/devtools/browser/<id>`
//! address; the probe that reads `http://host:port`'s `/json/version`,
//! `wss://` and a Secret reference in `headers`, with one Error code per
//! way the address cannot be used, are the `cdp_url` Session's (#743).
#![expect(
    clippy::disallowed_types,
    reason = "a Borrowed browser's connection is opened here alone, to the addresses its `cdp_url` was judged on, so no other lookup chooses where it goes (ADRs 0056 and 0060)"
)]

use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use goodfetch_cdp::{Connection, Framer, MAX_FRAME};
use goodfetch_engine::CallError;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::egress::{
    CdpUrlOrigin, EndpointPolicy, Policy, cdp_url_address, judge_cdp_url,
};
use goodfetch_engine::http_client;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_protocol::page::{BROWSER_LAUNCH_FAILED, BrowserLaunchFailedDetails};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SESSION_SOURCE, SETTING_INVALID,
    SettingInvalidDetails,
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Bytes, Error as WsError, Message, Utf8Bytes};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

/// How many bytes the engine's end and the bridge buffer between them.
const BUFFER: usize = 1024 * 1024;

/// The only scheme this engine reaches a `cdp_url` by.
const WEBSOCKET: &str = "ws";

/// A `cdp_url` and who gave it.
#[derive(Clone, Debug)]
pub(crate) struct CdpUrl {
    /// The address, as given.
    pub(crate) url: String,
    /// Who gave it.
    pub(crate) origin: CdpUrlOrigin,
}

impl CdpUrl {
    /// The address as details and messages name it, without its query or
    /// credentials.
    pub(crate) fn address(&self) -> String {
        cdp_url_address(&self.url)
    }

    /// The host and port the engine reaches it at, a `ws://` address's
    /// alone.
    pub(crate) fn host_and_port(&self) -> Option<(String, u16)> {
        let parsed = Url::parse(&self.url)
            .ok()
            .filter(|parsed| parsed.scheme() == WEBSOCKET)?;
        let host = match parsed.host()? {
            Host::Ipv6(ip) => ip.to_string(),
            host => host.to_string(),
        };
        Some((host, parsed.port_or_known_default().unwrap_or(80)))
    }
}

/// The `network.resolve` mapping of `settings`.
pub(crate) fn resolve_of(settings: Option<&Resolved>) -> HostMapping {
    settings.map(HostMapping::of).unwrap_or_default()
}

/// Opens the engine's connection to the browser at `cdp_url` for `run`,
/// once ADR 0056's rules allow its address.
///
/// # Errors
///
/// `cdp_url_not_allowed` when the rules refuse an address its host
/// resolves to; `capability_unavailable` for an address other than a
/// `ws://` one; `setting_invalid` for a Run whose domain lists hold no
/// host; and `browser_launch_failed` naming the address when nothing
/// answers there as a browser (#743 gives each case its own code).
pub(crate) async fn connect(run: &Run, cdp_url: &CdpUrl) -> Result<Connection, CallError> {
    let address = cdp_url.address();
    let (host, port) = cdp_url
        .host_and_port()
        .ok_or_else(|| unsupported(cdp_url))?;
    let settings = run.settings().map(RunSettings::resolved);
    let addresses = http_client::lookup(&host, port, &resolve_of(settings))
        .await
        .map_err(|_| unreachable(&address, "its host has no address"))?;
    let policy = session_policy(settings)?;
    let endpoints = settings.map(EndpointPolicy::of).unwrap_or_default();
    judge_cdp_url(&cdp_url.url, &addresses, cdp_url.origin, &policy, endpoints)
        .map_err(|refused| refused.error())?;
    let stream = TcpStream::connect(addresses.as_slice())
        .await
        .map_err(|error| unreachable(&address, &error.to_string()))?;
    let (socket, _) = tokio_tungstenite::client_async_with_config(
        cdp_url.url.as_str(),
        stream,
        Some(websocket_config()),
    )
    .await
    .map_err(|error| unreachable(&address, &error.to_string()))?;
    let log = crate::trace::command_log(run).await.map_err(|error| {
        CallError::new(
            &BROWSER_LAUNCH_FAILED,
            format!("the Run trace's CDP log cannot start: {error}"),
            &BrowserLaunchFailedDetails {
                executable: address.clone(),
                exit: None,
            },
        )
    })?;
    let (engine, bridge_end) = tokio::io::duplex(BUFFER);
    let (reader, writer) = tokio::io::split(engine);
    let (connection, driver) =
        Connection::new(reader, writer, run.metrics().counted_work().clone(), log);
    run.spawn(async move {
        let ended = driver.run().await;
        tracing::debug!(?ended, "a Borrowed browser's connection ended");
    });
    run.spawn(bridge(socket, bridge_end, run.cancellation().clone()));
    Ok(connection)
}

/// The WebSocket's limits: a frame and a message as long as the debugging
/// pipe's ([`MAX_FRAME`]), since Chromium sends each CDP message as one
/// frame, and tungstenite's own 16 MiB frame cap ended the connection on a
/// body of about 12 MiB (finding 563-3).
fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME))
}

/// The Session policy a Run's settings give, whose private ranges a
/// `cdp_url` that is an address a page can reach may use.
fn session_policy(settings: Option<&Resolved>) -> Result<Policy, CallError> {
    match settings {
        Some(settings) => Policy::of(settings),
        None => Policy::new(&["*"], &[] as &[&str]),
    }
    .map_err(|invalid| {
        CallError::new(
            &SETTING_INVALID,
            format!(
                "`{}` holds `{}`, which is no host",
                invalid.setting, invalid.entry
            ),
            &SettingInvalidDetails {
                key: invalid.setting.to_owned(),
                layer: None,
                reason: format!("`{}` is no host", invalid.entry),
            },
        )
    })
}

/// `capability_unavailable` for a `cdp_url` this engine does not reach.
fn unsupported(cdp_url: &CdpUrl) -> CallError {
    let address = cdp_url.address();
    CallError::new(
        &CAPABILITY_UNAVAILABLE,
        format!(
            "This engine reaches a `cdp_url` only at the browser's own `ws://…/devtools/browser/<id>` address, not `{address}`: give that address, which the browser's `/json/version` lists."
        ),
        &CapabilityUnavailableDetails {
            key: SESSION_SOURCE.name.to_owned(),
            layer: cdp_url.origin.layer,
            kind: "cdp_url".to_owned(),
            fix: "a `ws://…/devtools/browser/<id>` address".to_owned(),
        },
    )
}

/// The failure of reaching the browser at `address`.
fn unreachable(address: &str, why: &str) -> CallError {
    CallError::new(
        &BROWSER_LAUNCH_FAILED,
        format!("the browser at the `cdp_url` {address} cannot be reached: {why}"),
        &BrowserLaunchFailedDetails {
            executable: address.to_owned(),
            exit: None,
        },
    )
}

/// Carries the driver's messages between `end`, the engine's side, where
/// each is NUL-terminated, and the browser's WebSocket `socket`, where each
/// is a message, until either side ends or `stopping`; then the engine's
/// own connection closes, and the browser stays. Each way is carried apart
/// and waits only on its own writes, so a large command the engine writes
/// and a large message the browser sends cross without either waiting on
/// the other (finding 563-2), and either way stops at `stopping` whatever
/// it was writing.
///
/// # Cancel safety
///
/// Dropping the future drops the connection; the browser stays.
async fn bridge<S: AsyncRead + AsyncWrite + Unpin>(
    socket: WebSocketStream<S>,
    end: DuplexStream,
    stopping: CancellationToken,
) {
    let (from_engine, to_engine) = tokio::io::split(end);
    let (mut to_browser, from_browser) = socket.split();
    // Either way's end ends the other.
    let ended = stopping.child_token();
    tokio::join!(
        async {
            carry_to_browser(from_engine, &mut to_browser, &ended).await;
            ended.cancel();
        },
        async {
            carry_to_engine(from_browser, to_engine, &ended).await;
            ended.cancel();
        },
    );
    if let Err(error) = to_browser.close().await {
        tracing::debug!(%error, "a Borrowed browser's connection was gone already");
    }
}

/// Carries each message the engine writes to `from_engine`, NUL-terminated,
/// to the browser as one WebSocket message, until either side ends or
/// `ended`.
async fn carry_to_browser<S: Sink<Message> + Unpin>(
    mut from_engine: impl AsyncRead + Unpin,
    to_browser: &mut S,
    ended: &CancellationToken,
) {
    let mut framer = Framer::with_limit(MAX_FRAME);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = tokio::select! {
            () = ended.cancelled() => return,
            read = from_engine.read(&mut buffer) => read,
        };
        let Ok(read @ 1..) = read else { return };
        framer.push(&buffer[..read]);
        let sent = tokio::select! {
            () = ended.cancelled() => return,
            sent = send_frames(to_browser, &mut framer) => sent,
        };
        if !sent {
            return;
        }
    }
}

/// Carries each WebSocket message the browser sends on `from_browser` to
/// the engine, NUL-terminated, until either side ends or `ended`.
async fn carry_to_engine(
    mut from_browser: impl Stream<Item = Result<Message, WsError>> + Unpin,
    mut to_engine: impl AsyncWrite + Unpin,
    ended: &CancellationToken,
) {
    loop {
        let message = tokio::select! {
            () = ended.cancelled() => return,
            message = from_browser.next() => message,
        };
        let text = match message {
            Some(Ok(Message::Text(text))) => Bytes::from(text),
            Some(Ok(Message::Binary(bytes))) => bytes,
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
            Some(Ok(Message::Close(_)) | Err(_)) | None => return,
        };
        let written = tokio::select! {
            () = ended.cancelled() => return,
            written = async {
                to_engine.write_all(&text).await?;
                to_engine.write_all(&[0]).await
            } => written,
        };
        if written.is_err() {
            return;
        }
    }
}

/// Sends each whole message `framer` holds; false once the socket or a
/// message failed.
async fn send_frames<S: Sink<Message> + Unpin>(to_browser: &mut S, framer: &mut Framer) -> bool {
    loop {
        match framer.next_frame() {
            Ok(Some(frame)) => {
                let Ok(text) = Utf8Bytes::try_from(frame) else {
                    return false;
                };
                if to_browser.send(Message::Text(text)).await.is_err() {
                    return false;
                }
            }
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests;
