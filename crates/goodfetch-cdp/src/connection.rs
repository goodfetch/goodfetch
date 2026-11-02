//! The client's connection to one browser over its pipe: an actor, the
//! [`Driver`], owns both ends and the commands in flight; any task holding
//! a [`Connection`] sends commands through it and subscribes to the events
//! of one target's session (CDP's flat sessions, `Target.attachToTarget`
//! with `flatten`). When the browser closes its end, every command in
//! flight fails with [`Failure::Disconnected`] and every subscription ends;
//! when it detaches a target's session, that session's commands in flight
//! fail and its subscriptions end, each saying why ([`SubscriptionEnd`]).
//! An answer longer than one message the framing reads fails its own
//! command alone, with [`Failure::TooLarge`], and the connection goes on.
//!
//! Nothing waits on the driver without a bound: a command's whole wait,
//! for the driver and for its answer, is under its timeout, a subscription
//! starts without the driver, and the driver never waits on a subscriber,
//! so a browser that stops reading its pipe, or a subscription its holder
//! never reads, holds up no other call.
#![expect(
    clippy::disallowed_types,
    reason = "the subscribers' list is shared by every handle and the driver, each lock held for one list operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};

use crate::failure::{Failure, PartyText, Refusal};
use crate::framing::{Framer, FramingError, MAX_FRAME, frame};

/// How many requests wait for the driver before their senders wait.
const REQUESTS: usize = 64;

/// How many events wait for a subscriber before the driver ends its
/// subscription: one that falls this far behind is no longer read, and the
/// driver never waits for it.
const EVENTS: usize = 4_096;

/// The bytes read from the pipe at once.
const READ: usize = 64 * 1024;

/// A target's session on the connection: the `sessionId` its commands and
/// events carry.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub String);

/// An event as the browser sent it.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// `Domain.event`.
    pub method: String,
    /// Its params.
    pub params: Value,
}

impl Event {
    /// Its params as `E`'s, when it is `E`; params that do not read as
    /// `E`'s, which only a browser the bindings do not match sends, are
    /// logged at `warn` with the method, never with the params.
    pub fn read<E: crate::Event>(&self) -> Option<E> {
        if self.method != E::METHOD {
            return None;
        }
        E::deserialize(&self.params)
            .inspect_err(|_| {
                tracing::warn!(
                    cdp.method = E::METHOD,
                    "the browser sent an event whose params do not read as the bindings say"
                );
            })
            .ok()
    }
}

/// The events of one session, from when it was subscribed.
#[derive(Debug)]
pub struct Subscription {
    events: mpsc::Receiver<Arc<Event>>,
    /// Why the driver ended it, which it says before it drops its sender.
    why: oneshot::Receiver<SubscriptionEnd>,
    /// Why it ended, once known.
    ended: Option<SubscriptionEnd>,
}

/// Why a subscription ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionEnd {
    /// The browser detached its session's target: the target sends nothing
    /// more.
    Detached,
    /// Its holder fell [`EVENTS`] events behind, so the events after it
    /// stopped are lost while its target may still send more.
    FellBehind,
    /// The connection ended.
    Disconnected,
}

impl Subscription {
    /// The next event; `None` once it ended: the connection ended, its
    /// session's target detached, or its holder fell [`EVENTS`] behind,
    /// which [`ended`](Self::ended) tells apart.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub async fn next(&mut self) -> Option<Arc<Event>> {
        self.events.recv().await
    }

    /// Why it ended, once [`next`](Self::next) answered `None`; `None`
    /// while it goes on.
    pub fn ended(&mut self) -> Option<SubscriptionEnd> {
        if self.ended.is_none() && self.events.is_closed() && self.events.is_empty() {
            // The driver says why before it drops the events' sender, so a
            // sender dropped without a word is the connection's end.
            self.ended = Some(match self.why.try_recv() {
                Ok(why) => why,
                Err(_) => SubscriptionEnd::Disconnected,
            });
        }
        self.ended
    }
}

struct Request {
    method: &'static str,
    session: Option<SessionId>,
    params: Value,
    answer: oneshot::Sender<Result<Value, Failure>>,
}

/// One subscription as the driver holds it.
#[derive(Debug)]
struct Subscriber {
    /// The session whose events it takes.
    session: Option<SessionId>,
    events: mpsc::Sender<Arc<Event>>,
    /// Where the driver says why it ended it.
    why: oneshot::Sender<SubscriptionEnd>,
}

impl Subscriber {
    /// Ends it, saying `why`.
    fn end(self, why: SubscriptionEnd) {
        let _ = self.why.send(why);
    }
}

/// The subscriptions of the connection, which every handle adds to and the
/// driver hands each event to.
#[derive(Debug, Default)]
struct Subscribers {
    /// Each subscription's session and the sender of its events.
    list: Vec<Subscriber>,
    /// Whether the connection ended, so a new subscription ends at once.
    ended: bool,
}

/// The connection's subscribers, shared.
type Shared = Arc<Mutex<Subscribers>>;

/// The subscribers, a panic in another holder notwithstanding: each change
/// is one list operation, so none leaves the list half changed.
fn lock(subscribers: &Shared) -> MutexGuard<'_, Subscribers> {
    subscribers.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A handle to the browser's connection, which any task may clone.
#[derive(Clone, Debug)]
pub struct Connection {
    requests: mpsc::Sender<Request>,
    subscribers: Shared,
}

/// How the connection ended.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ended {
    /// The browser closed its end of the pipe.
    Closed,
    /// Every handle was dropped, so the client closed its end.
    Dropped,
    /// Reading or writing the pipe failed.
    Broken(std::io::ErrorKind),
}

/// The connection's actor: run it as a task of the browser's owner.
pub struct Driver<R, W> {
    reader: R,
    writer: W,
    requests: mpsc::Receiver<Request>,
    subscribers: Shared,
    /// The longest message it reads: [`MAX_FRAME`].
    frame_limit: usize,
    /// Where each command's encode and send are counted (ADR 0140's C1): the
    /// one seam every command crosses.
    work: CountedWork,
}

impl Connection {
    /// A connection over the browser's end of the pipe `reader` (what it
    /// writes) and `writer` (what it reads), and its driver, which counts
    /// each command's work on `work`.
    pub fn new<R, W>(reader: R, writer: W, work: CountedWork) -> (Self, Driver<R, W>)
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let subscribers = Shared::default();
        (
            Self {
                requests,
                subscribers: Arc::clone(&subscribers),
            },
            Driver {
                reader,
                writer,
                requests: receiver,
                subscribers,
                frame_limit: MAX_FRAME,
                work,
            },
        )
    }

    /// Sends `C` on `session` (the browser's own when `None`) and answers
    /// what it returns.
    ///
    /// # Errors
    ///
    /// Each [`Failure`]: the browser's refusal, no answer within `timeout`,
    /// counted from the call, the driver's own wait to send it included,
    /// the connection's end, or an answer that is not what `C` returns.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the command was written does not take it
    /// back: the browser still carries it out.
    pub async fn send<C: crate::Command>(
        &self,
        session: Option<&SessionId>,
        params: &C,
        timeout: Duration,
    ) -> Result<C::Returns, Failure> {
        let params = serde_json::to_value(params).expect("invariant: a command's params serialize");
        let answer = self.send_raw(C::METHOD, session, params, timeout).await?;
        read(C::METHOD, answer)
    }

    /// Sends `method` with its params as JSON, the way down for a command
    /// no binding names.
    ///
    /// # Errors
    ///
    /// As [`Connection::send`].
    ///
    /// # Cancel safety
    ///
    /// As [`Connection::send`].
    #[tracing::instrument(name = "cdp_command", level = "trace", skip_all, fields(cdp.method = method))]
    pub async fn send_raw(
        &self,
        method: &'static str,
        session: Option<&SessionId>,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        let (answer, answered) = oneshot::channel();
        let request = Request {
            method,
            session: session.cloned(),
            params,
            answer,
        };
        let sent = async {
            self.requests
                .send(request)
                .await
                .map_err(|_| Failure::Disconnected)?;
            answered.await.map_err(|_| Failure::Disconnected)?
        };
        tokio::time::timeout(timeout, sent)
            .await
            .unwrap_or(Err(Failure::TimedOut { method }))
    }

    /// The events of `session` (the browser's own when `None`) the driver
    /// reads from now on, without waiting for the driver; it ends with the
    /// connection, once the browser detached its session's target, or once
    /// its holder fell [`EVENTS`] events behind.
    pub fn subscribe(&self, session: Option<&SessionId>) -> Subscription {
        let (events, receiver) = mpsc::channel(EVENTS);
        let (why, said) = oneshot::channel();
        let mut subscribers = lock(&self.subscribers);
        // A connection that ended gives a subscription that ends at once.
        if !subscribers.ended {
            subscribers.list.push(Subscriber {
                session: session.cloned(),
                events,
                why,
            });
        }
        Subscription {
            events: receiver,
            why: said,
            ended: None,
        }
    }

    /// Whether the connection ended: the browser closed it, or reading or
    /// writing it failed. A subscription that ended while this is false fell
    /// behind.
    pub fn is_closed(&self) -> bool {
        lock(&self.subscribers).ended
    }
}

fn read<T: DeserializeOwned>(method: &'static str, answer: Value) -> Result<T, Failure> {
    serde_json::from_value(answer).map_err(|error| Failure::Unreadable {
        method,
        message: PartyText::new(error.to_string()),
    })
}

struct Pending {
    method: &'static str,
    /// The target's session it was sent on, if any.
    session: Option<SessionId>,
    answer: oneshot::Sender<Result<Value, Failure>>,
}

/// The browser's event that a target's session ended: its target closed,
/// or its browser context was disposed under it.
const DETACHED: &str = "Target.detachedFromTarget";

/// The code of Chromium's refusal of a command on a session it no longer
/// has.
const NO_SESSION: i64 = -32001;

impl<R, W> Driver<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    /// The driver, reading messages of at most `limit` bytes.
    #[cfg(test)]
    fn with_frame_limit(mut self, limit: usize) -> Self {
        self.frame_limit = limit;
        self
    }

    /// Runs the connection until the browser closes it or every handle is
    /// dropped, and says which.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the client's end: the commands in flight
    /// get no answer.
    pub async fn run(mut self) -> Ended {
        let mut framer = Framer::with_limit(self.frame_limit);
        let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
        let mut next_id: u64 = 1;
        let mut buffer = vec![0; READ];
        let ended = loop {
            tokio::select! {
                request = self.requests.recv() => {
                    let Some(Request { method, session, params, answer }) = request else {
                        break Ended::Dropped;
                    };
                    let id = next_id;
                    next_id += 1;
                    let mut message = json!({ "id": id, "method": method, "params": params });
                    if let Some(session) = &session {
                        message["sessionId"] = Value::String(session.0.clone());
                    }
                    tracing::trace!(target: "goodfetch::cdp::command", method, id, "sending");
                    self.work.count(Row::CdpEncodes);
                    let bytes = frame(&message);
                    self.work.count(Row::CdpSends);
                    if let Err(error) = self.writer.write_all(&bytes).await {
                        let _ = answer.send(Err(Failure::Disconnected));
                        break Ended::Broken(error.kind());
                    }
                    pending.insert(id, Pending { method, session, answer });
                }
                read = self.reader.read(&mut buffer) => {
                    match read {
                        Ok(0) => break Ended::Closed,
                        Ok(count) => framer.push(&buffer[..count]),
                        Err(error) => break Ended::Broken(error.kind()),
                    }
                    loop {
                        match framer.next_frame() {
                            Ok(Some(bytes)) => dispatch(&bytes, &mut pending, &self.subscribers),
                            Ok(None) => break,
                            Err(FramingError::TooLarge { answering }) => {
                                dropped(answering, &mut pending);
                            }
                        }
                    }
                }
            }
        };
        for (_, Pending { answer, .. }) in pending {
            let _ = answer.send(Err(Failure::Disconnected));
        }
        let mut subscribers = lock(&self.subscribers);
        subscribers.ended = true;
        subscribers.list.clear();
        ended
    }
}

/// Fails the command `answering` names, whose answer was past the framing's
/// limit and was dropped; a message that answers none, an event, is lost.
fn dropped(answering: Option<u64>, pending: &mut BTreeMap<u64, Pending>) {
    match answering.and_then(|id| pending.remove(&id)) {
        Some(Pending { method, answer, .. }) => {
            tracing::warn!(
                method,
                "the browser's answer was past the framing's limit and was dropped"
            );
            let _ = answer.send(Err(Failure::TooLarge { method }));
        }
        None => {
            tracing::warn!(
                "the browser sent a message past the framing's limit, which was dropped"
            );
        }
    }
}

/// Hands one message to the command it answers or to the subscribers of
/// its session, never waiting for one: a subscription its holder dropped
/// leaves, and one that fell [`EVENTS`] behind ends.
fn dispatch(bytes: &[u8], pending: &mut BTreeMap<u64, Pending>, subscribers: &Shared) {
    let Ok(mut message) = serde_json::from_slice::<Value>(bytes) else {
        tracing::warn!("the browser sent a message that is not JSON");
        return;
    };
    if let Some(id) = message.get("id").and_then(Value::as_u64) {
        let Some(Pending { method, answer, .. }) = pending.remove(&id) else {
            return;
        };
        let answered = match message.get("error") {
            Some(error) => Err(Failure::refused(method, error)),
            None => Ok(message.get_mut("result").map_or(Value::Null, Value::take)),
        };
        let _ = answer.send(answered);
        return;
    }
    // The message's own strings and params are taken, never copied: every
    // subscriber shares the one event.
    let Some(Value::String(method)) = message.get_mut("method").map(Value::take) else {
        return;
    };
    let session = match message.get_mut("sessionId").map(Value::take) {
        Some(Value::String(session)) => Some(SessionId(session)),
        _ => None,
    };
    let event = Arc::new(Event {
        method,
        params: message.get_mut("params").map_or(Value::Null, Value::take),
    });
    // A target's detach comes on its parent's session: the browser's own
    // for a page, the page's for a frame it auto-attached.
    let gone = (event.method == DETACHED)
        .then(|| event.params.get("sessionId").and_then(Value::as_str))
        .flatten()
        .map(|gone| SessionId(gone.to_owned()));
    if let Some(gone) = &gone {
        detached(gone, pending);
    }
    let mut subscribers = lock(subscribers);
    for subscriber in std::mem::take(&mut subscribers.list) {
        if subscriber.events.is_closed() {
            continue;
        }
        if gone
            .as_ref()
            .is_some_and(|gone| subscriber.session.as_ref() == Some(gone))
        {
            subscriber.end(SubscriptionEnd::Detached);
            continue;
        }
        if subscriber.session != session {
            subscribers.list.push(subscriber);
            continue;
        }
        match subscriber.events.try_send(Arc::clone(&event)) {
            Ok(()) => subscribers.list.push(subscriber),
            Err(mpsc::error::TrySendError::Closed(_)) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    method = event.method,
                    "a subscription of the browser's connection fell too far behind and ends"
                );
                subscriber.end(SubscriptionEnd::FellBehind);
            }
        }
    }
}

/// Fails at once each command in flight on the session `gone` the browser
/// detached, as its `Target.detachedFromTarget` names it: the browser
/// never answers one, and its target is gone.
fn detached(gone: &SessionId, pending: &mut BTreeMap<u64, Pending>) {
    let ids: Vec<u64> = pending
        .iter()
        .filter(|(_, command)| {
            command
                .session
                .as_ref()
                .is_some_and(|session| session == gone)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if let Some(Pending { method, answer, .. }) = pending.remove(&id) {
            let _ = answer.send(Err(Failure::Refused {
                method,
                refusal: Refusal::TargetGone,
                code: NO_SESSION,
                message: PartyText::new("the browser detached the target's session"),
            }));
        }
    }
}

#[cfg(test)]
mod tests;
