//! The correlation's side of reaching the engine again once the link
//! dropped (spec 606): what each call is sent again with, counted as its
//! stream is read, how the calls are held meanwhile, and the attempts
//! themselves, each a new link greeted with `hello` ahead of every call
//! held.

use std::future::{Future, poll_fn};
use std::pin::pin;

use goodfetch_protocol::connection::{EngineInstance, HELLO, HelloResult};
use goodfetch_protocol::jsonrpc::{Message, Request, RequestId, Version};
use goodfetch_protocol::log::LOG;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::mpsc;

use super::{Calls, Command, Ended, Held, Input, Pending, credited, encoded, write};
use crate::error::Error;
use crate::line;
use crate::logging;
use crate::relink::{self, Relinking};
use crate::write_queue::{self, WriteQueue};

/// What a call is sent again with on a new link, once its link dropped.
pub(super) struct Again {
    /// Its line as first sent.
    line: Vec<u8>,
    /// What a stream resumes after; none for a call sent again as it was.
    resumed: Option<Resumed>,
}

/// What a stream is sent again with: after the last item read, granted
/// the credits it still had.
struct Resumed {
    stream: relink::Stream,
    /// The credits granted on the current link.
    granted: u64,
    /// The items received on it, each of which used one.
    received: u64,
    /// Where the last item read stands: an event's position or a Record's
    /// number.
    last: Option<u64>,
}

impl Again {
    /// What a call of `method`, first sent as `line`, is sent again with;
    /// none when it never is (see [`relink::resend`]), nor when it is
    /// `notified` of items no new link resumes after, as a page's
    /// listener's.
    pub(super) fn of(method: &str, line: &[u8], notified: bool) -> Option<Self> {
        let resumed = match relink::resend(method) {
            relink::Resend::Never => return None,
            relink::Resend::AsItWas if notified => return None,
            relink::Resend::AsItWas => None,
            relink::Resend::Resumed(stream) => Some(Resumed {
                stream,
                granted: 0,
                received: 0,
                last: None,
            }),
        };
        Some(Self {
            line: line.to_vec(),
            resumed,
        })
    }

    /// Counts `credits` more granted to its stream, if it resumes one.
    pub(super) fn granted(&mut self, credits: u64) {
        if let Some(resumed) = &mut self.resumed {
            resumed.granted = resumed.granted.saturating_add(credits);
        }
    }

    /// Counts the items its stream's notification `method` carries, and
    /// where the last stands.
    pub(super) fn read(&mut self, method: &str, params: &Value) {
        let Some(resumed) = &mut self.resumed else {
            return;
        };
        if let Some((items, last)) = resumed.stream.read(method, params) {
            resumed.received = resumed.received.saturating_add(items);
            if last.is_some() {
                resumed.last = last;
            }
        }
    }

    /// The call `id`'s request on a new link and the notifications naming
    /// it, and what it is sent again with on the link after; none when its
    /// line no longer reads as a request. A stream of events resumes after
    /// the last as `instance`, the dropped link's engine, published it.
    fn resent(
        mut self,
        id: i64,
        instance: Option<&EngineInstance>,
    ) -> Option<(Vec<u8>, Vec<Vec<u8>>, Self)> {
        let Some(resumed) = &mut self.resumed else {
            return Some((self.line.clone(), Vec::new(), self));
        };
        let mut request: Request = serde_json::from_slice(&self.line).ok()?;
        if let (Some(params), Some(last)) = (&mut request.params, resumed.last) {
            resumed.stream.resume(params, last, instance);
        }
        let line = line::encode(&request).ok()?;
        let credits = resumed.granted.saturating_sub(resumed.received);
        resumed.granted = credits;
        resumed.received = 0;
        let notes = u32::try_from(credits)
            .ok()
            .filter(|credits| *credits > 0)
            .and_then(|credits| encoded(&credited(id, credits)))
            .into_iter()
            .collect();
        Some((line, notes, self))
    }
}

impl Calls {
    /// What the call `id`, sent or queued, is sent again with, if it is.
    pub(super) fn again_of(&mut self, id: i64) -> Option<&mut Again> {
        match self.pending.get_mut(&id) {
            Some(sent) => sent.again.as_mut(),
            None => self.queued.get_mut(&id)?.again.as_mut(),
        }
    }

    /// Holds every call from now on, once the link dropped: queues again,
    /// by its id, each call in flight it may send again, ahead of those
    /// queued before, which keep their timers, and fails every other one
    /// with `connection_lost`. A call whose engine kept its deadline is
    /// timed by the Client again, from when it was taken. The dropped
    /// link's output closes, so an answer to one of its engine's requests
    /// fails as closed.
    fn hold(&mut self) {
        self.holding = true;
        let (closed, _unwritten) = write_queue::write_queue();
        self.writes = closed;
        // A call cancelled and not answered yet holds no place on the new
        // link.
        self.in_flight.clear();
        let instance = self.instance.take();
        for (id, call) in std::mem::take(&mut self.pending) {
            let Pending {
                method,
                reply,
                deadline,
                engine_keeps,
                notified,
                cancellable,
                again,
            } = call;
            let deadline = deadline.or_else(|| {
                engine_keeps.map(|(at, kept)| (self.deadlines.insert_at(id, at), kept))
            });
            let resent = again.and_then(|again| again.resent(id, instance.as_ref()));
            let Some((line, notes, again)) = resent else {
                self.stop_timer(deadline.map(|(key, _)| key));
                let lost = if notified.is_some() {
                    Error::lost_stream()
                } else {
                    Error::lost_call()
                };
                let _gone = reply.send(Err(lost.in_call(&method)));
                continue;
            };
            let bytes = notes
                .iter()
                .fold(line.len(), |bytes, note| bytes.saturating_add(note.len()));
            self.queued_bytes = self.queued_bytes.saturating_add(bytes);
            self.queued.insert(
                id,
                Held {
                    method,
                    line,
                    reply,
                    deadline,
                    notified,
                    credits: None,
                    cancellable,
                    notes,
                    bytes,
                    again: Some(again),
                    id_heard: true,
                },
            );
        }
    }

    /// Sends the calls held over `writes`, a new link's output, whose
    /// engine's `hello` answered `said`.
    fn relinked(&mut self, writes: WriteQueue, said: HelloResult) {
        self.writes = writes;
        self.cap = said
            .max_calls_in_flight
            .map(|cap| usize::try_from(cap.get()).unwrap_or(usize::MAX));
        self.instance = Some(said.engine_instance);
        self.holding = false;
        self.drain();
    }
}

/// Runs `work` while taking calls and passing their deadlines: what it
/// gave, or `None` once the Client closed the Connection meanwhile.
async fn meanwhile<T>(
    work: impl Future<Output = T>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    calls: &mut Calls,
) -> Option<T> {
    let mut work = pin!(work);
    loop {
        tokio::select! {
            done = &mut work => return Some(done),
            command = commands.recv() => if !calls.command(command) {
                return None;
            },
            Some(expired) = poll_fn(|context| calls.deadlines.poll_expired(context)) => {
                calls.expire(expired.into_inner());
            }
        }
    }
}

/// Reaches the engine again as `relinking` says once the link dropped for
/// `cause`, holding the calls meanwhile and waiting the backoff before
/// each attempt: the new link's input, its held calls sent; how the
/// Connection ended otherwise.
pub(super) async fn relink(
    relinking: &Relinking,
    mut cause: String,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    calls: &mut Calls,
) -> Result<Input, Ended> {
    calls.hold();
    let expiry = relinking
        .within()
        .map(|within| tokio::time::sleep(within).deadline());
    let mut attempt = 0_u32;
    loop {
        let wait = tokio::time::sleep(relink::delay(attempt));
        attempt = attempt.saturating_add(1);
        if expiry.is_some_and(|expiry| wait.deadline() >= expiry) {
            return Err(Ended::GaveUp(cause));
        }
        meanwhile(wait, commands, calls)
            .await
            .ok_or(Ended::Closed)?;
        calls.next_id += 1;
        let greeting = greet(relinking, calls.next_id);
        let greeted = match expiry {
            Some(expiry) => meanwhile(tokio::time::timeout_at(expiry, greeting), commands, calls)
                .await
                .ok_or(Ended::Closed)?
                .unwrap_or_else(|_| {
                    Err(Attempt::Failed(
                        "timeout.connect passed while reaching the engine again".to_owned(),
                    ))
                }),
            None => meanwhile(greeting, commands, calls)
                .await
                .ok_or(Ended::Closed)?,
        };
        match greeted {
            Ok(Greeted {
                input,
                writes,
                said,
            }) => {
                calls.relinked(writes, said);
                relinking.reached();
                return Ok(input);
            }
            Err(Attempt::Failed(failed)) => {
                tracing::debug!(target: "goodfetch::client", cause = %failed, "an attempt to reach the engine again failed");
                cause = failed;
            }
            Err(Attempt::Ended(ended)) => return Err(ended),
        }
    }
}

/// A new link, greeted.
struct Greeted {
    input: Input,
    writes: WriteQueue,
    said: HelloResult,
}

/// How an attempt to reach the engine again that greeted no link went.
enum Attempt {
    /// It failed for this cause; the next one may not.
    Failed(String),
    /// The engine answered the greeting, and the Connection ends so.
    Ended(Ended),
}

/// Opens a new link as `relinking` says and greets it with `hello`, the
/// call `id`, ahead of every call held; the engine's logs meanwhile are
/// relayed, and nothing else it sends is read.
async fn greet(relinking: &Relinking, id: i64) -> Result<Greeted, Attempt> {
    let failed = |cause: &dyn std::fmt::Display| Attempt::Failed(cause.to_string());
    let link = relinking.reopen().await.map_err(|error| failed(&error))?;
    let (engine_output, engine_input, _exit) = link.split();
    let (writes, unwritten) = write_queue::write_queue();
    relinking.spawn(write(engine_input, unwritten));
    let hello = Request {
        jsonrpc: Version,
        id: RequestId::Number(id),
        method: HELLO.to_owned(),
        params: Some(serde_json::to_value(relinking.greeting()).map_err(|error| failed(&error))?),
    };
    let greeting = line::encode(&hello).map_err(|error| failed(&error))?;
    writes
        .send(greeting)
        .map_err(|_| failed(&"the new link's output closed"))?;
    let mut input = BufReader::new(engine_output).lines();
    loop {
        let read = input
            .next_line()
            .await
            .map_err(|error| failed(&error))?
            .ok_or_else(|| failed(&"the engine closed the new link before it answered `hello`"))?;
        match serde_json::from_str::<Message>(&read) {
            Ok(Message::Response(answer)) if answer.id == hello.id => {
                let said = serde_json::from_value(answer.result)
                    .map_err(|error| Attempt::Ended(Ended::Unreadable(error.to_string())))?;
                return Ok(Greeted {
                    input,
                    writes,
                    said,
                });
            }
            Ok(Message::Error(refused)) if refused.id.as_ref() == Some(&hello.id) => {
                return Err(Attempt::Ended(Ended::Refused(refused.error)));
            }
            Ok(Message::Notification(notification)) if notification.method == LOG => {
                logging::relay(notification.params);
            }
            _ => {
                tracing::debug!(target: "goodfetch::client", "a line before `hello`'s answer on a new link");
            }
        }
    }
}
