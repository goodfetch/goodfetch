//! One attempt on a WebSocket listener (ADR 0049): the runner opens a
//! WebSocket with the handshake a rule sets, such as a foreign `Host` or no
//! `Origin`, sends a first message, and records whether the listener
//! refused the handshake with a status, replied with a `result`, closed the
//! connection, or stayed silent.

use std::net::SocketAddr;

use serde_json::Value;
use tungstenite::handshake::client::generate_key;
use tungstenite::{Error, HandshakeError, Message};

use super::connection::{Connection, ListenerAddress, is_closed, is_silent};
use super::runner::Observed;

/// A WebSocket handshake.
pub(super) struct Handshake<'a> {
    /// The path, with its query string if any.
    pub(super) target: String,
    /// The `Host` header.
    pub(super) host: &'a str,
    /// The `Origin` header, if any.
    pub(super) origin: Option<&'a str>,
    /// The `Cookie` header, if any.
    pub(super) cookie: Option<String>,
}

/// Opens a WebSocket to `address` with `handshake`, sends `message` as its
/// first message, and reads until the listener replies with a `result`,
/// closes, or stays silent past the deadline. JSON-RPC errors sent before
/// closing are no reply.
pub(super) fn attempt(address: SocketAddr, handshake: &Handshake<'_>, message: &Value) -> Observed {
    let connection = match Connection::open(&ListenerAddress::Tcp(address)) {
        Ok(connection) => connection,
        Err(error) => {
            return Observed::Unreachable {
                error: error.kind(),
            };
        }
    };
    let mut request = tungstenite::http::Request::builder()
        .uri(format!("ws://{address}{}", handshake.target))
        .header("Host", handshake.host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key());
    if let Some(origin) = handshake.origin {
        request = request.header("Origin", origin);
    }
    if let Some(cookie) = &handshake.cookie {
        request = request.header("Cookie", cookie);
    }
    let Ok(request) = request.body(()) else {
        return Observed::Malformed;
    };
    let mut socket = match tungstenite::client(request, connection) {
        Ok((socket, _)) => socket,
        Err(HandshakeError::Failure(Error::Http(response))) => {
            return Observed::Answered {
                status: response.status().as_u16(),
                body_length: response.body().as_ref().map_or(0, Vec::len),
            };
        }
        Err(HandshakeError::Failure(error)) => return observed(&error),
        Err(HandshakeError::Interrupted(_)) => return Observed::StayedOpen,
    };
    if let Err(error) = socket.send(Message::text(message.to_string())) {
        return observed(&error);
    }
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let replied = serde_json::from_str::<Value>(&text)
                    .is_ok_and(|answer| answer.get("result").is_some());
                if replied {
                    return Observed::Replied;
                }
            }
            Ok(Message::Close(_)) => return Observed::Closed,
            Ok(_) => {}
            Err(error) => return observed(&error),
        }
    }
}

/// What a WebSocket's failure shows.
fn observed(error: &Error) -> Observed {
    match error {
        Error::ConnectionClosed | Error::AlreadyClosed => Observed::Closed,
        Error::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake) => {
            Observed::Closed
        }
        Error::Io(error) if is_silent(error) => Observed::StayedOpen,
        Error::Io(error) if is_closed(error) => Observed::Closed,
        Error::Io(error) => Observed::Unreachable {
            error: error.kind(),
        },
        _ => Observed::Malformed,
    }
}
