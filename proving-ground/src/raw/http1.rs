//! The raw server's HTTP/1.1 (RFC 9112): requests read one after another on
//! a keep-alive connection, each answer written as its page and framing say.

use std::fmt::Write as _;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::{Answer, Context, Cut, Framing, Page, Pages, READ_LIMIT, Request, reason, settle};
use crate::observation::{End, Protocol};

/// Serves requests until the client closes, asks to close, or an answer
/// closes.
pub(super) async fn serve<S, P>(mut stream: S, context: &Context, pages: &P)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    P: Pages,
{
    let mut buffered = Vec::new();
    let mut earlier = 0;
    loop {
        let Some(head) = read_head(&mut stream, &mut buffered).await else {
            return;
        };
        let Some(mut request) = parse_head(&head) else {
            let _ = write(&mut stream, &Page::text(400, "malformed request")).await;
            return;
        };
        let Some(body) = read_body(&mut stream, &mut buffered, &request).await else {
            return;
        };
        request.body = body;
        request.server_name.clone_from(&context.server_name);
        request.earlier_on_connection = earlier;
        request.recorded = context.observed.latest_request(0);
        earlier += 1;
        let closes = request
            .header("connection")
            .is_some_and(|value| value.eq_ignore_ascii_case("close"));
        match settle(pages.answer(&request).await).await {
            Answer::Page(page) => {
                let declared = (page.framing == Framing::Length).then_some(page.body.len());
                if write(&mut stream, &page).await.is_err() {
                    return;
                }
                context.delivered(request.recorded, declared, page.body.len(), End::Complete);
                let page_closes = page.headers.iter().any(|(name, value)| {
                    *name == "connection" && value.eq_ignore_ascii_case("close")
                });
                if closes || page_closes || page.framing == Framing::Close {
                    let _ = stream.shutdown().await;
                    return;
                }
            }
            Answer::Cut { page, sent, cut } => {
                let declared = (page.framing == Framing::Length).then_some(page.body.len());
                let sent = sent.min(page.body.len());
                let body = &page.body[..sent];
                let mut message = head_bytes(&page).into_bytes();
                if page.framing == Framing::Chunked && !body.is_empty() {
                    message.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
                    message.extend_from_slice(body);
                    message.extend_from_slice(b"\r\n");
                } else {
                    message.extend_from_slice(body);
                }
                let _ = send(&mut stream, &message).await;
                // HTTP/1.1 has no stream to reset: either cut closes the
                // connection.
                let (Cut::Connection | Cut::Stream) = cut;
                context.delivered(request.recorded, declared, sent, End::ConnectionClosed);
                // Dropped without a TLS `close_notify`: the client sees the
                // connection end mid-message.
                return;
            }
            Answer::Refuse(_) => {
                context.delivered(request.recorded, None, 0, End::ConnectionClosed);
                return;
            }
            Answer::Later(..) => unreachable!("invariant: `settle` resolves every wait"),
        }
    }
}

/// Reads up to the end of a request head; `None` at the connection's end or
/// past the read limit.
async fn read_head<S: AsyncRead + Unpin>(
    stream: &mut S,
    buffered: &mut Vec<u8>,
) -> Option<Vec<u8>> {
    loop {
        if let Some(at) = buffered.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = buffered.split_off(at + 4);
            return Some(std::mem::replace(buffered, rest));
        }
        if buffered.len() > READ_LIMIT || !fill(stream, buffered).await {
            return None;
        }
    }
}

/// Reads more bytes; `false` at the connection's end or on an error.
async fn fill<S: AsyncRead + Unpin>(stream: &mut S, buffered: &mut Vec<u8>) -> bool {
    let mut chunk = [0; 16 * 1024];
    match stream.read(&mut chunk).await {
        Ok(0) | Err(_) => false,
        Ok(read) => {
            buffered.extend_from_slice(&chunk[..read]);
            true
        }
    }
}

/// The request a head holds, without its body.
fn parse_head(head: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut start = lines.next()?.split(' ');
    let method = start.next()?.to_owned();
    let target = start.next()?.to_owned();
    if !start.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let authority = headers
        .iter()
        .find(|(name, _)| name == "host")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    Some(Request {
        protocol: Protocol::Http1,
        method,
        target,
        authority,
        headers,
        body: Vec::new(),
        server_name: None,
        earlier_on_connection: 0,
        recorded: None,
    })
}

/// Reads a request's body, by its `Content-Length` or chunked; `None` when
/// the connection ends first or the body is past the read limit.
async fn read_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    buffered: &mut Vec<u8>,
    request: &Request,
) -> Option<Vec<u8>> {
    if request
        .header("transfer-encoding")
        .is_some_and(|coding| coding.eq_ignore_ascii_case("chunked"))
    {
        return read_chunked(stream, buffered).await;
    }
    let length: usize = request
        .header("content-length")
        .map_or(Some(0), |length| length.parse().ok())?;
    if length > READ_LIMIT {
        return None;
    }
    while buffered.len() < length {
        if !fill(stream, buffered).await {
            return None;
        }
    }
    let rest = buffered.split_off(length);
    Some(std::mem::replace(buffered, rest))
}

/// Reads a chunked body to its last chunk, ignoring any trailer fields.
async fn read_chunked<S: AsyncRead + Unpin>(
    stream: &mut S,
    buffered: &mut Vec<u8>,
) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = loop {
            if let Some(at) = buffered.windows(2).position(|window| window == b"\r\n") {
                break at;
            }
            if buffered.len() > READ_LIMIT || !fill(stream, buffered).await {
                return None;
            }
        };
        let line = std::str::from_utf8(&buffered[..line_end]).ok()?;
        let size = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        buffered.drain(..line_end + 2);
        if size == 0 {
            // The trailer section ends with an empty line.
            while !buffered.starts_with(b"\r\n") {
                let at = buffered.windows(2).position(|window| window == b"\r\n");
                match at {
                    Some(at) => {
                        buffered.drain(..at + 2);
                    }
                    None if fill(stream, buffered).await => {}
                    None => return None,
                }
            }
            buffered.drain(..2);
            return Some(body);
        }
        if body.len() + size > READ_LIMIT {
            return None;
        }
        while buffered.len() < size + 2 {
            if !fill(stream, buffered).await {
                return None;
            }
        }
        body.extend_from_slice(&buffered[..size]);
        buffered.drain(..size + 2);
    }
}

/// An answer's head: its status line, its page's fields and its framing's.
fn head_bytes(page: &Page) -> String {
    let mut head = format!("HTTP/1.1 {} {}\r\n", page.status, reason(page.status));
    for (name, value) in &page.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    match page.framing {
        Framing::Length => {
            let _ = write!(head, "content-length: {}\r\n", page.body.len());
        }
        Framing::Chunked => head.push_str("transfer-encoding: chunked\r\n"),
        Framing::Close => head.push_str("connection: close\r\n"),
    }
    head.push_str("\r\n");
    head
}

/// Writes a whole answer.
async fn write<S: AsyncWrite + Unpin>(stream: &mut S, page: &Page) -> std::io::Result<()> {
    let mut message = head_bytes(page).into_bytes();
    match page.framing {
        Framing::Length | Framing::Close => message.extend_from_slice(&page.body),
        Framing::Chunked => {
            // Two chunks, so the body is seen to be chunked.
            let middle = page.body.len() / 2;
            for part in [&page.body[..middle], &page.body[middle..]] {
                if !part.is_empty() {
                    message.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
                    message.extend_from_slice(part);
                    message.extend_from_slice(b"\r\n");
                }
            }
            message.extend_from_slice(b"0\r\n\r\n");
        }
    }
    send(stream, &message).await
}

/// Writes `bytes` and flushes them.
async fn send<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> std::io::Result<()> {
    stream.write_all(bytes).await?;
    stream.flush().await
}
