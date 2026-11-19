//! The Connection's own Subscription (ADRs 0031 and 0081): one
//! `events.subscribe`, sent with its credits when the first reader of the
//! events the engine publishes on its event stream joins, such as a
//! Session's stream, and cancelled once the last leaves; the next to join
//! sends another. A task on the Connection's tracker reads it and hands
//! each event to the readers that take it, those of its name whose payload
//! names their handle. An event's credit goes back once each of them read
//! it or dropped it, so an unread stream slows the Subscription; the engine
//! then falls behind to `events.resync` rather than wait, and the events in
//! between are gone, which is logged.

use std::sync::{Arc, PoisonError};

use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::events::{EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventsSubscribeParams};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio_util::task::TaskTracker;

use crate::error::Error;
use crate::rpc::{CallId, Heard, Outgoing, Sender};
use crate::sync::{Mutex, MutexGuard};

/// The credits the Subscription grants when it is sent: at most this many
/// of its events wait in its readers.
const CREDITS: u32 = 16;

/// How many of its events its readers settle before it grants as many
/// again.
const REGRANT: u32 = 8;

/// The event that says the engine no longer held the Subscription's
/// position: the events since are gone.
const RESYNC: &str = "events.resync";

/// What a reader hears of the Subscription.
#[derive(Debug)]
pub(crate) enum Taken {
    /// An event's payload it takes, and the event's credit, which goes
    /// back once every reader that took it dropped its share.
    Event(Value, Arc<Credit>),
    /// The Subscription ended without its cancel: the engine answered it,
    /// with the error that ended it, if one did.
    Ended(Option<Error>),
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
}

/// One event's credit, shared by the readers that took the event: the last
/// share dropped settles it.
#[derive(Debug)]
pub(crate) struct Credit(Arc<Subscription>);

impl Drop for Credit {
    fn drop(&mut self) {
        self.0.settle();
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

impl Drop for Membership {
    fn drop(&mut self) {
        let mut current = lock(&self.core.current);
        let mut state = self.subscription.lock();
        state.readers.retain(|reader| reader.key != self.key);
        // Others still read it, or it ended, and its task with it.
        if !state.readers.is_empty() || state.stopped {
            return;
        }
        state.stopped = true;
        // Cancelled in order: after its call and the calls queued before
        // the leave, such as its last reader's own, and before anything sent
        // after it, such as the next reader's Subscription, even while its
        // id is not heard yet.
        self.subscription
            .sender
            .cancel_sent(&self.subscription.call);
        state.stop = None;
        drop(state);
        if current
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &self.subscription))
        {
            *current = None;
        }
    }
}

impl Published {
    /// The Subscription of the Connection whose calls go through `sender`,
    /// whose task runs on `handle`, tracked by `tasks`.
    pub(crate) fn new(sender: Sender, handle: Handle, tasks: TaskTracker) -> Self {
        Self(Arc::new(Core {
            sender,
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
    /// call. It hears what it takes on the receiver, until its membership
    /// is dropped.
    pub(crate) fn join(&self, interest: Interest) -> (Membership, mpsc::UnboundedReceiver<Taken>) {
        let (taken, receiver) = mpsc::unbounded_channel();
        let mut current = lock(&self.0.current);
        let subscription = Arc::clone(current.get_or_insert_with(|| self.subscribe()));
        let mut state = subscription.lock();
        let key = state.next_key;
        state.next_key += 1;
        state.readers.push(Reader {
            key,
            interest,
            taken,
        });
        drop(state);
        drop(current);
        let membership = Membership {
            core: Arc::clone(&self.0),
            subscription,
            key,
        };
        (membership, receiver)
    }

    /// Sends `events.subscribe` with its credits and starts the task that
    /// reads it.
    fn subscribe(&self) -> Arc<Subscription> {
        // The call's id, then at most a batch for each credit granted.
        let (notified, heard) = mpsc::channel(CREDITS as usize + 1);
        let (reply, replied) = oneshot::channel();
        let (stop, stopped) = oneshot::channel();
        let params = serde_json::to_value(EventsSubscribeParams::default())
            .expect("invariant: `events.subscribe`'s params serialize");
        let call = self.0.sender.call_granting(
            Outgoing {
                method: EVENTS_SUBSCRIBE.name.to_owned(),
                params,
                timeout: None,
                reply,
                notified: Some(notified),
            },
            CREDITS,
        );
        let subscription = Arc::new(Subscription {
            sender: self.0.sender.clone(),
            call,
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
        );
        self.0.tasks.spawn_on(reading, &self.0.handle);
        subscription
    }
}

/// Reads `subscription` until its last reader leaves, or it ends: hands
/// each batch's events to their readers, and tells them it ended.
async fn read(
    core: Arc<Core>,
    subscription: Arc<Subscription>,
    mut heard: mpsc::Receiver<Heard>,
    mut replied: oneshot::Receiver<Result<Value, Error>>,
    mut stopped: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut stopped => return,
            Some(heard) = heard.recv() => subscription.hear(heard),
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

    /// Hands a batch's events to their readers.
    fn hear(self: &Arc<Self>, heard: Heard) {
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

    /// Hands each event of `batch` to the readers that take it; one that
    /// none takes is settled at once.
    fn hand(self: &Arc<Self>, batch: EventBatch) {
        for envelope in batch.events {
            if envelope.event == RESYNC {
                tracing::warn!(
                    target: "goodfetch::client",
                    event = RESYNC,
                    "The Connection's Subscription fell behind: the events published since its last read are gone, so a Session's stream may have missed pages its `pages()` lists"
                );
            }
            let takers: Vec<mpsc::UnboundedSender<Taken>> = self
                .lock()
                .readers
                .iter()
                .filter(|reader| reader.interest.takes(&envelope.event, &envelope.data))
                .map(|reader| reader.taken.clone())
                .collect();
            // Each taker holds a share; the last share dropped settles it,
            // and with no taker that is now.
            let credit = Arc::new(Credit(Arc::clone(self)));
            for taker in takers {
                let _gone = taker.send(Taken::Event(envelope.data.clone(), Arc::clone(&credit)));
            }
        }
    }

    /// Counts one event settled, and grants as many again every
    /// [`REGRANT`], unless it stopped.
    fn settle(&self) {
        let mut state = self.lock();
        if state.stopped {
            return;
        }
        state.settled += 1;
        if state.settled == REGRANT {
            state.settled = 0;
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
        let mut failure = failure;
        for reader in readers {
            let told = failure.as_ref().map(copied);
            let _gone = reader.taken.send(Taken::Ended(failure.take().or(told)));
        }
    }
}

impl Interest {
    /// Whether it takes the event `event` whose payload is `data`.
    fn takes(&self, event: &str, data: &Value) -> bool {
        event == self.event && data.get(self.field) == Some(&self.id)
    }
}

/// `error` again, for another reader: the engine's answer as it sent it, or
/// `closed`.
fn copied(error: &Error) -> Error {
    if !error.is_answer() && error.code() == ErrorCode::CLOSED {
        Error::closed()
    } else {
        Error::answered(error.to_object())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
