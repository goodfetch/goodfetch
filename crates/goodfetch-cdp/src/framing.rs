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

/// The most room the buffer keeps between messages: past it, a large
/// message's room goes with it once it was read.
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
    /// The room the buffer keeps between messages: [`KEPT`], or the limit
    /// when it is smaller, since no message takes more.
    kept: usize,
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
            kept: limit.min(KEPT),
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
        if self.buffer.capacity() > self.kept && self.buffer.len() <= self.kept {
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

/// The framing half of the fuzz target's body (ADR 0067): feeds `stream`
/// to a framer of at most `limit` bytes in chunks of the lengths `cuts`
/// gives in turn, each byte one more than its value (the whole stream at
/// once when there are none), and reads every whole message after each
/// chunk, as the driver does after each read. It checks the framer against
/// the stream cut at its NULs: each message comes out once, whole and in
/// order, unless the framer held more than the limit of it before its NUL,
/// whatever the chunks; such a message, ended or not, is dropped once,
/// naming the command it answers, and the next one reads on; and once
/// what the buffer holds fits the room it keeps, it keeps no more. It
/// answers the messages that came out, in order, and panics only on a bug.
pub(crate) fn fuzz_frames(stream: &[u8], cuts: &[u8], limit: usize) -> Vec<Bytes> {
    let mut lengths = cuts.iter().map(|cut| usize::from(*cut) + 1).cycle();
    let mut chunks = Vec::new();
    let mut rest = stream;
    while !rest.is_empty() {
        let length = lengths.next().unwrap_or(rest.len()).min(rest.len());
        let (chunk, after) = rest.split_at(length);
        chunks.push(chunk);
        rest = after;
    }
    // Where each chunk ends in the stream: the framer reads after each.
    let ends: Vec<usize> = chunks
        .iter()
        .scan(0, |end, chunk| {
            *end += chunk.len();
            Some(*end)
        })
        .collect();
    // Each message, with whether the framer held more than the limit of it
    // before its NUL, or before the stream's end for the last, unended one.
    let mut start = 0;
    let mut messages: Vec<(&[u8], bool)> = stream
        .split(|byte| *byte == 0)
        .map(|message| {
            let nul = start + message.len();
            let held = ends
                .iter()
                .rev()
                .find(|end| **end <= nul)
                .map_or(0, |end| end.saturating_sub(start));
            start = nul + 1;
            (message, held > limit)
        })
        .collect();
    let (unended, unended_held_past) = messages
        .pop()
        .expect("invariant: a split yields at least one part");
    let mut framer = Framer::with_limit(limit);
    let mut read = Vec::new();
    let mut next = 0;
    let mut unended_dropped = false;
    for chunk in chunks {
        framer.push(chunk);
        let mut came_out = false;
        loop {
            match framer.next_frame() {
                Ok(None) => break,
                Ok(Some(frame)) => {
                    assert_eq!(
                        messages.get(next).copied(),
                        Some((&frame[..], false)),
                        "a message comes out once, whole and in order, unless held past the limit"
                    );
                    next += 1;
                    read.push(frame);
                }
                Err(FramingError::TooLarge { answering: named }) => {
                    let (message, held_past) = if let Some(message) = messages.get(next) {
                        next += 1;
                        *message
                    } else {
                        assert!(!unended_dropped, "a message is dropped once");
                        unended_dropped = true;
                        (unended, unended_held_past)
                    };
                    assert!(held_past, "only a message held past the limit is dropped");
                    assert_names(named, message, limit);
                }
            }
            came_out = true;
        }
        assert!(
            !came_out
                || framer.buffer.len() > framer.kept
                || framer.buffer.capacity() <= framer.kept,
            "the buffer lets go of a large message's room once it was read"
        );
    }
    assert_eq!(
        next,
        messages.len(),
        "every whole message comes out or is dropped"
    );
    assert_eq!(
        unended_dropped, unended_held_past,
        "an unended message held past the limit is dropped"
    );
    read
}

/// Checks that a message past `limit`, `message`, dropped naming `named`,
/// named the command it answers: the ID it opens with, when its digits end
/// within the bytes the framer held when it dropped it, and none when it
/// does not open as an answer.
fn assert_names(named: Option<u64>, message: &[u8], limit: usize) {
    let Some(rest) = message.strip_prefix(ANSWER_OPENING) else {
        assert_eq!(
            named, None,
            "a message that answers nothing names no command"
        );
        return;
    };
    let digits_end =
        ANSWER_OPENING.len() + rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits_end <= limit {
        assert_eq!(
            named,
            answering(message),
            "a dropped answer names its command"
        );
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
