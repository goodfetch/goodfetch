//! Rust's form of a listener (ADR 0081): an [`EventStream`] hands the
//! program the events one call streams, in order, as it reads them. The
//! call is sent when the method that returns the stream is called, not when
//! the stream is first polled, so nothing slips through before the first
//! read. It stays in flight, and the engine sends its events in `$/events`
//! batches under the credits the stream grants with `$/credit`: a credit
//! goes back only once the program has read the event, so an unread
//! lossless stream slows its producer (ADR 0024). The SDK keeps no queue:
//! at most the credits granted wait in the stream. Dropping the stream
//! cancels its call with `$/cancel`, without blocking and without a
//! warning (ADRs 0078, 0081).

use std::fmt;
use std::future::poll_fn;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use goodfetch_protocol::connection::{CANCEL, CancelParams};
use goodfetch_protocol::events::{CREDIT, CreditParams, EVENTS, EventBatch, EventEnvelope};
use goodfetch_protocol::jsonrpc::RequestId;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::error::Error;
use crate::rpc::{Heard, Outgoing, Sender};

/// The credits a stream grants when its call is sent: at most this many
/// events wait in it for the program.
const CREDITS: u32 = 16;

/// How many events the program reads before the stream grants as many
/// again.
const REGRANT: u32 = 8;

/// The events one call streams, read in order. One reader each: a second
/// reader calls the method again and gets its own call. It has its own
/// [`EventStream::next`] and implements [`futures_core::Stream`], so no
/// other crate is needed (ADR 0081, G7). Dropping it cancels its call,
/// without blocking and without a warning.
///
/// It ends, [`EventStream::next`] answering `None`, when its call does:
/// when its page closes or its Run ends. A call that fails, such as one
/// whose value is closed or whose engine lost events past its credits
/// (`page_events_lost`), hands its error once, then ends.
#[must_use = "a stream's call is sent when it is made; dropping it cancels the call"]
pub struct EventStream<T> {
    /// The event of the call's batches it hands, such as `page.console`;
    /// the others take no more than their credit.
    event: &'static str,
    /// What the call hears; closed once its reply came.
    heard: mpsc::Receiver<Heard>,
    /// The call's reply, until it is read.
    reply: Option<oneshot::Receiver<Result<Value, Error>>>,
    /// The call's id, once it was sent.
    id: Option<i64>,
    /// The events of the batches heard that the program has not read.
    waiting: std::collections::VecDeque<EventEnvelope>,
    /// The events read since the last grant.
    read: u32,
    /// Whether it handed its last item, after which it hands none.
    ended: bool,
    /// An error to hand before anything else: the call was never sent.
    refused: Option<Error>,
    sender: Sender,
    item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for EventStream<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventStream")
            .field("event", &self.event)
            .field("id", &self.id)
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

impl<T: DeserializeOwned> EventStream<T> {
    /// Sends the call of `method` with `params`, which streams `event`
    /// among others, and returns its stream; `params` that are an error,
    /// such as [`Error::Closed`] for a closed value, send nothing, and the
    /// stream hands that error once.
    pub(crate) fn open(
        sender: Sender,
        method: &str,
        params: Result<Value, Error>,
        event: &'static str,
    ) -> Self {
        // The call's id, then at most a batch for each credit granted.
        let (notified, heard) = mpsc::channel(CREDITS as usize + 1);
        let (reply, replied) = oneshot::channel();
        let refused = match params {
            Ok(params) => {
                sender.call(Outgoing {
                    method: method.to_owned(),
                    params,
                    timeout: None,
                    reply,
                    notified: Some(notified),
                });
                None
            }
            Err(error) => Some(error),
        };
        Self {
            event,
            heard,
            reply: refused.is_none().then_some(replied),
            id: None,
            waiting: std::collections::VecDeque::new(),
            read: 0,
            ended: false,
            refused,
            sender,
            item: PhantomData,
        }
    }

    /// The next event, or `None` once the stream has ended. Goodfetch's
    /// own method, so the `futures` crate is never required (ADR 0081).
    ///
    /// # Errors
    ///
    /// An item is [`Error::Shape`] when an event does not fit its type,
    /// after which the stream goes on; [`Error::Closed`] when the value or
    /// the Connection is closed, and [`Error::Failed`] with the engine's
    /// error when the call fails, after which it ends.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future before it completes loses no event.
    pub async fn next(&mut self) -> Option<Result<T, Error>> {
        poll_fn(|context| self.poll_item(context)).await
    }

    /// The next item, or `Pending` until one comes.
    fn poll_item(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<T, Error>>> {
        if self.ended {
            return Poll::Ready(None);
        }
        if let Some(error) = self.refused.take() {
            self.ended = true;
            return Poll::Ready(Some(Err(error)));
        }
        loop {
            while let Some(envelope) = self.waiting.pop_front() {
                self.read_one();
                if envelope.event == self.event {
                    return Poll::Ready(Some(
                        serde_json::from_value(envelope.data).map_err(Error::Shape),
                    ));
                }
            }
            match self.heard.poll_recv(context) {
                Poll::Ready(Some(heard)) => self.hear(heard),
                // Its reply came, or the Connection ended: nothing more is
                // heard.
                Poll::Ready(None) => return self.poll_end(context),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    /// Acts on what the call heard: grants its first credits once it is
    /// sent, and keeps a batch's events for the program.
    fn hear(&mut self, heard: Heard) {
        match heard {
            Heard::Sent(id) => {
                self.id = Some(id);
                self.credit(CREDITS);
            }
            Heard::Notified { method, params } if method == EVENTS => {
                match serde_json::from_value::<EventBatch>(params) {
                    Ok(batch) => self.waiting.extend(batch.events),
                    Err(error) => tracing::debug!(
                        target: "goodfetch::client",
                        %error,
                        "a batch of events that is no batch"
                    ),
                }
            }
            Heard::Notified { .. } => {}
        }
    }

    /// Notes one event read, and grants as many again every [`REGRANT`].
    fn read_one(&mut self) {
        self.read += 1;
        if self.read == REGRANT {
            self.read = 0;
            self.credit(REGRANT);
        }
    }

    /// Ends the stream with its call's reply: `None` once it answered, its
    /// error once when it failed.
    fn poll_end(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<T, Error>>> {
        let Some(reply) = self.reply.as_mut() else {
            self.ended = true;
            return Poll::Ready(None);
        };
        let replied = match Pin::new(reply).poll(context) {
            Poll::Ready(replied) => replied,
            Poll::Pending => return Poll::Pending,
        };
        self.reply = None;
        self.ended = true;
        match replied {
            Ok(Ok(_)) => Poll::Ready(None),
            Ok(Err(error)) => Poll::Ready(Some(Err(error))),
            Err(_) => Poll::Ready(Some(Err(Error::Closed))),
        }
    }

    /// Grants the call `credits` more events.
    fn credit(&self, credits: u32) {
        let Some(id) = self.id else {
            return;
        };
        let params = CreditParams {
            id: RequestId::Number(id),
            credits,
        };
        if let Ok(params) = serde_json::to_value(params) {
            self.sender.notify(CREDIT, params);
        }
    }
}

impl<T: DeserializeOwned> futures_core::Stream for EventStream<T> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_item(context)
    }
}

impl<T> Drop for EventStream<T> {
    /// Cancels the call unless it ended, without blocking and without a
    /// warning. A call not sent yet is cancelled once it is: the closed
    /// stream tells the Connection that no one reads it.
    fn drop(&mut self) {
        if self.ended || self.reply.is_none() {
            return;
        }
        self.heard.close();
        while self.id.is_none() {
            match self.heard.try_recv() {
                Ok(Heard::Sent(id)) => self.id = Some(id),
                Ok(Heard::Notified { .. }) => {}
                Err(_) => break,
            }
        }
        let Some(id) = self.id else {
            return;
        };
        if let Ok(params) = serde_json::to_value(CancelParams {
            id: RequestId::Number(id),
        }) {
            self.sender.notify(CANCEL, params);
        }
    }
}

/// C-SEND-SYNC: a stream moves into a spawned task (ADR 0081).
const _: () = {
    const fn send_static<T: Send + 'static>() {}
    send_static::<EventStream<Value>>();
};
