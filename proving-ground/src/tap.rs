//! What a connection's bytes say, read as they cross it, without changing
//! one: [`Wire`] sits on the socket, below TLS, and counts every byte and
//! reads the `ClientHello`; [`Tap`] sits on the plain text, above TLS, and
//! reads each request and answer as framed, HTTP/1.1 or HTTP/2 as the
//! connection's first bytes say, for the observations. A parser that meets
//! bytes it cannot read stops reading that connection, which goes on being
//! served and counted.

mod chunked;
mod http1;
mod http2;

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::client_hello;
use crate::observation::{ByteCount, Connection};

/// The most bytes read for a `ClientHello` before giving up on it.
const CLIENT_HELLO_LIMIT: usize = 64 * 1024;

/// The socket's bytes, counted, with the `ClientHello` read from the first
/// ones of a TLS connection.
pub(crate) struct Wire<S> {
    inner: S,
    connection: Arc<Connection>,
    /// The bytes read so far while the `ClientHello` is incomplete.
    client_hello: Option<Vec<u8>>,
}

impl<S> Wire<S> {
    /// Counts `inner`'s bytes for `connection`, reading a `ClientHello` first
    /// when `tls`.
    pub(crate) fn new(inner: S, connection: Arc<Connection>, tls: bool) -> Self {
        Self {
            inner,
            connection,
            client_hello: tls.then(Vec::new),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Wire<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(context, buffer))?;
        let read = &buffer.filled()[before..];
        this.connection.wire(ByteCount {
            received: read.len() as u64,
            sent: 0,
        });
        if let Some(captured) = &mut this.client_hello {
            captured.extend_from_slice(read);
            match client_hello::parse(captured) {
                Ok(Some(hello)) => {
                    this.connection.client_hello(hello);
                    this.client_hello = None;
                }
                Ok(None) if captured.len() < CLIENT_HELLO_LIMIT && !read.is_empty() => {}
                _ => this.client_hello = None,
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Wire<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let written = ready!(Pin::new(&mut this.inner).poll_write(context, buffer))?;
        this.connection.wire(ByteCount {
            received: 0,
            sent: written as u64,
        });
        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

/// The plain text of a connection, read for its requests and answers.
pub(crate) struct Tap<S> {
    inner: S,
    connection: Arc<Connection>,
    parser: Parser,
}

/// The HTTP/2 connection preface (RFC 9113, section 3.4).
const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

enum Parser {
    /// The first bytes, until they show the preface or not.
    Detecting(Vec<u8>),
    Http1(http1::Exchange),
    Http2(Box<http2::Exchange>),
}

impl<S> Tap<S> {
    pub(crate) fn new(inner: S, connection: Arc<Connection>) -> Self {
        Self {
            inner,
            connection,
            parser: Parser::Detecting(Vec::new()),
        }
    }
}

impl Parser {
    fn received(&mut self, connection: &Connection, bytes: &[u8]) {
        if let Self::Detecting(first) = self {
            first.extend_from_slice(bytes);
            let compared = first.len().min(PREFACE.len());
            if first[..compared] == PREFACE[..compared] && first.len() < PREFACE.len() {
                return;
            }
            let first = std::mem::take(first);
            if first.starts_with(PREFACE) {
                let mut exchange = Box::new(http2::Exchange::new());
                exchange.received(connection, &first[PREFACE.len()..]);
                *self = Self::Http2(exchange);
            } else {
                let mut exchange = http1::Exchange::default();
                exchange.received(connection, &first);
                *self = Self::Http1(exchange);
            }
            return;
        }
        match self {
            Self::Http1(exchange) => exchange.received(connection, bytes),
            Self::Http2(exchange) => exchange.received(connection, bytes),
            Self::Detecting(_) => {}
        }
    }

    fn sent(&mut self, connection: &Connection, bytes: &[u8]) {
        match self {
            Self::Http1(exchange) => exchange.sent(connection, bytes),
            Self::Http2(exchange) => exchange.sent(connection, bytes),
            // Nothing is answered before the first bytes are read.
            Self::Detecting(_) => {}
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Tap<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(context, buffer))?;
        this.parser
            .received(&this.connection, &buffer.filled()[before..]);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tap<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let written = ready!(Pin::new(&mut this.inner).poll_write(context, buffer))?;
        this.parser.sent(&this.connection, &buffer[..written]);
        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

/// Splits `bytes` at `at`, or takes them all.
fn split(bytes: &[u8], at: usize) -> (&[u8], &[u8]) {
    bytes.split_at(at.min(bytes.len()))
}

/// Header values and names as text, any invalid UTF-8 replaced.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;
    use crate::observation::Observations;

    const ORDER: &[u8] = b"POST /orders HTTP/1.1\r\nHost: catalog.test\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nab=c\r\n0\r\n\r\n";
    const PAGE: &[u8] = b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\n\r\n";
    const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";
    const NOT_FOUND: &[u8] = b"HTTP/1.1 404 Not Found\r\ncontent-length: 5\r\n\r\nnope!";
    const FOUND: &[u8] =
        b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n";

    /// The requests recorded from both requests and their answers, each
    /// direction fed in pieces of the given sizes, then the rest at once.
    fn observe(pieces: &[usize]) -> Value {
        let observations = Arc::new(Observations::new(BTreeMap::from([(
            "catalog.test".to_owned(),
            None,
        )])));
        let connection = observations.connection();
        let mut parser = Parser::Detecting(Vec::new());
        let requests = [ORDER, PAGE].concat();
        let answers = [CONTINUE, NOT_FOUND, FOUND].concat();
        let mut rest = requests.as_slice();
        for piece in pieces.iter().copied().chain([usize::MAX]) {
            let (taken, left) = split(rest, piece);
            rest = left;
            parser.received(&connection, taken);
        }
        let mut rest = answers.as_slice();
        for piece in pieces.iter().copied().chain([usize::MAX]) {
            let (taken, left) = split(rest, piece);
            rest = left;
            parser.sent(&connection, taken);
        }
        let document: Value = serde_json::from_slice(&observations.document()).expect("JSON");
        document["requests"]
            .as_array()
            .expect("requests")
            .iter()
            .map(|request| {
                json!({
                    "method": request["method"],
                    "status": request["status"],
                    "bytes": request["bytes"],
                })
            })
            .collect()
    }

    #[test]
    fn http1_answers_are_matched_to_their_requests_in_order() {
        assert_eq!(
            observe(&[]),
            json!([
                {
                    "method": "POST",
                    "status": 404,
                    "bytes": { "received": ORDER.len(), "sent": CONTINUE.len() + NOT_FOUND.len() },
                },
                {
                    "method": "GET",
                    "status": 200,
                    "bytes": { "received": PAGE.len(), "sent": FOUND.len() },
                },
            ])
        );
    }

    proptest! {
        #[test]
        fn http1_messages_read_the_same_however_their_bytes_are_split(
            pieces in proptest::collection::vec(1usize..48, 0..24)
        ) {
            prop_assert_eq!(observe(&pieces), observe(&[]));
        }
    }
}
