//! One Connection's life, whose states change only as the engine's
//! transition table says (ADR 0128): it opens with `hello`, then each request goes to
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
//! routed the same way. The Connection's end fails those still waiting. A `$/credit` adds credits to the streaming call it names, and a notification a call takes as its input goes to that call.
//!
//! A Connection to a Daemon or to an Attached engine's private endpoint
//! must open with a `hello` carrying that listener's token: anything else
//! is closed without an answer, so a stranger learns nothing (ADRs 0024 and
//! 0031). Each refusal is logged under [`REFUSAL_TARGET`], with its
//! listener and its reason, until the Audit log records it (area 4). An
//! Exposed Daemon's address carries its Connections as lines or over
//! WebSocket (ADR 0049), whose refused handshakes are logged the same way.
//! Only the Client of an Attached engine's stdio is told its private
//! endpoint, in the reply to its `hello`.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use goodfetch_protocol::catalog::{Product, Reach};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CANCEL, CallCancelledDetails, CancelParams, HELLO, HelloParams,
    PERSON_REQUIRED, PrivateEndpoint, StoppedBy, Surface,
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
use super::websocket::{
    self, Authorities, ConsoleSocket, MessageReader, MessageWriter, NotOpened, Prefixed, Rejoined,
};
use super::{
    failure_line, handshake, line, not_in_product, notification_line, record, record_cancel,
    success_line,
};
use crate::client::{Client, Outgoing};
use crate::composition::{
    CallContext, CallError, CallInput, CallStream, ConnectionContext, DaemonSide, Handler,
};
use crate::events::{Credits, Events};
use crate::log::LogEvents;
use crate::product::{Identity, Requested};
use crate::runs::Runs;
use crate::tasks::TaskOwner;
use crate::transition::engine::{
    ConnectionEffect, ConnectionEvent, ConnectionState, InvalidConnectionTransition,
};
use crate::transition::{self, Refusal};
use crate::transport::{Connection, ConnectionWriter};

/// How many bytes one read takes from the Connection.
const READ_CHUNK: usize = 8 * 1024;

/// How many calls one Connection keeps in flight before it stops reading.
const MAX_IN_FLIGHT: usize = 256;

/// How many lines wait for the writer before their senders wait.
const OUTGOING_LINES: usize = 64;

/// What a dispatch record names a method no catalog holds.
const UNKNOWN_METHOD: &str = "(unknown)";

/// The `tracing` target of a listener's refusals of a Connection.
pub const REFUSAL_TARGET: &str = "goodfetch::rpc::refusal";

/// What every Connection of an engine dispatches to.
pub(crate) struct Dispatch {
    pub(crate) identity: Arc<Identity>,
    pub(crate) handlers: BTreeMap<&'static str, Handler>,
    /// The process's log events, which each Connection receives at its
    /// own level.
    pub(crate) log_events: LogEvents,
}

/// How a Connection reached the engine, which says what its `hello` must
/// carry and what its reply tells.
pub(crate) enum Access {
    /// An Attached engine's stdio, or an in-process Connection: no token;
    /// its reply names the engine's private endpoint, when it has one.
    Attached(Option<PrivateEndpoint>),
    /// A Daemon's listener: its token, and what else the Daemon gives.
    Daemon(Arc<dyn DaemonSide>),
    /// An Exposed Daemon's address (ADR 0049): a Daemon's listener that
    /// also carries the protocol over WebSocket, its handshake held to the
    /// names the Daemon is reached at.
    Exposed(Arc<dyn DaemonSide>, Arc<Authorities>),
    /// An Attached engine's private endpoint: its token.
    PrivateEndpoint(Arc<str>),
    /// The Console's WebSocket on its own origin (ADR 0031): a Daemon's
    /// Connection whose sign-in cookie its handshake carried, so its
    /// `hello` carries no token.
    Console(Arc<dyn DaemonSide>),
}

impl Access {
    /// The token its `hello` must carry, if any.
    fn token(&self) -> Option<&str> {
        match self {
            Self::Attached(_) | Self::Console(_) => None,
            Self::Daemon(daemon) | Self::Exposed(daemon, _) => Some(daemon.token()),
            Self::PrivateEndpoint(token) => Some(token),
        }
    }

    /// The Daemon it reaches, if any.
    fn daemon(&self) -> Option<Arc<dyn DaemonSide>> {
        match self {
            Self::Daemon(daemon) | Self::Exposed(daemon, _) | Self::Console(daemon) => {
                Some(Arc::clone(daemon))
            }
            Self::Attached(_) | Self::PrivateEndpoint(_) => None,
        }
    }

    /// Its listener's name in the endpoint table (ADR 0067), for a
    /// Connection of newline-delimited JSON-RPC.
    fn listener(&self) -> &'static str {
        match self {
            Self::Attached(_) => "stdio",
            Self::Daemon(_) => "daemon_socket",
            Self::Exposed(..) => "exposed_daemon",
            Self::PrivateEndpoint(_) => "private_endpoint",
            Self::Console(_) => CONSOLE_LISTENER,
        }
    }

    /// Whether a stranger learns nothing from it: a first message that is
    /// no `hello` is closed unanswered on a listener a Client proves
    /// itself to.
    fn guarded(&self) -> bool {
        self.token().is_some() || matches!(self, Self::Console(_))
    }
}

/// The endpoint table's name of the Console's WebSocket (ADR 0067).
pub const CONSOLE_LISTENER: &str = "console";

/// The endpoint table's name of a Daemon's WebSocket listener (ADR 0067).
const WEBSOCKET_LISTENER: &str = "daemon_websocket";

/// How a Connection's messages arrive: as lines, or as WebSocket messages.
enum Reading {
    Lines {
        reader: Prefixed,
        framer: Framer,
        chunk: Vec<u8>,
    },
    Messages(MessageReader),
}

impl Reading {
    fn lines(reader: Prefixed) -> Self {
        Self::Lines {
            reader,
            framer: Framer::new(MAX_MESSAGE_BYTES),
            chunk: vec![0; READ_CHUNK],
        }
    }

    /// Reads what comes next and appends each message it ends to `frames`;
    /// `false` once the Connection has ended, after appending its last.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing the Connection sent.
    async fn read(&mut self, frames: &mut Vec<Frame>) -> bool {
        match self {
            Self::Lines {
                reader,
                framer,
                chunk,
            } => match reader.read(chunk).await {
                Ok(0) | Err(_) => {
                    frames.extend(framer.finish());
                    false
                }
                Ok(read) => {
                    framer.feed(&chunk[..read], frames);
                    true
                }
            },
            Self::Messages(messages) => messages.read(frames).await,
        }
    }
}

/// How a Connection's messages leave: as lines, or as WebSocket messages.
enum Writing {
    Lines(ConnectionWriter),
    Messages(MessageWriter),
}

/// How `connection`, reached through `access`, carries its messages, and
/// its listener's name; `None` when an Exposed Daemon's WebSocket handshake
/// was refused or failed, which closes it.
async fn carriage(
    connection: Connection,
    access: &Access,
) -> Option<(Reading, Writing, &'static str)> {
    let (mut reader, writer) = connection.split();
    let Access::Exposed(_, authorities) = access else {
        let reader = Prefixed::new(Vec::new(), reader);
        return Some((
            Reading::lines(reader),
            Writing::Lines(writer),
            access.listener(),
        ));
    };
    let mut first = vec![0; READ_CHUNK];
    let read = reader.read(&mut first).await.unwrap_or(0);
    first.truncate(read);
    let reader = Prefixed::new(first, reader);
    if !reader.opens_a_request() {
        return Some((
            Reading::lines(reader),
            Writing::Lines(writer),
            access.listener(),
        ));
    }
    match websocket::accept(Rejoined::new(reader, writer), authorities).await {
        Ok((messages, sent)) => Some((
            Reading::Messages(messages),
            Writing::Messages(sent),
            WEBSOCKET_LISTENER,
        )),
        Err(NotOpened::Refused(refusal)) => {
            tracing::warn!(target: REFUSAL_TARGET, listener = WEBSOCKET_LISTENER, reason = refusal.reason(), "a WebSocket handshake whose `Host` or `Origin` is not the Daemon's was refused");
            None
        }
        Err(NotOpened::Failed(error)) => {
            tracing::debug!(%error, "a request that is no WebSocket handshake closes");
            None
        }
    }
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
    access: Access,
) {
    let Some((reading, writing, listener)) = carriage(connection, &access).await else {
        return;
    };
    serve_carried(
        dispatch,
        Carried {
            reading,
            writing,
            listener,
        },
        owner,
        runs,
        events,
        access,
    )
    .await;
}

/// Serves the Console's WebSocket `socket`, a Connection of the Daemon
/// `daemon` reaches, until it ends, its tasks those of `owner`.
///
/// # Cancel safety
///
/// Dropping the future leaves the calls it started to `owner`, which stops
/// them.
pub(crate) async fn serve_console(
    dispatch: Arc<Dispatch>,
    socket: ConsoleSocket,
    owner: &TaskOwner,
    runs: Runs,
    events: Events,
    daemon: Arc<dyn DaemonSide>,
) {
    let carried = Carried {
        reading: Reading::Messages(socket.reader),
        writing: Writing::Messages(socket.writer),
        listener: CONSOLE_LISTENER,
    };
    serve_carried(
        dispatch,
        carried,
        owner,
        runs,
        events,
        Access::Console(daemon),
    )
    .await;
}

/// How a Connection's messages travel, and its listener's name.
struct Carried {
    reading: Reading,
    writing: Writing,
    listener: &'static str,
}

/// Serves the Connection `carried` gives until it ends.
async fn serve_carried(
    dispatch: Arc<Dispatch>,
    carried: Carried,
    owner: &TaskOwner,
    runs: Runs,
    events: Events,
    access: Access,
) {
    let Carried {
        mut reading,
        writing,
        listener,
    } = carried;
    let (outgoing, lines) = mpsc::channel(OUTGOING_LINES);
    owner.spawn(write_lines(writing, lines));
    let (completed, completions) = mpsc::channel(MAX_IN_FLIGHT);
    let (client, requests) = Client::new();
    let daemon = access.daemon();
    let log = dispatch.log_events.connection();
    let logging = CancellationToken::new();
    let forwarding = log.forward(outgoing.clone());
    let stop_logging = logging.clone();
    owner.spawn(async move {
        // Polled first, it sends every log event already waiting before
        // the Connection's end stops it.
        tokio::select! {
            biased;
            () = forwarding => {}
            () = stop_logging.cancelled() => {}
        }
    });
    let connection = Arc::new(ConnectionContext {
        identity: Arc::clone(&dispatch.identity),
        client,
        surface: OnceLock::new(),
        runs,
        events,
        daemon,
        log,
    });
    let mut session = Session {
        dispatch,
        owner,
        outgoing,
        completed,
        completions,
        in_flight: BTreeMap::new(),
        behind_gateway: false,
        state: ConnectionState::Opening,
        connection,
        requests,
        pending: BTreeMap::new(),
        next_request: 0,
        opened_client: None,
        access,
        listener,
    };
    session.run(&mut reading).await;
    logging.cancel();
    session.cancel_in_flight();
    drop(session);
    owner.wait().await;
}

/// Writes each line whole, or as one WebSocket message, flushing once no
/// other line waits; shuts the Connection's sending side down, or closes
/// its WebSocket, when every sender is gone.
async fn write_lines(mut writing: Writing, mut lines: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = lines.recv().await {
        let sent = match &mut writing {
            Writing::Lines(writer) => writer.write_all(&line).await.is_ok(),
            Writing::Messages(messages) => messages.feed(line).await,
        };
        if !sent {
            return;
        }
        if lines.is_empty() {
            let flushed = match &mut writing {
                Writing::Lines(writer) => writer.flush().await.is_ok(),
                Writing::Messages(messages) => messages.flush().await,
            };
            if !flushed {
                return;
            }
        }
    }
    match writing {
        Writing::Lines(mut writer) => {
            let _ = writer.shutdown().await;
        }
        Writing::Messages(messages) => messages.close().await,
    }
}

/// A call in flight.
struct InFlight {
    token: CancellationToken,
    cancellable: bool,
    credits: Option<Credits>,
    /// The notification it takes as its input, and where each goes.
    input: Option<(&'static str, mpsc::UnboundedSender<Value>)>,
}

/// A message read from the Connection, by the event of the engine's table
/// it is.
enum Message {
    Hello(Request),
    Request(Request),
    Notification(Notification),
    Reply {
        id: RequestId,
        answer: Result<Value, ErrorObject>,
    },
    Unreadable(DecodeError),
}

impl Message {
    fn of(incoming: Result<Incoming, DecodeError>) -> Self {
        match incoming {
            Ok(Incoming::Request(request)) if request.method == HELLO => Self::Hello(request),
            Ok(Incoming::Request(request)) => Self::Request(request),
            Ok(Incoming::Notification(notification)) => Self::Notification(notification),
            Ok(Incoming::Reply { id, answer }) => Self::Reply { id, answer },
            Err(error) => Self::Unreadable(error),
        }
    }

    fn event(&self) -> ConnectionEvent {
        match self {
            Self::Hello(_) => ConnectionEvent::Hello,
            Self::Request(_) => ConnectionEvent::Request,
            Self::Notification(_) => ConnectionEvent::Notification,
            Self::Reply { .. } => ConnectionEvent::Reply,
            Self::Unreadable(_) => ConnectionEvent::Unreadable,
        }
    }
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
    /// Where it is, which only the engine's table changes (ADR 0128).
    state: ConnectionState,
    connection: Arc<ConnectionContext>,
    requests: mpsc::Receiver<Outgoing>,
    pending: BTreeMap<RequestId, Pending>,
    next_request: i64,
    /// On a Daemon, the count of this Client, held while it is open.
    opened_client: Option<Box<dyn Send + Sync>>,
    /// How the Connection reached the engine.
    access: Access,
    /// Its listener's name in the endpoint table (ADR 0067).
    listener: &'static str,
}

/// A handler request the engine sent, waiting for its reply.
struct Pending {
    method: &'static str,
    reply: oneshot::Sender<Result<Value, ErrorObject>>,
}

impl Session<'_> {
    async fn run(&mut self, reading: &mut Reading) {
        let mut pending = Vec::new();
        let mut ended = false;
        let mut lost = false;
        loop {
            for frame in std::mem::take(&mut pending) {
                if let Flow::Close = self.handle(frame).await {
                    return;
                }
            }
            if ended {
                if !lost {
                    lost = true;
                    match self.transition(ConnectionEvent::ClientLost) {
                        Ok(ConnectionEffect::CancelCalls) => self.cancel_in_flight(),
                        Ok(
                            ConnectionEffect::Handshake
                            | ConnectionEffect::Serve
                            | ConnectionEffect::Answer
                            | ConnectionEffect::Refuse
                            | ConnectionEffect::Nothing,
                        ) => {}
                        Err(refused) => {
                            let _ = refused.logged();
                            return;
                        }
                    }
                }
                if self.in_flight.is_empty() {
                    if let Err(refused) = self.transition(ConnectionEvent::CallsEnded) {
                        let _ = refused.logged();
                    }
                    return;
                }
            }
            let open = !ended && self.in_flight.len() < MAX_IN_FLIGHT;
            let sending = self.state != ConnectionState::Opening;
            tokio::select! {
                biased;
                Some(id) = self.completions.recv() => {
                    self.in_flight.remove(&id);
                }
                Some(request) = self.requests.recv(), if sending => {
                    if let Flow::Close = self.send_request(request).await {
                        return;
                    }
                }
                more = reading.read(&mut pending), if open => {
                    ended = !more;
                }
            }
        }
    }

    /// Applies `event` to the Connection's state, as the engine's table
    /// says: its effect, or why it is refused.
    fn transition(
        &mut self,
        event: ConnectionEvent,
    ) -> Result<ConnectionEffect, InvalidConnectionTransition> {
        let (next, effect) = transition::engine::connection(self.state, event)?;
        self.state = next;
        Ok(effect)
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
        let message = Message::of(incoming);
        match self.transition(message.event()) {
            Ok(
                ConnectionEffect::Handshake
                | ConnectionEffect::Serve
                | ConnectionEffect::Answer
                | ConnectionEffect::CancelCalls
                | ConnectionEffect::Nothing,
            ) => self.serve(message).await,
            Ok(ConnectionEffect::Refuse) => self.refuse(message).await,
            Err(refused) => self.refuse_transition(refused, message).await,
        }
    }

    /// Serves a message the Connection's state lets through: checks its
    /// `hello`, calls a request's handler, routes a reply, acts on a
    /// notification, or answers a line it cannot read.
    async fn serve(&mut self, message: Message) -> Flow {
        match message {
            Message::Hello(request) => self.open(request).await,
            Message::Request(request) => self.request(request).await,
            Message::Notification(notification) => {
                self.notification(&notification);
                Flow::Continue
            }
            Message::Reply { id, answer } => {
                self.reply(&id, answer);
                Flow::Continue
            }
            Message::Unreadable(error) => {
                let failure = CallError::protocol(error.failure, error.detail);
                if self.send(failure_line(error.id, &failure)).await {
                    Flow::Continue
                } else {
                    Flow::Close
                }
            }
        }
    }

    /// Refuses a first message that is no `hello`, then closes: unanswered
    /// on a listener with a token, answered with its error otherwise.
    async fn refuse(&mut self, message: Message) -> Flow {
        if self.access.guarded() {
            self.refused("hello_not_first");
            return Flow::Close;
        }
        match message {
            Message::Hello(request) | Message::Request(request) => {
                let error = CallError::protocol(
                    ProtocolFailure::InvalidRequest,
                    "a Connection opens with `hello`",
                );
                record(self.named(&request.method), Some(error.code()));
                self.send(failure_line(Some(request.id), &error)).await;
            }
            Message::Unreadable(error) => {
                let failure = CallError::protocol(error.failure, error.detail);
                self.send(failure_line(error.id, &failure)).await;
            }
            Message::Notification(_) => {
                tracing::debug!("a notification before `hello` closes the Connection");
            }
            Message::Reply { id, .. } => {
                tracing::debug!(?id, "a Connection that does not open with `hello` closes");
            }
        }
        Flow::Close
    }

    /// Answers a message the engine's table refuses in the Connection's
    /// state: a program's second `hello` with `invalid_request`, the
    /// Connection going on; one only a bug causes, logged, by closing.
    async fn refuse_transition(
        &mut self,
        refused: InvalidConnectionTransition,
        message: Message,
    ) -> Flow {
        let refused = refused.logged();
        let request = match (refused.refusal, message) {
            (Refusal::Program(_), Message::Hello(request)) => request,
            (Refusal::Program(_) | Refusal::Internal, _) => return Flow::Close,
        };
        let error = CallError::protocol(
            ProtocolFailure::InvalidRequest,
            "a Connection opens with `hello` once",
        );
        record(self.named(&request.method), Some(error.code()));
        if self.send(failure_line(Some(request.id), &error)).await {
            Flow::Continue
        } else {
            Flow::Close
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

    /// The Connection's `hello`, which carries the token on a Daemon and
    /// on a private endpoint.
    async fn open(&mut self, request: Request) -> Flow {
        let daemon = self.connection.daemon.clone();
        let hello = serde_json::from_value::<HelloParams>(request.params.unwrap_or(Value::Null));
        if let Some(expected) = self.access.token() {
            let token = hello.as_ref().ok().and_then(|hello| hello.token.as_deref());
            match token {
                None => {
                    self.refused("token_missing");
                    return self.rejected();
                }
                Some(token) if !same_token(token, expected) => {
                    self.refused("token_wrong");
                    return self.rejected();
                }
                Some(_) => {}
            }
        }
        let answered = match hello {
            Ok(hello) => handshake::answer(
                &self.dispatch.identity,
                self.connection.events.instance(),
                &hello,
            )
            .map(|accepted| (accepted, hello.surface, hello.log_level)),
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
            Ok((mut accepted, surface, log_level)) => {
                record(HELLO, None);
                self.connection.log.set(log_level.unwrap_or_default());
                if let Err(refused) = self.transition(ConnectionEvent::Accepted) {
                    let _ = refused.logged();
                    return Flow::Close;
                }
                // A Connection answers one `hello`: the first sets it.
                let _ = self.connection.surface.set(surface);
                self.opened_client = daemon.map(|daemon| daemon.opened(surface));
                self.behind_gateway = accepted.behind_gateway;
                if let Access::Attached(private_endpoint) = &self.access {
                    accepted
                        .result
                        .private_endpoint
                        .clone_from(private_endpoint);
                }
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
                self.rejected()
            }
        }
    }

    /// Its `hello` was refused: it closes.
    fn rejected(&mut self) -> Flow {
        if let Err(refused) = self.transition(ConnectionEvent::Rejected) {
            let _ = refused.logged();
        }
        Flow::Close
    }

    async fn request(&mut self, request: Request) -> Flow {
        let Request {
            id, method, params, ..
        } = request;
        let refusal = if method == CANCEL {
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
            if handler.operation().reach == Reach::PersonOnly
                && !matches!(
                    self.connection.surface.get(),
                    Some(Surface::Cli | Surface::Console)
                )
            {
                Some(person_required(&method))
            } else {
                let handler = handler.clone();
                self.call(id, &handler, params);
                return Flow::Continue;
            }
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
        record(self.named(&method), Some(error.code()));
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
        let (input, call_input) = match handler.input() {
            Some(method) => {
                let (sender, incoming) = mpsc::unbounded_channel();
                (
                    Some((method, sender)),
                    Some(CallInput::new(
                        id.clone(),
                        incoming,
                        self.outgoing.clone(),
                        notification_line,
                    )),
                )
            }
            None => (None, None),
        };
        let context = CallContext::new(
            Arc::clone(&self.connection),
            token.clone(),
            stream,
            call_input,
        );
        let answer = handler.call(params.unwrap_or(Value::Object(Map::new())), context);
        self.in_flight.insert(
            id.clone(),
            InFlight {
                token: token.clone(),
                cancellable: operation.cancellable,
                credits,
                input,
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
            self.input(notification);
            return;
        }
        let params = notification.params.clone().unwrap_or(Value::Null);
        let Ok(CancelParams { id }) = serde_json::from_value::<CancelParams>(params) else {
            tracing::debug!("a `$/cancel` without a call's id is ignored");
            return;
        };
        if let Some(call) = self.in_flight.get(&id)
            && call.cancellable
        {
            record_cancel(CANCEL);
            call.token.cancel();
        }
    }

    /// Hands an input notification to the call in flight it names, which
    /// takes it; any other is ignored.
    fn input(&self, notification: &Notification) {
        #[derive(serde::Deserialize)]
        struct Named {
            id: RequestId,
        }
        let params = notification.params.clone().unwrap_or(Value::Null);
        let taker = serde_json::from_value::<Named>(params.clone())
            .ok()
            .and_then(|Named { id }| self.in_flight.get(&id))
            .and_then(|call| call.input.as_ref())
            .filter(|(method, _)| *method == notification.method);
        if let Some((_, sender)) = taker {
            let _ = sender.send(params);
        } else {
            tracing::debug!("an unknown notification is ignored");
        }
    }

    /// Adds a `$/credit`'s credits to the call it names; one for a call no
    /// longer in flight, or for one that does not stream, is ignored.
    fn credit(&self, notification: &Notification) {
        let params = notification.params.clone().unwrap_or(Value::Null);
        let Ok(CreditParams { id, credits }) = serde_json::from_value::<CreditParams>(params)
        else {
            tracing::debug!("a `$/credit` that is not one is ignored");
            return;
        };
        if let Some(call_credits) = self
            .in_flight
            .get(&id)
            .and_then(|call| call.credits.as_ref())
        {
            call_credits.grant(credits);
        }
    }

    /// Sends a line; `false` once the writer is gone.
    async fn send(&self, line: Vec<u8>) -> bool {
        self.outgoing.send(line).await.is_ok()
    }
}

impl Session<'_> {
    /// `method` as a record names it: a name of the catalog, `hello` or
    /// `$/cancel`, or [`UNKNOWN_METHOD`] for any other, which a Client
    /// wrote and no log keeps (ADR 0132's bounded values).
    fn named<'method>(&self, method: &'method str) -> &'method str {
        let known = method == HELLO
            || method == CANCEL
            || self.dispatch.handlers.contains_key(method)
            || foxhound_names::OPERATIONS.contains(&method);
        if known { method } else { UNKNOWN_METHOD }
    }

    /// Logs the listener's refusal of the Connection, with its reason.
    fn refused(&self, reason: &'static str) {
        let listener = self.listener;
        tracing::warn!(target: REFUSAL_TARGET, listener, reason, "a Connection that did not open with `hello` and its token was closed");
    }
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

/// `person_required`: `method` is a person's answer, which only the Console
/// and the CLI's interactive prompt ask for (ADRs 0011 and 0041).
fn person_required(method: &str) -> CallError {
    CallError::new(
        &PERSON_REQUIRED,
        format!(
            "`{method}` is a person's answer: give it in the Console, or in the CLI's interactive prompt"
        ),
        &serde_json::Map::new(),
    )
}
