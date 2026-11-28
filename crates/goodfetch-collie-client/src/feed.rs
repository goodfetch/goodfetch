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
    pending_reply: Option<Result<Value, Error>>,
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
                    pending_reply: None,
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
                    pending_reply: None,
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
        poll_channels(
            &mut self.heard,
            &mut self.replied,
            &mut self.pending_reply,
            &mut self.answered,
            context,
        )
    }
}

fn poll_channels(
    heard: &mut mpsc::Receiver<Heard>,
    replied: &mut Option<oneshot::Receiver<Result<Value, Error>>>,
    pending_reply: &mut Option<Result<Value, Error>>,
    answered: &mut bool,
    context: &mut Context<'_>,
) -> Poll<FeedEvent> {
    // Drain notifications before observing the final reply so no trailing
    // `$/enqueued` batch is lost ahead of `Finished`.
    if let Poll::Ready(Some(event)) = heard.poll_recv(context) {
        return Poll::Ready(heard_event(event));
    }
    if let Some(outcome) = pending_reply.take() {
        return Poll::Ready(FeedEvent::Finished(outcome));
    }
    let Some(receiver) = replied.as_mut() else {
        return Poll::Ready(FeedEvent::Finished(Err(Error::closed())));
    };
    match Pin::new(receiver).poll(context) {
        Poll::Ready(result) => {
            *replied = None;
            let outcome = match result {
                Ok(Ok(value)) => {
                    *answered = true;
                    Ok(value)
                }
                Ok(Err(error)) => {
                    *answered = error.is_answer();
                    Err(error)
                }
                Err(_) => Err(Error::closed()),
            };
            // Check once more in case a notification was queued right before
            // the reply on the reader task; hold the resolved outcome so
            // `replied` is never polled again after completion.
            if let Ok(event) = heard.try_recv() {
                *pending_reply = Some(outcome);
                return Poll::Ready(heard_event(event));
            }
            Poll::Ready(FeedEvent::Finished(outcome))
        }
        Poll::Pending => Poll::Pending,
    }
}

fn heard_event(event: Heard) -> FeedEvent {
    match event {
        Heard::Sent(id) => FeedEvent::Sent(id),
        Heard::Notified { method, params } => FeedEvent::Notified { method, params },
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        if !self.answered {
            self.sender.cancel_call(&self.call_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::task::{Context, Poll, Waker};

    use serde_json::json;
    use tokio::sync::{mpsc, oneshot};

    use super::{FeedEvent, poll_channels};
    use crate::rpc::Heard;

    #[test]
    fn poll_channels_drains_notifications_ahead_of_held_reply_without_repoll_panic() {
        let (notified, mut heard) = mpsc::channel(4);
        let (reply, replied) = oneshot::channel();
        let mut replied = Some(replied);
        let mut pending_reply = None;
        let mut answered = false;
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        let mut poll = |heard: &mut mpsc::Receiver<Heard>,
                        replied: &mut Option<
            oneshot::Receiver<Result<serde_json::Value, crate::error::Error>>,
        >,
                        pending: &mut Option<Result<serde_json::Value, crate::error::Error>>,
                        answered: &mut bool| {
            poll_channels(heard, replied, pending, answered, &mut context)
        };

        // Both a trailing notification and the final reply arrive before poll.
        notified
            .try_send(Heard::Notified {
                method: "$/enqueued".to_owned(),
                params: json!({ "answers": [] }),
            })
            .unwrap();
        reply.send(Ok(json!({ "added": 1 }))).unwrap();

        assert!(matches!(
            poll(&mut heard, &mut replied, &mut pending_reply, &mut answered),
            Poll::Ready(FeedEvent::Notified { .. })
        ));
        assert!(matches!(
            poll(&mut heard, &mut replied, &mut pending_reply, &mut answered),
            Poll::Ready(FeedEvent::Finished(Ok(_)))
        ));
        assert!(answered);
        assert!(replied.is_none());

        // Pending reply held while a trailing notification drains never re-polls `replied`.
        pending_reply = Some(Ok(json!({ "added": 2 })));
        notified
            .try_send(Heard::Notified {
                method: "$/enqueued".to_owned(),
                params: json!({ "answers": [1] }),
            })
            .unwrap();
        assert!(matches!(
            poll(&mut heard, &mut replied, &mut pending_reply, &mut answered),
            Poll::Ready(FeedEvent::Notified { .. })
        ));
        assert!(matches!(
            poll(&mut heard, &mut replied, &mut pending_reply, &mut answered),
            Poll::Ready(FeedEvent::Finished(Ok(_)))
        ));
    }
}
