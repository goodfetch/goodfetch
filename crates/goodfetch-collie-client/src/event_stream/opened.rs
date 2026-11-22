//! The values the events of one call open (ADR 0081), such as the
//! `WebSocket` values of a page's `web_socket`, whose own events the same call
//! carries, each naming its value by ID. The call's own stream hands each
//! value as it opens, and each value's own stream hands its own events
//! alone, in the call's order; its close is the last it hands. The streams
//! share the call: whichever one the program reads reads the call's
//! batches and puts each event in the queue of the stream it is for, so a
//! value's stream goes on while the call's own stream is not read. An event
//! is settled, and its credit counts toward the next grant, once its stream
//! hands it, or at once when no stream will: a value's event after its
//! close or once its stream is dropped. The call stays lossless (ADR 0024):
//! a stream left unread holds its events' credits, and once the engine
//! waits on them, every stream of the call waits. Dropping the call's own
//! stream cancels the call; each value's stream then hands what it holds and
//! ends.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll, Wake, Waker};

use goodfetch_protocol::events::EventEnvelope;
use serde_json::{Value, json};

use super::Flow;
use crate::error::Error;
use crate::sync::{Mutex, MutexGuard};

/// What a listener's call opens: the event that opens a value, such as
/// `page.web_socket`, the namespace of that value's own events, such as
/// `web_socket`, whose `close` is its last, the property each of them
/// names the value by, such as `web_socket_id`, and the value's own events
/// this Client reads. An event of the namespace a newer engine adds is
/// passed over, as a stream passes over the events it does not read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opens {
    pub(crate) opener: &'static str,
    pub(crate) namespace: &'static str,
    pub(crate) id: &'static str,
    pub(crate) events: &'static [&'static str],
}

/// The call's batches, as its streams read them: its [`Flow`], or in the
/// loom tests a stand-in, since a Flow reads Tokio's channels, which loom
/// cannot run (ADR 0060).
pub(super) trait CallFlow {
    /// The call's next event, handed or passed over, its error once, or
    /// `None` once it ended.
    fn poll_envelope(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<EventEnvelope, Error>>>;

    /// Settles an event, which counts toward the call's next credits.
    fn settle(&mut self);

    /// Cancels the call.
    fn end(&mut self);
}

impl CallFlow for Flow {
    fn poll_envelope(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<EventEnvelope, Error>>> {
        Flow::poll_envelope(self, context)
    }

    fn settle(&mut self) {
        Flow::settle(self);
    }

    fn end(&mut self) {
        Flow::end(self);
    }
}

/// The call's own stream: the values its events open.
pub(super) struct Opening<F: CallFlow = Flow> {
    shared: Arc<Shared<F>>,
}

/// One opened value's stream: its own events, each `{event, data}`.
pub(super) struct Opened<F: CallFlow = Flow> {
    shared: Arc<Shared<F>>,
    key: u64,
}

/// A value opened: its opening payload and its own stream.
type OpenedValue<F> = (Value, Opened<F>);

/// The call and its streams' queues, behind one lock, and the wakers of
/// the streams waiting on the call.
struct Shared<F> {
    call: Mutex<Call<F>>,
    waiting: Arc<Waiting>,
}

/// The call its streams share.
struct Call<F> {
    flow: F,
    opens: Opens,
    /// The opening payloads the call's own stream has not handed, each
    /// with the key of the value it opened.
    opened: VecDeque<(Value, u64)>,
    /// The values opened and not closed, or closed with events unread.
    values: Vec<Held>,
    /// The key the next value gets; the call's own stream is key 0.
    next_key: u64,
    state: State,
}

/// Where the call stands.
enum State {
    /// It reads on.
    Open,
    /// It failed: the error the call's own stream hands once.
    Failed(Error),
    /// It ended, its error handed if it had one.
    Ended,
}

impl State {
    /// Whether the call ended, failed or not.
    fn ended(&self) -> bool {
        !matches!(self, Self::Open)
    }
}

/// An opened value's queue.
struct Held {
    key: u64,
    /// The ID its events name it by.
    id: Value,
    /// Its events not handed yet, each `{event, data}`.
    queue: VecDeque<Value>,
    /// Whether its close was heard, after which it takes nothing more.
    closed: bool,
    /// Whether its stream was dropped, so that no stream reads it.
    gone: bool,
}

/// The wakers of the streams waiting on the call, which the call's own
/// channels wake, each stream's under its key.
#[derive(Default)]
struct Waiting(Mutex<Vec<(u64, Waker)>>);

impl fmt::Debug for Opening {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let call = lock(&self.shared.call);
        formatter
            .debug_struct("Opening")
            .field("opens", &call.opens)
            .field("id", &call.flow.id)
            .field("ended", &call.state.ended())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Opened {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let call = lock(&self.shared.call);
        let id = call
            .values
            .iter()
            .find(|held| held.key == self.key)
            .map(|held| &held.id);
        formatter
            .debug_struct("Opened")
            .field("namespace", &call.opens.namespace)
            .field("id", &id)
            .finish_non_exhaustive()
    }
}

impl Waiting {
    /// Wakes `key` when the call hears something, unless it is already
    /// waiting.
    fn wait(&self, key: u64, waker: &Waker) {
        let mut waiting = lock(&self.0);
        match waiting.iter_mut().find(|(waiter, _)| *waiter == key) {
            Some((_, held)) if held.will_wake(waker) => {}
            Some((_, held)) => held.clone_from(waker),
            None => waiting.push((key, waker.clone())),
        }
    }

    /// Wakes the waiting streams `woken` names but `running`, which runs.
    fn wake_named(&self, running: u64, woken: &Woken) {
        let wakers: Vec<Waker> = {
            let mut waiting = lock(&self.0);
            let (now, later): (Vec<_>, Vec<_>) = waiting
                .drain(..)
                .partition(|(waiter, _)| *waiter != running && woken.names(*waiter));
            *waiting = later;
            now.into_iter().map(|(_, waker)| waker).collect()
        };
        for waker in wakers {
            waker.wake();
        }
    }
}

/// The streams a read of the call has something new for.
#[derive(Default)]
struct Woken {
    /// The streams it queued something for.
    keys: Vec<u64>,
    /// Whether the call ended, which every stream hears.
    all: bool,
}

impl Woken {
    /// Whether the stream `key` has something new.
    fn names(&self, key: u64) -> bool {
        self.all || self.keys.contains(&key)
    }
}

impl Wake for Waiting {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let woken = std::mem::take(&mut *lock(&self.0));
        for (_, waker) in woken {
            waker.wake();
        }
    }
}

/// The key of the call's own stream.
const OWN: u64 = 0;

impl<F: CallFlow> Opening<F> {
    /// The call's own stream of `flow`, whose events open values as
    /// `opens` says.
    pub(super) fn new(flow: F, opens: Opens) -> Self {
        Self {
            shared: Arc::new(Shared {
                call: Mutex::new(Call {
                    flow,
                    opens,
                    opened: VecDeque::new(),
                    values: Vec::new(),
                    next_key: OWN + 1,
                    state: State::Open,
                }),
                waiting: Arc::default(),
            }),
        }
    }

    /// The next value opened: its opening payload and its own stream, or
    /// the call's error once, or `None` once it ended.
    pub(super) fn poll_opened(
        &self,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<OpenedValue<F>, Error>>> {
        let polled = self.shared.poll(OWN, context, |call| {
            if let Some((data, key)) = call.opened.pop_front() {
                call.flow.settle();
                return Some(Some(Ok((data, key))));
            }
            match std::mem::replace(&mut call.state, State::Ended) {
                State::Open => {
                    call.state = State::Open;
                    None
                }
                State::Failed(error) => Some(Some(Err(error))),
                State::Ended => Some(None),
            }
        });
        polled.map(|item| {
            item.map(|read| {
                read.map(|(data, key)| {
                    let opened = Opened {
                        shared: Arc::clone(&self.shared),
                        key,
                    };
                    (data, opened)
                })
            })
        })
    }
}

impl<F: CallFlow> Drop for Opening<F> {
    /// Cancels the call: each value's stream hands what it holds, then
    /// ends.
    fn drop(&mut self) {
        let mut call = lock(&self.shared.call);
        call.flow.end();
        call.state = State::Ended;
        let unread: Vec<u64> = call.opened.drain(..).map(|(_, key)| key).collect();
        call.values.retain(|held| !unread.contains(&held.key));
        drop(call);
        let all = Woken {
            keys: Vec::new(),
            all: true,
        };
        self.shared.waiting.wake_named(OWN, &all);
    }
}

impl<F: CallFlow> Opened<F> {
    /// The value's next own event, `{event, data}`, or `None` once it
    /// handed its close or the call ended.
    pub(super) fn poll_event(&self, context: &mut Context<'_>) -> Poll<Option<Value>> {
        let key = self.key;
        self.shared.poll(key, context, |call| {
            let index = call.values.iter().position(|held| held.key == key);
            if let Some(event) = index.and_then(|index| call.values[index].queue.pop_front()) {
                call.flow.settle();
                return Some(Some(event));
            }
            let done = index.is_none_or(|index| call.values[index].closed);
            if done || call.state.ended() {
                if let Some(index) = index {
                    call.values.swap_remove(index);
                }
                return Some(None);
            }
            None
        })
    }
}

impl<F: CallFlow> Drop for Opened<F> {
    /// Its value takes nothing more: the events it held are settled.
    fn drop(&mut self) {
        let mut call = lock(&self.shared.call);
        let Some(index) = call.values.iter().position(|held| held.key == self.key) else {
            return;
        };
        let held = &mut call.values[index];
        held.gone = true;
        let unread = held.queue.len();
        held.queue.clear();
        if held.closed || call.state.ended() {
            call.values.swap_remove(index);
        }
        (0..unread).for_each(|_| call.flow.settle());
    }
}

impl<F: CallFlow> Shared<F> {
    /// What `take` finds for the stream `key`, reading the call's batches
    /// until it finds something or the call has nothing more for now.
    fn poll<T>(
        &self,
        key: u64,
        context: &mut Context<'_>,
        mut take: impl FnMut(&mut Call<F>) -> Option<T>,
    ) -> Poll<T> {
        let mut call = lock(&self.call);
        let waker = Waker::from(Arc::clone(&self.waiting));
        let mut heard = Context::from_waker(&waker);
        let mut woken = Woken::default();
        let polled = loop {
            if let Some(found) = take(&mut call) {
                break Poll::Ready(found);
            }
            // Waits before it reads, so a batch heard meanwhile wakes it.
            self.waiting.wait(key, context.waker());
            match call.flow.poll_envelope(&mut heard) {
                Poll::Ready(Some(Ok(envelope))) => woken.keys.extend(call.route(envelope)),
                Poll::Ready(Some(Err(error))) => {
                    call.state = State::Failed(error);
                    woken.all = true;
                }
                Poll::Ready(None) => {
                    call.state = State::Ended;
                    woken.all = true;
                }
                Poll::Pending => break Poll::Pending,
            }
        };
        drop(call);
        // What it routed may be other streams'.
        self.waiting.wake_named(key, &woken);
        polled
    }
}

impl<F: CallFlow> Call<F> {
    /// Puts `envelope` in the queue of the stream it is for, whose key it
    /// returns, or settles it when none will hand it.
    fn route(&mut self, envelope: EventEnvelope) -> Option<u64> {
        let EventEnvelope { event, data, .. } = envelope;
        let id = data.get(self.opens.id).cloned().unwrap_or(Value::Null);
        if event == self.opens.opener {
            let key = self.next_key;
            self.next_key += 1;
            self.values.push(Held {
                key,
                id,
                queue: VecDeque::new(),
                closed: false,
                gone: false,
            });
            self.opened.push_back((data, key));
            return Some(OWN);
        }
        let own = event
            .strip_prefix(self.opens.namespace)
            .and_then(|rest| rest.strip_prefix('.'));
        let read = own.is_some() && self.opens.events.contains(&event.as_str());
        let index = read
            .then(|| {
                self.values
                    .iter()
                    .position(|held| !held.closed && held.id == id)
            })
            .flatten();
        let Some(index) = index else {
            self.flow.settle();
            return None;
        };
        let closes = own == Some("close");
        let held = &mut self.values[index];
        held.closed = closes;
        if held.gone {
            if closes {
                self.values.swap_remove(index);
            }
            self.flow.settle();
            return None;
        }
        held.queue.push_back(json!({"event": event, "data": data}));
        Some(held.key)
    }
}

/// Takes `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(all(test, goodfetch_loom))]
mod loom_tests;
