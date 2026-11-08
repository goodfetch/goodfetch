//! The protocol over WebSocket (ADR 0049), the carriage of edge and
//! serverless Clients on an Exposed Daemon's address, beside newline-
//! delimited JSON-RPC on the same address: a Connection whose first byte
//! opens an HTTP request line is a WebSocket handshake, any other is lines.
//! The handshake is refused with `403` when its `Host` is not one of the
//! names the Daemon is reached at, or its `Origin` is missing or not one of
//! them over `http` (ADRs 0017 and 0067), so a page of another origin, or
//! one that rebinds a name to the Daemon's address, never opens it. Each
//! text or binary message then carries one JSON-RPC message, and each
//! message the engine sends is one text message; the `hello` with its token
//! still comes first. A message longer than
//! [`MAX_MESSAGE_BYTES`](super::framing::MAX_MESSAGE_BYTES) ends in its
//! typed limit error, and the Connection then closes, since a WebSocket
//! cannot skip it.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_protocol::audit::RefusalReason;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{StatusCode, header};
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error, Message};

use super::framing::{Frame, MAX_MESSAGE_BYTES};
use crate::transport::{ConnectionReader, ConnectionWriter};

/// The names a listener is reached at, as `host:port`: a WebSocket
/// handshake's `Host` must be one of them, and its `Origin` one of them
/// over `http` (ADR 0017).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorities {
    names: Vec<String>,
}

impl Authorities {
    /// The names of a listener at the loopback `address`: the address
    /// itself and `localhost`, each with its port.
    pub fn loopback(address: SocketAddr) -> Self {
        let port = address.port();
        Self {
            names: vec![address.to_string(), format!("localhost:{port}")],
        }
    }

    /// The rule a request or handshake with `host` and `origin` breaks,
    /// if any: its `Host` first, then its `Origin`.
    pub(crate) fn refusal(&self, host: Option<&str>, origin: Option<&str>) -> Option<Refusal> {
        if !host.is_some_and(|host| self.admits_host(host)) {
            return Some(Refusal::HostForeign);
        }
        match origin {
            None => Some(Refusal::OriginMissing),
            Some(origin) if !self.admits_origin(origin) => Some(Refusal::OriginForeign),
            Some(_) => None,
        }
    }

    /// These names and `name`, as `host:port`, or `host` for a default
    /// port.
    #[must_use]
    pub(crate) fn with_name(mut self, name: impl Into<String>) -> Self {
        self.names.push(name.into());
        self
    }

    /// Only `name`, as `host:port`, or `host` for a default port.
    pub(crate) fn only(name: impl Into<String>) -> Self {
        Self {
            names: vec![name.into()],
        }
    }

    /// The origins of these names over `http`, as a CSP source list does.
    pub(crate) fn origins(&self) -> Vec<String> {
        self.names
            .iter()
            .map(|name| format!("http://{name}"))
            .collect()
    }

    /// Whether `host`, a request's `Host`, is one of these names.
    pub(crate) fn admits_host(&self, host: &str) -> bool {
        self.names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(host))
    }

    /// Whether `origin`, a request's `Origin`, is one of these names over
    /// `http`.
    pub(crate) fn admits_origin(&self, origin: &str) -> bool {
        origin
            .strip_prefix("http://")
            .is_some_and(|authority| self.admits_host(authority))
    }
}

/// Why a handshake was refused, by its endpoint table's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    HostForeign,
    OriginMissing,
    OriginForeign,
}

impl Refusal {
    /// The rule, which the refusal's log record carries.
    pub(crate) fn reason(self) -> RefusalReason {
        match self {
            Self::HostForeign => RefusalReason::HostForeign,
            Self::OriginMissing => RefusalReason::OriginMissing,
            Self::OriginForeign => RefusalReason::OriginForeign,
        }
    }
}

/// Why no WebSocket opened.
#[derive(Debug)]
pub(super) enum NotOpened {
    /// The handshake broke a rule; it was answered `403`.
    Refused(Refusal),
    /// The request was no WebSocket handshake, or the Connection failed.
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

/// The handshake on `stream`, its `Host` and `Origin` held to
/// `authorities`: the WebSocket's two directions, or why it did not open.
///
/// # Cancel safety
///
/// Dropping the future drops the Connection.
pub(super) async fn accept(
    stream: Rejoined,
    authorities: &Authorities,
) -> Result<(MessageReader, MessageWriter), NotOpened> {
    let mut refused = None;
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's handshake callback answers its own `ErrorResponse`"
    )]
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        match refusal(request, authorities) {
            None => Ok(response),
            Some(refusal) => {
                refused = Some(refusal);
                let mut forbidden = ErrorResponse::new(None);
                *forbidden.status_mut() = StatusCode::FORBIDDEN;
                Err(forbidden)
            }
        }
    };
    let config = config();
    let stream: Carried = Box::new(stream);
    let opened = tokio_tungstenite::accept_hdr_async_with_config(stream, check, Some(config)).await;
    match (opened, refused) {
        (Ok(socket), _) => {
            let (sink, stream) = socket.split();
            Ok((MessageReader(stream), MessageWriter(sink)))
        }
        (Err(_), Some(refusal)) => Err(NotOpened::Refused(refusal)),
        (Err(error), None) => Err(NotOpened::Failed(error)),
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

type Carried = Box<dyn Duplex>;

/// A WebSocket the Console's server opened on its own origin, its
/// handshake already accepted and its sign-in already checked, which the
/// Daemon serves as a Connection (ADR 0031).
pub(crate) struct ConsoleSocket {
    pub(super) reader: MessageReader,
    pub(super) writer: MessageWriter,
}

impl ConsoleSocket {
    /// The WebSocket on `upgraded`, the connection whose handshake the
    /// Console's server answered with `101`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops the connection.
    pub(crate) async fn opened(upgraded: impl Duplex + 'static) -> Self {
        let carried: Carried = Box::new(upgraded);
        let socket = WebSocketStream::from_raw_socket(carried, Role::Server, Some(config())).await;
        let (sink, stream) = socket.split();
        Self {
            reader: MessageReader(stream),
            writer: MessageWriter(sink),
        }
    }
}

/// The rule `request` breaks, if any: its `Host` first, then its `Origin`.
fn refusal(request: &Request, authorities: &Authorities) -> Option<Refusal> {
    let value = |name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    authorities.refusal(value(header::HOST), value(header::ORIGIN))
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
