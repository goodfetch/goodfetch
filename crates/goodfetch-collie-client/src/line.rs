//! The Connection's framing: one JSON-RPC message as one line of the
//! newline-delimited JSON every transport carries, the one encoding the
//! Connection's calls and its answers to the engine's requests both write,
//! and the output they are written through, which counts the bytes handed
//! to its writer and not yet written (issue 3189).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tokio::sync::mpsc;

use crate::sync::AtomicUsize;

/// `message` as one line, its newline included.
pub(crate) fn encode(message: &impl Serialize) -> serde_json::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    Ok(line)
}

/// The Connection's output: the lines its writer writes, in the order
/// sent, and how many of their bytes it has not written yet.
#[derive(Clone, Debug)]
pub(crate) struct Output {
    lines: mpsc::UnboundedSender<Vec<u8>>,
    unwritten: Arc<AtomicUsize>,
}

/// An [`Output`] that does not keep the Connection's output open.
#[derive(Debug)]
pub(crate) struct WeakOutput {
    lines: mpsc::WeakUnboundedSender<Vec<u8>>,
    unwritten: Arc<AtomicUsize>,
}

/// What the writer takes lines from, and tells of each line written.
#[derive(Debug)]
pub(crate) struct Written {
    lines: mpsc::UnboundedReceiver<Vec<u8>>,
    unwritten: Arc<AtomicUsize>,
}

/// A new output and what its writer takes lines from.
pub(crate) fn output() -> (Output, Written) {
    let (lines, taken) = mpsc::unbounded_channel();
    let unwritten = Arc::new(AtomicUsize::new(0));
    (
        Output {
            lines,
            unwritten: Arc::clone(&unwritten),
        },
        Written {
            lines: taken,
            unwritten,
        },
    )
}

impl Output {
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

    /// The output, without keeping it open.
    pub(crate) fn downgrade(&self) -> WeakOutput {
        WeakOutput {
            lines: self.lines.downgrade(),
            unwritten: Arc::clone(&self.unwritten),
        }
    }
}

impl WeakOutput {
    /// The output, while it is open.
    pub(crate) fn upgrade(&self) -> Option<Output> {
        Some(Output {
            lines: self.lines.upgrade()?,
            unwritten: Arc::clone(&self.unwritten),
        })
    }
}

impl Written {
    /// The next line to write, once every sender is gone none.
    pub(crate) async fn next(&mut self) -> Option<Vec<u8>> {
        self.lines.recv().await
    }

    /// Notes that `line` was written.
    pub(crate) fn wrote(&self, line: &[u8]) {
        self.unwritten.fetch_sub(line.len(), Ordering::Relaxed);
    }
}
