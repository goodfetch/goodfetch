//! One Connection's life, whose states change only as the engine's
//! transition table says (ADR 0128): it opens with `hello`, then each request goes to
//! its handler as a task of the Connection's owner, each call under its own
//! cancellation token that `$/cancel` trips, while one writer task sends
//! every line whole. A refused `hello` is answered before any other message
//! and the Connection then closes (ADR 0066). At the end of input, calls in
//! flight are cancelled at their next cancellation point, answered, and the
//! Connection closes once they all are. A Connection the engine closes
//! while its Client still sends, such as a refused one, ends its sending
//! side first and drops what still comes until the Client closes too, for a
//! short while at most, so a Client that wrote behind its `hello` reads the
//! refusal and the end rather than a failed write or a reset.
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
//! listener and its reason, and recorded in the Audit log (`endpoint`). An
//! Exposed Daemon's address carries its Connections as lines or over
//! WebSocket (ADR 0049), whose refused handshakes are logged the same way.
//! Only the Client of an Attached engine's stdio is told its private
//! endpoint, in the reply to its `hello`. The Console's WebSocket, whose
//! sign-in its handshake proved, opens with a `hello` that carries no token
//! (ADR 0031).
//!
//! Size: one Connection's life, the transition table's states and every
//! carriage that reaches them, kept whole so each message passes one loop.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::catalog::{Product, Reach};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CANCEL, CallCancelledDetails, CancelParams, HELLO, HelloParams,
    PERSON_REQUIRED, PrivateEndpoint, StoppedBy, Surface,
};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::foxhound_names;
use goodfetch_protocol::jsonrpc::{
    ErrorObject, Notification, ProtocolFailure, Request, RequestId, TraceContext, Version,
};
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::event_stream::{EventStream, PostedReader, StreamedWriter};
use super::framing::{DecodeError, Frame, Framer, Incoming, MAX_MESSAGE_BYTES, decode};
use super::websocket::{
    self, ConsoleSocket, MessageReader, MessageWriter, NotOpened, Prefixed, Rejoined,
};
use super::{
    failure_line, handshake, line, not_in_product, notification_line, notification_value_line,
    record, record_cancel, success_line, trace_context,
};
use crate::call_error::CallError;
use crate::client::{Client, Outgoing};
use crate::clock::Clock;
use crate::composition::{
    CallContext, CallInput, CallNotifier, CallStream, ConnectionContext, DaemonSide, Driver,
    Handler, InputSender,
};
use crate::endpoint::{Authorities, REFUSAL_TARGET, same_token};
use crate::engine_context::EngineContext;
use crate::events::{Credits, Events};
use crate::log::{ConnectionLog, LogEvents};
use crate::mask::{Masks, Scope};
use crate::people::SignInDriver;
use crate::product::{Identity, Requested};
use crate::runs::Runs;
use crate::tasks::TaskOwner;
use crate::trace;
use crate::transition::engine::{
    ConnectionEffect, ConnectionEvent, ConnectionState, InvalidConnectionTransition,
};
use crate::transition::{self, Refusal};
use crate::transport::{Connection, ConnectionWriter};
use goodfetch_protocol::run::RunKind;

/// How many bytes one read takes from the Connection.
const READ_CHUNK: usize = 8 * 1024;

/// How many calls one Connection keeps in flight before it stops reading.
const MAX_IN_FLIGHT: usize = 256;

/// How many lines wait for the writer before their senders wait.
const OUTGOING_LINES: usize = 64;

/// How long a Connection the engine closed still reads what its Client
/// sends, waiting for the Client to close too.
const LINGER: Duration = Duration::from_secs(2);

/// What a dispatch record names a method no catalog holds.
const UNKNOWN_METHOD: &str = "(unknown)";

/// What every Connection of an engine dispatches to.
pub(crate) struct Dispatch {
    pub(crate) identity: Arc<Identity>,
    pub(crate) handlers: BTreeMap<&'static str, Handler>,
    /// The driver of each kind of Run that has one, which a resume starts
    /// again.
    pub(crate) drivers: Arc<BTreeMap<RunKind, Driver>>,
    /// The process's log events, which each Connection receives at its
    /// own level; `None` when the engine was given none, so no layer feeds
    /// them and no buffer is kept for them.
    pub(crate) log_events: Option<LogEvents>,
    /// Every live Run's resolved values, masked in every message the
    /// Connection sends (spec #564).
    pub(crate) masks: Masks,
    /// The engine's Counted work, what its Connections' messages add to
    /// (ADR 0140's E1).
    pub(crate) work: CountedWork,
}

/// How a Connection reached the engine, which says what its `hello` must
/// carry and what its reply tells.
pub(crate) enum Access {
    /// An Attached engine's stdio, or an in-process Connection: no token;
    /// its reply names the engine's private endpoint, when it has one, and
    /// the sign-in link of the Console it serves for its script, when it
    /// serves one.
    Attached(Option<PrivateEndpoint>, Option<String>),
    /// A Daemon's listener: its token, and what else the Daemon gives.
    Daemon(Arc<dyn DaemonSide>),
    /// An Exposed Daemon's address (ADR 0049): a Daemon's listener that
    /// also carries the protocol over WebSocket, its handshake held to the
    /// names the Daemon is reached at.
    Exposed(Arc<dyn DaemonSide>, Arc<Authorities>),
    /// An Attached engine's private endpoint: its token.
    PrivateEndpoint(Arc<str>),
    /// The Console's WebSocket on its own origin (ADR 0031): a Connection
    /// whose sign-in cookie its handshake carried, so its `hello` carries
    /// no token; the Daemon's, or none on an Attached engine serving its
    /// script's own Console, and the sign-in driver its refusals are
    /// recorded through.
    Console(ConsoleSide),
}

/// What a Console's Connection reaches beside the engine: the Daemon that
/// serves the Console, or none on an Attached engine serving its script's
/// own, and the Console's sign-in driver, which records its refusals.
pub(crate) struct ConsoleSide {
    pub(crate) daemon: Option<Arc<dyn DaemonSide>>,
    pub(crate) sign_ins: SignInDriver,
}

impl Access {
    /// The token its `hello` must carry, if any.
    fn token(&self) -> Option<&str> {
        match self {
            Self::Attached(..) | Self::Console(..) => None,
            Self::Daemon(daemon) | Self::Exposed(daemon, _) => Some(daemon.token()),
            Self::PrivateEndpoint(token) => Some(token),
        }
    }

    /// The Daemon it reaches, if any.
    fn daemon(&self) -> Option<Arc<dyn DaemonSide>> {
        match self {
            Self::Daemon(daemon) | Self::Exposed(daemon, _) => Some(Arc::clone(daemon)),
            Self::Console(side) => side.daemon.clone(),
            Self::Attached(..) | Self::PrivateEndpoint(_) => None,
        }
    }

    /// Its listener in the endpoint table (ADR 0067), for a Connection of
    /// newline-delimited JSON-RPC.
    fn listener(&self) -> Listener {
        match self {
            Self::Attached(..) => Listener::Stdio,
            Self::Daemon(_) => Listener::DaemonSocket,
            Self::Exposed(..) => Listener::ExposedDaemon,
            Self::PrivateEndpoint(_) => Listener::PrivateEndpoint,
            Self::Console(..) => Listener::Console,
        }
    }

    /// Whether a stranger learns nothing from it: a first message that is
    /// no `hello` is closed unanswered on a listener a Client proves
    /// itself to.
    fn guarded(&self) -> bool {
        self.token().is_some() || matches!(self, Self::Console(..))
    }
}

/// How a Connection's messages arrive: as lines, as WebSocket messages, or
/// in the Console's `POST` bodies.
enum Reading {
    Lines {
        reader: Prefixed,
        framer: Framer,
        chunk: Vec<u8>,
    },
    Messages(MessageReader),
    Posted(PostedReader),
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
            Self::Posted(posted) => posted.read(frames).await,
        }
    }
}

/// How a Connection's messages leave: as lines, as WebSocket messages, or
/// as the events of the Console's event stream.
enum Writing {
    Lines(ConnectionWriter),
    Messages(MessageWriter),
    Streamed(StreamedWriter),
}

/// How `connection`, reached through `access`, carries its messages, and
/// its listener; `None` when an Exposed Daemon's WebSocket handshake
/// was refused or failed, which closes it.
async fn carriage(
    connection: Connection,
    access: &Access,
    engine: &EngineContext,
) -> Option<(Reading, Writing, Listener)> {
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
            Listener::DaemonWebsocket,
        )),
        Err(NotOpened::Refused(refusal)) => {
            engine
                .refusals()
                .refused(Listener::DaemonWebsocket, refusal.reason())
                .await;
            None
        }
        Err(NotOpened::Failed(error)) => {
            tracing::debug!(%error, "a request that is no WebSocket handshake closes");
            None
        }
    }
}

/// Serves `connection` until it ends, its tasks those of `owner`, its calls
/// given `engine`, lingering on its clock after the engine closes it.
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
    engine: EngineContext,
) {
    let Some((reading, writing, listener)) = carriage(connection, &access, &engine).await else {
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
        engine,
    )
    .await;
}

/// A Connection the Console's server opened on its own origin, its sign-in
/// already checked, which the Daemon serves (ADR 0031): a WebSocket, or,
/// where a proxy refuses `WebSocket`s, an event stream and the page's
/// `POST`s.
pub(crate) enum ConsoleConnection {
    WebSocket(ConsoleSocket),
    EventStream(EventStream),
}

/// Serves the Console's Connection `console`, a Connection of the Daemon
/// `daemon` reaches, until it ends, its tasks those of `owner`, its calls
/// given `engine`, lingering on its clock after the engine closes it.
///
/// # Cancel safety
///
/// Dropping the future leaves the calls it started to `owner`, which stops
/// them.
pub(crate) async fn serve_console(
    dispatch: Arc<Dispatch>,
    console: ConsoleConnection,
    owner: &TaskOwner,
    runs: Runs,
    events: Events,
    side: ConsoleSide,
    engine: EngineContext,
) {
    let (reading, writing) = match console {
        ConsoleConnection::WebSocket(socket) => (
            Reading::Messages(socket.reader),
            Writing::Messages(socket.writer),
        ),
        ConsoleConnection::EventStream(stream) => (
            Reading::Posted(stream.reader),
            Writing::Streamed(stream.writer),
        ),
    };
    let carried = Carried {
        reading,
        writing,
        listener: Listener::Console,
    };
    serve_carried(
        dispatch,
        carried,
        owner,
        runs,
        events,
        Access::Console(side),
        engine,
    )
    .await;
}

/// How a Connection's messages travel, and its listener.
struct Carried {
    reading: Reading,
    writing: Writing,
    listener: Listener,
}

/// Serves the Connection `carried` gives until it ends, its calls given
/// `engine`, lingering on its clock after the engine closes it.
async fn serve_carried(
    dispatch: Arc<Dispatch>,
    carried: Carried,
    owner: &TaskOwner,
    runs: Runs,
    events: Events,
    access: Access,
    engine: EngineContext,
) {
    let clock = engine.clock();
    let Carried {
        mut reading,
        writing,
        listener,
    } = carried;
    let (outgoing, lines) = mpsc::channel(OUTGOING_LINES);
    owner.spawn(write_lines(
        writing,
        lines,
        dispatch.masks.clone(),
        dispatch.work.clone(),
    ));
    let (completed, completions) = mpsc::channel(MAX_IN_FLIGHT);
    let (client, requests) = Client::new(dispatch.work.clone());
    let daemon = access.daemon();
    let log = dispatch
        .log_events
        .as_ref()
        .map_or_else(ConnectionLog::none, LogEvents::connection);
    let logging = CancellationToken::new();
    let forwarding = log.forward(outgoing.clone(), dispatch.work.clone());
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
        behind_gateway: OnceLock::new(),
        drivers: Arc::clone(&dispatch.drivers),
        runs,
        events,
        engine,
        daemon,
        log,
        work: dispatch.work.clone(),
    });
    let mut session = Session {
        dispatch,
        owner,
        outgoing,
        completed,
        completions,
        in_flight: BTreeMap::new(),
        state: ConnectionState::Opening,
        connection,
        requests,
        pending: BTreeMap::new(),
        next_request: 0,
        opened_client: None,
        access,
        listener,
    };
    let ended = session.run(&mut reading).await;
    logging.cancel();
    session.cancel_in_flight();
    drop(session);
    owner.wait().await;
    if !ended {
        linger(&mut reading, clock).await;
    }
}

/// Once the engine closed a Connection whose Client still sends, such as
/// one whose `hello` it refused, reads and drops what comes until the
/// Client closes too, for at most [`LINGER`]. A Client that wrote behind
/// its last message then reads the engine's last message and the end, and
/// never meets a Connection already gone: its write never fails, and no
/// reset of unread bytes destroys what the engine sent (ADR 0066).
///
/// # Cancel safety
///
/// Dropping the future closes the Connection at once.
async fn linger(reading: &mut Reading, clock: Clock) {
    let drained = async {
        let mut dropped = Vec::new();
        while reading.read(&mut dropped).await {
            dropped.clear();
        }
    };
    tokio::select! {
        () = drained => {}
        () = clock.sleep(LINGER) => {}
    }
}

/// Writes each line whole, or as one WebSocket message, flushing once no
/// other line waits; shuts the Connection's sending side down, or closes
/// its WebSocket, when every sender is gone. Every line passes through
/// `masks` first, as what a call returns or an event carries: no message
/// the Connection sends holds a value a Run resolved (spec #564). Every line
/// counts once on `work`, as the one encode into bytes each was made by,
/// whoever made it (ADR 0140's E1).
async fn write_lines(
    mut writing: Writing,
    mut lines: mpsc::Receiver<Vec<u8>>,
    masks: Masks,
    work: CountedWork,
) {
    while let Some(line) = lines.recv().await {
        work.count(Row::MessageEncodes);
        let line = masks.apply_json_line(line, Scope::Returned);
        let sent = match &mut writing {
            Writing::Lines(writer) => writer.write_all(&line).await.is_ok(),
            Writing::Messages(messages) => messages.feed(line).await,
            Writing::Streamed(streamed) => streamed.feed(line).await,
        };
        if !sent {
            return;
        }
        if lines.is_empty() {
            let flushed = match &mut writing {
                Writing::Lines(writer) => writer.flush().await.is_ok(),
                Writing::Messages(messages) => messages.flush().await,
                Writing::Streamed(_) => true,
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
        Writing::Streamed(streamed) => streamed.close(),
    }
}

/// A call in flight.
struct InFlight {
    token: CancellationToken,
    cancellable: bool,
    credits: Option<Credits>,
    /// The notification it takes as its input, and where each goes.
    input: Option<(&'static str, InputSender)>,
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
    /// Its listener in the endpoint table (ADR 0067).
    listener: Listener,
}

/// A handler request the engine sent, waiting for its reply.
struct Pending {
    method: &'static str,
    reply: oneshot::Sender<Result<Value, ErrorObject>>,
}

impl Session<'_> {
    /// Serves the Connection until the Client's input ended and every call
    /// was answered, or the engine closed it; answers whether the input
    /// ended.
    async fn run(&mut self, reading: &mut Reading) -> bool {
        let mut pending = Vec::new();
        let mut ended = false;
        let mut lost = false;
        loop {
            for frame in std::mem::take(&mut pending) {
                if let Flow::Close = self.handle(frame).await {
                    return ended;
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
                            return true;
                        }
                    }
                }
                if self.in_flight.is_empty() {
                    if let Err(refused) = self.transition(ConnectionEvent::CallsEnded) {
                        let _ = refused.logged();
                    }
                    return true;
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
                        return ended;
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
            Frame::Line(line) => decode(&line, &self.dispatch.work),
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
            self.refused(RefusalReason::HelloNotFirst).await;
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
                self.dispatch.work.count(Row::MessageEncodes);
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
        let (hello, meta) = match trace_context(request.params) {
            Ok((params, meta)) => (
                serde_json::from_value::<HelloParams>(params.unwrap_or(Value::Null)).map_err(
                    |error| CallError::protocol(ProtocolFailure::InvalidParams, error.to_string()),
                ),
                meta,
            ),
            Err(error) => (Err(error), None),
        };
        if let Some(expected) = self.access.token() {
            let token = hello.as_ref().ok().and_then(|hello| hello.token.as_deref());
            match token {
                None => {
                    self.refused(RefusalReason::TokenMissing).await;
                    return self.rejected();
                }
                Some(token) if !same_token(token, expected) => {
                    self.refused(RefusalReason::TokenWrong).await;
                    return self.rejected();
                }
                Some(_) => {}
            }
        }
        // Opened once the token is checked: a stranger's `hello` has no
        // span, and its Trace context is not read.
        let span = trace::server_span(HELLO, &request.id, None, meta.as_ref());
        let answered = match hello {
            Ok(hello) => handshake::answer(
                &self.dispatch.identity,
                self.connection.events.instance(),
                &hello,
            )
            .map(|accepted| {
                let surface = accepted.surface;
                (accepted, surface, hello.log_level)
            }),
            Err(error) => Err(handshake::Refused::Invalid(error)),
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
                // Each CLI command is a Connection of its own, and the Runs
                // it starts on the Daemon outlive it (ADRs 0024 and 0088);
                // so do those a person starts or resumes from the Kennel
                // (ADR 0141).
                if surface.runs_outlive_connection() {
                    self.connection.runs.outlive_connection().await;
                }
                let holding = accepted.holding;
                self.opened_client = daemon.map(|daemon| daemon.opened(surface, holding));
                let _ = self.connection.behind_gateway.set(accepted.behind_gateway);
                if let Access::Attached(private_endpoint, console_link) = &self.access {
                    accepted
                        .result
                        .private_endpoint
                        .clone_from(private_endpoint);
                    accepted.result.console_link.clone_from(console_link);
                }
                self.dispatch.work.count(Row::MessageEncodes);
                let result = serde_json::to_value(accepted.result)
                    .expect("invariant: a `hello` reply encodes");
                drop(span);
                if self.send(success_line(request.id, result)).await {
                    Flow::Continue
                } else {
                    Flow::Close
                }
            }
            Err(error) => {
                record(HELLO, Some(error.code()));
                trace::record_failure(&span, error.code(), error.wire_code());
                drop(span);
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
        } else if self.still_in_flight(&id) {
            Some(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                format!("a call with the id {id:?} is already in flight"),
            ))
        } else if let Some(handler) = self.dispatch.handlers.get(method.as_str()) {
            if handler.operation().reach == Reach::PersonOnly
                && !self
                    .connection
                    .surface
                    .get()
                    .copied()
                    .is_some_and(Surface::is_a_person)
            {
                Some(person_required(&method))
            } else {
                let handler = handler.clone();
                match trace_context(params) {
                    Ok((params, meta)) => {
                        self.call(id, &handler, params, meta.as_ref());
                        return Flow::Continue;
                    }
                    Err(error) => Some(error),
                }
            }
        } else if !self.dispatch.identity.product.contains(Product::Foxhound)
            && foxhound_names::OPERATIONS.contains(&method.as_str())
        {
            Some(not_in_product(
                &self.dispatch.identity,
                Requested::Name(&method),
                Product::Foxhound,
                self.connection
                    .behind_gateway
                    .get()
                    .copied()
                    .unwrap_or(false),
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

    /// Starts `handler` on `params` as a task of the Connection, in the
    /// request's span, the child of the span the Trace context `meta` names.
    fn call(
        &mut self,
        id: RequestId,
        handler: &Handler,
        params: Option<Value>,
        meta: Option<&TraceContext>,
    ) {
        let token = self.owner.token().child_token();
        let operation = handler.operation();
        let span = trace::server_span(operation.name, &id, params.as_ref(), meta);
        let credits = handler.streams().then(Credits::new);
        let notifier = CallNotifier::new(
            id.clone(),
            self.outgoing.clone(),
            notification_value_line,
            self.dispatch.work.clone(),
        );
        let stream = credits
            .clone()
            .map(|credits| CallStream::new(notifier.clone(), credits));
        let (input, call_input) = match handler.input() {
            Some((method, room)) => {
                let (sender, call_input) = CallInput::new(notifier, room);
                (Some((method, sender)), Some(call_input))
            }
            None => (None, None),
        };
        let context = CallContext::new(
            Arc::clone(&self.connection),
            id.clone(),
            token.clone(),
            stream,
            call_input,
        );
        let answer = handler
            .call(params.unwrap_or(Value::Object(Map::new())), context)
            .instrument(span.clone());
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
                        &CallCancelledDetails {
                            by: StoppedBy::Client,
                            closed: None,
                        },
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
                    trace::record_failure(&span, error.code(), error.wire_code());
                    failure_line(Some(id.clone()), &error)
                }
            };
            drop(span);
            // Ended before its answer is sent: the Client may reuse the id
            // as soon as the answer arrives, and `still_in_flight` then
            // finds this end waiting.
            let _ = completed.send(id).await;
            let _ = outgoing.send(line).await;
        });
    }

    /// Whether the call `id` names is still in flight, once every call that
    /// ended was taken out: a call ends before its answer is sent, so the
    /// id of a call whose answer the Client read is free again (JSON-RPC
    /// 2.0), however the session's loop was scheduled.
    fn still_in_flight(&mut self, id: &RequestId) -> bool {
        while let Ok(ended) = self.completions.try_recv() {
            self.in_flight.remove(&ended);
        }
        self.in_flight.contains_key(id)
    }

    fn notification(&mut self, notification: &Notification) {
        if notification.method == CREDIT {
            self.credit(notification);
            return;
        }
        if notification.method != CANCEL {
            self.input(notification);
            return;
        }
        let params = self.params_copied(notification);
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
    fn input(&mut self, notification: &Notification) {
        #[derive(serde::Deserialize)]
        struct Named {
            id: RequestId,
        }
        let params = self.params_copied(notification);
        self.dispatch.work.count(Row::MessageDeepCopies);
        let taker = serde_json::from_value::<Named>(params.clone())
            .ok()
            .and_then(|Named { id }| self.in_flight.get_mut(&id))
            .and_then(|call| call.input.as_mut())
            .filter(|(method, _)| *method == notification.method);
        if let Some((_, sender)) = taker {
            sender.hand_over(params);
        } else {
            tracing::debug!("an unknown notification is ignored");
        }
    }

    /// Adds a `$/credit`'s credits to the call it names; one for a call no
    /// longer in flight, or for one that does not stream, is ignored.
    fn credit(&self, notification: &Notification) {
        let params = self.params_copied(notification);
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

    /// A copy of `notification`'s params, counted (ADR 0140's E1).
    fn params_copied(&self, notification: &Notification) -> Value {
        self.dispatch.work.count(Row::MessageDeepCopies);
        notification.params.clone().unwrap_or(Value::Null)
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

    /// Logs the listener's refusal of the Connection, with its reason, and
    /// records it in the Audit log: on the Console's WebSocket through its
    /// sign-in driver, as the Console's.
    async fn refused(&self, reason: RefusalReason) {
        let listener = self.listener;
        if let Access::Console(ConsoleSide { sign_ins, .. }) = &self.access {
            tracing::warn!(target: REFUSAL_TARGET, listener = listener.as_str(), reason = reason.as_str(), "a Connection that did not open with `hello` was closed");
            sign_ins.refused(listener, reason.as_str()).await;
        } else {
            self.connection
                .engine
                .refusals()
                .refused(listener, reason)
                .await;
        }
    }
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
