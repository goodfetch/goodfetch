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
//!
//! A caller that must not wait for an answer before it goes on, as a Page's
//! owner in its loop, starts its command with [`Connection::start`]: the
//! command is queued at once, in the order started, and its [`Answer`] is a
//! future the caller polls when it will. Dropping the answer leaves the wait
//! alone; the command, queued, is still written.
//!
//! The driver owns its state: the commands in flight, and the table of
//! readers by session, so an event visits the readers of its session alone
//! (ADR 0140's `C1.event_visits`). A subscription starts without the driver
//! by joining a short list beside it, under a lock held for one push; the
//! driver takes that list into its table before it hands on each event, so
//! a subscription that started before the browser sent an event gets it. A
//! command whose caller left its wait, by its timeout or its drop, leaves
//! the commands in flight: its caller tells the driver, one bounded word
//! for any number of callers (`C1.pending_orphans` reads 0). The browser's
//! answer to such a command, or to any command no caller waits for, is
//! dropped with a `debug` log of its ID: no other command takes it and no
//! subscription sees it.
//!
//! The driver reads a message's envelope alone, what routes it: its `id`,
//! `method` and session, and the session a detach names. An answer's result
//! is decoded for its command; an event keeps the browser's message whole,
//! and its first reader decodes its params once, from the message's bytes,
//! for all its readers (ADR 0140's `C1.event_decodes`), so an event no reader
//! of its session wants is never decoded. Every buffer stays bounded: a subscriber holds at most
//! [`EVENTS`](readers::EVENTS) events and its quota of bytes,
//! [`EVENT_BYTES`], past which it
//! ends as one that fell behind.
//!
//! A reader of several sessions, as a Page's reads those of its
//! out-of-process frames and of its workers (spec #2256, "One reader per
//! Page"), is listed in the table under each of them ([`MoreSessions`])
//! with a clone of its one bounded events' sender, so its events share one
//! buffer and one order; only its first session's listing holds where the
//! driver says why it ended. The detach of one of the other sessions takes
//! that listing out alone; the reader ends with its first session's detach,
//! by falling behind, or with the connection. Such a reader hands each
//! event on to the parts it reads for through a [`Handing`], as the driver
//! hands its own, and the parts subscribe to it through its [`Handout`] as
//! they would to the connection (#2265).
//!
//! A task that waits for the connection's end awaits
//! [`Connection::closed`], reading no event.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::Instrument as _;

pub use self::answer::Answer;
use self::answer::Waiting;
use self::envelope::{Envelope, range_in, text};
pub use self::event::Event;
use self::readers::{EVENT_BYTES, Readers, Shared, join, lock};
pub use self::readers::{Handing, Handout, MoreSessions, Subscription, SubscriptionEnd};
use crate::failure::{Failure, PartyText, Refusal};
use crate::framing::{Framer, FramingError, MAX_FRAME, frame};

mod answer;
mod envelope;
mod event;
mod readers;

/// How many requests wait for the driver before their senders wait.
const REQUESTS: usize = 64;

/// The bytes read from the pipe at once.
const READ: usize = 64 * 1024;

/// The domain whose messages pause a page's requests and answer them: each
/// of its commands and events is `C5.fetch_messages_no_rule`'s work (ADR 0140).
const FETCH: &str = "Fetch.";

/// The commands that answer a paused request, each one `C5.answers`' unit of
/// work (ADR 0140): it goes on, is answered or fails.
const ANSWERS: [&str; 5] = [
    "Fetch.continueRequest",
    "Fetch.continueResponse",
    "Fetch.continueWithAuth",
    "Fetch.failRequest",
    "Fetch.fulfillRequest",
];

/// Counts `method`'s work on the interception's rows: a `Fetch` message, and
/// an answer to a paused request.
fn count_interception(work: &CountedWork, method: &str) {
    if method.starts_with(FETCH) {
        work.count(Row::InterceptedFetchMessages);
        if ANSWERS.contains(&method) {
            work.count(Row::InterceptedAnswers);
        }
    }
}

/// A target's session on the connection: the `sessionId` its commands and
/// events carry.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub String);

struct Request {
    method: &'static str,
    session: Option<SessionId>,
    params: Value,
    answer: oneshot::Sender<Result<Value, Failure>>,
}

/// Where a connection records the method of each command it sends: the
/// Run trace's CDP command log (spec #577), given to the connection once,
/// and only while that log is on (ADR 0140's C1).
#[derive(Clone, Debug)]
pub struct CommandLog(Arc<dyn CommandLogImpl>);

impl CommandLog {
    /// The log `log` keeps.
    pub fn new(log: impl CommandLogImpl + 'static) -> Self {
        Self(Arc::new(log))
    }
}

/// What keeps a [`CommandLog`]: the Run trace's writer, which lives above
/// this crate (ADR 0140's note of #1891).
pub trait CommandLogImpl: Debug + Send + Sync {
    /// Records that a command whose method is `method` is being sent.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the record.
    fn record(&self, method: &'static str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// A handle to the browser's connection, which any task may clone.
#[derive(Clone, Debug)]
pub struct Connection {
    requests: mpsc::Sender<Request>,
    /// Where a caller that left its command's wait tells the driver.
    left: mpsc::Sender<()>,
    joining: Shared,
    /// Whether the driver ended; its sender dropped says so too.
    closed: watch::Receiver<bool>,
    /// The work a typed command's params serialization counts on: C1's
    /// encodes, beside the driver's frame (ADR 0140).
    work: CountedWork,
    /// Where each command's method is recorded before it is sent; none
    /// while the CDP command log is off, so no command sends its method.
    log: Option<CommandLog>,
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
    /// The word that a caller left its command's wait.
    left: mpsc::Receiver<()>,
    joining: Shared,
    /// Where it says it ended.
    closed: watch::Sender<bool>,
    /// The longest message it reads: [`MAX_FRAME`].
    frame_limit: usize,
    /// The bytes each subscriber may leave untaken: [`EVENT_BYTES`].
    event_bytes: usize,
    /// Where each command's encode and send are counted (ADR 0140's C1): the
    /// one seam every command crosses.
    work: CountedWork,
}

impl Connection {
    /// A connection over the browser's end of the pipe `reader` (what it
    /// writes) and `writer` (what it reads), and its driver, which counts
    /// each command's work on `work`; each command's method is recorded in
    /// `log`, if any, before it is sent.
    pub fn new<R, W>(
        reader: R,
        writer: W,
        work: CountedWork,
        log: Option<CommandLog>,
    ) -> (Self, Driver<R, W>)
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let (left, told) = mpsc::channel(1);
        let joining = Shared::default();
        let (ended, closed) = watch::channel(false);
        (
            Self {
                requests,
                left,
                joining: Arc::clone(&joining),
                closed,
                work: work.clone(),
                log,
            },
            Driver {
                reader,
                writer,
                requests: receiver,
                left: told,
                joining,
                closed: ended,
                frame_limit: MAX_FRAME,
                event_bytes: EVENT_BYTES,
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
        // A serialization of its own before the frame's: C1 counts both
        // (#1861's Decision 3, X21).
        self.work.count(Row::CdpEncodes);
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
        let (request, mut waiting) = self.request(method, session, params);
        let sent = async {
            self.queue(request).await?;
            waiting.answer().await
        };
        tokio::time::timeout(timeout, sent)
            .await
            .unwrap_or(Err(Failure::TimedOut { method }))
    }

    /// Starts `C` on `session` (the browser's own when `None`) without
    /// waiting for anything: the command is queued for the driver at once,
    /// so commands started one after another are written in that order, and
    /// what it returns comes back later, through the [`Answer`] given. While
    /// the CDP command log is on, or when 64 commands already wait for the
    /// driver, the command is recorded and queued as its answer is first
    /// awaited instead, as [`Connection::send`] does.
    ///
    /// The answer's errors are [`Connection::send`]'s, its `timeout` counted
    /// from this call.
    pub fn start<C>(
        &self,
        session: Option<&SessionId>,
        params: &C,
        timeout: Duration,
    ) -> Answer<C::Returns>
    where
        C: crate::Command,
        C::Returns: Send + 'static,
    {
        let method = C::METHOD;
        // A serialization of its own before the frame's, as `send`'s.
        self.work.count(Row::CdpEncodes);
        let params = serde_json::to_value(params).expect("invariant: a command's params serialize");
        let (request, mut waiting) = self.request(method, session, params);
        let unqueued = if self.log.is_some() {
            Some(request)
        } else {
            match self.requests.try_send(request) {
                Err(mpsc::error::TrySendError::Full(request)) => Some(request),
                // A closed queue drops the command, whose answer then says
                // the connection ended.
                Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => None,
            }
        };
        let connection = unqueued.is_some().then(|| self.clone());
        // Its deadline starts now, though the answer is first polled later.
        let answered = tokio::time::timeout(timeout, async move {
            if let (Some(request), Some(connection)) = (unqueued, connection) {
                connection.queue(request).await?;
            }
            waiting.answer().await
        });
        let answer = async move {
            answered
                .await
                .unwrap_or(Err(Failure::TimedOut { method }))
                .and_then(|answer| read(method, answer))
        };
        Answer::new(answer.instrument(tracing::trace_span!("cdp_command", cdp.method = method)))
    }

    /// The request of `method` on `session` with `params`, and its caller's
    /// wait for its answer.
    fn request(
        &self,
        method: &'static str,
        session: Option<&SessionId>,
        params: Value,
    ) -> (Request, Waiting) {
        let (answer, answered) = oneshot::channel();
        let request = Request {
            method,
            session: session.cloned(),
            params,
            answer,
        };
        (request, Waiting::new(answered, self.left.clone()))
    }

    /// Records `request`'s method in the CDP command log, if on, then
    /// queues it for the driver, waiting for its place.
    async fn queue(&self, request: Request) -> Result<(), Failure> {
        if let Some(log) = &self.log {
            self.work.count(Row::CdpLogSendsOff);
            log.0.record(request.method).await;
        }
        self.requests
            .send(request)
            .await
            .map_err(|_| Failure::Disconnected)
    }

    /// The events of `session` (the browser's own when `None`) the driver
    /// reads from now on, without waiting for the driver; it ends with the
    /// connection, once the browser detached its session's target, or once
    /// its holder fell 4,096 events behind or left more than its quota of
    /// bytes untaken.
    pub fn subscribe(&self, session: Option<&SessionId>) -> Subscription {
        // A connection that ended gives a subscription that ends at once.
        join(&self.joining, &self.joining, session)
    }

    /// A handout of the events a reader of this connection reads, and the
    /// handing its reader hands each on with: what lets one reader read for
    /// several parts, each subscribed to it as to the connection.
    pub fn handout(&self) -> (Handout, Handing) {
        Handing::new(&self.joining)
    }

    /// Whether the connection ended: the browser closed it, or reading or
    /// writing it failed. A subscription that ended while this is false fell
    /// behind, or its session's target detached.
    pub fn is_closed(&self) -> bool {
        lock(&self.joining).ended.is_some()
    }

    /// Waits until the connection ended, as [`is_closed`](Self::is_closed)
    /// says, or its driver was dropped without running to its end.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing.
    pub async fn closed(&self) {
        let mut closed = self.closed.clone();
        // A sender dropped is a driver that will say nothing more.
        let _ = closed.wait_for(|closed| *closed).await;
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
    /// Kept for its test: the framer holds a message past the real
    /// [`MAX_FRAME`] (256 MiB) before it drops it, too much for the Quick suite.
    #[cfg(test)]
    fn with_frame_limit(mut self, limit: usize) -> Self {
        self.frame_limit = limit;
        self
    }

    /// The driver, letting each subscriber leave `quota` bytes untaken.
    /// Kept for its tests: the real [`EVENT_BYTES`] (64 MiB) is too much to
    /// send through the Quick suite.
    #[cfg(test)]
    fn with_event_bytes(mut self, quota: usize) -> Self {
        self.event_bytes = quota;
        self
    }

    /// Runs the connection until the browser closes it or every handle is
    /// dropped, and says which. Every command still waiting when it ends,
    /// including one queued as it ends, fails with [`Failure::Disconnected`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the client's end: the commands in flight,
    /// and any queued as it drops, get no answer while a handle lives.
    pub async fn run(mut self) -> Ended {
        let mut framer = Framer::with_limit(self.frame_limit);
        let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
        let mut readers = Readers::new(self.event_bytes);
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
                    count_interception(&self.work, method);
                    if let Err(error) = self.writer.write_all(&bytes).await {
                        let _ = answer.send(Err(Failure::Disconnected));
                        break Ended::Broken(error.kind());
                    }
                    // A caller that left before its command was written was
                    // already looked for: its command does not wait.
                    if !answer.is_closed() {
                        pending.insert(id, Pending { method, session, answer });
                    }
                }
                Some(()) = self.left.recv() => forget(&mut pending),
                read = self.reader.read(&mut buffer) => {
                    match read {
                        Ok(0) => break Ended::Closed,
                        Ok(count) => framer.push(&buffer[..count]),
                        Err(error) => break Ended::Broken(error.kind()),
                    }
                    loop {
                        match framer.next_frame() {
                            Ok(Some(bytes)) => {
                                dispatch(bytes, &mut pending, &mut readers, &self.joining, &self.work);
                            }
                            Ok(None) => break,
                            Err(FramingError::TooLarge { answering }) => {
                                dropped(answering, &mut pending);
                            }
                        }
                    }
                }
            }
        };
        // A caller that left before the end told the driver before its
        // handle went: its word is taken first, so its command is no orphan.
        if self.left.try_recv().is_ok() {
            forget(&mut pending);
        }
        for (_, Pending { answer, .. }) in pending {
            if answer.is_closed() {
                self.work.count(Row::CdpPendingOrphans);
            }
            let _ = answer.send(Err(Failure::Disconnected));
        }
        readers.end(SubscriptionEnd::Disconnected, &self.joining);
        self.closed.send_replace(true);
        // A send takes its place in the queue, then queues its request: a
        // request queued after the receiver dropped is never dropped while a
        // handle lives, so its command would wait for good, and every one
        // queued behind it (#2525). After `close`, `recv` waits for each
        // place taken to be filled and then answers `None`.
        self.requests.close();
        while let Some(Request { answer, .. }) = self.requests.recv().await {
            let _ = answer.send(Err(Failure::Disconnected));
        }
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

/// Takes out of `pending` each command whose caller left its wait, by its
/// timeout or its drop. The browser may still answer one, and that answer is
/// then one no command waits for.
fn forget(pending: &mut BTreeMap<u64, Pending>) {
    pending.retain(|_, command| !command.answer.is_closed());
}

/// Hands one message to the command it answers or to the readers of its
/// session, those that joined since the last event among them, never
/// waiting for one. Only its envelope is read: an event's params are left
/// in the message for its reader to decode. An answer no command waits for,
/// as its caller left, its session was detached or it was never sent, is
/// dropped. An event's work is counted on `work`.
fn dispatch(
    bytes: Bytes,
    pending: &mut BTreeMap<u64, Pending>,
    readers: &mut Readers,
    joining: &Shared,
    work: &CountedWork,
) {
    let Ok(envelope) = serde_json::from_slice::<Envelope<'_>>(&bytes) else {
        tracing::warn!("the browser sent a message that is not a JSON object");
        return;
    };
    if let Some(id) = envelope
        .id
        .and_then(|id| serde_json::from_str::<u64>(id.get()).ok())
    {
        let Some(Pending { method, answer, .. }) = pending.remove(&id) else {
            tracing::debug!(
                id,
                "the browser answered a command no caller waits for, and the answer is dropped"
            );
            return;
        };
        let _ = answer.send(answered(method, envelope));
        return;
    }
    let Some(method) = envelope.method.and_then(text) else {
        return;
    };
    count_interception(work, &method);
    let session = envelope.session.and_then(text).map(SessionId);
    // A target's detach comes on its parent's session: the browser's own
    // for a page, the page's for a frame it auto-attached. Its params are
    // read for the session alone, which the driver routes by.
    let gone = envelope
        .params
        .filter(|_| method == DETACHED)
        .and_then(|params| serde_json::from_str::<Envelope<'_>>(params.get()).ok())
        .and_then(|params| params.session)
        .and_then(text)
        .map(SessionId);
    let params = envelope.params.map(|params| range_in(&bytes, params.get()));
    let number = readers.take_in(joining);
    let event = Arc::new(Event::sent(
        (method, session, number),
        bytes,
        params,
        work.clone(),
    ));
    if let Some(gone) = &gone {
        detached(gone, pending);
        readers.detached(gone, joining);
    }
    readers.hand(&event, Some(work), joining);
}

/// What a command's answer says: the browser's refusal, or what it returned,
/// decoded from the browser's message.
fn answered(method: &'static str, envelope: Envelope<'_>) -> Result<Value, Failure> {
    if let Some(error) = envelope.error {
        let error = serde_json::from_str::<Value>(error.get()).unwrap_or(Value::Null);
        return Err(Failure::refused(method, &error));
    }
    envelope.result.map_or(Ok(Value::Null), |result| {
        serde_json::from_str(result.get()).map_err(|error| Failure::Unreadable {
            method,
            message: PartyText::new(error.to_string()),
        })
    })
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

/// The fuzz target's body (ADR 0067): bytes as the browser writes them to
/// the pipe, read as the driver reads them. `data`'s first byte, modulo 8,
/// is how many of the next bytes cut the rest into chunks, and the framing
/// is checked as [`fuzz_frames`](crate::framing::fuzz_frames) says, under a
/// limit of 256 bytes. Each message that comes out is handed on as the
/// driver hands it, with commands 0 to 7 in flight, the odd ones on session
/// `S`, and a subscription to the browser's session and one to `S`: an
/// answer reaches its command in flight once, and an event reaches the
/// subscription of its session while it lasts. It panics only on a bug.
pub fn fuzz_pipe(data: &[u8]) {
    let (cuts, stream) = data.split_first().map_or((data, data), |(count, rest)| {
        rest.split_at(usize::from(count % 8).min(rest.len()))
    });
    let messages = crate::framing::fuzz_frames(stream, cuts, 256);
    let session = SessionId("S".to_owned());
    let (connection, driver) = Connection::new(
        tokio::io::empty(),
        tokio::io::sink(),
        CountedWork::default(),
        None,
    );
    let mut subscriptions = [None, Some(&session)].map(|on| (on, connection.subscribe(on)));
    let mut pending = BTreeMap::new();
    let mut readers = Readers::new(EVENT_BYTES);
    let mut answers = Vec::new();
    for id in 0..8 {
        let (answer, answered) = oneshot::channel();
        let on = (id % 2 == 1).then(|| session.clone());
        pending.insert(
            id,
            Pending {
                method: "Browser.getVersion",
                session: on,
                answer,
            },
        );
        answers.push(answered);
    }
    for bytes in messages {
        // The members as an object's map reads them, the last of each name.
        let message = serde_json::from_slice::<BTreeMap<String, Box<RawValue>>>(&bytes).ok();
        let member = |name: &str| message.as_ref()?.get(name);
        let id = member("id").and_then(|id| serde_json::from_str::<u64>(id.get()).ok());
        let answering = id.filter(|id| pending.contains_key(id));
        let event = message
            .as_ref()
            .filter(|_| id.is_none())
            .and_then(|_| member("method").and_then(|method| text(method)))
            .map(|method| (method, member("sessionId").and_then(|on| text(on))));
        dispatch(
            bytes,
            &mut pending,
            &mut readers,
            &driver.joining,
            &driver.work,
        );
        if let Some(id) = answering {
            let answered = usize::try_from(id)
                .ok()
                .and_then(|id| answers.get_mut(id))
                .expect("invariant: the commands in flight are 0 to 7")
                .try_recv();
            assert!(answered.is_ok(), "an answer reaches its command in flight");
            continue;
        }
        for (on, subscription) in &mut subscriptions {
            let received = subscription.taken();
            let Some((method, event_on)) = &event else {
                assert!(received.is_err(), "only an event reaches a subscription");
                continue;
            };
            if subscription.ended().is_some() || on.map(|on| on.0.as_str()) != event_on.as_deref() {
                assert!(
                    received.is_err(),
                    "an event reaches only the subscription of its session"
                );
                continue;
            }
            let Ok(received) = received else {
                panic!("an event reaches the subscription of its session");
            };
            assert_eq!(&received.method, method, "an event keeps its method");
        }
    }
}

#[cfg(test)]
mod tests;
