//! One attempt on an HTTP/1.1 listener: the runner sends one `GET` with the
//! headers a rule sets, and records the answer's status and body length,
//! or that the listener closed the connection or stayed silent. It writes
//! its own requests, since a rule needs headers an HTTP client would fix,
//! such as a foreign `Host`.

use std::io::{Read as _, Write as _};
use std::net::SocketAddr;

use super::connection::{Connection, ListenerAddress, ListenerTls, is_closed, is_silent};
use super::runner::Observed;

/// A `GET` request.
pub(super) struct Request<'a> {
    /// The path, with its query string if any.
    pub(super) target: String,
    /// The `Host` header.
    pub(super) host: &'a str,
    /// The bearer token of the `Authorization` header, if any.
    pub(super) bearer: Option<&'a str>,
    /// The `Origin` header, if any.
    pub(super) origin: Option<&'a str>,
    /// The `Cookie` header, if any.
    pub(super) cookie: Option<&'a str>,
}

/// Sends `request` to `address`, over `tls` if any, and reads its answer.
pub(super) fn attempt(
    address: SocketAddr,
    tls: Option<&ListenerTls>,
    request: &Request<'_>,
) -> Observed {
    let mut connection = match Connection::open(&ListenerAddress::Tcp(address), tls) {
        Ok(connection) => connection,
        Err(error) => {
            return Observed::Unreachable {
                error: error.kind(),
            };
        }
    };
    let authorization = request
        .bearer
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let origin = request
        .origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    let cookie = request
        .cookie
        .map(|cookie| format!("Cookie: {cookie}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\n{authorization}{origin}{cookie}Connection: close\r\n\r\n",
        request.target, request.host
    );
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
                if let Some(observed) = parse(&answer, false) {
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

/// What `answer` shows once it is complete, or `None` while more is to
/// come. At the connection's end, a partial answer is malformed and none
/// at all is a closed connection.
pub(super) fn parse(answer: &[u8], at_end: bool) -> Option<Observed> {
    let incomplete = || at_end.then_some(Observed::Malformed);
    if answer.is_empty() && at_end {
        return Some(Observed::Closed);
    }
    let Some(head_end) = answer.windows(4).position(|window| window == b"\r\n\r\n") else {
        return incomplete();
    };
    let head = String::from_utf8_lossy(&answer[..head_end]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|status| status.parse::<u16>().ok());
    let Some(status) = status else {
        return Some(Observed::Malformed);
    };
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Some(Observed::Malformed);
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        }
    }
    let body = &answer[head_end + 4..];
    let body_length = if chunked {
        chunked_length(body)
    } else if let Some(length) = length {
        (body.len() >= length).then_some(length)
    } else if at_end {
        Some(body.len())
    } else {
        None
    };
    match body_length {
        Some(body_length) => Some(Observed::Answered {
            status,
            body_length,
        }),
        None => incomplete(),
    }
}

/// The length of a complete chunked body, or `None` while it is cut.
fn chunked_length(mut body: &[u8]) -> Option<usize> {
    let mut total = 0;
    loop {
        let line_end = body.windows(2).position(|window| window == b"\r\n")?;
        let size = std::str::from_utf8(&body[..line_end]).ok()?;
        let size = size.split(';').next()?.trim();
        let size = usize::from_str_radix(size, 16).ok()?;
        if size == 0 {
            return Some(total);
        }
        body = body.get(line_end + 2 + size + 2..)?;
        total += size;
    }
}
