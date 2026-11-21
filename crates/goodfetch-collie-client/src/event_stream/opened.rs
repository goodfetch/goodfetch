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
/// `web_socket`, whose `close` is its last, and the property each of them
/// names the value by, such as `web_socket_id`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opens {
    pub(crate) opener: &'static str,
    pub(crate) namespace: &'static str,
    pub(crate) id: &'static str,
}

/// The call's own stream: the values its events open.
pub(crate) struct Opening {
    shared: Arc<Shared>,
}

/// One opened value's stream: its own events, each `{event, data}`.
pub(crate) struct Opened {
    shared: Arc<Shared>,
    key: u64,
}

/// The call and its streams' queues, behind one lock, and the wakers of
/// the streams waiting on the call.
struct Shared {
    call: Mutex<Call>,
    waiting: Arc<Waiting>,
}

/// The call its streams share.
struct Call {
    flow: Flow,
    opens: Opens,
    /// The opening payloads the call's own stream has not handed, each
    /// with the key of the value it opened.
    opened: VecDeque<(Value, u64)>,
    /// The values opened and not closed, or closed with events unread.
    values: Vec<Held>,
    /// The key the next value gets; the call's own stream is key 0.
    next_key: u64,
    state: State,
    /// Whether the call's own stream was dropped.
    gone: bool,
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
    /// Whether no stream reads it: it was dropped, or never made.
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

    /// Wakes every waiting stream but `key`, which is running.
    fn wake_others(&self, key: u64) {
        let woken: Vec<Waker> = {
            let mut waiting = lock(&self.0);
            let (others, own): (Vec<_>, Vec<_>) =
                waiting.drain(..).partition(|(waiter, _)| *waiter != key);
            *waiting = own;
            others.into_iter().map(|(_, waker)| waker).collect()
        };
        woken.into_iter().for_each(Waker::wake);
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

impl Opening {
    /// The call's own stream of `flow`, whose events open values as
    /// `opens` says.
    pub(super) fn new(flow: Flow, opens: Opens) -> Self {
        Self {
            shared: Arc::new(Shared {
                call: Mutex::new(Call {
                    flow,
                    opens,
                    opened: VecDeque::new(),
                    values: Vec::new(),
                    next_key: OWN + 1,
                    state: State::Open,
                    gone: false,
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
    ) -> Poll<Option<Result<(Value, Opened), Error>>> {
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

impl Drop for Opening {
    /// Cancels the call: each value's stream hands what it holds, then
    /// ends.
    fn drop(&mut self) {
        let mut call = lock(&self.shared.call);
        call.gone = true;
        call.flow.end();
        call.state = State::Ended;
        let unread: Vec<u64> = call.opened.drain(..).map(|(_, key)| key).collect();
        call.values.retain(|held| !unread.contains(&held.key));
        drop(call);
        self.shared.waiting.wake_others(OWN);
    }
}

impl Opened {
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

impl Drop for Opened {
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

impl Shared {
    /// What `take` finds for the stream `key`, reading the call's batches
    /// until it finds something or the call has nothing more for now.
    fn poll<T>(
        &self,
        key: u64,
        context: &mut Context<'_>,
        mut take: impl FnMut(&mut Call) -> Option<T>,
    ) -> Poll<T> {
        let mut call = lock(&self.call);
        let waker = Waker::from(Arc::clone(&self.waiting));
        let mut heard = Context::from_waker(&waker);
        loop {
            if let Some(found) = take(&mut call) {
                return Poll::Ready(found);
            }
            // Waits before it reads, so a batch heard meanwhile wakes it.
            self.waiting.wait(key, context.waker());
            match call.flow.poll_envelope(&mut heard) {
                Poll::Ready(Some(Ok(envelope))) => call.route(envelope),
                Poll::Ready(Some(Err(error))) => call.state = State::Failed(error),
                Poll::Ready(None) => call.state = State::Ended,
                Poll::Pending => return Poll::Pending,
            }
            // What it routed may be another stream's.
            self.waiting.wake_others(key);
        }
    }
}

impl Call {
    /// Puts `envelope` in the queue of the stream it is for, or settles it
    /// when none will hand it.
    fn route(&mut self, envelope: EventEnvelope) {
        let EventEnvelope { event, data, .. } = envelope;
        let id = data.get(self.opens.id).cloned().unwrap_or(Value::Null);
        if event == self.opens.opener {
            let key = self.next_key;
            self.next_key += 1;
            if self.gone {
                self.flow.settle();
                return;
            }
            self.values.push(Held {
                key,
                id,
                queue: VecDeque::new(),
                closed: false,
                gone: false,
            });
            self.opened.push_back((data, key));
            return;
        }
        let own = event
            .strip_prefix(self.opens.namespace)
            .and_then(|rest| rest.strip_prefix('.'));
        let index = own.and_then(|_| {
            self.values
                .iter()
                .position(|held| !held.closed && held.id == id)
        });
        let Some(index) = index else {
            self.flow.settle();
            return;
        };
        let closes = own == Some("close");
        let held = &mut self.values[index];
        held.closed = closes;
        if held.gone {
            if closes {
                self.values.swap_remove(index);
            }
            self.flow.settle();
            return;
        }
        held.queue.push_back(json!({"event": event, "data": data}));
    }
}

/// Takes `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
