//! The protocol over WebSocket to an Exposed Daemon (ADR 0049; spec #606's
//! Transports): the subprotocol `goodfetch.v<major>` of the protocol
//! version the client speaks, and the same JSON-RPC messages as over stdio,
//! one per WebSocket message each way, so a Connection's newline-delimited
//! lines cross it unchanged. tungstenite does the handshake and the
//! framing, as it does in the engine. TypeScript's `websocket-transport.ts`
//! and Python's `websocket_transport.py` are its twins.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures_core::Stream;
use futures_util::Sink;
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use tokio::io::{AsyncRead, AsyncWrite, Join, ReadBuf};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::{ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error, Message};

use super::{ExposedDaemon, Link};

/// The bytes a WebSocket is opened on: a Connection's two directions.
type Carrier = Join<Box<dyn AsyncRead + Send + Unpin>, Box<dyn AsyncWrite + Send + Unpin>>;

/// `link`, to `exposed`, as the Connection over a WebSocket opened on it.
/// The handshake carries the Daemon's own `Host` and `Origin`, as a Client
/// outside a browser does (ADRs 0049 and 0067), and offers the subprotocol
/// of the protocol version the client speaks.
///
/// # Errors
///
/// When the handshake failed: the Daemon refused it, `link` ended, or it
/// answered no subprotocol offered, which a Daemon of another protocol
/// major does and a WebSocket does not tell apart.
///
/// # Cancel safety
///
/// Dropping the future closes `link`.
pub(super) async fn open(link: Link, exposed: &ExposedDaemon) -> io::Result<Link> {
    let (reader, writer, exit) = link.split();
    let authority = &exposed.authority;
    let mut request = format!("ws://{authority}/")
        .into_client_request()
        .map_err(io::Error::other)?;
    let headers = request.headers_mut();
    headers.insert(ORIGIN, header(&format!("http://{authority}"))?);
    headers.insert(
        SEC_WEBSOCKET_PROTOCOL,
        header(&PROTOCOL_VERSION.websocket_subprotocol())?,
    );
    // As over stdio, the client takes a message of any length.
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    // tungstenite refuses an answer naming no subprotocol, or another.
    let (socket, _answer) = tokio_tungstenite::client_async_with_config(
        request,
        tokio::io::join(reader, writer),
        Some(config),
    )
    .await
    .map_err(io::Error::other)?;
    let (reader, writer) = tokio::io::split(Messages::new(socket));
    Ok(Link {
        reader: Box::new(reader),
        writer: Box::new(writer),
        exit,
    })
}

/// `value` as a header's value.
fn header(value: &str) -> io::Result<HeaderValue> {
    HeaderValue::from_str(value).map_err(io::Error::other)
}

/// A WebSocket open to an Exposed Daemon, as a Connection's bytes: each
/// message received is one line to read, and each whole line written goes
/// as one text message. The Daemon's close ends the bytes to read, and
/// closing the output sends this side's close, `1000`.
struct Messages {
    socket: WebSocketStream<Carrier>,
    /// The line received last, its newline added.
    received: Vec<u8>,
    /// How much of `received` was read.
    read: usize,
    /// What was written past its last whole line.
    unsent: Vec<u8>,
    /// Whether the Daemon's side ended.
    ended: bool,
    /// Whether this side's close is sent.
    closing: bool,
}

impl Messages {
    fn new(socket: WebSocketStream<Carrier>) -> Self {
        Self {
            socket,
            received: Vec::new(),
            read: 0,
            unsent: Vec::new(),
            ended: false,
            closing: false,
        }
    }

    /// Takes `message`'s bytes as the next line to read.
    fn receive(&mut self, message: &[u8]) {
        self.received.clear();
        self.received.extend_from_slice(message);
        self.received.push(b'\n');
        self.read = 0;
    }

    /// Hands the socket each whole line written, one text message each.
    fn poll_send_lines(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while let Some(end) = self.unsent.iter().position(|byte| *byte == b'\n') {
            ready!(Pin::new(&mut self.socket).poll_ready(cx)).map_err(io_error)?;
            let rest = self.unsent.split_off(end + 1);
            let mut line = std::mem::replace(&mut self.unsent, rest);
            line.pop();
            let text = String::from_utf8(line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Pin::new(&mut self.socket)
                .start_send(Message::text(text))
                .map_err(io_error)?;
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for Messages {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            let unread = &this.received[this.read..];
            if !unread.is_empty() {
                let taken = unread.len().min(buf.remaining());
                buf.put_slice(&unread[..taken]);
                this.read += taken;
                return Poll::Ready(Ok(()));
            }
            if this.ended {
                return Poll::Ready(Ok(()));
            }
            // Reading on past the Daemon's close sends its answer.
            match ready!(Pin::new(&mut this.socket).poll_next(cx)) {
                Some(Ok(Message::Text(text))) => this.receive(text.as_bytes()),
                Some(Ok(Message::Binary(data))) => this.receive(&data),
                Some(Ok(_)) => {}
                None
                | Some(Err(
                    Error::ConnectionClosed
                    | Error::AlreadyClosed
                    | Error::Protocol(ProtocolError::ResetWithoutClosingHandshake),
                )) => this.ended = true,
                Some(Err(error)) => return Poll::Ready(Err(io_error(error))),
            }
        }
    }
}

impl AsyncWrite for Messages {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.poll_send_lines(cx))?;
        this.unsent.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_send_lines(cx))?;
        Pin::new(&mut this.socket).poll_flush(cx).map_err(io_error)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_send_lines(cx))?;
        if !this.closing {
            ready!(Pin::new(&mut this.socket).poll_ready(cx)).or_else(already_closed)?;
            Pin::new(&mut this.socket)
                .start_send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "".into(),
                })))
                .or_else(already_closed)?;
            this.closing = true;
        }
        Pin::new(&mut this.socket)
            .poll_close(cx)
            .map(|closed| closed.or_else(already_closed))
    }
}

/// Success when `error` says the WebSocket is closed already, as it is
/// once the Daemon closed it first; `error` otherwise.
fn already_closed(error: Error) -> io::Result<()> {
    match error {
        Error::ConnectionClosed | Error::AlreadyClosed => Ok(()),
        error => Err(io_error(error)),
    }
}

/// `error` as the OS's error it is, or one carrying it.
fn io_error(error: Error) -> io::Error {
    match error {
        Error::Io(error) => error,
        error => io::Error::other(error),
    }
}
