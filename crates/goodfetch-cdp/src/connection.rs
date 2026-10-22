//! The client's connection to one browser over its pipe: an actor, the
//! [`Driver`], owns both ends and the commands in flight; any task holding
//! a [`Connection`] sends commands through it and subscribes to the events
//! of one target's session (CDP's flat sessions, `Target.attachToTarget`
//! with `flatten`). When the browser closes its end, every command in
//! flight fails with [`Failure::Disconnected`] and every subscription ends.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};

use crate::failure::{Failure, PartyText};
use crate::framing::{Framer, FramingError, frame};

/// How many requests wait for the driver before their senders wait.
const REQUESTS: usize = 64;

/// How many events wait for a subscriber before the driver waits for it.
const EVENTS: usize = 256;

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
    /// Its params as `E`'s, when it is `E`.
    pub fn read<E: crate::Event>(&self) -> Option<E> {
        if self.method != E::METHOD {
            return None;
        }
        serde_json::from_value(self.params.clone()).ok()
    }
}

/// The events of one session, from when it was subscribed.
#[derive(Debug)]
pub struct Subscription {
    events: mpsc::Receiver<Event>,
}

impl Subscription {
    /// The next event; `None` once the connection ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub async fn next(&mut self) -> Option<Event> {
        self.events.recv().await
    }
}

enum Request {
    Command {
        method: &'static str,
        session: Option<SessionId>,
        params: Value,
        answer: oneshot::Sender<Result<Value, Failure>>,
    },
    Subscribe {
        session: Option<SessionId>,
        events: mpsc::Sender<Event>,
        subscribed: oneshot::Sender<()>,
    },
}

/// A handle to the browser's connection, which any task may clone.
#[derive(Clone, Debug)]
pub struct Connection {
    requests: mpsc::Sender<Request>,
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
    /// The browser sent a message past the framing's limit.
    TooLarge,
}

/// The connection's actor: run it as a task of the browser's owner.
pub struct Driver<R, W> {
    reader: R,
    writer: W,
    requests: mpsc::Receiver<Request>,
}

impl Connection {
    /// A connection over the browser's end of the pipe `reader` (what it
    /// writes) and `writer` (what it reads), and its driver.
    pub fn new<R, W>(reader: R, writer: W) -> (Self, Driver<R, W>)
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        let (requests, receiver) = mpsc::channel(REQUESTS);
        (
            Self { requests },
            Driver {
                reader,
                writer,
                requests: receiver,
            },
        )
    }

    /// Sends `C` on `session` (the browser's own when `None`) and answers
    /// what it returns.
    ///
    /// # Errors
    ///
    /// Each [`Failure`]: the browser's refusal, no answer within `timeout`,
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
        self.requests
            .send(Request::Command {
                method,
                session: session.cloned(),
                params,
                answer,
            })
            .await
            .map_err(|_| Failure::Disconnected)?;
        match tokio::time::timeout(timeout, answered).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(Failure::Disconnected),
            Err(_) => Err(Failure::TimedOut { method }),
        }
    }

    /// The events of `session` (the browser's own when `None`) the driver
    /// reads once it answers.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the subscription.
    pub async fn subscribe(&self, session: Option<&SessionId>) -> Subscription {
        let (events, receiver) = mpsc::channel(EVENTS);
        let (subscribed, confirmed) = oneshot::channel();
        // A connection that ended gives a subscription that ends at once.
        if self
            .requests
            .send(Request::Subscribe {
                session: session.cloned(),
                events,
                subscribed,
            })
            .await
            .is_ok()
        {
            // Every event the driver reads from now on reaches it.
            let _ = confirmed.await;
        }
        Subscription { events: receiver }
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
    answer: oneshot::Sender<Result<Value, Failure>>,
}

impl<R, W> Driver<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    /// Runs the connection until the browser closes it or every handle is
    /// dropped, and says which.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the client's end: the commands in flight
    /// get no answer.
    pub async fn run(mut self) -> Ended {
        let mut framer = Framer::default();
        let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
        let mut subscribers: Vec<(Option<SessionId>, mpsc::Sender<Event>)> = Vec::new();
        let mut next_id: u64 = 1;
        let mut buffer = vec![0; READ];
        let ended = loop {
            tokio::select! {
                request = self.requests.recv() => {
                    let Some(request) = request else { break Ended::Dropped };
                    match request {
                        Request::Command { method, session, params, answer } => {
                            let id = next_id;
                            next_id += 1;
                            let mut message = json!({ "id": id, "method": method, "params": params });
                            if let Some(session) = session {
                                message["sessionId"] = Value::String(session.0);
                            }
                            tracing::trace!(target: "goodfetch::cdp::command", method, id, "sending");
                            if let Err(error) = self.writer.write_all(&frame(&message)).await {
                                let _ = answer.send(Err(Failure::Disconnected));
                                break Ended::Broken(error.kind());
                            }
                            pending.insert(id, Pending { method, answer });
                        }
                        Request::Subscribe { session, events, subscribed } => {
                            subscribers.push((session, events));
                            let _ = subscribed.send(());
                        }
                    }
                }
                read = self.reader.read(&mut buffer) => {
                    match read {
                        Ok(0) => break Ended::Closed,
                        Ok(count) => framer.push(&buffer[..count]),
                        Err(error) => break Ended::Broken(error.kind()),
                    }
                    let mut too_large = false;
                    loop {
                        match framer.next_frame() {
                            Ok(Some(bytes)) => {
                                dispatch(&bytes, &mut pending, &mut subscribers).await;
                            }
                            Ok(None) => break,
                            Err(FramingError::TooLarge) => {
                                too_large = true;
                                break;
                            }
                        }
                    }
                    if too_large {
                        break Ended::TooLarge;
                    }
                }
            }
        };
        for (_, Pending { answer, .. }) in pending {
            let _ = answer.send(Err(Failure::Disconnected));
        }
        ended
    }
}

/// Hands one message to the command it answers or to the subscribers of
/// its session.
async fn dispatch(
    bytes: &[u8],
    pending: &mut BTreeMap<u64, Pending>,
    subscribers: &mut Vec<(Option<SessionId>, mpsc::Sender<Event>)>,
) {
    let Ok(message) = serde_json::from_slice::<Value>(bytes) else {
        tracing::warn!("the browser sent a message that is not JSON");
        return;
    };
    if let Some(id) = message.get("id").and_then(Value::as_u64) {
        let Some(Pending { method, answer }) = pending.remove(&id) else {
            return;
        };
        let answered = match message.get("error") {
            Some(error) => Err(Failure::refused(method, error)),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = answer.send(answered);
        return;
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return;
    };
    let session = message
        .get("sessionId")
        .and_then(Value::as_str)
        .map(|session| SessionId(session.to_owned()));
    let event = Event {
        method: method.to_owned(),
        params: message.get("params").cloned().unwrap_or(Value::Null),
    };
    let mut index = 0;
    while index < subscribers.len() {
        let (subscribed, events) = &subscribers[index];
        if *subscribed == session && events.send(event.clone()).await.is_err() {
            subscribers.swap_remove(index);
            continue;
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests;
