//! Framing and correlation: one JSON-RPC message per line each way, each
//! reply matched to its call by id (ADR 0038). One task on the Connection's
//! runtime owns the calls in flight, their deadlines and the input; a
//! second writes the output, so a slow reader never holds the input up.
//! Callers only exchange messages with them, so a call's future runs on any
//! executor (ADR 0081). A streaming call, such as one whose engine sends a
//! file in `$/chunk` notifications, is told its id once it is sent and
//! hears the notifications that name that id, until its reply comes; the
//! Connection sends its own notifications, such as `$/credit`, the same
//! way it sends calls.

use std::collections::BTreeMap;
use std::future::poll_fn;
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

use crate::error::Error;
use crate::handler::Streams;
use crate::line;
use crate::logging;
use crate::opening_process::OpeningProcess;
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
    /// How long the reply may take, or `None` to wait for it as long as
    /// the Connection lasts.
    pub(crate) timeout: Option<Duration>,
    /// Where the reply goes.
    pub(crate) reply: oneshot::Sender<Result<Value, Error>>,
    /// Where a streaming call hears its id and its notifications, as many
    /// at once as the credits it grants allow; none for a call that streams
    /// nothing.
    pub(crate) notified: Option<mpsc::Sender<Heard>>,
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

/// A call's id, which the Connection notes as it sends the call, so that
/// one who cannot wait to hear it can still cancel the call in order.
#[derive(Clone, Debug, Default)]
pub(crate) struct CallId(Arc<OnceLock<i64>>);

impl CallId {
    /// Its id, once the call was sent.
    pub(crate) fn get(&self) -> Option<i64> {
        self.0.get().copied()
    }
}

/// Where a call's id is noted once sent, and the credits a streaming call
/// is granted then, if any.
struct Noted {
    id: CallId,
    credits: Option<u32>,
}

/// What the Connection's correlating task acts on.
enum Command {
    /// Sends a call, notes its id where named, then grants it the credits
    /// named, if any, with `$/credit`, before anything sent after it.
    Call(Outgoing, Option<Noted>),
    /// Sends a notification.
    Notify(Notification),
    /// Cancels a call sent before it, once its id is noted, while it waits
    /// for its reply.
    CancelSent(CallId),
    /// Ends the Connection: no more calls, the output closed, every call
    /// in flight failed as closed.
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
}

/// The handle every call goes through: cheap to clone, `Send + Sync`.
#[derive(Clone, Debug)]
pub(crate) struct Sender {
    commands: mpsc::UnboundedSender<Command>,
    /// How the Connection ended, set before it takes no more calls.
    ended: Arc<OnceLock<Ended>>,
    opener: OpeningProcess,
}

impl Sender {
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

    /// Cancels the call `call`, sent before this, with `$/cancel`, after it
    /// and before anything sent after this, whether or not its id was heard
    /// yet; one never sent, or no longer waiting for its reply (answered, or
    /// failed by the Connection), is not cancelled.
    pub(crate) fn cancel_sent(&self, call: &CallId) {
        self.hand(Command::CancelSent(call.clone()));
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
/// `tracker`; the engine's requests go to `streams`.
pub(crate) fn start(
    link: Link,
    runtime: &Handle,
    tracker: &TaskTracker,
    streams: Streams,
) -> Sender {
    let (reader, writer, exit) = link.split();
    let (commands, commands_in) = mpsc::unbounded_channel();
    let (lines, lines_in) = mpsc::unbounded_channel();
    tracker.spawn_on(write(writer, lines_in), runtime);
    let ended = Arc::new(OnceLock::new());
    let ending = Ending {
        exit,
        ended: Arc::clone(&ended),
    };
    logging::spawn_under_opener(
        correlate(reader, commands_in, lines, streams, ending),
        runtime,
        tracker,
    );
    Sender {
        commands,
        ended,
        opener: OpeningProcess::current(),
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
async fn write(
    mut writer: Box<dyn AsyncWrite + Send + Unpin>,
    mut lines: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(line) = lines.recv().await {
        if let Err(error) = writer.write_all(&line).await {
            tracing::debug!(target: "goodfetch::client", %error, "the Connection's output closed");
            return;
        }
        if let Err(error) = writer.flush().await {
            tracing::debug!(target: "goodfetch::client", %error, "the Connection's output closed");
            return;
        }
    }
    if let Err(error) = writer.shutdown().await {
        tracing::debug!(target: "goodfetch::client", %error, "the Connection's output was already closed");
    }
}

/// A call in flight.
struct Pending {
    method: String,
    reply: oneshot::Sender<Result<Value, Error>>,
    deadline: Option<Key>,
    notified: Option<mpsc::Sender<Heard>>,
}

impl Pending {
    /// Fails the call with `error`, which the runtime raised, as the call
    /// reads it: headed by its operation (ADR 0079).
    fn fail(self, error: Error) {
        let _gone = self.reply.send(Err(error.in_call(&self.method)));
    }
}

/// Sends calls, matches replies and deadlines to them, hands the engine's
/// requests to `streams`, and fails what is left as closed when the
/// Connection ends, by `close()`, by every handle dropped or by the engine
/// closing its output, which also ends the streams. When the engine's
/// side ends it and `ending` tells how its engine ended, what is left fails
/// with `engine_exited` instead. Every call made after fails alike.
async fn correlate(
    reader: Box<dyn AsyncRead + Send + Unpin>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    lines: mpsc::UnboundedSender<Vec<u8>>,
    streams: Streams,
    ending: Ending,
) {
    let mut input = BufReader::new(reader).lines();
    let mut pending: BTreeMap<i64, Pending> = BTreeMap::new();
    let mut deadlines: DelayQueue<i64> = DelayQueue::new();
    let mut next_id: i64 = 0;
    // Whether the engine's side ended the Connection, rather than the Client.
    let mut dropped = false;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Call(call, noted)) => {
                    next_id += 1;
                    let id = next_id;
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
                            continue;
                        }
                    };
                    if lines.send(line).is_err() {
                        let _gone = call.reply.send(Err(Error::closed().in_call(&method)));
                        continue;
                    }
                    let deadline = call.timeout.map(|timeout| deadlines.insert(id, timeout));
                    let unheard = call.notified.as_ref().is_some_and(|notified| {
                        matches!(notified.try_send(Heard::Sent(id)), Err(mpsc::error::TrySendError::Closed(_)))
                    });
                    if let Some(noted) = noted {
                        // Its caller cancels it with `CancelSent`.
                        let _noted = noted.id.0.set(id);
                        if let Some(credits) = noted.credits {
                            send(&credited(id, credits), &lines);
                        }
                    } else if unheard {
                        // Its caller stopped listening before it was sent,
                        // so never learnt its id: the call is cancelled here.
                        send(&cancelled(id), &lines);
                    }
                    pending.insert(id, Pending { method, reply: call.reply, deadline, notified: call.notified });
                }
                Some(Command::Notify(notification)) => send(&notification, &lines),
                Some(Command::CancelSent(call)) => {
                    if let Some(id) = call.get().filter(|id| pending.contains_key(id)) {
                        send(&cancelled(id), &lines);
                    }
                }
                Some(Command::Close) | None => break,
            },
            Some(expired) = poll_fn(|context| deadlines.poll_expired(context)) => {
                let id = expired.into_inner();
                if let Some(call) = pending.remove(&id) {
                    tracing::debug!(target: "goodfetch::client", id, "a call timed out");
                    call.fail(Error::timed_out());
                }
            }
            line = input.next_line() => match line {
                Ok(Some(line)) => answer(&line, &mut pending, &mut deadlines, &streams, &lines),
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
    // Calls made while it waits to hear how the engine ended queue, and
    // fail below with that evidence.
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
    for call in std::mem::take(&mut pending).into_values() {
        call.fail(ended.error());
    }
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

/// Sends `notification` through `lines`.
fn send(notification: &Notification, lines: &mpsc::UnboundedSender<Vec<u8>>) {
    let Ok(mut line) = serde_json::to_vec(notification) else {
        tracing::warn!(target: "goodfetch::client", method = %notification.method, "a notification that is no JSON was not sent");
        return;
    };
    line.push(b'\n');
    let _closed = lines.send(line);
}

/// Resolves the call `line` answers, or hands the engine's request `line`
/// is to `streams`, whose answers go out through `lines`; a reply to no
/// call in flight, late past its timeout or never asked, changes nothing.
fn answer(
    line: &str,
    pending: &mut BTreeMap<i64, Pending>,
    deadlines: &mut DelayQueue<i64>,
    streams: &Streams,
    lines: &mpsc::UnboundedSender<Vec<u8>>,
) {
    let (id, outcome) = match serde_json::from_str::<Message>(line) {
        Ok(Message::Response(response)) => (Some(response.id), Ok(response.result)),
        Ok(Message::Error(failure)) => (failure.id, Err(Error::answered(failure.error))),
        Ok(Message::Notification(notification)) if notification.method == LOG => {
            logging::relay(notification.params);
            return;
        }
        Ok(Message::Notification(notification)) => {
            hand_over(notification, pending, deadlines);
            return;
        }
        Ok(Message::Request(request)) => {
            streams.hand(request, lines);
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
    let Some(call) = pending.remove(&id) else {
        tracing::debug!(target: "goodfetch::client", id, "a reply to a call no longer in flight");
        return;
    };
    if let Some(key) = call.deadline {
        deadlines.remove(&key);
    }
    let _gone = call.reply.send(outcome);
}

/// Hands `notification` to the streaming call in flight whose id its
/// params name; one that names no such call changes nothing. A call that
/// has not taken what it was handed fails: its engine sent past the
/// credits the call granted.
fn hand_over(
    notification: Notification,
    pending: &mut BTreeMap<i64, Pending>,
    deadlines: &mut DelayQueue<i64>,
) {
    let Notification { method, params, .. } = notification;
    let id = params
        .as_ref()
        .and_then(|params| params.get("id"))
        .and_then(Value::as_i64);
    let call = id.and_then(|id| Some((id, pending.get(&id)?.notified.as_ref()?)));
    let Some((id, notified)) = call else {
        tracing::trace!(target: "goodfetch::client", %method, "a notification no one awaits");
        return;
    };
    let heard = Heard::Notified {
        method,
        params: params.unwrap_or(Value::Null),
    };
    if let Err(mpsc::error::TrySendError::Full(_)) = notified.try_send(heard) {
        let Some(call) = pending.remove(&id) else {
            return;
        };
        if let Some(key) = call.deadline {
            deadlines.remove(&key);
        }
        tracing::debug!(target: "goodfetch::client", id, "a streaming call's engine sent past its credits");
        call.fail(Error::shape(serde::de::Error::custom(
            "the engine streamed past the credits the call granted",
        )));
    }
}
