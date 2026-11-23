//! The raw server: answers the connections of the host names whose pages
//! break HTTP on purpose, writing every byte itself rather than through
//! hyper, so a page can end its body before its `Content-Length`, leave out a
//! chunked body's last chunk, reset an HTTP/2 stream after its headers, send
//! a `GOAWAY` below an open stream or close a keep-alive connection as it is
//! reused, as a real site does. It reads and writes through the connection's
//! [`Tap`](crate::tap::Tap), so its requests and answers are observed as any
//! other's, and records for each answer what it sent against what its head
//! declared ([`Delivery`]).
//!
//! It speaks HTTP/1.1 and HTTP/2 (RFC 9112 and RFC 9113) only as far as the
//! clients of the Proving ground need: every answer body fits HTTP/2's
//! default initial window (65,535 bytes), so it never waits for a
//! `WINDOW_UPDATE`, but a filler body, which HTTP/1.1 alone sends.

mod http1;
mod http2;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::observation::{Connection, Delivery, End, Protocol, RequestId};

/// The largest request head or body the raw server reads.
const READ_LIMIT: usize = 1024 * 1024;

/// One request, as the raw server read it.
pub(crate) struct Request {
    pub(crate) protocol: Protocol,
    pub(crate) method: String,
    /// The path and query, as sent.
    pub(crate) target: String,
    /// The `Host` header, or HTTP/2's `:authority`, port included.
    pub(crate) authority: String,
    /// The header fields in the order sent, names in lowercase.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    /// The server name the connection's TLS handshake asked for.
    pub(crate) server_name: Option<String>,
    /// How many requests the connection carried before this one.
    pub(crate) earlier_on_connection: usize,
    /// The request's record in the observations, if it was recorded.
    pub(crate) recorded: Option<RequestId>,
    /// The observer of the request's connection.
    pub(crate) observed: Arc<Connection>,
}

impl Request {
    /// The first value of header `name` (lowercase), if any.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    /// The target's path, without its query.
    pub(crate) fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(path, _)| path)
    }
}

/// An answer's status, header fields and body.
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub(crate) status: u16,
    /// Fields beyond the framing ones the server writes itself, names in
    /// lowercase.
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Vec<u8>,
    /// How HTTP/1.1 delimits the body; HTTP/2 always sends its length.
    pub(crate) framing: Framing,
}

impl Page {
    /// A page of `status` with a body of media type `content_type`.
    pub(crate) fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: vec![("content-type", content_type.to_owned())],
            body: body.into(),
            framing: Framing::Length,
        }
    }

    /// A page of `status` with no `Content-Type`, whose type a client
    /// sniffs from `body` (ADR 0119).
    pub(crate) fn untyped(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            framing: Framing::Length,
        }
    }

    /// An HTML page of `status`.
    pub(crate) fn html(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(status, "text/html; charset=utf-8", body)
    }

    /// A plain-text page of `status`.
    pub(crate) fn text(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(status, "text/plain; charset=utf-8", body)
    }

    /// The page with one more header field.
    #[must_use]
    pub(crate) fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// The page with its HTTP/1.1 body delimited by `framing`.
    #[must_use]
    pub(crate) fn framed(mut self, framing: Framing) -> Self {
        self.framing = framing;
        self
    }
}

/// How an HTTP/1.1 answer's body is delimited (RFC 9112, section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Framing {
    /// By its `Content-Length`.
    Length,
    /// By the chunked transfer coding.
    Chunked,
    /// By the connection's close, with neither.
    Close,
}

/// What the raw server does with one request.
#[derive(Clone, Debug)]
pub(crate) enum Answer {
    /// Sends the page whole.
    Page(Page),
    /// Sends the page's head and the first `sent` bytes of its body, its
    /// framing as whole, then breaks it off as `cut` says.
    Cut { page: Page, sent: usize, cut: Cut },
    /// Answers nothing and refuses the request as `refusal` says.
    Refuse(Refusal),
    /// Sends the page's head, then [`FILLER`] bytes as its body, `length`
    /// of them or, with none, without end until the client goes, so no page
    /// holds the body it sends (ADR 0067's endless and oversized bodies).
    /// Only HTTP/1.1 sends it; over HTTP/2, whose window the raw server
    /// never waits past, the stream is reset.
    Filler { page: Page, length: Option<u64> },
    /// Sends the page's head, its body's length declared, and the first
    /// `sent` bytes of its body, then nothing until the client goes, so a
    /// client reading the whole body waits on it for good. Only HTTP/1.1
    /// sends it; over HTTP/2 the stream is reset.
    Stalled { page: Page, sent: usize },
    /// Waits, then answers.
    Later(Duration, Box<Self>),
}

/// The byte a [`Answer::Filler`] body repeats.
pub(crate) const FILLER: u8 = b'a';

/// How long a [`Cut::Late`] connection stays open after the bytes it sent.
pub(crate) const LATE: Duration = Duration::from_millis(75);

/// How a body is broken off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// The connection is closed with no TLS `close_notify`.
    Connection,
    /// The connection is closed so, but [`LATE`] after the bytes sent, as a
    /// site that stalls mid-body and then drops it: a client reading the
    /// body as it arrives has read them before the body's end (#1732).
    Late,
    /// Over HTTP/2, the stream is reset with `RST_STREAM` (`INTERNAL_ERROR`);
    /// over HTTP/1.1, the connection is closed.
    Stream,
}

/// How a request is refused without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The connection is closed before any byte of an answer.
    Close,
    /// Over HTTP/2, the stream is reset with this error code; over
    /// HTTP/1.1, the connection is closed.
    Reset(ErrorCode),
    /// Over HTTP/2, a `GOAWAY` names as last stream the one the connection
    /// answered last, below this request's, and the connection closes; over
    /// HTTP/1.1, the connection is closed.
    GoAway,
}

/// An HTTP/2 error code (RFC 9113, section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    NoError,
    InternalError,
    RefusedStream,
    EnhanceYourCalm,
    Http11Required,
}

impl ErrorCode {
    fn value(self) -> u32 {
        match self {
            Self::NoError => 0x0,
            Self::InternalError => 0x2,
            Self::RefusedStream => 0x7,
            Self::EnhanceYourCalm => 0xb,
            Self::Http11Required => 0xd,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::NoError => "NO_ERROR",
            Self::InternalError => "INTERNAL_ERROR",
            Self::RefusedStream => "REFUSED_STREAM",
            Self::EnhanceYourCalm => "ENHANCE_YOUR_CALM",
            Self::Http11Required => "HTTP_1_1_REQUIRED",
        }
    }
}

/// The pages the raw server answers with.
pub(crate) trait Pages: Send + Sync + 'static {
    /// The answer to `request`.
    fn answer(&self, request: &Request) -> impl Future<Output = Answer> + Send;
}

/// Serves one connection to its end: HTTP/2 when ALPN chose `h2`, else
/// HTTP/1.1. A connection that breaks concerns only its client.
///
/// # Cancel safety
///
/// Dropping the future drops the connection mid-answer.
pub(crate) async fn serve<S, P>(
    stream: S,
    http2: bool,
    server_name: Option<String>,
    observed: Arc<Connection>,
    pages: &P,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    P: Pages,
{
    let context = Context {
        server_name,
        observed,
    };
    if http2 {
        http2::serve(stream, &context, pages).await;
    } else {
        http1::serve(stream, &context, pages).await;
    }
}

/// What both protocols' loops share.
struct Context {
    server_name: Option<String>,
    observed: Arc<Connection>,
}

impl Context {
    /// Records the delivery of the answer to the request recorded as
    /// `recorded`.
    fn delivered(
        &self,
        recorded: Option<RequestId>,
        declared_length: Option<usize>,
        body_sent: usize,
        end: End,
    ) {
        if let Some(recorded) = recorded {
            self.observed.delivered(
                recorded,
                Delivery {
                    declared_length: declared_length.map(|length| length as u64),
                    body_sent: body_sent as u64,
                    end,
                },
            );
        }
    }
}

/// Resolves `answer`'s waits, in order, and returns what is left to send.
async fn settle(mut answer: Answer) -> Answer {
    while let Answer::Later(wait, then) = answer {
        tokio::time::sleep(wait).await;
        answer = *then;
    }
    answer
}

/// The reason phrase of `status` (RFC 9110, section 15), for HTTP/1.1.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Status",
    }
}
