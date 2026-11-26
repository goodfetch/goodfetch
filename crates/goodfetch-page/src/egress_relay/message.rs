//! The plain HTTP/1.1 messages on one end of a connection the egress relay
//! carries, read one after another (RFC 9112): each head parsed from the
//! bytes read, its body passed on as it came and ended as its framing
//! says, and the bytes past it kept for the next message, so no byte of
//! one message is read as the next. Every line read is bounded (STYLE.md).

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::head::{self, Answered, Body, HEAD_LIMIT, Head, Unread};

/// The longest line of a chunked body the relay reads: a chunk's size with
/// its extensions, or a trailer field.
const LINE_LIMIT: usize = 4096;

/// What the relay reads at most at once.
const READ_SIZE: usize = 16 * 1024;

/// The bytes read from one end of a connection, those past the messages
/// read kept for the next.
pub(crate) struct Incoming<R> {
    stream: R,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> Incoming<R> {
    /// `stream`, `read` the bytes already read from it.
    pub(crate) fn new(stream: R, read: Vec<u8>) -> Self {
        Self {
            stream,
            buffer: read,
        }
    }

    /// The stream and the bytes read from it but not yet taken.
    pub(crate) fn into_parts(self) -> (R, Vec<u8>) {
        (self.stream, self.buffer)
    }

    /// Reads more: false at the stream's end.
    ///
    /// # Errors
    ///
    /// The stream's.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: a read either completes into the buffer or takes
    /// nothing.
    pub(crate) async fn fill(&mut self) -> io::Result<bool> {
        self.buffer.reserve(READ_SIZE);
        let read = self.stream.read_buf(&mut self.buffer).await?;
        Ok(read > 0)
    }

    /// The next request head.
    ///
    /// # Errors
    ///
    /// [`Unread`] as [`head::read`] says.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: the bytes read stay for the next call.
    pub(crate) async fn request(&mut self) -> Result<Head, Unread> {
        loop {
            if let Some((head, length)) = head::parse(&self.buffer)? {
                self.buffer.drain(..length);
                return Ok(head);
            }
            if self.buffer.len() >= HEAD_LIMIT {
                return Err(Unread::Malformed);
            }
            if !self.fill().await.map_err(|_| Unread::Closed)? {
                return Err(Unread::Closed);
            }
        }
    }

    /// The next answer head to a `HEAD` request when `to_head`: its bytes
    /// as they came, and how it is framed.
    ///
    /// # Errors
    ///
    /// [`Unread`] when no whole head came, or one the relay does not read.
    pub(crate) async fn answer(&mut self, to_head: bool) -> Result<(Vec<u8>, Answered), Unread> {
        loop {
            if let Some(answered) = head::answered(&self.buffer, to_head)? {
                let bytes = self.buffer.drain(..answered.length).collect();
                return Ok((bytes, answered));
            }
            if self.buffer.len() >= HEAD_LIMIT {
                return Err(Unread::Malformed);
            }
            if !self.fill().await.map_err(|_| Unread::Closed)? {
                return Err(Unread::Closed);
            }
        }
    }

    /// Passes a body framed as `body` to `to`, byte for byte: how many
    /// bytes went.
    ///
    /// # Errors
    ///
    /// Either stream's, `UnexpectedEof` when the stream ends inside the
    /// body, and `InvalidData` for chunks out of form.
    pub(crate) async fn pass_body(
        &mut self,
        body: Body,
        to: &mut (impl AsyncWrite + Unpin),
    ) -> io::Result<u64> {
        match body {
            Body::Length(length) => self.pass_exactly(length, to).await,
            Body::Chunked => self.pass_chunks(to).await,
            Body::UntilClose => self.pass_to_end(to).await,
        }
    }

    /// Passes the next `length` bytes to `to`.
    async fn pass_exactly(
        &mut self,
        length: u64,
        to: &mut (impl AsyncWrite + Unpin),
    ) -> io::Result<u64> {
        let mut left = length;
        while left > 0 {
            if self.buffer.is_empty() && !self.fill().await? {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let taken =
                usize::try_from(left).map_or(self.buffer.len(), |left| left.min(self.buffer.len()));
            to.write_all(&self.buffer[..taken]).await?;
            self.buffer.drain(..taken);
            left = left.saturating_sub(widened(taken));
        }
        Ok(length)
    }

    /// Passes a chunked body to `to`, its trailer fields included (RFC
    /// 9112, section 7.1).
    async fn pass_chunks(&mut self, to: &mut (impl AsyncWrite + Unpin)) -> io::Result<u64> {
        let mut passed = 0_u64;
        loop {
            let line = self.line().await?;
            let size = chunk_size(&line)?;
            to.write_all(&line).await?;
            passed = passed.saturating_add(widened(line.len()));
            if size == 0 {
                break;
            }
            self.pass_exactly(size, to).await?;
            let end = self.line().await?;
            if end != b"\r\n" {
                return Err(io::ErrorKind::InvalidData.into());
            }
            to.write_all(&end).await?;
            passed = passed
                .saturating_add(size)
                .saturating_add(widened(end.len()));
        }
        let mut trailers = 0_usize;
        loop {
            let line = self.line().await?;
            trailers = trailers.saturating_add(line.len());
            if trailers > HEAD_LIMIT {
                return Err(io::ErrorKind::InvalidData.into());
            }
            to.write_all(&line).await?;
            passed = passed.saturating_add(widened(line.len()));
            if line == b"\r\n" {
                return Ok(passed);
            }
        }
    }

    /// Passes every byte to `to` until the stream ends.
    async fn pass_to_end(&mut self, to: &mut (impl AsyncWrite + Unpin)) -> io::Result<u64> {
        let early = widened(self.buffer.len());
        to.write_all(&self.buffer).await?;
        self.buffer.clear();
        let rest = tokio::io::copy(&mut self.stream, to).await?;
        Ok(early.saturating_add(rest))
    }

    /// The next line, its line ending included, within [`LINE_LIMIT`].
    async fn line(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                if end >= LINE_LIMIT {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                return Ok(self.buffer.drain(..=end).collect());
            }
            if self.buffer.len() >= LINE_LIMIT {
                return Err(io::ErrorKind::InvalidData.into());
            }
            if !self.fill().await? {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }
}

/// `bytes` as a count of bytes passed.
pub(crate) fn widened(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// The size a chunk's line gives, `1*HEXDIG [ BWS ";" chunk-ext ] CRLF`.
fn chunk_size(line: &[u8]) -> io::Result<u64> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidData);
    let line = line.strip_suffix(b"\r\n").ok_or_else(invalid)?;
    let digits = line
        .iter()
        .take_while(|byte| byte.is_ascii_hexdigit())
        .count();
    let rest = line[digits..].trim_ascii_start();
    if digits == 0 || digits > 16 || !(rest.is_empty() || rest.starts_with(b";")) {
        return Err(invalid());
    }
    let digits = std::str::from_utf8(&line[..digits]).map_err(|_| invalid())?;
    u64::from_str_radix(digits, 16).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::chunk_size;

    #[rstest::rstest]
    #[case::a_size(b"1a\r\n", Some(26))]
    #[case::an_extension(b"5;name=value\r\n", Some(5))]
    #[case::space_before_an_extension(b"5 ;name\r\n", Some(5))]
    #[case::the_last(b"0\r\n", Some(0))]
    #[case::no_digit(b";x\r\n", None)]
    #[case::a_sign(b"+5\r\n", None)]
    #[case::no_carriage_return(b"5\n", None)]
    #[case::text_after(b"5 x\r\n", None)]
    #[case::too_long(b"10000000000000000\r\n", None)]
    fn a_chunk_s_size_is_read_from_its_line_alone(#[case] line: &[u8], #[case] size: Option<u64>) {
        assert_eq!(chunk_size(line).ok(), size);
    }
}
