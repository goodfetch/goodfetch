//! A bidirectional streaming call on a [`ClientConnection`](crate::ClientConnection),
//! such as Foxhound's `run.add_seeds`: the call stays in flight while the
//! engine grants credits with `$/credit` and sends replies in notifications
//! such as `$/enqueued`, and the Client sends batches in notifications such as
//! `$/seeds` (ADR 0024). Dropping [`Feed`] before its call answers cancels the
//! call with `$/cancel`, without blocking.

use std::future::Future as _;
use std::pin::Pin;
use std::task::{Context, Poll};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::error::Error;
use crate::rpc::{CallId, Heard, Outgoing, Sender};

/// How many unread notifications a [`Feed`] holds before backpressure applies:
/// enough for a full credit window of `$/credit` + `$/enqueued` notifications.
const FEED_QUEUE: usize = 512;

/// What a [`Feed`] hears from the engine on its in-flight call.
#[derive(Debug)]
pub enum FeedEvent {
    /// The call was written to the engine under this JSON-RPC request ID.
    Sent(i64),
    /// A notification naming the call's ID arrived from the engine.
    Notified {
        /// Its JSON-RPC method, such as `$/credit` or `$/enqueued`.
        method: String,
        /// Its JSON params.
        params: Value,
    },
    /// The call itself answered (or failed).
    Finished(Result<Value, Error>),
}

/// One bidirectional streaming call in flight on a
/// [`ClientConnection`](crate::ClientConnection). Dropping it before
/// [`FeedEvent::Finished`] cancels the call with `$/cancel`.
#[derive(Debug)]
#[must_use = "a feed's call is sent when opened; dropping it cancels the call"]
pub struct Feed {
    sender: Sender,
    call_id: CallId,
    heard: mpsc::Receiver<Heard>,
    replied: Option<oneshot::Receiver<Result<Value, Error>>>,
    answered: bool,
}

impl Feed {
    /// Opens a bidirectional streaming call of `method` with `params`.
    pub(crate) fn open(sender: Sender, method: &str, params: Result<Value, Error>) -> Self {
        let (notified, heard) = mpsc::channel(FEED_QUEUE);
        let (reply, replied) = oneshot::channel();
        match params {
            Ok(params) => {
                let call_id = sender.call_noted(
                    Outgoing {
                        method: method.to_owned(),
                        params,
                        timeout: None,
                        reply,
                        notified: Some(notified),
                        streams_itself: true,
                    },
                    true,
                );
                Self {
                    sender,
                    call_id,
                    heard,
                    replied: Some(replied),
                    answered: false,
                }
            }
            Err(error) => {
                let _ = reply.send(Err(error));
                Self {
                    sender,
                    call_id: CallId::default(),
                    heard,
                    replied: Some(replied),
                    answered: true,
                }
            }
        }
    }

    /// Sends a client-to-engine notification (such as `$/seeds`) on the
    /// Connection.
    ///
    /// # Errors
    ///
    /// `shape` if `params` does not serialize as JSON.
    pub fn notify(&self, method: &str, params: &impl Serialize) -> Result<(), Error> {
        let params = serde_json::to_value(params).map_err(Error::shape)?;
        self.sender.notify(method, params);
        Ok(())
    }

    /// Decodes a JSON notification payload received from [`poll_event`](Self::poll_event).
    ///
    /// # Errors
    ///
    /// `shape` if `value` does not match `T`.
    pub fn decode<T: DeserializeOwned>(value: Value) -> Result<T, Error> {
        serde_json::from_value(value).map_err(Error::shape)
    }

    /// Polls for the next event on this call: its sent ID, an incoming
    /// notification naming its ID, or its final reply.
    pub fn poll_event(&mut self, context: &mut Context<'_>) -> Poll<FeedEvent> {
        // Drain notifications before observing the final reply so no trailing
        // `$/enqueued` batch is lost ahead of `Finished`.
        if let Poll::Ready(Some(heard)) = self.heard.poll_recv(context) {
            return Poll::Ready(match heard {
                Heard::Sent(id) => FeedEvent::Sent(id),
                Heard::Notified { method, params } => FeedEvent::Notified { method, params },
            });
        }
        let Some(replied) = self.replied.as_mut() else {
            return Poll::Ready(FeedEvent::Finished(Err(Error::closed())));
        };
        match Pin::new(replied).poll(context) {
            Poll::Ready(result) => {
                // Check once more in case a notification was queued right
                // before the reply on the reader task.
                if let Ok(heard) = self.heard.try_recv() {
                    return Poll::Ready(match heard {
                        Heard::Sent(id) => FeedEvent::Sent(id),
                        Heard::Notified { method, params } => {
                            FeedEvent::Notified { method, params }
                        }
                    });
                }
                self.replied = None;
                let outcome = match result {
                    Ok(Ok(value)) => {
                        self.answered = true;
                        Ok(value)
                    }
                    Ok(Err(error)) => {
                        self.answered = error.is_answer();
                        Err(error)
                    }
                    Err(_) => Err(Error::closed()),
                };
                Poll::Ready(FeedEvent::Finished(outcome))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        if !self.answered {
            self.sender.cancel_call(&self.call_id);
        }
    }
}
