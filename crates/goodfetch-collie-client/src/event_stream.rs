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
//!
//! A stream of events the engine publishes on its event stream, such as a
//! Session's pages, gets them from the Connection's own Subscription
//! instead, which [`crate::published`] shares between such streams: its
//! call only arms the engine, and the stream joins the Subscription before
//! the call is sent and leaves it once dropped or ended. Its own queue
//! holds at most the Connection's [`crate::ListenerQueue`] unread events,
//! so a stream left unread slows no other: past it, the stream loses its
//! own events, hands those it held, then ends with `listener_events_lost`.

use std::collections::VecDeque;
use std::fmt;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::{Context, Poll};

use goodfetch_protocol::events::{EVENTS, EventBatch, EventEnvelope};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::error::Error;
use crate::published::{CREDITS, Interest, Membership, Published, REGRANT, Received, Taken};
use crate::rpc::{Heard, Outgoing, Sender};

/// What a stream builds each event's payload into.
type Build<T> = Box<dyn Fn(Value) -> Result<T, Error> + Send + Sync>;

/// The events one call streams, read in order. One reader each: a second
/// reader calls the method again and gets its own call. It has its own
/// [`EventStream::next`] and implements [`futures_core::Stream`], so no
/// other crate is needed (ADR 0081, G7). Dropping it cancels its call,
/// without blocking and without a warning.
///
/// It ends, [`EventStream::next`] answering `None`, when its call does:
/// when its page or Session closes or its Run ends. A call that fails, such
/// as one whose value is closed or whose engine lost events past its
/// credits (`page_events_lost`), hands its error once, then ends. A
/// Session's stream that held as many unread events as the Connection's
/// [`crate::ListenerQueue`] loses its next ones, and once it handed those it
/// held, hands `listener_events_lost` naming how many it lost, then ends.
#[must_use = "a stream's call is sent when it is made; dropping it cancels the call"]
pub struct EventStream<T> {
    flow: Flow,
    build: Build<T>,
}

/// What a stream reads, whatever it builds of it.
struct Flow {
    /// The event it hands, such as `page.console`; the others its call
    /// streams take no more than their credit.
    event: &'static str,
    /// What the call hears; closed once its reply came.
    heard: mpsc::Receiver<Heard>,
    /// The call's reply, until it is read.
    reply: Option<oneshot::Receiver<Result<Value, Error>>>,
    /// The call's id, once it was sent.
    id: Option<i64>,
    /// Whether it handed its last item, after which it hands none.
    ended: bool,
    /// An error to hand before anything else: the call was never sent.
    refused: Option<Error>,
    sender: Sender,
    source: Source,
}

/// The batches a stream's own call streams.
#[derive(Default)]
struct Batches {
    /// The events of the batches heard that the program has not read.
    waiting: VecDeque<EventEnvelope>,
    /// The events read since the last grant.
    read: u32,
}

/// Where a stream's events come from.
enum Source {
    /// Its own call's batches.
    Own(Batches),
    /// The Connection's Subscription, which it joined before its call was
    /// sent.
    Published {
        /// Its place on the Subscription, until it leaves.
        membership: Option<Membership>,
        /// The events its queue holds, then its end.
        taken: Received,
    },
}

impl<T> fmt::Debug for EventStream<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventStream")
            .field("event", &self.flow.event)
            .field("id", &self.flow.id)
            .field("ended", &self.flow.ended)
            .finish_non_exhaustive()
    }
}

impl<T> EventStream<T> {
    /// Sends the call of `method` with `params`, which streams `event`
    /// among others, and returns its stream, which builds each payload
    /// with `build`; `params` that are an error, such as `closed` for a
    /// closed value, send nothing, and the stream hands that error once.
    pub(crate) fn open(
        sender: Sender,
        method: &str,
        params: Result<Value, Error>,
        event: &'static str,
        build: Build<T>,
    ) -> Self {
        // The call's id, then at most a batch for each credit granted.
        let (notified, heard) = mpsc::channel(CREDITS as usize + 1);
        let source = Source::Own(Batches::default());
        Self {
            flow: Flow::open(sender, method, params, event, notified, heard, source),
            build,
        }
    }

    /// Joins `published`, the Connection's Subscription, as the reader of
    /// the events `interest` names, then sends the call of `method` with
    /// `params`, which arms them, and returns its stream, which builds each
    /// payload with `build`; `params` that are an error send and join
    /// nothing, and the stream hands that error once.
    pub(crate) fn published(
        published: &Published,
        method: &str,
        params: Result<Value, Error>,
        interest: Interest,
        build: Build<T>,
    ) -> Self {
        // The call's id: it streams nothing itself.
        let (notified, heard) = mpsc::channel(1);
        let event = interest.event;
        let source = if params.is_ok() {
            let (membership, taken) = published.join(interest);
            Source::Published {
                membership: Some(membership),
                taken,
            }
        } else {
            Source::Own(Batches::default())
        };
        Self {
            flow: Flow::open(
                published.sender(),
                method,
                params,
                event,
                notified,
                heard,
                source,
            ),
            build,
        }
    }

    /// The stream that hands `map` of each of this one's items.
    pub(crate) fn map<U>(self, map: impl Fn(T) -> U + Send + Sync + 'static) -> EventStream<U>
    where
        T: 'static,
    {
        let Self { flow, build } = self;
        EventStream {
            flow,
            build: Box::new(move |data| build(data).map(&map)),
        }
    }

    /// The next event, or `None` once the stream has ended. Goodfetch's
    /// own method, so the `futures` crate is never required (ADR 0081).
    ///
    /// # Errors
    ///
    /// An item is `internal` when an event does not fit its type, after
    /// which the stream goes on; `closed` when the value or the Connection
    /// is closed, the engine's error when the call fails, and
    /// `listener_events_lost` when a Session's stream lost events past its
    /// queue, after which it ends.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future before it completes loses no event.
    pub async fn next(&mut self) -> Option<Result<T, Error>> {
        poll_fn(|context| self.poll_item(context)).await
    }

    /// The next item, or `Pending` until one comes.
    fn poll_item(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<T, Error>>> {
        self.flow
            .poll_payload(context)
            .map(|item| item.map(|payload| payload.and_then(&self.build)))
    }
}

impl Flow {
    /// Sends the call of `method` with `params`, whose sends it hears on
    /// `heard`, unless `params` is an error.
    fn open(
        sender: Sender,
        method: &str,
        params: Result<Value, Error>,
        event: &'static str,
        notified: mpsc::Sender<Heard>,
        heard: mpsc::Receiver<Heard>,
        source: Source,
    ) -> Self {
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
            ended: false,
            refused,
            sender,
            source,
        }
    }

    /// The next payload, or `Pending` until one comes.
    fn poll_payload(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<Value, Error>>> {
        if self.ended {
            return Poll::Ready(None);
        }
        if let Some(error) = self.refused.take() {
            self.ended = true;
            return Poll::Ready(Some(Err(error)));
        }
        if matches!(self.source, Source::Published { .. }) {
            self.poll_published(context)
        } else {
            self.poll_own(context)
        }
    }

    /// The next payload of the call's own batches.
    fn poll_own(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<Value, Error>>> {
        let Source::Own(batches) = &mut self.source else {
            return Poll::Ready(None);
        };
        loop {
            while let Some(envelope) = batches.waiting.pop_front() {
                batches.read += 1;
                if batches.read == REGRANT {
                    batches.read = 0;
                    credit(&self.sender, self.id, REGRANT);
                }
                if envelope.event == self.event {
                    return Poll::Ready(Some(Ok(envelope.data)));
                }
            }
            match self.heard.poll_recv(context) {
                // Its first credits go once it was sent, which the
                // program's first read polls for.
                Poll::Ready(Some(Heard::Sent(id))) => {
                    self.id = Some(id);
                    credit(&self.sender, self.id, CREDITS);
                }
                Poll::Ready(Some(Heard::Notified { method, params })) if method == EVENTS => {
                    match serde_json::from_value::<EventBatch>(params) {
                        Ok(batch) => batches.waiting.extend(batch.events),
                        Err(error) => tracing::debug!(
                            target: "goodfetch::client",
                            %error,
                            "a batch of events that is no batch"
                        ),
                    }
                }
                Poll::Ready(Some(Heard::Notified { .. })) => {}
                // Its reply came, or the Connection ended: nothing more is
                // heard.
                Poll::Ready(None) => return self.poll_end(context),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    /// The next payload the Subscription hands: the stream ends when its
    /// call does, once the events taken before are read, or when the
    /// Subscription does.
    fn poll_published(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<Value, Error>>> {
        // The call's id is all it hears.
        while let Poll::Ready(Some(heard)) = self.heard.poll_recv(context) {
            if let Heard::Sent(id) = heard {
                self.id = Some(id);
            }
        }
        loop {
            let Source::Published { membership, taken } = &mut self.source else {
                return Poll::Ready(None);
            };
            match taken.poll_recv(context) {
                Poll::Ready(Some(Taken::Event(data))) => {
                    return Poll::Ready(Some(Ok(data)));
                }
                // It read what it held: it leaves, and says what it lost.
                Poll::Ready(Some(Taken::Lost(lost))) => {
                    self.end();
                    return Poll::Ready(Some(Err(Error::listener_events_lost(self.event, lost))));
                }
                Poll::Ready(Some(Taken::Ended(failure))) => {
                    self.end();
                    return Poll::Ready(failure.map(Err));
                }
                // It left, and read every event it took before.
                Poll::Ready(None) => {
                    self.ended = true;
                    return Poll::Ready(None);
                }
                Poll::Pending => {}
            }
            // Its call ended: it waits for the Subscription to let it go.
            let Some(reply) = self.reply.as_mut() else {
                return Poll::Pending;
            };
            let replied = match Pin::new(reply).poll(context) {
                Poll::Ready(replied) => replied,
                Poll::Pending => return Poll::Pending,
            };
            self.reply = None;
            match replied {
                // Its call ended: the events published before its answer
                // are still handed, then its receiver ends.
                Ok(Ok(_)) => {
                    if let Some(membership) = membership {
                        membership.finish();
                    }
                }
                Ok(Err(error)) => {
                    self.leave();
                    self.ended = true;
                    return Poll::Ready(Some(Err(error)));
                }
                Err(_) => {
                    self.leave();
                    self.ended = true;
                    return Poll::Ready(Some(Err(Error::closed())));
                }
            }
        }
    }

    /// Ends the stream with its call's reply: `None` once it answered, its
    /// error once when it failed.
    fn poll_end(&mut self, context: &mut Context<'_>) -> Poll<Option<Result<Value, Error>>> {
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
            Err(_) => Poll::Ready(Some(Err(Error::closed()))),
        }
    }

    /// Cancels the call unless it ended. A call not sent yet is cancelled
    /// once it is: the closed stream tells the Connection that no one
    /// reads it.
    fn cancel(&mut self) {
        // Ended, or never sent: nothing to cancel.
        if self.reply.take().is_none() {
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
        if let Some(id) = self.id {
            self.sender.cancel(id);
        }
    }

    /// Leaves the Subscription, if it joined it.
    fn leave(&mut self) {
        if let Source::Published { membership, .. } = &mut self.source {
            drop(membership.take());
        }
    }

    /// Ends the stream before its call does: cancels the call and leaves
    /// the Subscription.
    fn end(&mut self) {
        self.cancel();
        self.leave();
        self.ended = true;
    }
}

/// Grants the call `id`, once it was sent, `credits` more events.
fn credit(sender: &Sender, id: Option<i64>, credits: u32) {
    if let Some(id) = id {
        sender.credit(id, credits);
    }
}

impl<T> futures_core::Stream for EventStream<T> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_item(context)
    }
}

impl Drop for Flow {
    /// Cancels the call unless it ended, without blocking and without a
    /// warning, then leaves the Subscription, which its last reader
    /// cancels.
    fn drop(&mut self) {
        self.cancel();
        self.leave();
    }
}

/// C-SEND-SYNC: a stream moves into a spawned task, and any
/// `futures_core::Stream` consumer reads it (ADR 0081).
const _: () = {
    const fn send_static_stream<T: Send + Sync + 'static + futures_core::Stream>() {}
    send_static_stream::<EventStream<Value>>();
};
