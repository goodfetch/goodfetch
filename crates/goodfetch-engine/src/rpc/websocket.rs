//! The protocol over WebSocket (ADR 0049), the carriage of edge and
//! serverless Clients on an Exposed Daemon's address, beside newline-
//! delimited JSON-RPC on the same address: a Connection whose first byte
//! opens an HTTP request line is a WebSocket handshake, any other is lines.
//! The handshake is refused with `403` when its `Host` is not one of the
//! names the Daemon is reached at, or its `Origin` is missing or not one of
//! them over `http` (ADRs 0017 and 0067), so a page of another origin, or
//! one that rebinds a name to the Daemon's address, never opens it. A
//! handshake that offers the subprotocol `goodfetch.v<major>` of the
//! protocol version is answered with it (spec #606's Transports). Each
//! text or binary message then carries one JSON-RPC message, and each
//! message the engine sends is one text message; the `hello` with its token
//! still comes first. A message longer than
//! [`MAX_MESSAGE_BYTES`](super::framing::MAX_MESSAGE_BYTES) ends in its
//! typed limit error, and the Connection then closes, since a WebSocket
//! cannot skip it. A handshake at
//! [`CHANNEL_PATH`](goodfetch_protocol::live_view::CHANNEL_PATH) opens a
//! Live view channel instead (ADR 0031), under the same `Host` and
//! `Origin` rules, where `live_view.enabled` is on; a path a listener does
//! not serve is answered `404`.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::live_view::CHANNEL_PATH;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode, header};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error, Message};

use super::framing::{Frame, MAX_MESSAGE_BYTES};
use crate::endpoint::{self, Authorities, Hosts, Origins, Refusal};
use crate::transport::{ConnectionReader, ConnectionWriter};

/// Why no WebSocket opened.
#[derive(Debug)]
pub(super) enum NotOpened {
    /// The handshake for that path broke a rule; it was answered `403`.
    Refused(Refusal, Path),
    /// The request was no WebSocket handshake, was for a path not served,
    /// or the Connection failed.
    Failed(Error),
}

/// What a Connection receives, the bytes already read from it first.
pub(super) struct Prefixed {
    read: Vec<u8>,
    at: usize,
    reader: ConnectionReader,
}

impl Prefixed {
    pub(super) fn new(read: Vec<u8>, reader: ConnectionReader) -> Self {
        Self {
            read,
            at: 0,
            reader,
        }
    }

    /// Whether the bytes already read open an HTTP request line, whose
    /// method is a token of capital letters, rather than a JSON-RPC line,
    /// which opens with `{` or whitespace.
    pub(super) fn opens_a_request(&self) -> bool {
        self.read.first().is_some_and(u8::is_ascii_uppercase)
    }
}

impl AsyncRead for Prefixed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.at < this.read.len() {
            let left = &this.read[this.at..];
            let taken = left.len().min(buffer.remaining());
            buffer.put_slice(&left[..taken]);
            this.at += taken;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.reader).poll_read(context, buffer)
    }
}

/// A Connection's two directions joined again, as a WebSocket takes it.
pub(super) struct Rejoined {
    reader: Prefixed,
    writer: ConnectionWriter,
}

impl Rejoined {
    pub(super) fn new(reader: Prefixed, writer: ConnectionWriter) -> Self {
        Self { reader, writer }
    }
}

impl AsyncRead for Rejoined {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(context, buffer)
    }
}

impl AsyncWrite for Rejoined {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(context)
    }
}

/// Which WebSocket a handshake asks for, by its path: a Live view channel
/// at [`CHANNEL_PATH`] (ADR 0031), the protocol at any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Path {
    Protocol,
    LiveView,
}

impl Path {
    fn of(request: &Request) -> Self {
        if request.uri().path() == CHANNEL_PATH {
            Self::LiveView
        } else {
            Self::Protocol
        }
    }
}

/// What a handshake opened.
pub(super) enum Opened {
    /// The protocol's WebSocket, its two directions.
    Protocol(MessageReader, MessageWriter),
    /// A Live view channel, boxed: its stream is several times the
    /// protocol's two halves.
    LiveView(Box<LiveViewSocket>),
}

/// A Live view channel's WebSocket.
pub(super) type LiveViewSocket = WebSocketStream<Carried>;

/// The handshake on `stream`, its `Host` and `Origin` held to
/// `authorities`, for one of the `paths` it serves: what it opened, or why
/// it did not open. A handshake for a path it does not serve is answered
/// `404`, a closed door rather than a refusal.
///
/// # Cancel safety
///
/// Dropping the future drops the Connection.
pub(super) async fn accept(
    stream: Rejoined,
    authorities: &Authorities,
    paths: &[Path],
) -> Result<Opened, NotOpened> {
    let mut asked = None;
    let mut refused = None;
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's handshake callback answers its own `ErrorResponse`"
    )]
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let path = Path::of(request);
        asked = Some(path);
        let mut answer = ErrorResponse::new(None);
        if !paths.contains(&path) {
            *answer.status_mut() = StatusCode::NOT_FOUND;
            return Err(answer);
        }
        match refusal(request, authorities) {
            None if path == Path::Protocol => Ok(with_subprotocol(request, response)),
            None => Ok(response),
            Some(refusal) => {
                refused = Some(refusal);
                *answer.status_mut() = StatusCode::FORBIDDEN;
                Err(answer)
            }
        }
    };
    let config = config();
    let stream: Carried = Box::new(stream);
    let opened = tokio_tungstenite::accept_hdr_async_with_config(stream, check, Some(config)).await;
    match (opened, refused, asked) {
        (Ok(socket), _, Some(Path::LiveView)) => Ok(Opened::LiveView(Box::new(socket))),
        (Ok(socket), _, _) => {
            let (sink, stream) = socket.split();
            Ok(Opened::Protocol(MessageReader(stream), MessageWriter(sink)))
        }
        (Err(_), Some(refusal), path) => {
            Err(NotOpened::Refused(refusal, path.unwrap_or(Path::Protocol)))
        }
        (Err(error), None, _) => Err(NotOpened::Failed(error)),
    }
}

/// A WebSocket's limits: one JSON-RPC message per message, at most
/// [`MAX_MESSAGE_BYTES`] long.
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

/// The bytes a WebSocket runs over: a Connection rejoined, or the
/// connection an HTTP server upgraded.
pub(crate) trait Duplex: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> Duplex for T {}

pub(super) type Carried = Box<dyn Duplex>;

/// `response` naming the subprotocol of the protocol's major,
/// `goodfetch.v<major>`, when `request` offers it among its
/// `Sec-WebSocket-Protocol`s. A handshake that offers none, or only others,
/// is answered without one: the Client's WebSocket then fails one it
/// offered (RFC 6455, section 4.1), and one that offered none opens.
fn with_subprotocol(request: &Request, mut response: Response) -> Response {
    let ours = PROTOCOL_VERSION.websocket_subprotocol();
    let offered = request
        .headers()
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|offered| offered.trim() == ours);
    if offered && let Ok(value) = HeaderValue::from_str(&ours) {
        response
            .headers_mut()
            .insert(header::SEC_WEBSOCKET_PROTOCOL, value);
    }
    response
}

/// The rule `request` breaks, if any: its `Host` first, then its `Origin`.
fn refusal(request: &Request, authorities: &Authorities) -> Option<Refusal> {
    let value = |name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    endpoint::refusal(
        value(header::HOST),
        value(header::ORIGIN),
        Hosts::Named(authorities),
        Origins::Required(&[authorities]),
    )
}

/// The messages a WebSocket's Client sends.
pub(super) struct MessageReader(SplitStream<WebSocketStream<Carried>>);

impl MessageReader {
    /// Reads the next message and appends its JSON-RPC message to
    /// `frames`; `false` once the WebSocket has ended, after appending a
    /// message too large to read, if that ended it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no message.
    pub(super) async fn read(&mut self, frames: &mut Vec<Frame>) -> bool {
        loop {
            match self.0.next().await {
                Some(Ok(Message::Text(text))) => {
                    frames.push(Frame::Line(text.as_bytes().to_vec()));
                    return true;
                }
                Some(Ok(Message::Binary(bytes))) => {
                    frames.push(Frame::Line(bytes.to_vec()));
                    return true;
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Err(Error::Capacity(_))) => {
                    frames.push(Frame::TooLarge);
                    return false;
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return false,
            }
        }
    }
}

/// What the engine sends a WebSocket's Client.
pub(super) struct MessageWriter(SplitSink<WebSocketStream<Carried>, Message>);

impl MessageWriter {
    /// Queues `line`, one JSON-RPC message and its newline, as one text
    /// message; `false` once the WebSocket is gone.
    pub(super) async fn feed(&mut self, mut line: Vec<u8>) -> bool {
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let text = String::from_utf8(line).expect("invariant: the engine's messages are JSON");
        self.0.feed(Message::text(text)).await.is_ok()
    }

    /// Sends what is queued; `false` once the WebSocket is gone.
    pub(super) async fn flush(&mut self) -> bool {
        self.0.flush().await.is_ok()
    }

    /// Closes the WebSocket.
    pub(super) async fn close(mut self) {
        let _ = self.0.close().await;
    }
}
