//! A request head the browser sends the egress relay, read within a bound
//! (STYLE.md: every buffer is bounded): a `CONNECT` to an authority, or an
//! absolute-form HTTP request (RFC 9112, section 3.2), with the
//! `Proxy-Authorization` the browser gave, and the head the relay sends
//! on for it.

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
    /// The other fields in the order sent, hop-by-hop ones left out.
    fields: Vec<(String, Vec<u8>)>,
    /// The request's path and query, for a plain request.
    path: String,
}

/// Why a head was refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// The connection closed or failed before a whole head came.
    Closed,
    /// The head is longer than [`HEAD_LIMIT`], or not HTTP/1.1 as the
    /// relay takes it.
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
    Ok(Some((
        Head {
            method,
            target,
            authorization,
            fields: kept,
            path,
        },
        length,
    )))
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
    /// so the site answers it as it answers the browser's own request: the
    /// egress relay closes the browser's connection after one answer
    /// instead ([`closing`]).
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

/// The fields of an answer's head that concern only its connection, which
/// the relay's own `Connection: close` replaces.
const CONNECTION_FIELDS: [&str; 3] = ["connection", "keep-alive", "proxy-connection"];

/// Reads a site's answer head to a plain request from `stream`, within
/// [`HEAD_LIMIT`]: the head the browser gets for it, its fields as sent
/// but those of its connection, and `Connection: close`, so the browser
/// sends no other request on a connection whose host the relay judged
/// once; and the bytes that came after it. An interim answer (`1xx`) goes
/// to the browser as it came, ahead of the final one, and a protocol
/// switch (`101`) is the last head read.
///
/// # Errors
///
/// [`Unread`] when no whole head came, or one the relay does not read.
///
/// # Cancel safety
///
/// Not cancel safe: dropping the future loses the bytes read.
pub(crate) async fn closing(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<(Vec<u8>, Vec<u8>), Unread> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0; 2048];
    let mut to_browser = Vec::new();
    loop {
        while let Some((status, length)) = answer_head(&buffer)? {
            let rest = buffer.split_off(length);
            if (100..200).contains(&status) && status != 101 {
                to_browser.extend_from_slice(&buffer);
            } else {
                to_browser.extend_from_slice(&marked_closing(&buffer));
                return Ok((to_browser, rest));
            }
            buffer = rest;
        }
        if buffer.len() >= HEAD_LIMIT {
            return Err(Unread::Malformed);
        }
        let read = stream.read(&mut chunk).await.map_err(|_| Unread::Closed)?;
        if read == 0 {
            return Err(Unread::Closed);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// The status and length of the answer head `buffer` starts with, if whole.
fn answer_head(buffer: &[u8]) -> Result<Option<(u16, usize)>, Unread> {
    let mut fields = [httparse::EMPTY_HEADER; FIELD_LIMIT];
    let mut response = httparse::Response::new(&mut fields);
    match response.parse(buffer) {
        Ok(httparse::Status::Complete(length)) if length <= HEAD_LIMIT => {
            Ok(Some((response.code.ok_or(Unread::Malformed)?, length)))
        }
        Ok(httparse::Status::Complete(_)) | Err(_) => Err(Unread::Malformed),
        Ok(httparse::Status::Partial) => Ok(None),
    }
}

/// `head`, a whole answer head, its connection's fields left out and
/// `Connection: close` added; its other lines kept byte for byte.
fn marked_closing(head: &[u8]) -> Vec<u8> {
    let mut marked = Vec::with_capacity(head.len() + 19);
    let mut lines = head.split_inclusive(|byte| *byte == b'\n');
    if let Some(status) = lines.next() {
        marked.extend_from_slice(status);
    }
    for line in lines {
        if line == b"\r\n" || line == b"\n" {
            break;
        }
        let name = line
            .split(|byte| *byte == b':')
            .next()
            .unwrap_or_default()
            .trim_ascii();
        if !CONNECTION_FIELDS
            .iter()
            .any(|field| name.eq_ignore_ascii_case(field.as_bytes()))
        {
            marked.extend_from_slice(line);
        }
    }
    marked.extend_from_slice(b"Connection: close\r\n\r\n");
    marked
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
