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
use std::time::Duration;

use goodfetch_protocol::jsonrpc::{Message, Notification, Request, RequestId, Version};
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
use crate::transport::Link;

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

/// What the Connection's correlating task acts on.
enum Command {
    /// Sends a call.
    Call(Outgoing),
    /// Sends a notification.
    Notify(Notification),
    /// Ends the Connection: no more calls, the output closed, every call
    /// in flight failed as closed.
    Close,
}

/// The handle every call goes through: cheap to clone, `Send + Sync`.
#[derive(Clone, Debug)]
pub(crate) struct Sender(mpsc::UnboundedSender<Command>);

impl Sender {
    /// Sends `call`; its reply fails as closed when the Connection is.
    pub(crate) fn call(&self, call: Outgoing) {
        if let Err(mpsc::error::SendError(Command::Call(call))) = self.0.send(Command::Call(call)) {
            let _closed = call.reply.send(Err(Error::Closed));
        }
    }

    /// Sends the notification `method` with `params`; one sent once the
    /// Connection is closed goes nowhere.
    pub(crate) fn notify(&self, method: &str, params: Value) {
        let _closed = self.0.send(Command::Notify(Notification {
            jsonrpc: Version,
            method: method.to_owned(),
            params: Some(params),
        }));
    }

    /// Ends the Connection without waiting.
    pub(crate) fn close(&self) {
        let _already_closed = self.0.send(Command::Close);
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
    let (reader, writer) = link.split();
    let (commands, commands_in) = mpsc::unbounded_channel();
    let (lines, lines_in) = mpsc::unbounded_channel();
    tracker.spawn_on(write(writer, lines_in), runtime);
    tracker.spawn_on(correlate(reader, commands_in, lines, streams), runtime);
    Sender(commands)
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
    reply: oneshot::Sender<Result<Value, Error>>,
    deadline: Option<Key>,
    notified: Option<mpsc::Sender<Heard>>,
}

/// Sends calls, matches replies and deadlines to them, hands the engine's
/// requests to `streams`, and fails what is left as closed when the
/// Connection ends, by `close()`, by every handle dropped or by the engine
/// closing its output, which also ends the streams.
async fn correlate(
    reader: Box<dyn AsyncRead + Send + Unpin>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    lines: mpsc::UnboundedSender<Vec<u8>>,
    streams: Streams,
) {
    let mut input = BufReader::new(reader).lines();
    let mut pending: BTreeMap<i64, Pending> = BTreeMap::new();
    let mut deadlines: DelayQueue<i64> = DelayQueue::new();
    let mut next_id: i64 = 0;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Call(call)) => {
                    next_id += 1;
                    let id = next_id;
                    let request = Request {
                        jsonrpc: Version,
                        id: RequestId::Number(id),
                        method: call.method,
                        params: Some(call.params),
                    };
                    let line = match line::encode(&request) {
                        Ok(line) => line,
                        Err(error) => {
                            let _gone = call.reply.send(Err(Error::Shape(error)));
                            continue;
                        }
                    };
                    if lines.send(line).is_err() {
                        let _gone = call.reply.send(Err(Error::Closed));
                        continue;
                    }
                    let deadline = call.timeout.map(|timeout| deadlines.insert(id, timeout));
                    if let Some(notified) = &call.notified {
                        let _gone = notified.try_send(Heard::Sent(id));
                    }
                    pending.insert(id, Pending { reply: call.reply, deadline, notified: call.notified });
                }
                Some(Command::Notify(notification)) => {
                    let Ok(mut line) = serde_json::to_vec(&notification) else {
                        tracing::warn!(target: "goodfetch::client", method = %notification.method, "a notification that is no JSON was not sent");
                        continue;
                    };
                    line.push(b'\n');
                    let _closed = lines.send(line);
                }
                Some(Command::Close) | None => break,
            },
            Some(expired) = poll_fn(|context| deadlines.poll_expired(context)) => {
                let id = expired.into_inner();
                if let Some(call) = pending.remove(&id) {
                    tracing::debug!(target: "goodfetch::client", id, "a call timed out");
                    let _gone = call.reply.send(Err(Error::TimedOut));
                }
            }
            line = input.next_line() => match line {
                Ok(Some(line)) => answer(&line, &mut pending, &mut deadlines, &streams, &lines),
                Ok(None) => {
                    tracing::debug!(target: "goodfetch::client", "the engine closed the Connection");
                    break;
                }
                Err(error) => {
                    tracing::debug!(target: "goodfetch::client", %error, "the Connection's input failed");
                    break;
                }
            },
        }
    }
    streams.end();
    commands.close();
    while let Ok(command) = commands.try_recv() {
        if let Command::Call(call) = command {
            let _gone = call.reply.send(Err(Error::Closed));
        }
    }
    for call in std::mem::take(&mut pending).into_values() {
        let _gone = call.reply.send(Err(Error::Closed));
    }
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
        Ok(Message::Error(failure)) => (failure.id, Err(Error::Failed(Box::new(failure.error)))),
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
        let _gone = call.reply.send(Err(Error::Shape(serde::de::Error::custom(
            "the engine streamed past the credits the call granted",
        ))));
    }
}
