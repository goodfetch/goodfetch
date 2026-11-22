//! Framing and correlation: one JSON-RPC message per line each way, each
//! reply matched to its call by id (ADR 0038). One task on the Connection's
//! runtime owns the calls, their deadlines and the input; a second writes
//! the output, so a slow reader never holds the input up. Callers only
//! exchange messages with them, so a call's future runs on any executor
//! (ADR 0081). A streaming call, such as one whose engine sends a file in
//! `$/chunk` notifications, is told its id once it is sent and hears the
//! notifications that name that id, until its reply comes; the Connection
//! sends its own notifications, such as `$/credit`, the same way it sends
//! calls.
//!
//! The Connection sends at most the `max_calls_in_flight` its engine's
//! `hello` answered and queues the other calls in the order they were
//! made, each sent as an answer frees a place (ADR 0038's note of
//! 2026-11-21; issue 3189). A call cancelled once sent keeps its place
//! until the engine answers it; a queued one cancelled, or past its
//! timeout, is never sent. Past the Connection's own bounds, a call fails
//! at once, unsent, with `connection_overloaded` (see [`CallBounds`]).

use std::collections::{BTreeMap, BTreeSet};
use std::future::poll_fn;
use std::num::NonZeroU64;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use goodfetch_protocol::connection::{CANCEL, CancelParams, EngineExitedDetails};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::jsonrpc::{Message, Notification, Request, RequestId, Version};
use goodfetch_protocol::log::LOG;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio_util::task::TaskTracker;
use tokio_util::time::DelayQueue;
use tokio_util::time::delay_queue::Key;

use crate::call_bounds::CallBounds;
use crate::error::Error;
use crate::handler::Streams;
use crate::line::{self, Output, Written};
use crate::logging;
use crate::opening_process::OpeningProcess;
use crate::trace_context::{self, TraceContext, TraceContextSource};
use crate::transport::{Exit, Link};

/// How long a Connection whose engine ended it waits to hear how, before
/// its calls fail without that evidence, as the other SDKs wait.
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// What a call's caller hands the Connection's tasks.
pub(crate) struct Outgoing {
    /// The operation.
    pub(crate) method: String,
    /// Its params.
    pub(crate) params: Value,
    /// How long the call may wait, or `None` to wait for its reply as long
    /// as the Connection lasts.
    pub(crate) timeout: Option<Deadline>,
    /// Where the reply goes.
    pub(crate) reply: oneshot::Sender<Result<Value, Error>>,
    /// Where a streaming call hears its id and its notifications, as many
    /// at once as the credits it grants allow; none for a call that streams
    /// nothing.
    pub(crate) notified: Option<mpsc::Sender<Heard>>,
}

/// How long a call may wait, from when the Connection takes it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadline {
    /// The timeout's setting, such as `timeout` or `timeout.navigation`,
    /// which a call queued past it names.
    pub(crate) setting: &'static str,
    /// How long.
    pub(crate) after: Duration,
    /// Who keeps it once the call is sent.
    pub(crate) kept_by: KeptBy,
}

/// Who keeps a call's [`Deadline`] once the call is sent; until then, the
/// Connection bounds its wait in the queue by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeptBy {
    /// A timer of the Client's, until the reply.
    Client,
    /// The engine, which the call's params tell (ADR 0080).
    Engine,
}

/// What a streaming call hears before its reply.
#[derive(Debug)]
pub(crate) enum Heard {
    /// It was sent with this id.
    Sent(i64),
    /// The engine sent the notification `method` naming its id.
    Notified {
        /// The notification.
        method: String,
        /// Its params.
        params: Value,
    },
}

/// A call's id, which the Connection notes as it takes the call, before
/// sending it or queuing it, so that one who cannot wait to hear it can
/// still cancel the call in order.
#[derive(Clone, Debug, Default)]
pub(crate) struct CallId(Arc<OnceLock<i64>>);

impl CallId {
    /// Its id, once the Connection took the call.
    pub(crate) fn get(&self) -> Option<i64> {
        self.0.get().copied()
    }
}

/// Where a call's id is noted once taken, and the credits a streaming
/// call is granted once sent, if any.
struct Noted {
    id: CallId,
    credits: Option<u32>,
}

/// What the Connection's correlating task acts on.
enum Command {
    /// Takes a call, notes its id where named, and sends it or queues it;
    /// once sent, grants it the credits named, if any, with `$/credit`,
    /// before anything sent after it.
    Call(Outgoing, Option<Noted>),
    /// Sends a notification, after its call when that call is queued.
    Notify(Notification),
    /// Cancels a call taken before it, once its id is noted: one queued is
    /// never sent, and one sent and waiting for its reply gets `$/cancel`.
    CancelSent(CallId),
    /// Sends at most this many calls at once, as the engine's `hello`
    /// answered.
    Cap(usize),
    /// Ends the Connection: no more calls, the output closed, every call
    /// in flight or queued failed as cancelled by the close.
    Close,
}

/// How a Connection ended, which fails each call left waiting and every
/// later one alike.
#[derive(Debug)]
enum Ended {
    /// The Client closed it.
    Closed,
    /// The engine's side ended it, with nothing to tell how its engine
    /// ended.
    Dropped,
    /// Its engine ended, as `details` tell.
    Exited(EngineExitedDetails),
}

impl Ended {
    fn error(&self) -> Error {
        match self {
            Self::Closed => Error::closed(),
            Self::Dropped => Error::dropped(),
            Self::Exited(details) => Error::engine_exited(details),
        }
    }

    /// The error a call still in flight or queued when the Connection
    /// ended fails with: the Client's close cancelled it (ADR 0078), and
    /// any other end fails it as it fails a later call.
    fn in_flight_error(&self) -> Error {
        match self {
            Self::Closed => Error::cancelled_by_close(),
            ended => ended.error(),
        }
    }
}

/// The handle every call goes through: cheap to clone, `Send + Sync`.
#[derive(Clone, Debug)]
pub(crate) struct Sender {
    commands: mpsc::UnboundedSender<Command>,
    /// How the Connection ended, set before it takes no more calls.
    ended: Arc<OnceLock<Ended>>,
    opener: OpeningProcess,
    /// What reads the Trace context each call carries, when the SDK has
    /// one (ADR 0132).
    trace_context_source: Option<Arc<dyn TraceContextSource>>,
}

impl Sender {
    /// The calling thread's Trace context, which a call reads as it
    /// becomes a future, on the thread that makes it, or a blocking call's
    /// caller's (ADR 0132).
    pub(crate) fn trace_context(&self) -> Option<TraceContext> {
        trace_context::current(self.trace_context_source.as_deref())
    }

    /// `connection_forked` unless the calling process opened the
    /// Connection.
    pub(crate) fn ensure_opener(&self) -> Result<(), Error> {
        self.opener.ensure_current()
    }

    /// Whether the calling process opened the Connection.
    pub(crate) fn in_opener(&self) -> bool {
        self.opener.is_current()
    }

    /// Sends `call`; its reply fails as the Connection ended once it has.
    pub(crate) fn call(&self, call: Outgoing) {
        self.send_call(call, None);
    }

    /// Sends the streaming call `call` and grants it `credits` with
    /// `$/credit` right after it, before any message sent after this one,
    /// whose caller cannot wait to hear its id first. Its caller cancels it
    /// with [`Sender::cancel_sent`], even when it stopped listening before
    /// it was sent.
    pub(crate) fn call_granting(&self, call: Outgoing, credits: u32) -> CallId {
        self.call_noting(call, Some(credits))
    }

    /// Sends `call` and notes its id, so that its caller, who cannot wait
    /// to hear it, cancels it with [`Sender::cancel_sent`] once it stops
    /// waiting for its reply (ADR 0080).
    pub(crate) fn call_cancellable(&self, call: Outgoing) -> CallId {
        self.call_noting(call, None)
    }

    fn call_noting(&self, call: Outgoing, credits: Option<u32>) -> CallId {
        let id = CallId::default();
        self.send_call(
            call,
            Some(Noted {
                id: id.clone(),
                credits,
            }),
        );
        id
    }

    fn send_call(&self, call: Outgoing, noted: Option<Noted>) {
        if let Err(forked) = self.ensure_opener() {
            let _gone = call.reply.send(Err(forked.in_call(&call.method)));
            return;
        }
        if let Err(mpsc::error::SendError(Command::Call(call, _))) =
            self.commands.send(Command::Call(call, noted))
        {
            let ended = self.ended.get().map_or_else(Error::closed, Ended::error);
            let _closed = call.reply.send(Err(ended.in_call(&call.method)));
        }
    }

    /// Grants the streaming call `id` `credits` more items with
    /// `$/credit`.
    pub(crate) fn credit(&self, id: i64, credits: u32) {
        self.hand(Command::Notify(credited(id, credits)));
    }

    /// Cancels the streaming call `id` with `$/cancel`.
    pub(crate) fn cancel(&self, id: i64) {
        self.hand(Command::Notify(cancelled(id)));
    }

    /// Cancels the call `call`, made before this, whether or not its id
    /// was heard yet: one still queued is never sent, and one sent gets
    /// `$/cancel`, after it and before anything sent after this; one no
    /// longer waiting for its reply (answered, or failed by the
    /// Connection) is not cancelled.
    pub(crate) fn cancel_sent(&self, call: &CallId) {
        self.hand(Command::CancelSent(call.clone()));
    }

    /// Sends at most `cap` calls at once from now on, as the engine's
    /// `hello` answered, before any call but `hello` is made.
    pub(crate) fn cap_calls_in_flight(&self, cap: NonZeroU64) {
        // A cap past what the machine addresses caps nothing.
        let cap = usize::try_from(cap.get()).unwrap_or(usize::MAX);
        self.hand(Command::Cap(cap));
    }

    /// Ends the Connection without waiting.
    pub(crate) fn close(&self) {
        self.hand(Command::Close);
    }

    /// Hands `command` to the Connection's tasks, unless they ended or the
    /// calling process is not the opener, where they never run.
    fn hand(&self, command: Command) {
        if self.in_opener() {
            let _closed = self.commands.send(command);
        }
    }
}

/// Starts the Connection's tasks over `link` on `runtime`, tracked by
/// `tracker`, holding the calls within `bounds`; the engine's requests go
/// to `streams`, and each call carries the Trace context
/// `trace_context_source` reads.
pub(crate) fn start(
    link: Link,
    runtime: &Handle,
    tracker: &TaskTracker,
    streams: Streams,
    bounds: CallBounds,
    trace_context_source: Option<Arc<dyn TraceContextSource>>,
) -> Sender {
    let (reader, writer, exit) = link.split();
    let (commands, commands_in) = mpsc::unbounded_channel();
    let (output, written) = line::output();
    tracker.spawn_on(write(writer, written), runtime);
    let ended = Arc::new(OnceLock::new());
    let ending = Ending {
        exit,
        ended: Arc::clone(&ended),
    };
    logging::spawn_under_opener(
        correlate(reader, commands_in, Calls::new(output, bounds), streams, ending),
        runtime,
        tracker,
    );
    Sender {
        commands,
        ended,
        opener: OpeningProcess::current(),
        trace_context_source,
    }
}

/// What the correlating task learns, and tells every handle, of how the
/// Connection ended.
struct Ending {
    /// How its engine ended, when the transport can tell.
    exit: Option<Exit>,
    /// Where it notes how the Connection ended.
    ended: Arc<OnceLock<Ended>>,
}

/// Writes each line until every sender is gone, then closes the output,
/// which tells the engine the Connection is over.
async fn write(mut writer: Box<dyn AsyncWrite + Send + Unpin>, mut lines: Written) {
    while let Some(line) = lines.next().await {
        if let Err(error) = writer.write_all(&line).await {
            tracing::debug!(target: "goodfetch::client", %error, "the Connection's output closed");
            return;
        }
        if let Err(error) = writer.flush().await {
            tracing::debug!(target: "goodfetch::client", %error, "the Connection's output closed");
            return;
        }
        lines.wrote(&line);
    }
    if let Err(error) = writer.shutdown().await {
        tracing::debug!(target: "goodfetch::client", %error, "the Connection's output was already closed");
    }
}

/// A call sent and waiting for its reply.
struct Pending {
    method: String,
    reply: oneshot::Sender<Result<Value, Error>>,
    /// Its timer of the Client's, if any.
    deadline: Option<Key>,
    notified: Option<mpsc::Sender<Heard>>,
    /// Whether the catalog says the engine cancels it, so its deadline
    /// passing sends its `$/cancel` (ADR 0080).
    cancellable: bool,
}

impl Pending {
    /// Fails the call with `error`, which the runtime raised, as the call
    /// reads it: headed by its operation (ADR 0079).
    fn fail(self, error: Error) {
        let _gone = self.reply.send(Err(error.in_call(&self.method)));
    }
}

/// A call taken, encoded and not sent yet: queued behind the engine's cap,
/// or about to be sent.
struct Held {
    method: String,
    line: Vec<u8>,
    reply: oneshot::Sender<Result<Value, Error>>,
    /// Its deadline and when it passes.
    deadline: Option<(Key, Deadline)>,
    notified: Option<mpsc::Sender<Heard>>,
    /// The credits it is granted once sent, if any.
    credits: Option<u32>,
    cancellable: bool,
    /// The notifications naming it, sent after it.
    notes: Vec<Vec<u8>>,
    /// Its bytes and its notes'.
    bytes: usize,
}

impl Held {
    /// Fails the call with `error`, headed by its operation (ADR 0079).
    fn fail(self, error: Error) {
        let _gone = self.reply.send(Err(error.in_call(&self.method)));
    }
}

/// The Connection's calls, sent and queued, their deadlines and the
/// output they are written through.
struct Calls {
    output: Output,
    bounds: CallBounds,
    /// The most calls sent at once, once the engine's `hello` told it.
    cap: Option<usize>,
    next_id: i64,
    /// The calls sent and waiting for their reply.
    pending: BTreeMap<i64, Pending>,
    /// The calls queued behind the cap, by id, which is the order made.
    queued: BTreeMap<i64, Held>,
    /// The queued calls' bytes and their notes'.
    queued_bytes: usize,
    /// The calls sent and not answered yet, which hold the cap's places:
    /// one cancelled or past its timeout too, until the engine answers it.
    in_flight: BTreeSet<i64>,
    deadlines: DelayQueue<i64>,
}

impl Calls {
    fn new(output: Output, bounds: CallBounds) -> Self {
        Self {
            output,
            bounds,
            cap: None,
            next_id: 0,
            pending: BTreeMap::new(),
            queued: BTreeMap::new(),
            queued_bytes: 0,
            in_flight: BTreeSet::new(),
            deadlines: DelayQueue::new(),
        }
    }

    /// Takes `call`: refuses it past the Connection's bounds, then sends
    /// it while the cap has a free place and nothing is queued, or queues
    /// it.
    fn take(&mut self, call: Outgoing, noted: Option<&Noted>) {
        if let Some(refused) = self
            .bounds
            .refuses_pending(self.pending.len().saturating_add(self.queued.len()))
        {
            let _gone = call.reply.send(Err(refused.in_call(&call.method)));
            return;
        }
        self.next_id += 1;
        let id = self.next_id;
        let request = Request {
            jsonrpc: Version,
            id: RequestId::Number(id),
            method: call.method,
            params: Some(call.params),
        };
        let encoded = line::encode(&request);
        let method = request.method;
        let line = match encoded {
            Ok(line) => line,
            Err(error) => {
                let _gone = call.reply.send(Err(Error::shape(error).in_call(&method)));
                return;
            }
        };
        let waiting = self.output.unwritten().saturating_add(self.queued_bytes);
        if let Some(refused) = self.bounds.refuses_bytes(waiting, line.len()) {
            let _gone = call.reply.send(Err(refused.in_call(&method)));
            return;
        }
        if let Some(noted) = noted {
            // Its caller cancels it with `CancelSent`, queued or sent.
            let _noted = noted.id.0.set(id);
        }
        let held = Held {
            method,
            bytes: line.len(),
            line,
            reply: call.reply,
            deadline: call
                .timeout
                .map(|deadline| (self.deadlines.insert(id, deadline.after), deadline)),
            notified: call.notified,
            credits: noted.and_then(|noted| noted.credits),
            cancellable: noted.is_some(),
            notes: Vec::new(),
        };
        if self.queued.is_empty() && self.has_place() {
            self.dispatch(id, held);
        } else {
            self.queued_bytes = self.queued_bytes.saturating_add(held.bytes);
            self.queued.insert(id, held);
        }
    }

    /// Whether the cap leaves a place for one more call sent.
    fn has_place(&self) -> bool {
        self.cap.is_none_or(|cap| self.in_flight.len() < cap)
    }

    /// Sends the held call `id`, then its credits and notes; one whose
    /// caller stopped listening before it was sent, and so never learnt
    /// its id, is never sent.
    fn dispatch(&mut self, id: i64, held: Held) {
        let Held {
            method,
            line,
            reply,
            deadline,
            notified,
            credits,
            cancellable,
            notes,
            ..
        } = held;
        let unheard = notified.as_ref().is_some_and(mpsc::Sender::is_closed);
        if unheard && !cancellable {
            if let Some((key, _)) = deadline {
                self.deadlines.remove(&key);
            }
            return;
        }
        if self.output.send(line).is_err() {
            if let Some((key, _)) = deadline {
                self.deadlines.remove(&key);
            }
            let _gone = reply.send(Err(Error::closed().in_call(&method)));
            return;
        }
        self.in_flight.insert(id);
        let deadline = deadline.and_then(|(key, deadline)| match deadline.kept_by {
            KeptBy::Client => Some(key),
            KeptBy::Engine => {
                self.deadlines.remove(&key);
                None
            }
        });
        if let Some(notified) = &notified {
            let _unheard = notified.try_send(Heard::Sent(id));
        }
        if let Some(credits) = credits {
            self.send(&credited(id, credits));
        }
        for note in notes {
            let _closed = self.output.send(note);
        }
        self.pending.insert(
            id,
            Pending {
                method,
                reply,
                deadline,
                notified,
                cancellable,
            },
        );
    }

    /// Sends the queued calls, in the order they were made, while the cap
    /// has places.
    fn drain(&mut self) {
        while self.has_place() {
            let Some((id, held)) = self.queued.pop_first() else {
                return;
            };
            self.queued_bytes = self.queued_bytes.saturating_sub(held.bytes);
            self.dispatch(id, held);
        }
    }

    /// Sends at most `cap` calls at once from now on.
    fn cap(&mut self, cap: usize) {
        self.cap = Some(cap);
        self.drain();
    }

    /// Takes the call `id` out of the queue, if it is queued.
    fn unqueue(&mut self, id: i64) -> Option<Held> {
        let held = self.queued.remove(&id)?;
        self.queued_bytes = self.queued_bytes.saturating_sub(held.bytes);
        Some(held)
    }

    /// Fails the call `id`, whose deadline passed: one queued with that
    /// deadline, never sent, and one sent with the Client's timeout,
    /// cancelled when the engine cancels it.
    fn expire(&mut self, id: i64) {
        if let Some(held) = self.unqueue(id) {
            tracing::debug!(target: "goodfetch::client", id, "a queued call timed out");
            // Its timer started as it was taken, so it waited `after`.
            let error = held.deadline.map_or_else(Error::timed_out, |(_, deadline)| {
                Error::queue_timed_out(deadline.setting, deadline.after)
            });
            held.fail(error);
            return;
        }
        if let Some(call) = self.pending.remove(&id) {
            tracing::debug!(target: "goodfetch::client", id, "a call timed out");
            // The engine's work for it ends too, as for a dropped call; its
            // late answer then changes nothing.
            if call.cancellable {
                self.send(&cancelled(id));
            }
            call.fail(Error::timed_out());
        }
    }

    /// Cancels the call `call`: one queued leaves the queue unsent, and
    /// one sent and waiting for its reply gets `$/cancel`.
    fn cancel_sent(&mut self, call: &CallId) {
        let Some(id) = call.get() else {
            return;
        };
        if let Some(held) = self.unqueue(id) {
            if let Some((key, _)) = held.deadline {
                self.deadlines.remove(&key);
            }
            return;
        }
        if self.pending.contains_key(&id) {
            self.send(&cancelled(id));
        }
    }

    /// Sends `notification`, or holds it until its call, still queued, is
    /// sent.
    fn notify(&mut self, notification: &Notification) {
        let Some(line) = encoded(notification) else {
            return;
        };
        let queued = named(notification.params.as_ref()).and_then(|id| self.queued.get_mut(&id));
        if let Some(held) = queued {
            held.bytes = held.bytes.saturating_add(line.len());
            self.queued_bytes = self.queued_bytes.saturating_add(line.len());
            held.notes.push(line);
            return;
        }
        let _closed = self.output.send(line);
    }

    /// Sends `notification`, about a call already sent.
    fn send(&self, notification: &Notification) {
        if let Some(line) = encoded(notification) {
            let _closed = self.output.send(line);
        }
    }

    /// Resolves the call `line` answers, freeing its place for the next
    /// queued one, or hands the engine's request `line` is to `streams`,
    /// whose answers go out through the output; a reply to no call in
    /// flight, late past its timeout or never asked, resolves nothing.
    fn answer(&mut self, line: &str, streams: &Streams) {
        let (id, outcome) = match serde_json::from_str::<Message>(line) {
            Ok(Message::Response(response)) => (Some(response.id), Ok(response.result)),
            Ok(Message::Error(failure)) => (failure.id, Err(Error::answered(failure.error))),
            Ok(Message::Notification(notification)) if notification.method == LOG => {
                logging::relay(notification.params);
                return;
            }
            Ok(Message::Notification(notification)) => {
                self.hand_over(notification);
                return;
            }
            Ok(Message::Request(request)) => {
                streams.hand(request, &self.output);
                return;
            }
            Err(error) => {
                tracing::warn!(target: "goodfetch::client", %error, "the engine sent a line that is no message");
                return;
            }
        };
        let Some(RequestId::Number(id)) = id else {
            tracing::debug!(target: "goodfetch::client", "a reply to no call of this Connection");
            return;
        };
        if let Some(call) = self.pending.remove(&id) {
            if let Some(key) = call.deadline {
                self.deadlines.remove(&key);
            }
            let _gone = call.reply.send(outcome);
        } else {
            tracing::debug!(target: "goodfetch::client", id, "a reply to a call no longer waiting for it");
        }
        if self.in_flight.remove(&id) {
            self.drain();
        }
    }

    /// Hands `notification` to the streaming call in flight whose id its
    /// params name; one that names no such call changes nothing. A call
    /// that has not taken what it was handed fails: its engine sent past
    /// the credits the call granted.
    fn hand_over(&mut self, notification: Notification) {
        let Notification { method, params, .. } = notification;
        let call = named(params.as_ref())
            .and_then(|id| Some((id, self.pending.get(&id)?.notified.as_ref()?)));
        let Some((id, notified)) = call else {
            tracing::trace!(target: "goodfetch::client", %method, "a notification no one awaits");
            return;
        };
        let heard = Heard::Notified {
            method,
            params: params.unwrap_or(Value::Null),
        };
        if let Err(mpsc::error::TrySendError::Full(_)) = notified.try_send(heard) {
            let Some(call) = self.pending.remove(&id) else {
                return;
            };
            if let Some(key) = call.deadline {
                self.deadlines.remove(&key);
            }
            tracing::debug!(target: "goodfetch::client", id, "a streaming call's engine sent past its credits");
            call.fail(Error::shape(serde::de::Error::custom(
                "the engine streamed past the credits the call granted",
            )));
        }
    }

    /// Fails every call sent or queued as `ended` fails them.
    fn end(&mut self, ended: &Ended) {
        for call in std::mem::take(&mut self.queued).into_values() {
            call.fail(ended.in_flight_error());
        }
        for call in std::mem::take(&mut self.pending).into_values() {
            call.fail(ended.in_flight_error());
        }
    }
}

/// Takes calls, matches replies and deadlines to them, hands the engine's
/// requests to `streams`, and fails what is left when the Connection ends,
/// by `close()`, by every handle dropped or by the engine closing its
/// output, which also ends the streams. When the Client ended it, a call
/// in flight or queued fails with `call_cancelled` and one made behind the
/// close with `closed`; when the engine's side did, all fail as closed, or
/// with `engine_exited` when `ending` tells how its engine ended. Every
/// call made after fails as one made behind the close.
async fn correlate(
    reader: Box<dyn AsyncRead + Send + Unpin>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut calls: Calls,
    streams: Streams,
    ending: Ending,
) {
    let mut input = BufReader::new(reader).lines();
    // Whether the engine's side ended the Connection, rather than the Client.
    let mut dropped = false;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Call(call, noted)) => calls.take(call, noted.as_ref()),
                Some(Command::Notify(notification)) => calls.notify(&notification),
                Some(Command::CancelSent(call)) => calls.cancel_sent(&call),
                Some(Command::Cap(cap)) => calls.cap(cap),
                Some(Command::Close) | None => break,
            },
            Some(expired) = poll_fn(|context| calls.deadlines.poll_expired(context)) => {
                calls.expire(expired.into_inner());
            }
            line = input.next_line() => match line {
                Ok(Some(line)) => calls.answer(&line, &streams),
                Ok(None) => {
                    tracing::debug!(target: "goodfetch::client", "the engine closed the Connection");
                    dropped = true;
                    break;
                }
                Err(error) => {
                    tracing::debug!(target: "goodfetch::client", %error, "the Connection's input failed");
                    dropped = true;
                    break;
                }
            },
        }
    }
    streams.end();
    // Calls made while it waits to hear how the engine ended wait in the
    // commands, and fail below with that evidence.
    let ended = match ending.exit {
        Some(exit) if dropped => Ended::Exited(
            tokio::time::timeout(EXIT_GRACE, exit)
                .await
                .unwrap_or_else(|_| EngineExitedDetails::without_evidence()),
        ),
        _ if dropped => Ended::Dropped,
        _ => Ended::Closed,
    };
    let ended = ending.ended.get_or_init(|| ended);
    commands.close();
    while let Ok(command) = commands.try_recv() {
        if let Command::Call(call, _) = command {
            let _gone = call.reply.send(Err(ended.error().in_call(&call.method)));
        }
    }
    calls.end(ended);
}

/// The `$/cancel` of the call `id`.
fn cancelled(id: i64) -> Notification {
    Notification {
        jsonrpc: Version,
        method: CANCEL.to_owned(),
        params: serde_json::to_value(CancelParams {
            id: RequestId::Number(id),
        })
        .ok(),
    }
}

/// The `$/credit` that grants the call `id` `credits` more items.
fn credited(id: i64, credits: u32) -> Notification {
    Notification {
        jsonrpc: Version,
        method: CREDIT.to_owned(),
        params: serde_json::to_value(CreditParams {
            id: RequestId::Number(id),
            credits,
        })
        .ok(),
    }
}

/// The call a notification's params name by its `id`, if any.
fn named(params: Option<&Value>) -> Option<i64> {
    params?.get("id")?.as_i64()
}

/// `notification` as one line, or none, with a warning, when it is no
/// JSON.
fn encoded(notification: &Notification) -> Option<Vec<u8>> {
    let encoded = line::encode(notification);
    if encoded.is_err() {
        tracing::warn!(target: "goodfetch::client", method = %notification.method, "a notification that is no JSON was not sent");
    }
    encoded.ok()
}
