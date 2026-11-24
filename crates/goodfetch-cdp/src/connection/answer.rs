//! A caller's side of a command's answer: its wait, which tells the driver
//! when it leaves, and the [`Answer`] a started command gives.

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::failure::Failure;

/// A caller's wait for its command's answer. When it ends before the answer
/// came, by its timeout or its drop, it tells the driver, which takes the
/// command out of its pending map.
pub(super) struct Waiting {
    answered: oneshot::Receiver<Result<Value, Failure>>,
    /// Where it tells the driver: one word in it stands for any number of
    /// callers that left, as the driver then looks at every command.
    left: mpsc::Sender<()>,
    /// Whether the answer came, or the driver dropped the command unanswered.
    over: bool,
}

impl Waiting {
    /// The wait for the answer `answered` brings, which tells `left` when
    /// it ends first.
    pub(super) fn new(
        answered: oneshot::Receiver<Result<Value, Failure>>,
        left: mpsc::Sender<()>,
    ) -> Self {
        Self {
            answered,
            left,
            over: false,
        }
    }

    /// The command's answer, once the browser gives it.
    pub(super) async fn answer(&mut self) -> Result<Value, Failure> {
        let answer = (&mut self.answered).await;
        self.over = true;
        answer.map_err(|_| Failure::Disconnected)?
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.over {
            // Closed first, so the driver, told, sees that no one waits.
            self.answered.close();
            // A full channel already holds the word.
            let _ = self.left.try_send(());
        }
    }
}

/// What a command [`Connection::start`](super::Connection::start) started
/// returns, once the browser answers it: a future its holder awaits, or
/// polls in its own loop beside its other work, so it waits for no answer
/// before it goes on. Dropping it leaves the command's wait: a command
/// already queued is still written and carried out, and its answer is
/// dropped.
pub struct Answer<T>(Pin<Box<dyn Future<Output = Result<T, Failure>> + Send>>);

impl<T> Answer<T> {
    /// The answer `answer` brings.
    pub(super) fn new(answer: impl Future<Output = Result<T, Failure>> + Send + 'static) -> Self {
        Self(Box::pin(answer))
    }
}

impl<T> Future for Answer<T> {
    type Output = Result<T, Failure>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.as_mut().poll(context)
    }
}

impl<T> fmt::Debug for Answer<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Answer").finish_non_exhaustive()
    }
}
