//! The Connection's own Subscription (ADRs 0031 and 0081): one
//! `events.subscribe`, sent with its credits when the first reader of the
//! events the engine publishes on its event stream joins, such as a
//! Session's stream, and cancelled once the last leaves; the next to join
//! sends another. A task on the Connection's tracker reads it and hands
//! each event to the readers that take it, those of its name whose payload
//! names their handle. An event's credit goes back as soon as the task
//! hears it, since each reader's own queue takes it or loses it: a queue
//! holds at most the Connection's [`ListenerQueue`] unread, and past it the
//! reader loses each event of its own until it read those it held, then
//! ends with `listener_events_lost`, so an unread stream slows no other.
//! Should the Subscription still fall behind, the engine sends
//! `events.resync` rather than wait, and the events in between are gone,
//! which is logged.

use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll};

use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::events::{
    EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventsSubscribeParams, RESYNC,
};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio_util::task::TaskTracker;

use crate::error::Error;
use crate::listener_queue::ListenerQueue;
use crate::rpc::{CallId, Heard, Outgoing, Sender};
use crate::sync::{Mutex, MutexGuard};

/// The credits a stream of events grants when its call is sent, the
/// Subscription's or a call's own: at most this many of its events are
/// unsettled, those a call's own stream has not read, or those the
/// Subscription has not heard yet.
pub(crate) const CREDITS: u32 = 16;

/// How many of its events the program settles before the stream grants as
/// many again.
pub(crate) const REGRANT: u32 = 8;

/// What a reader hears of the Subscription.
#[derive(Debug)]
pub(crate) enum Taken {
    /// An event's payload it takes; its credit is already back.
    Event(Value),
    /// The Subscription ended without its cancel: the engine answered it,
    /// with the error that ended it, if one did.
    Ended(Option<Error>),
    /// Its queue was full, and it read every event it held: it lost this
    /// many of its own, and hears nothing more.
    Lost(u64),
}

/// What one reader takes: the events named `event` whose payload's
/// `field` is `id`, such as a page of its Session.
#[derive(Debug)]
pub(crate) struct Interest {
    pub(crate) event: &'static str,
    pub(crate) field: &'static str,
    pub(crate) id: Value,
}

/// The Connection's Subscription, shared by the readers that join it.
#[derive(Clone, Debug)]
pub(crate) struct Published(Arc<Core>);

#[derive(Debug)]
struct Core {
    sender: Sender,
    /// How many events each reader's queue holds.
    bound: ListenerQueue,
    /// The runtime its task runs on, and the tracker it is spawned on.
    handle: Handle,
    tasks: TaskTracker,
    /// The Subscription in flight, if any.
    current: Mutex<Option<Arc<Subscription>>>,
}

/// One `events.subscribe` in flight.
#[derive(Debug)]
struct Subscription {
    sender: Sender,
    /// Its call's id, once sent.
    call: CallId,
    /// Where a reader whose own call ended asks its task to let it go once
    /// it handed the events heard before.
    finished: mpsc::UnboundedSender<u64>,
    /// How many events each reader's queue holds.
    bound: ListenerQueue,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    readers: Vec<Reader>,
    /// The key the next reader gets.
    next_key: u64,
    /// The events settled since the last grant.
    settled: u32,
    /// Whether its last reader left, or it ended: it grants nothing more.
    stopped: bool,
    /// Dropped when its last reader leaves, which stops its task.
    stop: Option<oneshot::Sender<()>>,
}

#[derive(Debug)]
struct Reader {
    key: u64,
    interest: Interest,
    taken: mpsc::UnboundedSender<Taken>,
    queue: Arc<Mutex<Queue>>,
}

/// What a reader's queue holds and lost, which the Subscription's task fills
/// and the reader's [`Received`] empties.
#[derive(Debug, Default)]
struct Queue {
    /// The events sent and not read yet.
    held: usize,
    /// The events lost since it was full.
    lost: u64,
}

/// What a reader reads of the Subscription: the events its queue holds,
/// then the end.
#[derive(Debug)]
pub(crate) struct Received {
    taken: mpsc::UnboundedReceiver<Taken>,
    queue: Arc<Mutex<Queue>>,
}

impl Received {
    /// The next thing the reader hears: an event, the end of the
    /// Subscription, or, once its queue lost events and it read every one
    /// it held, how many it lost; `None` once it left.
    pub(crate) fn poll_recv(&mut self, context: &mut Context<'_>) -> Poll<Option<Taken>> {
        let polled = self.taken.poll_recv(context);
        let mut queue = lock(&self.queue);
        match polled {
            Poll::Ready(Some(Taken::Event(data))) => {
                queue.held -= 1;
                Poll::Ready(Some(Taken::Event(data)))
            }
            Poll::Ready(Some(other)) => Poll::Ready(Some(other)),
            // Nothing more is sent once it lost one, so `held` at 0 means
            // it read every event it held.
            Poll::Ready(None) | Poll::Pending if queue.lost > 0 && queue.held == 0 => {
                let lost = std::mem::take(&mut queue.lost);
                Poll::Ready(Some(Taken::Lost(lost)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Reader {
    /// Queues `data`, unless its queue holds `bound` events or lost one
    /// already, which counts it lost. Sent under the queue's lock, so the reader never sees
    /// an event it holds missing.
    fn take(&self, data: &Value, bound: ListenerQueue) {
        let mut queue = lock(&self.queue);
        if queue.lost > 0 || queue.held >= bound.get() {
            queue.lost += 1;
            return;
        }
        // A reader that left hears nothing more.
        if self.taken.send(Taken::Event(data.clone())).is_ok() {
            queue.held += 1;
        }
    }
}

/// A reader's place on the Subscription: dropping it leaves, and the last
/// to leave cancels the Subscription.
#[derive(Debug)]
pub(crate) struct Membership {
    core: Arc<Core>,
    subscription: Arc<Subscription>,
    key: u64,
}

impl Membership {
    /// Leaves once the Subscription handed every event heard before this
    /// call, such as those the engine published before it answered the
    /// reader's own call: its receiver then ends.
    pub(crate) fn finish(&self) {
        let _ended = self.subscription.finished.send(self.key);
    }
}

impl Drop for Membership {
    fn drop(&mut self) {
        self.subscription.leave(&self.core, self.key);
    }
}

impl Published {
    /// The Subscription of the Connection whose calls go through `sender`,
    /// whose task runs on `handle`, tracked by `tasks`.
    pub(crate) fn new(
        sender: Sender,
        handle: Handle,
        tasks: TaskTracker,
        bound: ListenerQueue,
    ) -> Self {
        Self(Arc::new(Core {
            sender,
            bound,
            handle,
            tasks,
            current: Mutex::new(None),
        }))
    }

    /// What its calls go through.
    pub(crate) fn sender(&self) -> Sender {
        self.0.sender.clone()
    }

    /// Joins the reader of `interest`, sending the Subscription with its
    /// credits when none is in flight, before anything sent after this
    /// call. It hears what it takes through the [`Received`] it gets, until
    /// its membership is dropped.
    pub(crate) fn join(&self, interest: Interest) -> (Membership, Received) {
        let (taken, receiver) = mpsc::unbounded_channel();
        let queue = Arc::new(Mutex::new(Queue::default()));
        let mut current = lock(&self.0.current);
        let subscription = Arc::clone(current.get_or_insert_with(|| self.subscribe()));
        let mut state = subscription.lock();
        let key = state.next_key;
        state.next_key += 1;
        state.readers.push(Reader {
            key,
            interest,
            taken,
            queue: Arc::clone(&queue),
        });
        drop(state);
        drop(current);
        let membership = Membership {
            core: Arc::clone(&self.0),
            subscription,
            key,
        };
        (
            membership,
            Received {
                taken: receiver,
                queue,
            },
        )
    }

    /// Sends `events.subscribe` with its credits and starts the task that
    /// reads it.
    fn subscribe(&self) -> Arc<Subscription> {
        // The call's id, then at most a batch for each credit granted.
        let (notified, heard) = mpsc::channel(CREDITS as usize + 1);
        let (reply, replied) = oneshot::channel();
        let (stop, stopped) = oneshot::channel();
        let (finished, finishing) = mpsc::unbounded_channel();
        let params = serde_json::to_value(EventsSubscribeParams::default())
            .expect("invariant: `events.subscribe`'s params serialize");
        let call = self.0.sender.call_granting(
            Outgoing {
                method: EVENTS_SUBSCRIBE.name.to_owned(),
                params,
                timeout: None,
                reply,
                notified: Some(notified),
                streams_itself: true,
            },
            CREDITS,
        );
        let subscription = Arc::new(Subscription {
            sender: self.0.sender.clone(),
            call,
            finished,
            bound: self.0.bound,
            state: Mutex::new(State {
                readers: Vec::new(),
                next_key: 0,
                settled: 0,
                stopped: false,
                stop: Some(stop),
            }),
        });
        let reading = read(
            Arc::clone(&self.0),
            Arc::clone(&subscription),
            heard,
            replied,
            stopped,
            finishing,
        );
        self.0.tasks.spawn_on(reading, &self.0.handle);
        subscription
    }
}

/// Reads `subscription` until its last reader leaves, or it ends: hands
/// each batch's events to their readers, lets a finished reader go once
/// it handed what was heard before, and tells them it ended.
async fn read(
    core: Arc<Core>,
    subscription: Arc<Subscription>,
    mut heard: mpsc::Receiver<Heard>,
    mut replied: oneshot::Receiver<Result<Value, Error>>,
    mut stopped: oneshot::Receiver<()>,
    mut finishing: mpsc::UnboundedReceiver<u64>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut stopped => return,
            Some(heard) = heard.recv() => subscription.hear(heard),
            Some(key) = finishing.recv() => {
                // Its call's answer came after these on the Connection.
                while let Ok(heard) = heard.try_recv() {
                    subscription.hear(heard);
                }
                subscription.leave(&core, key);
            }
            reply = &mut replied => {
                let failure = match reply {
                    Ok(Ok(_)) => None,
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some(Error::closed()),
                };
                subscription.ended(&core, failure);
                return;
            }
        }
    }
}

impl Subscription {
    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Lets the reader `key` go, if it has not left yet; the last to leave
    /// cancels it and stops its task.
    fn leave(&self, core: &Core, key: u64) {
        let mut current = lock(&core.current);
        let mut state = self.lock();
        let Some(at) = state.readers.iter().position(|reader| reader.key == key) else {
            return;
        };
        // Dropped last, so that its stream ends after the cancel is queued.
        let _left = state.readers.swap_remove(at);
        // Others still read it, or it ended, and its task with it.
        if !state.readers.is_empty() || state.stopped {
            return;
        }
        state.stopped = true;
        // Cancelled in order: after its call and the calls queued before
        // the leave, such as its last reader's own, and before anything sent
        // after it, such as the next reader's Subscription, even while its
        // id is not heard yet.
        self.sender.cancel_call(&self.call);
        state.stop = None;
        drop(state);
        if current
            .as_ref()
            .is_some_and(|held| std::ptr::eq(Arc::as_ptr(held), self))
        {
            *current = None;
        }
    }

    /// Hands a batch's events to their readers.
    fn hear(&self, heard: Heard) {
        match heard {
            Heard::Notified { method, params } if method == EVENTS => {
                match serde_json::from_value::<EventBatch>(params) {
                    Ok(batch) => self.hand(batch),
                    Err(error) => tracing::debug!(
                        target: "goodfetch::client",
                        %error,
                        "a batch of events that is no batch"
                    ),
                }
            }
            Heard::Sent(_) | Heard::Notified { .. } => {}
        }
    }

    /// Settles each event of `batch`, then hands it to the readers that
    /// take it: their own queues take it or lose it, so no reader holds a
    /// credit back, and the credits go before any of its events is read.
    fn hand(&self, batch: EventBatch) {
        self.settle(batch.events.len());
        for envelope in batch.events {
            if envelope.event == RESYNC.name {
                tracing::warn!(
                    target: "goodfetch::client",
                    event = RESYNC.name,
                    "The Connection's Subscription fell behind: the events published since its last read are gone, so a Session's stream may have missed pages its `pages()` lists"
                );
            }
            let state = self.lock();
            for reader in &state.readers {
                if reader.interest.takes(&envelope.event, &envelope.data) {
                    reader.take(&envelope.data, self.bound);
                }
            }
        }
    }

    /// Counts `events` settled, and grants as many again for each
    /// [`REGRANT`] of them, unless it stopped.
    fn settle(&self, events: usize) {
        let mut state = self.lock();
        if state.stopped {
            return;
        }
        let events = u32::try_from(events).unwrap_or(u32::MAX);
        state.settled = state.settled.saturating_add(events);
        while state.settled >= REGRANT {
            state.settled -= REGRANT;
            if let Some(id) = self.call.get() {
                self.sender.credit(id, REGRANT);
            }
        }
    }

    /// The engine answered it: the next reader sends another, and each of
    /// its readers hears it, with `failure` if one ended it.
    fn ended(&self, core: &Core, failure: Option<Error>) {
        let mut current = lock(&core.current);
        if current
            .as_ref()
            .is_some_and(|held| std::ptr::eq(Arc::as_ptr(held), self))
        {
            *current = None;
        }
        drop(current);
        let readers = {
            let mut state = self.lock();
            state.stopped = true;
            std::mem::take(&mut state.readers)
        };
        // The last reader gets the error itself, the others a copy.
        let last = readers.len().saturating_sub(1);
        let mut failure = failure;
        for (at, reader) in readers.into_iter().enumerate() {
            let told = if at == last {
                failure.take()
            } else {
                failure.as_ref().map(copied)
            };
            // One that lost events hears nothing more: once it read those
            // it held, it says what it lost.
            if lock(&reader.queue).lost == 0 {
                let _gone = reader.taken.send(Taken::Ended(told));
            }
        }
    }
}

impl Interest {
    /// Whether it takes the event `event` whose payload is `data`.
    fn takes(&self, event: &str, data: &Value) -> bool {
        event == self.event && data.get(self.field) == Some(&self.id)
    }
}

/// `error` again, for another reader: the engine's answer as it sent it,
/// or the close's own failure, `closed` or the close's cancellation.
fn copied(error: &Error) -> Error {
    if error.is_answer() || !error.is_ended_by_close() {
        Error::answered(error.to_object())
    } else if error.code() == ErrorCode::CLOSED {
        Error::closed()
    } else {
        Error::cancelled_by_close()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
