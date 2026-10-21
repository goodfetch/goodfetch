//! One Connection's life: it opens with `hello`, then each request goes to
//! its handler as a task of the Connection's owner, each call under its own
//! cancellation token that `$/cancel` trips, while one writer task sends
//! every line whole. A refused `hello` is answered before any other message
//! and the Connection then closes (ADR 0066). At the end of input, calls in
//! flight are cancelled at their next cancellation point, answered, and the
//! Connection closes once they all are.
//!
//! The engine also sends the Client handler requests, such as a crawl's
//! `crawl_request.handle`, each under an id of its own, and routes each
//! reply back to its sender; a sender's cancel of one still unanswered
//! goes to the Client as `$/cancel`, and the reply that still comes is
//! routed the same way. The Connection's end fails those still waiting. A `$/credit` adds credits to the streaming call it names.
//!
//! A Daemon's Connection must open with a `hello` carrying the Daemon's
//! token: anything else is closed without an answer, so a stranger learns
//! nothing (ADR 0024). Each refusal is logged under [`REFUSAL_TARGET`],
//! with its reason, until the Audit log records it (area 4).

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CANCEL, CallCancelledDetails, CancelParams, HELLO, HelloParams, StoppedBy,
};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::foxhound_names;
use goodfetch_protocol::jsonrpc::{
    ErrorObject, Notification, ProtocolFailure, Request, RequestId, Version,
};
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::framing::{DecodeError, Frame, Framer, Incoming, MAX_MESSAGE_BYTES, decode};
use super::{
    failure_line, handshake, line, not_in_product, notification_line, record, success_line,
};
use crate::client::{Client, Outgoing};
use crate::composition::{
    CallContext, CallError, CallStream, ConnectionContext, DaemonSide, Handler,
};
use crate::events::{Credits, Events};
use crate::product::{Identity, Requested};
use crate::runs::Runs;
use crate::tasks::TaskOwner;
use crate::transport::{Connection, ConnectionReader, ConnectionWriter};

/// How many bytes one read takes from the Connection.
const READ_CHUNK: usize = 8 * 1024;

/// How many calls one Connection keeps in flight before it stops reading.
const MAX_IN_FLIGHT: usize = 256;

/// How many lines wait for the writer before their senders wait.
const OUTGOING_LINES: usize = 64;

/// The `tracing` target of a Daemon's refusals of a Connection.
pub const REFUSAL_TARGET: &str = "goodfetch::daemon::refusal";

/// What every Connection of an engine dispatches to.
pub(crate) struct Dispatch {
    pub(crate) identity: Arc<Identity>,
    pub(crate) handlers: BTreeMap<&'static str, Handler>,
}

/// Serves `connection` until it ends, its tasks those of `owner`.
///
/// # Cancel safety
///
/// Dropping the future leaves the calls it started to `owner`, which stops
/// them.
pub(crate) async fn serve(
    dispatch: Arc<Dispatch>,
    connection: Connection,
    owner: &TaskOwner,
    runs: Runs,
    events: Events,
    daemon: Option<Arc<dyn DaemonSide>>,
) {
    let (mut reader, writer) = connection.split();
    let (outgoing, lines) = mpsc::channel(OUTGOING_LINES);
    owner.spawn(write_lines(writer, lines));
    let (completed, completions) = mpsc::channel(MAX_IN_FLIGHT);
    let (client, requests) = Client::new();
    let connection = Arc::new(ConnectionContext {
        identity: Arc::clone(&dispatch.identity),
        client,
        runs,
        events,
        daemon,
    });
    let mut session = Session {
        dispatch,
        owner,
        outgoing,
        completed,
        completions,
        in_flight: BTreeMap::new(),
        behind_gateway: false,
        opened: false,
        connection,
        requests,
        pending: BTreeMap::new(),
        next_request: 0,
        opened_client: None,
    };
    session.run(&mut reader).await;
    session.cancel_in_flight();
    drop(session);
    owner.wait().await;
}

/// Writes each line whole, flushing once no other line waits; shuts the
/// Connection's sending side down when every sender is gone.
async fn write_lines(mut writer: ConnectionWriter, mut lines: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = lines.recv().await {
        if writer.write_all(&line).await.is_err() {
            return;
        }
        if lines.is_empty() && writer.flush().await.is_err() {
            return;
        }
    }
    let _ = writer.shutdown().await;
}

/// A call in flight.
struct InFlight {
    token: CancellationToken,
    cancellable: bool,
    credits: Option<Credits>,
}

/// Whether the Connection goes on after a message.
enum Flow {
    Continue,
    Close,
}

struct Session<'owner> {
    dispatch: Arc<Dispatch>,
    owner: &'owner TaskOwner,
    outgoing: mpsc::Sender<Vec<u8>>,
    completed: mpsc::Sender<RequestId>,
    completions: mpsc::Receiver<RequestId>,
    in_flight: BTreeMap<RequestId, InFlight>,
    behind_gateway: bool,
    opened: bool,
    connection: Arc<ConnectionContext>,
    requests: mpsc::Receiver<Outgoing>,
    pending: BTreeMap<RequestId, Pending>,
    next_request: i64,
    /// On a Daemon, the count of this Client, held while it is open.
    opened_client: Option<Box<dyn Send + Sync>>,
}

/// A handler request the engine sent, waiting for its reply.
struct Pending {
    method: &'static str,
    reply: oneshot::Sender<Result<Value, ErrorObject>>,
}

impl Session<'_> {
    async fn run(&mut self, reader: &mut ConnectionReader) {
        let mut framer = Framer::new(MAX_MESSAGE_BYTES);
        let mut pending = Vec::new();
        let mut chunk = vec![0; READ_CHUNK];
        let mut ended = false;
        loop {
            for frame in std::mem::take(&mut pending) {
                if let Flow::Close = self.handle(frame).await {
                    return;
                }
            }
            if ended {
                self.cancel_in_flight();
                if self.in_flight.is_empty() {
                    return;
                }
            }
            let reading = !ended && self.in_flight.len() < MAX_IN_FLIGHT;
            tokio::select! {
                biased;
                Some(id) = self.completions.recv() => {
                    self.in_flight.remove(&id);
                }
                Some(request) = self.requests.recv(), if self.opened => {
                    if let Flow::Close = self.send_request(request).await {
                        return;
                    }
                }
                read = reader.read(&mut chunk), if reading => match read {
                    Ok(0) | Err(_) => {
                        ended = true;
                        pending.extend(framer.finish());
                    }
                    Ok(read) => framer.feed(&chunk[..read], &mut pending),
                },
            }
        }
    }

    fn cancel_in_flight(&self) {
        for call in self.in_flight.values() {
            if call.cancellable {
                call.token.cancel();
            }
        }
    }

    async fn handle(&mut self, frame: Frame) -> Flow {
        let incoming = match frame {
            Frame::TooLarge => Err(DecodeError::too_large()),
            Frame::Line(line) => decode(&line),
        };
        if !self.opened && self.connection.daemon.is_some() {
            if let Ok(Incoming::Request(request)) = incoming {
                return self.open(request).await;
            }
            refused("hello_not_first");
            return Flow::Close;
        }
        match (incoming, self.opened) {
            (Err(error), opened) => {
                let failure = CallError::protocol(error.failure, error.detail);
                if self.send(failure_line(error.id, &failure)).await && opened {
                    Flow::Continue
                } else {
                    Flow::Close
                }
            }
            (Ok(Incoming::Request(request)), false) => self.open(request).await,
            (Ok(Incoming::Notification(notification)), false) => {
                tracing::debug!(method = %notification.method, "a Connection that does not open with `hello` closes");
                Flow::Close
            }
            (Ok(Incoming::Request(request)), true) => self.request(request).await,
            (Ok(Incoming::Notification(notification)), true) => {
                self.notification(&notification);
                Flow::Continue
            }
            (Ok(Incoming::Reply { id, .. }), false) => {
                tracing::debug!(?id, "a Connection that does not open with `hello` closes");
                Flow::Close
            }
            (Ok(Incoming::Reply { id, answer }), true) => {
                self.reply(&id, answer);
                Flow::Continue
            }
        }
    }

    /// Sends a handler request under the next id of the engine's own, or
    /// `$/cancel` of one still waiting for its reply.
    async fn send_request(&mut self, request: Outgoing) -> Flow {
        let line = match request {
            Outgoing::Request {
                method,
                params,
                sent,
                reply,
            } => {
                let id = RequestId::Number(self.next_request);
                self.next_request += 1;
                let message = Request {
                    jsonrpc: Version,
                    id: id.clone(),
                    method: method.to_owned(),
                    params: Some(params),
                };
                let _ = sent.send(id.clone());
                self.pending.insert(id, Pending { method, reply });
                line(&message)
            }
            Outgoing::Cancel(id) => {
                if !self.pending.contains_key(&id) {
                    return Flow::Continue;
                }
                notification_line(CANCEL, &CancelParams { id })
            }
        };
        if self.send(line).await {
            Flow::Continue
        } else {
            Flow::Close
        }
    }

    /// Routes a reply to the handler request it answers; a reply whose
    /// sender stopped waiting, or that answers no request, is dropped.
    fn reply(&mut self, id: &RequestId, answer: Result<Value, ErrorObject>) {
        let Some(pending) = self.pending.remove(id) else {
            tracing::debug!(?id, "a reply that answers no handler request is ignored");
            return;
        };
        record(
            pending.method,
            answer.as_ref().err().map(|error| error.data.code.as_str()),
        );
        let _ = pending.reply.send(answer);
    }

    /// The first request, which must be `hello`, carrying the token on a
    /// Daemon.
    async fn open(&mut self, request: Request) -> Flow {
        let daemon = self.connection.daemon.clone();
        if request.method != HELLO {
            if daemon.is_some() {
                refused("hello_not_first");
                return Flow::Close;
            }
            let error = CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "a Connection opens with `hello`",
            );
            record(&request.method, Some(error.code()));
            self.send(failure_line(Some(request.id), &error)).await;
            return Flow::Close;
        }
        let hello = serde_json::from_value::<HelloParams>(request.params.unwrap_or(Value::Null));
        if let Some(daemon) = &daemon {
            let token = hello.as_ref().ok().and_then(|hello| hello.token.as_deref());
            match token {
                None => {
                    refused("token_missing");
                    return Flow::Close;
                }
                Some(token) if !same_token(token, daemon.token()) => {
                    refused("token_wrong");
                    return Flow::Close;
                }
                Some(_) => {}
            }
        }
        let answered = match hello {
            Ok(hello) => handshake::answer(&self.dispatch.identity, &hello)
                .map(|accepted| (accepted, hello.surface)),
            Err(error) => Err(handshake::Refused::Invalid(CallError::protocol(
                ProtocolFailure::InvalidParams,
                error.to_string(),
            ))),
        };
        let answered = match answered {
            Ok(accepted) => Ok(accepted),
            Err(refusal) => {
                let handover = match &daemon {
                    Some(daemon) if refusal.by_product_or_version() => daemon.refusing().await,
                    _ => false,
                };
                Err(refusal.into_error(handover))
            }
        };
        match answered {
            Ok((accepted, surface)) => {
                record(HELLO, None);
                self.opened = true;
                self.opened_client = daemon.map(|daemon| daemon.opened(surface));
                self.behind_gateway = accepted.behind_gateway;
                let result = serde_json::to_value(accepted.result)
                    .expect("invariant: a `hello` reply encodes");
                if self.send(success_line(request.id, result)).await {
                    Flow::Continue
                } else {
                    Flow::Close
                }
            }
            Err(error) => {
                record(HELLO, Some(error.code()));
                self.send(failure_line(Some(request.id), &error)).await;
                Flow::Close
            }
        }
    }

    async fn request(&mut self, request: Request) -> Flow {
        let Request {
            id, method, params, ..
        } = request;
        let refusal = if method == HELLO {
            Some(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "a Connection opens with `hello` once",
            ))
        } else if method == CANCEL {
            Some(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "`$/cancel` is a notification",
            ))
        } else if self.in_flight.contains_key(&id) {
            Some(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                format!("a call with the id {id:?} is already in flight"),
            ))
        } else if let Some(handler) = self.dispatch.handlers.get(method.as_str()) {
            let handler = handler.clone();
            self.call(id, &handler, params);
            return Flow::Continue;
        } else if !self.dispatch.identity.product.contains(Product::Foxhound)
            && foxhound_names::OPERATIONS.contains(&method.as_str())
        {
            Some(not_in_product(
                &self.dispatch.identity,
                Requested::Name(&method),
                Product::Foxhound,
                self.behind_gateway,
            ))
        } else {
            Some(CallError::protocol(
                ProtocolFailure::MethodNotFound,
                format!("no Product carries the operation `{method}`"),
            ))
        };
        let Some(error) = refusal else {
            return Flow::Continue;
        };
        record(&method, Some(error.code()));
        if self.send(failure_line(Some(id), &error)).await {
            Flow::Continue
        } else {
            Flow::Close
        }
    }

    /// Starts `handler` on `params` as a task of the Connection.
    fn call(&mut self, id: RequestId, handler: &Handler, params: Option<Value>) {
        let token = self.owner.token().child_token();
        let operation = handler.operation();
        let credits = handler.streams().then(Credits::new);
        let stream = credits.clone().map(|credits| CallStream {
            id: id.clone(),
            credits,
            outgoing: self.outgoing.clone(),
        });
        let context = CallContext::new(Arc::clone(&self.connection), token.clone(), stream);
        let answer = handler.call(params.unwrap_or(Value::Object(Map::new())), context);
        self.in_flight.insert(
            id.clone(),
            InFlight {
                token: token.clone(),
                cancellable: operation.cancellable,
                credits,
            },
        );
        let (outgoing, completed) = (self.outgoing.clone(), self.completed.clone());
        self.owner.spawn(async move {
            let answered = if operation.cancellable {
                tokio::select! {
                    biased;
                    answered = answer => answered,
                    () = token.cancelled() => Err(CallError::new(
                        &CALL_CANCELLED,
                        "The call was cancelled by its Client.",
                        &CallCancelledDetails { by: StoppedBy::Client },
                    )),
                }
            } else {
                answer.await
            };
            let line = match answered {
                Ok(result) => {
                    record(operation.name, None);
                    success_line(id.clone(), result)
                }
                Err(error) => {
                    record(operation.name, Some(error.code()));
                    failure_line(Some(id.clone()), &error)
                }
            };
            let _ = outgoing.send(line).await;
            let _ = completed.send(id).await;
        });
    }

    fn notification(&self, notification: &Notification) {
        if notification.method == CREDIT {
            self.credit(notification);
            return;
        }
        if notification.method != CANCEL {
            tracing::debug!(method = %notification.method, "an unknown notification is ignored");
            return;
        }
        let params = notification.params.clone().unwrap_or(Value::Null);
        match serde_json::from_value::<CancelParams>(params) {
            Ok(CancelParams { id }) => {
                if let Some(call) = self.in_flight.get(&id)
                    && call.cancellable
                {
                    call.token.cancel();
                }
            }
            Err(error) => tracing::debug!(%error, "a `$/cancel` without a call's id is ignored"),
        }
    }

    /// Adds a `$/credit`'s credits to the call it names; one for a call no
    /// longer in flight, or for one that does not stream, is ignored.
    fn credit(&self, notification: &Notification) {
        let params = notification.params.clone().unwrap_or(Value::Null);
        match serde_json::from_value::<CreditParams>(params) {
            Ok(CreditParams { id, credits }) => {
                if let Some(call_credits) = self
                    .in_flight
                    .get(&id)
                    .and_then(|call| call.credits.as_ref())
                {
                    call_credits.grant(credits);
                }
            }
            Err(error) => tracing::debug!(%error, "a `$/credit` that is not one is ignored"),
        }
    }

    /// Sends a line; `false` once the writer is gone.
    async fn send(&self, line: Vec<u8>) -> bool {
        self.outgoing.send(line).await.is_ok()
    }
}

/// Logs a Daemon's refusal of a Connection, with its reason.
fn refused(reason: &'static str) {
    tracing::warn!(target: REFUSAL_TARGET, reason, "the Daemon closed a Connection that did not open with `hello` and its token");
}

/// Whether two tokens are equal, in time that depends only on their
/// lengths, so a Client cannot guess one byte at a time.
fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}
