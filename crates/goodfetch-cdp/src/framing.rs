//! The pipe's framing, with no I/O: each message is one JSON text followed
//! by a NUL byte, in both directions (Chromium's `--remote-debugging-pipe`).
//! A message past [`MAX_FRAME`] bytes is dropped through its NUL rather than
//! grow the buffer without a bound (STYLE.md, Memory), naming the command it
//! answers, and the messages after it read on. Each message is sliced out of
//! the bytes read, never copied, and the buffer lets go of the room a large
//! message took once it is read.

use bytes::{Buf as _, Bytes, BytesMut};

/// The largest message the connection reads: a full-page screenshot's
/// Base64 stays well below it.
pub const MAX_FRAME: usize = 256 * 1024 * 1024;

/// The room the buffer keeps between messages: past it, a large message's
/// room goes with it once it was read.
const KEPT: usize = 1024 * 1024;

/// How an answer opens as Chromium writes it: its command's ID first.
const ANSWER_OPENING: &[u8] = b"{\"id\":";

/// A frame the pipe cannot carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FramingError {
    /// A message grew past the framer's limit before its NUL: the framer
    /// drops it, through its NUL, and reads on.
    #[error("a CDP message is longer than the framing's limit")]
    TooLarge {
        /// The ID of the command it answers, when it opens with one.
        answering: Option<u64>,
    },
}

/// Collects the bytes the browser writes and yields each whole message.
#[derive(Debug)]
pub struct Framer {
    buffer: BytesMut,
    /// Where the next search for a NUL starts: bytes before it hold none.
    searched: usize,
    limit: usize,
    /// Whether the bytes up to the next NUL belong to a message past the
    /// limit, which is dropped.
    dropping: bool,
}

impl Default for Framer {
    fn default() -> Self {
        Self::with_limit(MAX_FRAME)
    }
}

impl Framer {
    /// A framer whose messages are at most `limit` bytes, [`MAX_FRAME`] by
    /// default.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            buffer: BytesMut::new(),
            searched: 0,
            limit,
            dropping: false,
        }
    }

    /// Adds `bytes` as read from the pipe.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole message, without its NUL; `None` until one is whole.
    ///
    /// # Errors
    ///
    /// [`FramingError::TooLarge`], once for each message that grew longer
    /// than the limit before its NUL: its bytes are dropped through its NUL,
    /// and the next call reads on.
    pub fn next_frame(&mut self) -> Result<Option<Bytes>, FramingError> {
        if self.dropping {
            let Some(offset) = self.buffer.iter().position(|byte| *byte == 0) else {
                self.buffer.clear();
                self.let_go();
                return Ok(None);
            };
            self.buffer.advance(offset + 1);
            self.dropping = false;
            self.let_go();
        }
        match self.buffer[self.searched..]
            .iter()
            .position(|byte| *byte == 0)
        {
            Some(offset) => {
                let end = self.searched + offset;
                let frame = self.buffer.split_to(end).freeze();
                self.buffer.advance(1);
                self.searched = 0;
                self.let_go();
                Ok(Some(frame))
            }
            None if self.buffer.len() > self.limit => {
                let answering = answering(&self.buffer);
                self.buffer.clear();
                self.searched = 0;
                self.dropping = true;
                self.let_go();
                Err(FramingError::TooLarge { answering })
            }
            None => {
                self.searched = self.buffer.len();
                Ok(None)
            }
        }
    }

    /// Lets go of the room a large message took, once the buffer holds
    /// little.
    fn let_go(&mut self) {
        if self.buffer.capacity() > KEPT && self.buffer.len() <= KEPT {
            self.buffer = BytesMut::from(&self.buffer[..]);
        }
    }
}

/// The ID of the command the message opening with `bytes` answers, when it
/// opens as Chromium writes an answer.
fn answering(bytes: &[u8]) -> Option<u64> {
    let rest = bytes.strip_prefix(ANSWER_OPENING)?;
    let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    std::str::from_utf8(rest.get(..digits)?).ok()?.parse().ok()
}

/// `message` as the pipe carries it: its JSON, then a NUL.
pub(crate) fn frame(message: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
    bytes.push(0);
    bytes
}

#[cfg(test)]
mod tests;
