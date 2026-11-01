//! No Session cookie to a host reached through an invalid-certificate
//! exception (ADR 0042; deep review finding 564-1): the interception
//! withholds them only from the requests the Fetch domain pauses, and a
//! WebSocket handshake, among others, is never paused. The terminating
//! relay sees every decrypted byte of such a host's connection, which it
//! holds to HTTP/1.1, so it reads each request head whole before it goes
//! up, and ends the connection at the first that carries a `Cookie`
//! header, before any byte of it reaches the host. A request's body passes
//! as its framing says, `Content-Length` or chunked, and once a request
//! asks for an upgrade, a WebSocket's, the bytes after its head are no HTTP
//! and pass as they are.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// The protocol a guarded host's connection is held to, as ALPN names it.
pub(crate) const HTTP_1_1: &[u8] = b"http/1.1";

/// The longest request head the relay reads before it ends the connection.
const HEAD_LIMIT: usize = 256 * 1024;

/// The most header fields a request head carries.
const FIELDS: usize = 256;

/// The most bytes read at once.
const CHUNK: usize = 16 * 1024;

/// How a guarded connection's requests went up.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Guarded {
    /// The browser's side ended, every request having gone up: these many
    /// bytes.
    Ended(u64),
    /// A request carried a `Cookie` header, or a head the relay cannot
    /// read, and the connection ends before any byte of it went up.
    Withheld,
}

/// How a request's body is framed, and what follows it.
enum Framing {
    /// None: the next request's head follows.
    Empty,
    /// This many bytes.
    Length(u64),
    /// Chunked (RFC 9112, section 7.1).
    Chunked,
    /// An upgrade, a WebSocket's: what follows is no HTTP.
    Upgraded,
}

/// The browser's bytes read but not yet sent up.
struct Pending<'a, R> {
    from: &'a mut R,
    read: Vec<u8>,
}

impl<R: AsyncRead + Unpin> Pending<'_, R> {
    /// Reads more of the browser's bytes: false once its side ended.
    async fn fill(&mut self) -> io::Result<bool> {
        let mut chunk = vec![0; CHUNK];
        let count = self.from.read(&mut chunk).await?;
        self.read.extend_from_slice(&chunk[..count]);
        Ok(count > 0)
    }

    /// Sends the first `count` bytes read up to `to`.
    async fn send(&mut self, count: usize, to: &mut (impl AsyncWrite + Unpin)) -> io::Result<u64> {
        to.write_all(&self.read[..count]).await?;
        self.read.drain(..count);
        Ok(u64::try_from(count).unwrap_or(u64::MAX))
    }
}

/// Copies the browser's requests from `from` to `to`, the host's side, each
/// head read whole and judged first; `to` is closed at the end.
pub(crate) async fn requests_without_cookies<R, W>(from: &mut R, to: &mut W) -> Guarded
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut pending = Pending {
        from,
        read: Vec::new(),
    };
    let guarded = match guard(&mut pending, to).await {
        Ok(guarded) => guarded,
        Err(_) => Guarded::Ended(0),
    };
    if matches!(guarded, Guarded::Ended(_)) {
        let _ = to.shutdown().await;
    }
    guarded
}

async fn guard<R, W>(pending: &mut Pending<'_, R>, to: &mut W) -> io::Result<Guarded>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut sent = 0_u64;
    loop {
        let (length, framing) = loop {
            match head(&pending.read) {
                Read::Whole { length, framing } => break (length, framing),
                Read::Refused => return Ok(Guarded::Withheld),
                Read::Partial if pending.read.len() > HEAD_LIMIT => {
                    return Ok(Guarded::Withheld);
                }
                Read::Partial => {
                    if !pending.fill().await? {
                        return Ok(Guarded::Ended(sent));
                    }
                }
            }
        };
        sent += pending.send(length, to).await?;
        match framing {
            Framing::Empty => {}
            Framing::Length(length) => sent += body(pending, to, length).await?,
            Framing::Chunked => sent += chunks(pending, to).await?,
            Framing::Upgraded => {
                let left = pending.read.len();
                sent += pending.send(left, to).await?;
                sent += tokio::io::copy(pending.from, to).await?;
                return Ok(Guarded::Ended(sent));
            }
        }
    }
}

/// What the bytes read hold of a request head.
enum Read {
    /// A whole head of `length` bytes, its body framed as `framing`.
    Whole { length: usize, framing: Framing },
    /// Not all of it yet.
    Partial,
    /// One that carries a `Cookie` header, or no HTTP/1.1 head.
    Refused,
}

/// Reads the request head `bytes` start with.
fn head(bytes: &[u8]) -> Read {
    let mut fields = [httparse::EMPTY_HEADER; FIELDS];
    let mut request = httparse::Request::new(&mut fields);
    let length = match request.parse(bytes) {
        Ok(httparse::Status::Complete(length)) => length,
        Ok(httparse::Status::Partial) => return Read::Partial,
        Err(_) => return Read::Refused,
    };
    let named = |name: &'static str| {
        request
            .headers
            .iter()
            .filter(move |field| field.name.eq_ignore_ascii_case(name))
            .map(|field| String::from_utf8_lossy(field.value).into_owned())
    };
    if named("cookie").next().is_some() {
        return Read::Refused;
    }
    let chunked = named("transfer-encoding").any(|coding| {
        coding
            .rsplit(',')
            .next()
            .is_some_and(|last| last.trim().eq_ignore_ascii_case("chunked"))
    });
    let framing = if named("upgrade").next().is_some() || request.method == Some("CONNECT") {
        Framing::Upgraded
    } else if chunked {
        Framing::Chunked
    } else if let Some(declared) = named("content-length").next() {
        match declared.trim().parse() {
            Ok(0) => Framing::Empty,
            Ok(length) => Framing::Length(length),
            Err(_) => return Read::Refused,
        }
    } else {
        Framing::Empty
    };
    Read::Whole { length, framing }
}

/// Sends `length` bytes of a body up.
async fn body<R, W>(pending: &mut Pending<'_, R>, to: &mut W, length: u64) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut left = length;
    while left > 0 {
        if pending.read.is_empty() && !pending.fill().await? {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let count = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(pending.read.len());
        left -= pending.send(count, to).await?;
    }
    Ok(length)
}

/// Sends a chunked body up, to its last chunk and its trailer section.
async fn chunks<R, W>(pending: &mut Pending<'_, R>, to: &mut W) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut sent = 0;
    loop {
        let (line, size) = loop {
            match httparse::parse_chunk_size(&pending.read) {
                Ok(httparse::Status::Complete(read)) => break read,
                Ok(httparse::Status::Partial) if pending.read.len() <= HEAD_LIMIT => {
                    if !pending.fill().await? {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                }
                _ => return Err(io::ErrorKind::InvalidData.into()),
            }
        };
        sent += pending.send(line, to).await?;
        if size == 0 {
            return Ok(sent + trailers(pending, to).await?);
        }
        sent += body(pending, to, size.saturating_add(2)).await?;
    }
}

/// Sends a chunked body's trailer section up, to the empty line that ends
/// it.
async fn trailers<R, W>(pending: &mut Pending<'_, R>, to: &mut W) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut sent = 0;
    loop {
        let Some(end) = pending.read.windows(2).position(|pair| pair == b"\r\n") else {
            if pending.read.len() > HEAD_LIMIT || !pending.fill().await? {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            continue;
        };
        sent += pending.send(end + 2, to).await?;
        if end == 0 {
            return Ok(sent);
        }
    }
}
