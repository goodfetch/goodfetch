//! The protocol over Server-Sent Events plus `POST` (ADR 0031), the
//! Console's second transport, for a network whose proxy refuses
//! `WebSocket`s: the engine sends each JSON-RPC message as the data of one
//! event of a `text/event-stream` answer, and the page sends its messages
//! in `POST` bodies, one per line, as newline-delimited JSON-RPC frames
//! them. The Console's server checks each request and carries the bytes;
//! this module turns them into the messages a Connection reads and writes,
//! so the Connection's life is the WebSocket's.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::body::{Body, Bytes, Frame as BodyFrame};
use tokio::sync::mpsc;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::framing::{Frame, Framer, MAX_MESSAGE_BYTES};

/// How many `POST` bodies wait for the Connection before a `POST` waits.
const POSTED: usize = 16;

/// How many events wait for the stream before the Connection's writer waits.
const STREAMED: usize = 64;

/// The page's side of an event stream: where each `POST` body goes.
#[derive(Clone, Debug)]
pub(crate) struct Posting(mpsc::Sender<Vec<Frame>>);

/// The messages of a Connection carried over an event stream, which the
/// Daemon serves as it serves a WebSocket's.
pub(crate) struct EventStream {
    pub(super) reader: PostedReader,
    pub(super) writer: StreamedWriter,
}

/// The body of the `text/event-stream` answer: one event per message the
/// engine sends, ending once the Connection closes. Dropping it, as the
/// page's going away does, ends the Connection.
pub(crate) struct EventStreamBody {
    first: Option<Bytes>,
    events: mpsc::Receiver<Bytes>,
    _open: DropGuard,
}

impl Body for EventStreamBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<BodyFrame<Bytes>, Infallible>>> {
        if let Some(first) = self.first.take() {
            return Poll::Ready(Some(Ok(BodyFrame::data(first))));
        }
        self.events
            .poll_recv(context)
            .map(|event| event.map(|bytes| Ok(BodyFrame::data(bytes))))
    }
}

/// A new Connection over an event stream: what the Daemon serves, where
/// the page's `POST` bodies go, and the body of the `text/event-stream`
/// answer, which starts with `first`.
pub(crate) fn opened(first: Bytes) -> (EventStream, Posting, EventStreamBody) {
    let (posting, posted) = mpsc::channel(POSTED);
    let (streaming, streamed) = mpsc::channel(STREAMED);
    let gone = CancellationToken::new();
    (
        EventStream {
            reader: PostedReader {
                posted,
                gone: gone.clone(),
            },
            writer: StreamedWriter(streaming),
        },
        Posting(posting),
        EventStreamBody {
            first: Some(first),
            events: streamed,
            _open: gone.drop_guard(),
        },
    )
}

impl Posting {
    /// Hands the messages of a `POST` body to the Connection, in order;
    /// `false` once the Connection has ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops the body's messages.
    pub(crate) async fn post(&self, body: &[u8]) -> bool {
        self.0.send(posted(body, MAX_MESSAGE_BYTES)).await.is_ok()
    }

    /// Whether the Connection has ended.
    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// The messages of a `POST` body: its lines, each at most `limit` bytes,
/// the last one with or without its newline; a blank line is none.
fn posted(body: &[u8], limit: usize) -> Vec<Frame> {
    let mut framer = Framer::new(limit);
    let mut messages = Vec::new();
    framer.feed(body, &mut messages);
    messages.extend(framer.finish());
    messages
}

/// The messages the page posts, until its event stream's answer is gone.
pub(super) struct PostedReader {
    posted: mpsc::Receiver<Vec<Frame>>,
    gone: CancellationToken,
}

impl PostedReader {
    /// Appends the messages of the next `POST` body to `frames`; `false`
    /// once the event stream's answer is gone, as when the page left.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no body.
    pub(super) async fn read(&mut self, frames: &mut Vec<Frame>) -> bool {
        tokio::select! {
            posted = self.posted.recv() => match posted {
                Some(posted) => {
                    frames.extend(posted);
                    true
                }
                None => false,
            },
            () = self.gone.cancelled() => false,
        }
    }
}

/// What the engine sends the page, as events.
pub(super) struct StreamedWriter(mpsc::Sender<Bytes>);

impl StreamedWriter {
    /// Sends `line`, one JSON-RPC message and its newline, as one event;
    /// `false` once the event stream is gone.
    pub(super) async fn feed(&mut self, line: Vec<u8>) -> bool {
        self.0.send(event(&line)).await.is_ok()
    }

    /// Closes the event stream: its answer ends.
    pub(super) fn close(self) {
        drop(self);
    }
}

/// The event that carries `line`: its JSON, which holds no newline, as
/// one `data` field.
fn event(line: &[u8]) -> Bytes {
    let json = line.strip_suffix(b"\n").unwrap_or(line);
    let mut event = Vec::with_capacity(json.len() + 8);
    event.extend_from_slice(b"data: ");
    event.extend_from_slice(json);
    event.extend_from_slice(b"\n\n");
    Bytes::from(event)
}

/// The fuzz target `console_post_fallback`'s body for a `POST` body (ADR
/// 0067): its messages under a small limit, each a line of the body with no
/// newline, never longer than the limit, and no more of them than its
/// lines. It panics only on a bug.
pub fn fuzz_posted(body: &[u8]) {
    const LIMIT: usize = 4096;
    let frames = posted(body, LIMIT);
    let lines = body.split(|byte| *byte == b'\n').count();
    assert!(frames.len() <= lines, "more messages than the body's lines");
    for frame in frames {
        if let Frame::Line(line) = frame {
            assert!(line.len() <= LIMIT, "a message longer than the limit");
            assert!(!line.contains(&b'\n'), "a message spans two lines");
            let _ = super::framing::decode(&line);
        }
    }
}
