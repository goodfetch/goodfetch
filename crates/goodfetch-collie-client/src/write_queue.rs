//! The Connection's write queue: the lines its writer writes, in the
//! order sent, and how many of their bytes it has not written yet, which
//! `connection.max_queued_write_bytes` bounds (issue 3189).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use crate::sync::AtomicUsize;

/// What sends lines to the Connection's writer, and reads how many of
/// their bytes it has not written yet.
#[derive(Clone, Debug)]
pub(crate) struct WriteQueue {
    lines: mpsc::UnboundedSender<Vec<u8>>,
    unwritten: Arc<AtomicUsize>,
}

/// A [`WriteQueue`] that does not keep the Connection's writer going.
#[derive(Debug)]
pub(crate) struct WeakWriteQueue {
    lines: mpsc::WeakUnboundedSender<Vec<u8>>,
    unwritten: Arc<AtomicUsize>,
}

/// What the writer takes lines from, and tells of each line written.
#[derive(Debug)]
pub(crate) struct QueuedLines {
    lines: mpsc::UnboundedReceiver<Vec<u8>>,
    unwritten: Arc<AtomicUsize>,
}

/// A new write queue and what its writer takes lines from.
pub(crate) fn write_queue() -> (WriteQueue, QueuedLines) {
    let (lines, taken) = mpsc::unbounded_channel();
    let unwritten = Arc::new(AtomicUsize::new(0));
    (
        WriteQueue {
            lines,
            unwritten: Arc::clone(&unwritten),
        },
        QueuedLines {
            lines: taken,
            unwritten,
        },
    )
}

impl WriteQueue {
    /// Hands `line` to the writer, or gives it back once the writer is
    /// gone.
    pub(crate) fn send(&self, line: Vec<u8>) -> Result<(), Vec<u8>> {
        let length = line.len();
        self.unwritten.fetch_add(length, Ordering::Relaxed);
        self.lines.send(line).map_err(|refused| {
            self.unwritten.fetch_sub(length, Ordering::Relaxed);
            refused.0
        })
    }

    /// How many bytes it was handed and has not written yet.
    pub(crate) fn unwritten(&self) -> usize {
        self.unwritten.load(Ordering::Relaxed)
    }

    /// The queue, without keeping the writer going.
    pub(crate) fn downgrade(&self) -> WeakWriteQueue {
        WeakWriteQueue {
            lines: self.lines.downgrade(),
            unwritten: Arc::clone(&self.unwritten),
        }
    }
}

impl WeakWriteQueue {
    /// The queue, while its writer goes on.
    pub(crate) fn upgrade(&self) -> Option<WriteQueue> {
        Some(WriteQueue {
            lines: self.lines.upgrade()?,
            unwritten: Arc::clone(&self.unwritten),
        })
    }
}

impl QueuedLines {
    /// The next line to write, once every sender is gone none.
    pub(crate) async fn next(&mut self) -> Option<Vec<u8>> {
        self.lines.recv().await
    }

    /// Notes that `line` was written.
    pub(crate) fn wrote(&self, line: &[u8]) {
        self.unwritten.fetch_sub(line.len(), Ordering::Relaxed);
    }
}
