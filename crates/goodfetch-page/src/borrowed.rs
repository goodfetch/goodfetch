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

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_cdp::{Connection, Framer, MAX_FRAME};
use goodfetch_engine::CallError;
use goodfetch_engine::egress::{
    CdpUrlOrigin, EndpointPolicy, Policy, cdp_url_address, judge_cdp_url,
};
use goodfetch_engine::http_client::{self, Resolve};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_protocol::page::{BROWSER_LAUNCH_FAILED, BrowserLaunchFailedDetails};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, NETWORK_RESOLVE, SESSION_SOURCE,
    SETTING_INVALID, SettingInvalidDetails,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
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
}

/// The `network.resolve` mapping of `settings`.
pub(crate) fn resolve_of(settings: Option<&Resolved>) -> Resolve {
    settings
        .map(|settings| settings.hosts(NETWORK_RESOLVE.name))
        .unwrap_or_default()
        .into_iter()
        .fold(Resolve::new(), |mapping, (host, addresses)| {
            mapping.with(host, addresses)
        })
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
    let parsed = Url::parse(&cdp_url.url)
        .ok()
        .filter(|parsed| parsed.scheme() == WEBSOCKET)
        .ok_or_else(|| unsupported(cdp_url))?;
    let host = match parsed.host() {
        Some(Host::Ipv6(ip)) => ip.to_string(),
        Some(host) => host.to_string(),
        None => return Err(unsupported(cdp_url)),
    };
    let port = parsed.port_or_known_default().unwrap_or(80);
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
    let (socket, _) = tokio_tungstenite::client_async(cdp_url.url.as_str(), stream)
        .await
        .map_err(|error| unreachable(&address, &error.to_string()))?;
    let (engine, bridge_end) = tokio::io::duplex(BUFFER);
    let (reader, writer) = tokio::io::split(engine);
    let (connection, driver) = Connection::new(reader, writer);
    run.spawn(async move {
        let ended = driver.run().await;
        tracing::debug!(?ended, "a Borrowed browser's connection ended");
    });
    run.spawn(bridge(socket, bridge_end, run.cancellation().clone()));
    Ok(connection)
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
/// own connection closes, and the browser stays.
///
/// # Cancel safety
///
/// Dropping the future drops the connection; the browser stays.
async fn bridge(
    mut socket: WebSocketStream<TcpStream>,
    end: DuplexStream,
    stopping: CancellationToken,
) {
    let (mut from_engine, mut to_engine) = tokio::io::split(end);
    let mut framer = Framer::with_limit(MAX_FRAME);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        tokio::select! {
            () = stopping.cancelled() => break,
            message = socket.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text.as_bytes().to_vec(),
                    Some(Ok(Message::Binary(bytes))) => bytes.to_vec(),
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                };
                if to_engine.write_all(&text).await.is_err()
                    || to_engine.write_all(&[0]).await.is_err()
                {
                    break;
                }
            }
            read = from_engine.read(&mut buffer) => {
                let Ok(read @ 1..) = read else { break };
                framer.push(&buffer[..read]);
                if !send_frames(&mut socket, &mut framer).await {
                    break;
                }
            }
        }
    }
    if let Err(error) = socket.close(None).await {
        tracing::debug!(%error, "a Borrowed browser's connection was gone already");
    }
}

/// Sends each whole message `framer` holds; false once the socket or a
/// message failed.
async fn send_frames(socket: &mut WebSocketStream<TcpStream>, framer: &mut Framer) -> bool {
    loop {
        match framer.next_frame() {
            Ok(Some(frame)) => {
                let Ok(text) = String::from_utf8(frame) else {
                    return false;
                };
                if socket.send(Message::text(text)).await.is_err() {
                    return false;
                }
            }
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}
