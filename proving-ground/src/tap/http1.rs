//! An HTTP/1.1 connection's requests and answers as framed (RFC 9112):
//! each request's head and body, and each answer's, matched in order.

use std::collections::VecDeque;

use super::chunked::{Chunked, Consumed};
use super::{split, text};
use crate::observation::{ByteCount, Connection, Part, Protocol, RequestHead, RequestId};

/// The longest head read; a longer one stops the parser.
const HEAD_LIMIT: usize = 64 * 1024;

/// Both directions of one HTTP/1.1 connection.
#[derive(Default)]
pub(super) struct Exchange {
    incoming: Direction,
    outgoing: Direction,
    /// The requests not yet answered, oldest first.
    waiting: VecDeque<Waiting>,
}

struct Waiting {
    request: Option<RequestId>,
    /// A `HEAD` request's answer has no body.
    head: bool,
}

/// Where one direction is.
enum Direction {
    /// Reading a head, these bytes so far.
    Head(Vec<u8>),
    /// Reading the body of the message whose head came last.
    Body(Option<RequestId>, Body),
    /// An answer's body that runs to the connection's end.
    Unbounded(Option<RequestId>),
    /// Bytes the parser does not read: after a malformed message, or after
    /// an answer switching protocols.
    Opaque(Option<RequestId>),
}

impl Default for Direction {
    fn default() -> Self {
        Self::Head(Vec::new())
    }
}

enum Body {
    Length(u64),
    Chunked(Chunked),
}

impl Exchange {
    pub(super) fn received(&mut self, connection: &Connection, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match &mut self.incoming {
                Direction::Head(head) => {
                    let Some((used, complete)) = read_head(head, bytes) else {
                        self.incoming = Direction::Opaque(None);
                        return;
                    };
                    bytes = &bytes[used..];
                    if let Some(complete) = complete {
                        self.request(connection, &complete);
                    }
                }
                Direction::Body(request, body) => {
                    let request = *request;
                    let consumed = body.consume(bytes);
                    count_body(connection, request, &consumed, true);
                    bytes = &bytes[consumed.used..];
                    if consumed.ended {
                        self.incoming = Direction::default();
                    }
                }
                Direction::Unbounded(request) => {
                    count(connection, *request, bytes.len(), true, Part::Body);
                    return;
                }
                Direction::Opaque(request) => {
                    count(connection, *request, bytes.len(), true, Part::Framing);
                    return;
                }
            }
        }
    }

    /// Records a request whose head is complete, and reads its body next.
    fn request(&mut self, connection: &Connection, head: &[u8]) {
        let mut lines = head.split(|byte| *byte == b'\n').map(trim_cr);
        let start = text(lines.next().unwrap_or_default());
        let mut parts = start.split(' ');
        let (Some(method), Some(target), Some(_version)) =
            (parts.next(), parts.next(), parts.next())
        else {
            self.incoming = Direction::Opaque(None);
            return;
        };
        let headers = fields(lines);
        let authority = value(&headers, "host").unwrap_or_default().to_owned();
        let body = framing(&headers);
        let is_head = method == "HEAD";
        let request = connection.request(RequestHead {
            protocol: Protocol::Http1,
            method: method.to_owned(),
            target: target.to_owned(),
            authority,
            pseudo_header_order: None,
            headers,
            http2: None,
        });
        count(connection, request, head.len(), true, Part::Framing);
        self.waiting.push_back(Waiting {
            request,
            head: is_head,
        });
        self.incoming = match body {
            Some(body) => Direction::Body(request, body),
            None => Direction::default(),
        };
    }

    pub(super) fn sent(&mut self, connection: &Connection, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match &mut self.outgoing {
                Direction::Head(head) => {
                    let Some((used, complete)) = read_head(head, bytes) else {
                        self.outgoing = Direction::Opaque(None);
                        return;
                    };
                    bytes = &bytes[used..];
                    if let Some(complete) = complete {
                        self.answer(connection, &complete);
                    }
                }
                Direction::Body(request, body) => {
                    let request = *request;
                    let consumed = body.consume(bytes);
                    count_body(connection, request, &consumed, false);
                    bytes = &bytes[consumed.used..];
                    if consumed.ended {
                        self.outgoing = Direction::default();
                    }
                }
                Direction::Unbounded(request) => {
                    count(connection, *request, bytes.len(), false, Part::Body);
                    return;
                }
                Direction::Opaque(request) => {
                    count(connection, *request, bytes.len(), false, Part::Framing);
                    return;
                }
            }
        }
    }

    /// Matches an answer whose head is complete to the oldest request
    /// waiting, and reads its body next. An interim answer (1xx) keeps the
    /// request waiting for its final one.
    fn answer(&mut self, connection: &Connection, head: &[u8]) {
        let mut lines = head.split(|byte| *byte == b'\n').map(trim_cr);
        let start = text(lines.next().unwrap_or_default());
        let status = start
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok());
        let request = self.waiting.front().and_then(|waiting| waiting.request);
        count(connection, request, head.len(), false, Part::Framing);
        let Some(status) = status else {
            self.outgoing = Direction::Opaque(request);
            return;
        };
        if status == 101 {
            self.outgoing = Direction::Opaque(request);
            return;
        }
        if (100..200).contains(&status) {
            return;
        }
        let waiting = self.waiting.pop_front();
        if let Some(request) = request {
            connection.answered(request, status);
        }
        let headers = fields(lines);
        let bodiless =
            waiting.is_some_and(|waiting| waiting.head) || status == 204 || status == 304;
        self.outgoing = if bodiless {
            Direction::default()
        } else {
            match framing(&headers) {
                Some(body) => Direction::Body(request, body),
                None if value(&headers, "content-length").is_some() => Direction::default(),
                // No length: the body runs to the connection's end.
                None => Direction::Unbounded(request),
            }
        };
    }
}

impl Body {
    fn consume(&mut self, bytes: &[u8]) -> Consumed {
        match self {
            Self::Length(left) => {
                let (taken, _) = split(bytes, usize::try_from(*left).unwrap_or(usize::MAX));
                *left -= taken.len() as u64;
                Consumed {
                    used: taken.len(),
                    data: taken.len(),
                    ended: *left == 0,
                }
            }
            Self::Chunked(chunked) => chunked.consume(bytes),
        }
    }
}

/// Adds `bytes` to `head` up to the end of the head: how many it used, and
/// the whole head (its final empty line included) once complete; `None`
/// when the head grows past the limit.
fn read_head(head: &mut Vec<u8>, bytes: &[u8]) -> Option<(usize, Option<Vec<u8>>)> {
    let searched_from = head.len().saturating_sub(3);
    head.extend_from_slice(bytes);
    if let Some(at) = head[searched_from..]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
    {
        let end = searched_from + at + 4;
        let used = bytes.len() - (head.len() - end);
        head.truncate(end);
        return Some((used, Some(std::mem::take(head))));
    }
    (head.len() <= HEAD_LIMIT).then_some((bytes.len(), None))
}

fn trim_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// A head's header fields, in order, names as sent and values without
/// their surrounding whitespace.
fn fields<'a>(lines: impl Iterator<Item = &'a [u8]>) -> Vec<(String, String)> {
    lines
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let at = line.iter().position(|byte| *byte == b':')?;
            Some((text(&line[..at]), text(&line[at + 1..]).trim().to_owned()))
        })
        .collect()
}

fn value<'a>(headers: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.as_str())
}

/// How a message's body is framed; `None` when it has none or runs to the
/// connection's end.
fn framing(headers: &[(String, String)]) -> Option<Body> {
    let chunked = value(headers, "transfer-encoding").is_some_and(|codings| {
        codings
            .rsplit(',')
            .next()
            .is_some_and(|last| last.trim().eq_ignore_ascii_case("chunked"))
    });
    if chunked {
        return Some(Body::Chunked(Chunked::default()));
    }
    value(headers, "content-length")
        .and_then(|length| length.parse::<u64>().ok())
        .filter(|length| *length > 0)
        .map(Body::Length)
}

/// Counts what a body consumed: its data, and the framing around it.
fn count_body(
    connection: &Connection,
    request: Option<RequestId>,
    consumed: &Consumed,
    received: bool,
) {
    count(connection, request, consumed.data, received, Part::Body);
    count(
        connection,
        request,
        consumed.used - consumed.data,
        received,
        Part::Framing,
    );
}

/// Counts `length` bytes of a request's messages, received or sent.
fn count(
    connection: &Connection,
    request: Option<RequestId>,
    length: usize,
    received: bool,
    part: Part,
) {
    if let Some(request) = request
        && length > 0
    {
        let length = length as u64;
        connection.request_bytes(
            request,
            if received {
                ByteCount {
                    received: length,
                    sent: 0,
                }
            } else {
                ByteCount {
                    received: 0,
                    sent: length,
                }
            },
            part,
        );
    }
}
