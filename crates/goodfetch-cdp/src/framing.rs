//! The pipe's framing, with no I/O: each message is one JSON text followed
//! by a NUL byte, in both directions (Chromium's `--remote-debugging-pipe`).
//! A message past [`MAX_FRAME`] bytes ends the connection rather than grow
//! the buffer without a bound (STYLE.md, Memory). Each message is sliced
//! out of the bytes read, never copied, and the buffer lets go of the room
//! a large message took once it is read.

use bytes::{Buf as _, Bytes, BytesMut};

/// The largest message the connection reads: a full-page screenshot's
/// Base64 stays well below it.
pub const MAX_FRAME: usize = 256 * 1024 * 1024;

/// The room the buffer keeps between messages: past it, a large message's
/// room goes with it once it was read.
const KEPT: usize = 1024 * 1024;

/// A frame the pipe cannot carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FramingError {
    /// A message grew past the framer's limit before its NUL.
    #[error("a CDP message is longer than the framing's limit")]
    TooLarge,
}

/// Collects the bytes the browser writes and yields each whole message.
#[derive(Debug)]
pub struct Framer {
    buffer: BytesMut,
    /// Where the next search for a NUL starts: bytes before it hold none.
    searched: usize,
    limit: usize,
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
    /// [`FramingError::TooLarge`] when the message being read is already
    /// longer than the limit.
    pub fn next_frame(&mut self) -> Result<Option<Bytes>, FramingError> {
        match self.buffer[self.searched..]
            .iter()
            .position(|byte| *byte == 0)
        {
            Some(offset) => {
                let end = self.searched + offset;
                let frame = self.buffer.split_to(end).freeze();
                self.buffer.advance(1);
                self.searched = 0;
                if self.buffer.capacity() > KEPT && self.buffer.len() <= KEPT {
                    self.buffer = BytesMut::from(&self.buffer[..]);
                }
                Ok(Some(frame))
            }
            None if self.buffer.len() > self.limit => Err(FramingError::TooLarge),
            None => {
                self.searched = self.buffer.len();
                Ok(None)
            }
        }
    }

    /// Whether bytes of an unfinished message are left.
    pub fn has_partial(&self) -> bool {
        !self.buffer.is_empty()
    }
}

/// `message` as the pipe carries it: its JSON, then a NUL.
pub(crate) fn frame(message: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
    bytes.push(0);
    bytes
}

#[cfg(test)]
mod tests;
