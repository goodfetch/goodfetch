//! One attempt on an HTTP proxy: the runner sends one `CONNECT` with the
//! `Proxy-Authorization: Basic` credentials a rule gives, if any, and
//! records the answer's status and body length, or that the proxy closed
//! the connection or stayed silent. A `2xx` answer opens a tunnel, which
//! has no body (RFC 9110, section 9.3.6).

use std::io::{Read as _, Write as _};
use std::net::SocketAddr;

use data_encoding::BASE64;

use super::connection::{Connection, ListenerAddress, ListenerTls, is_closed, is_silent};
use super::http::parse;
use super::runner::Observed;

/// Sends a `CONNECT` to `target` through the proxy at `address`, over `tls`
/// if any, with `credentials`, a username and a password, if any, and reads
/// its answer.
pub(super) fn attempt(
    address: SocketAddr,
    tls: Option<&ListenerTls>,
    target: &str,
    credentials: Option<(&str, &str)>,
) -> Observed {
    let mut connection = match Connection::open(&ListenerAddress::Tcp(address), tls) {
        Ok(connection) => connection,
        Err(error) => {
            return Observed::Unreachable {
                error: error.kind(),
            };
        }
    };
    let authorization = credentials
        .map(|(username, password)| {
            format!(
                "Proxy-Authorization: Basic {}\r\n",
                BASE64.encode(format!("{username}:{password}").as_bytes())
            )
        })
        .unwrap_or_default();
    let head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{authorization}\r\n");
    if let Err(error) = connection.write_all(head.as_bytes()) {
        return if is_closed(&error) {
            Observed::Closed
        } else {
            Observed::Unreachable {
                error: error.kind(),
            }
        };
    }
    let mut answer = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match connection.read(&mut buffer) {
            Ok(0) => return parse(&answer, true).unwrap_or(Observed::Malformed),
            Ok(read) => {
                answer.extend_from_slice(&buffer[..read]);
                if let Some(observed) = tunnel(&answer).or_else(|| parse(&answer, false)) {
                    return observed;
                }
            }
            Err(error) if is_silent(&error) => return Observed::StayedOpen,
            Err(error) if is_closed(&error) => {
                return parse(&answer, true).unwrap_or(Observed::Malformed);
            }
            Err(error) => {
                return Observed::Unreachable {
                    error: error.kind(),
                };
            }
        }
    }
}

/// The tunnel `answer` opened, once its whole head names a `2xx` status:
/// what follows is the tunnel's, never a body.
fn tunnel(answer: &[u8]) -> Option<Observed> {
    let end = answer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&answer[..end]);
    let status = head.split(' ').nth(1)?.parse::<u16>().ok()?;
    (200..300).contains(&status).then_some(Observed::Answered {
        status,
        body_length: 0,
    })
}
