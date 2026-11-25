//! A request head the browser sends the egress relay, read within a bound
//! (STYLE.md: every buffer is bounded): a `CONNECT` to an authority, or an
//! absolute-form HTTP request (RFC 9112, section 3.2), with the
//! `Proxy-Authorization` the browser gave, how its body ends, and the head
//! the relay sends on for it; and the answer heads a plain request gets,
//! read for where each answer ends (RFC 9112, section 6.3).

use tokio::io::{AsyncRead, AsyncReadExt as _};

/// The longest request head the relay reads; a longer one is refused.
pub(crate) const HEAD_LIMIT: usize = 16 * 1024;

/// The most header fields a head may carry.
const FIELD_LIMIT: usize = 128;

/// The header fields that concern only the connection to the relay, never
/// sent on (RFC 9110, section 7.6.1), and the browser's credentials for
/// the relay.
const HOP_BY_HOP: [&str; 5] = [
    "connection",
    "keep-alive",
    "proxy-authorization",
    "proxy-connection",
    "te",
];

/// Where a request asks the relay to go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A tunnel to `host` on `port`.
    Tunnel { host: String, port: u16 },
    /// A plain HTTP request to `url`, whose host and port are `host` and
    /// `port`.
    Absolute {
        url: String,
        host: String,
        port: u16,
    },
}

impl Target {
    /// Its host, an IPv6 address without brackets.
    pub(crate) fn host(&self) -> &str {
        match self {
            Self::Tunnel { host, .. } | Self::Absolute { host, .. } => host,
        }
    }

    /// Its port.
    pub(crate) fn port(&self) -> u16 {
        match self {
            Self::Tunnel { port, .. } | Self::Absolute { port, .. } => *port,
        }
    }

    /// Its authority, `host:port`, an IPv6 address in brackets.
    pub(crate) fn authority(&self) -> String {
        authority(self.host(), self.port())
    }

    /// The URL the egress policy judges it as: a plain request's own, and
    /// for a tunnel the URL of its authority, `http:` on port 80 and
    /// `https:` on any other, since a tunnel names no scheme.
    pub(crate) fn url(&self) -> String {
        match self {
            Self::Absolute { url, .. } => url.clone(),
            Self::Tunnel { port: 80, .. } => format!("http://{}/", self.authority()),
            Self::Tunnel { .. } => format!("https://{}/", self.authority()),
        }
    }
}

/// `host:port`, an IPv6 address in brackets.
pub(crate) fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// A request head as the relay read it.
#[derive(Debug)]
pub(crate) struct Head {
    pub(crate) method: String,
    pub(crate) target: Target,
    /// The `Proxy-Authorization` field's value, if the browser gave one.
    pub(crate) authorization: Option<String>,
    /// Where a plain request's body ends: none for a tunnel's head.
    pub(crate) body: Body,
    /// Whether the browser asked to close its connection after this
    /// request's answer.
    pub(crate) closes: bool,
    /// The other fields in the order sent, hop-by-hop ones left out.
    fields: Vec<(String, Vec<u8>)>,
    /// The request's path and query, for a plain request.
    path: String,
}

/// Where a message's body ends (RFC 9112, section 6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Body {
    /// After these many bytes: none for zero.
    Length(u64),
    /// After its last chunk and its trailer fields.
    Chunked,
    /// When its connection closes, which only an answer's may.
    UntilClose,
}

/// Why a head was refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// The connection closed or failed before a whole head came.
    Closed,
    /// The head is longer than [`HEAD_LIMIT`], not HTTP/1.1 as the relay
    /// takes it, or leaves where its body ends in doubt.
    Malformed,
}

/// Reads one request head from `stream`: the head, and the bytes that
/// came after it.
///
/// # Errors
///
/// [`Unread`] when no whole head came, or one the relay does not take.
///
/// # Cancel safety
///
/// Not cancel safe: dropping the future loses the bytes read.
pub(crate) async fn read(stream: &mut (impl AsyncRead + Unpin)) -> Result<(Head, Vec<u8>), Unread> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0; 2048];
    loop {
        let read = stream.read(&mut chunk).await.map_err(|_| Unread::Closed)?;
        if read == 0 {
            return Err(Unread::Closed);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(parsed) = parse(&buffer)? {
            let (head, length) = parsed;
            return Ok((head, buffer.split_off(length)));
        }
        if buffer.len() >= HEAD_LIMIT {
            return Err(Unread::Malformed);
        }
    }
}

/// Parses `buffer`'s head, if whole: the head and its length.
pub(crate) fn parse(buffer: &[u8]) -> Result<Option<(Head, usize)>, Unread> {
    let mut fields = [httparse::EMPTY_HEADER; FIELD_LIMIT];
    let mut request = httparse::Request::new(&mut fields);
    let length = match request.parse(buffer) {
        Ok(httparse::Status::Complete(length)) => length,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(_) => return Err(Unread::Malformed),
    };
    if length > HEAD_LIMIT || request.version != Some(1) {
        return Err(Unread::Malformed);
    }
    let method = request.method.ok_or(Unread::Malformed)?.to_owned();
    let target = request.path.ok_or(Unread::Malformed)?;
    let (target, path) = if method == "CONNECT" {
        (tunnel(target)?, String::new())
    } else {
        absolute(target)?
    };
    let mut authorization = None;
    let mut kept = Vec::with_capacity(request.headers.len());
    for field in request.headers.iter() {
        let name = field.name.to_ascii_lowercase();
        if name == "proxy-authorization" {
            authorization = std::str::from_utf8(field.value).ok().map(str::to_owned);
        }
        if !HOP_BY_HOP.contains(&name.as_str()) {
            kept.push((field.name.to_owned(), field.value.to_vec()));
        }
    }
    let (body, closes) = match &target {
        Target::Tunnel { .. } => (Body::Length(0), false),
        Target::Absolute { .. } => (
            request_body(request.headers)?,
            has_token(request.headers, "connection", "close")
                || has_token(request.headers, "proxy-connection", "close"),
        ),
    };
    Ok(Some((
        Head {
            method,
            target,
            authorization,
            body,
            closes,
            fields: kept,
            path,
        },
        length,
    )))
}

/// Where a request's body ends: in chunks when its last transfer coding is
/// `chunked`, else after its one `Content-Length`, else none. A request
/// whose end is in doubt, with both fields, another last coding, `chunked`
/// twice or lengths that differ, is refused, so no byte of one request is
/// ever read as the next (RFC 9112, sections 6.1 and 6.3).
fn request_body(fields: &[httparse::Header<'_>]) -> Result<Body, Unread> {
    let length = content_length(fields)?;
    let codings = codings(fields);
    if codings.is_empty() {
        return Ok(Body::Length(length.unwrap_or(0)));
    }
    let chunked = codings
        .iter()
        .filter(|coding| coding.eq_ignore_ascii_case(b"chunked"))
        .count();
    let last_chunked = codings
        .last()
        .is_some_and(|coding| coding.eq_ignore_ascii_case(b"chunked"));
    if length.is_some() || chunked != 1 || !last_chunked {
        return Err(Unread::Malformed);
    }
    Ok(Body::Chunked)
}

/// The one length the `Content-Length` fields give, if any: each value
/// decimal digits alone, and all of them the same.
fn content_length(fields: &[httparse::Header<'_>]) -> Result<Option<u64>, Unread> {
    let mut length = None;
    for field in fields {
        if !field.name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        for value in field.value.split(|byte| *byte == b',') {
            let value = value.trim_ascii();
            if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(Unread::Malformed);
            }
            let value = std::str::from_utf8(value)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(Unread::Malformed)?;
            if length.is_some_and(|length| length != value) {
                return Err(Unread::Malformed);
            }
            length = Some(value);
        }
    }
    Ok(length)
}

/// The transfer codings of `fields`, in the order applied.
fn codings<'a>(fields: &[httparse::Header<'a>]) -> Vec<&'a [u8]> {
    fields
        .iter()
        .filter(|field| field.name.eq_ignore_ascii_case("transfer-encoding"))
        .flat_map(|field| field.value.split(|byte| *byte == b','))
        .map(<[u8]>::trim_ascii)
        .filter(|coding| !coding.is_empty())
        .collect()
}

/// Whether a `name` field of `fields` lists `token`.
fn has_token(fields: &[httparse::Header<'_>], name: &str, token: &str) -> bool {
    fields
        .iter()
        .filter(|field| field.name.eq_ignore_ascii_case(name))
        .flat_map(|field| field.value.split(|byte| *byte == b','))
        .any(|listed| listed.trim_ascii().eq_ignore_ascii_case(token.as_bytes()))
}

/// A `CONNECT`'s authority-form target, `host:port`.
fn tunnel(target: &str) -> Result<Target, Unread> {
    let url = url::Url::parse(&format!("http://{target}/")).map_err(|_| Unread::Malformed)?;
    let host = host_of(&url)?;
    let port = target
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .ok_or(Unread::Malformed)?;
    Ok(Target::Tunnel { host, port })
}

/// A plain request's absolute-form `http:` target, and its path and query.
fn absolute(target: &str) -> Result<(Target, String), Unread> {
    let url = url::Url::parse(target).map_err(|_| Unread::Malformed)?;
    if url.scheme() != "http" {
        return Err(Unread::Malformed);
    }
    let host = host_of(&url)?;
    let port = url.port_or_known_default().ok_or(Unread::Malformed)?;
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok((
        Target::Absolute {
            url: url.to_string(),
            host,
            port,
        },
        path,
    ))
}

/// The host of `url`, an IPv6 address without brackets.
fn host_of(url: &url::Url) -> Result<String, Unread> {
    match url.host() {
        Some(url::Host::Ipv6(address)) => Ok(address.to_string()),
        Some(host) => Ok(host.to_string()),
        None => Err(Unread::Malformed),
    }
}

impl Head {
    /// The head sent on for a plain request: to the site in origin form,
    /// or to an upstream HTTP proxy in absolute form with `authorization`,
    /// its credentials; the hop-by-hop fields left out and nothing added,
    /// so the site answers it as it answers the browser's own request.
    pub(crate) fn as_sent(&self, to_proxy: bool, authorization: Option<&str>) -> Vec<u8> {
        let target = match (&self.target, to_proxy) {
            (Target::Absolute { url, .. }, true) => url.as_str(),
            _ => self.path.as_str(),
        };
        let mut head = format!("{} {target} HTTP/1.1\r\n", self.method).into_bytes();
        for (name, value) in &self.fields {
            head.extend_from_slice(name.as_bytes());
            head.extend_from_slice(b": ");
            head.extend_from_slice(value);
            head.extend_from_slice(b"\r\n");
        }
        if let Some(authorization) = authorization {
            head.extend_from_slice(b"Proxy-Authorization: ");
            head.extend_from_slice(authorization.as_bytes());
            head.extend_from_slice(b"\r\n");
        }
        head.extend_from_slice(b"\r\n");
        head
    }
}

/// An answer head to a plain request, as the relay reads it to find where
/// the answer ends; its bytes go to the browser as they came.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Answered {
    pub(crate) status: u16,
    /// The head's length, its blank line included.
    pub(crate) length: usize,
    /// Where the answer's body ends.
    pub(crate) body: Body,
    /// Whether the site closes the connection after this answer.
    pub(crate) closes: bool,
}

/// The answer head `buffer` starts with, if whole, to a `HEAD` request when
/// `to_head` (RFC 9112, section 6.3): no body after a `HEAD`, an interim
/// (`1xx`), a `204` or a `304`; chunks when the last transfer coding is
/// `chunked`; the site's close for another coding; the one
/// `Content-Length`; else the site's close. An answer whose length is in
/// doubt is refused, and one with both a coding and a length is read by
/// its coding and its connection closed after it, as an intermediary must
/// (RFC 9112, section 6.1).
///
/// # Errors
///
/// [`Unread::Malformed`] for a head longer than [`HEAD_LIMIT`], not HTTP,
/// or whose lengths differ.
pub(crate) fn answered(buffer: &[u8], to_head: bool) -> Result<Option<Answered>, Unread> {
    let mut fields = [httparse::EMPTY_HEADER; FIELD_LIMIT];
    let mut response = httparse::Response::new(&mut fields);
    let length = match response.parse(buffer) {
        Ok(httparse::Status::Complete(length)) if length <= HEAD_LIMIT => length,
        Ok(httparse::Status::Complete(_)) | Err(_) => return Err(Unread::Malformed),
        Ok(httparse::Status::Partial) => return Ok(None),
    };
    let status = response.code.ok_or(Unread::Malformed)?;
    let fields = &*response.headers;
    let mut closes = has_token(fields, "connection", "close")
        || (response.version == Some(0) && !has_token(fields, "connection", "keep-alive"));
    let content_length = content_length(fields)?;
    let codings = codings(fields);
    let body = if to_head || (100..200).contains(&status) || status == 204 || status == 304 {
        Body::Length(0)
    } else if let Some(last) = codings.last() {
        closes |= content_length.is_some();
        if last.eq_ignore_ascii_case(b"chunked") {
            Body::Chunked
        } else {
            Body::UntilClose
        }
    } else {
        content_length.map_or(Body::UntilClose, Body::Length)
    };
    closes |= body == Body::UntilClose;
    Ok(Some(Answered {
        status,
        length,
        body,
        closes,
    }))
}

/// An answer head an upstream proxy sent to a `CONNECT`, read within the
/// same bound: its status, and the head the browser gets for it, with no
/// body and the connection closed.
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) to_browser: Vec<u8>,
}

/// Reads an upstream proxy's answer head to a `CONNECT`, nothing past it.
///
/// # Errors
///
/// [`Unread`] when no whole head came.
pub(crate) async fn answer(stream: &mut (impl AsyncRead + Unpin)) -> Result<Answer, Unread> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= HEAD_LIMIT {
            return Err(Unread::Malformed);
        }
        let byte = stream.read_u8().await.map_err(|_| Unread::Closed)?;
        head.push(byte);
    }
    let mut fields = [httparse::EMPTY_HEADER; FIELD_LIMIT];
    let mut response = httparse::Response::new(&mut fields);
    match response.parse(&head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err(Unread::Malformed),
    }
    let status = response.code.ok_or(Unread::Malformed)?;
    let mut to_browser = format!(
        "HTTP/1.1 {status} {}\r\n",
        response.reason.unwrap_or_default()
    )
    .into_bytes();
    for field in response.headers.iter() {
        let name = field.name.to_ascii_lowercase();
        let dropped = HOP_BY_HOP.contains(&name.as_str())
            || name == "content-length"
            || name == "transfer-encoding";
        if !dropped {
            to_browser.extend_from_slice(field.name.as_bytes());
            to_browser.extend_from_slice(b": ");
            to_browser.extend_from_slice(field.value);
            to_browser.extend_from_slice(b"\r\n");
        }
    }
    to_browser.extend_from_slice(b"Content-Length: 0\r\nConnection: close\r\n\r\n");
    Ok(Answer { status, to_browser })
}
