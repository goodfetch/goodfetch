//! One Connection's life, whose states change only as the engine's
//! transition table says (ADR 0128): it opens with `hello`, then each request goes to
//! its handler as a task of the Connection's owner, each call under its own
//! cancellation token that `$/cancel` trips, while one writer task sends
//! every line whole. A refused `hello` is answered before any other message
//! and the Connection then closes (ADR 0066). At the end of input, calls in
//! flight are cancelled at their next cancellation point, answered, and the
//! Connection closes once they all are. A call whose handler panics is
//! answered `engine_panicked` and gives its place in flight back, as any
//! call that ended (ADR 0148). A Connection whose own task panicked ends
//! as when its Client goes away, and the panic counts as one that ended a
//! Connection: its writer's and its log forwarder's, tasks of its owner,
//! are told to the task that serves it, whose own panic is caught where it
//! runs, so none reaches the engine. A Connection reads its Client
//! whatever the number of calls in flight: at its cap only a new call is
//! refused, with `too_many_calls`, so the `$/cancel`, the `$/credit` or the
//! reply that frees a call is always read. A Connection the engine closes
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
//! listener and its reason, and recorded in the Audit log (`endpoint`). On
//! such a listener a Connection must also open within `timeout.hello`, its
//! WebSocket handshake included, and its first message must fit in
//! `network.max_hello_size`: one that does not is closed as soon as it
//! passes either bound, unanswered and logged the same way, so a Client
//! that never proves itself cannot hold a Connection or its buffer (#2324).
//! An Exposed Daemon's address carries its Connections as lines or over
//! WebSocket (ADR 0049), whose refused handshakes are logged the same way.
//! Only the Client of an Attached engine's stdio is told its private
//! endpoint, in the reply to its `hello`.
//!
//! Size: one Connection's life, the transition table's states and every
//! carriage that reaches them, kept whole so each message passes one loop.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::FutureExt as _;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::catalog::{Product, Reach, SettingScope};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CANCEL, CallCancelledDetails, CancelParams, Form, HELLO, HelloParams,
    NETWORK_MAX_HELLO_SIZE, PERSON_REQUIRED, PanicEnded, PrivateEndpoint, StoppedBy, Surface,
    TIMEOUT_HELLO, TOO_MANY_CALLS, TooManyCallsDetails,
};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::foxhound_names;
use goodfetch_protocol::jsonrpc::{ErrorObject, ProtocolFailure, RequestId};
use goodfetch_protocol::settings::RunOrigin;
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::framing::{
    Call, DecodeError, Frame, Framer, Incoming, MAX_MESSAGE_BYTES, Received, decode,
};
use super::websocket::{self, MessageReader, MessageWriter, NotOpened, Prefixed, Rejoined};
use super::{
    Line, failure_line, handshake, not_in_product, notification_line, record, record_cancel,
    success_line,
};
use crate::call_error::CallError;
use crate::client::{Client, Outgoing};
use crate::clock::Clock;
use crate::composition::params::Arrived;
use crate::composition::{
    Answered, CallChannels, CallContext, CallInput, CallNotifier, CallStream, ConnectionContext,
    DaemonSide, Driver, Handler, Handlers, InputSender,
};
use crate::crash_report;
use crate::endpoint::{Authorities, same_token};
use crate::engine_context::EngineContext;
use crate::events::{Credits, Events};
use crate::log::{ConnectionLog, LogEvents};
use crate::mask::{Masks, Scope};
use crate::metrics::Metrics;
use crate::product::{Identity, Requested};
use crate::run::translated;
use crate::runs::Runs;
use crate::settings::{Opening as SettingsOpening, SettingsError, check_scope, get_or_default};
use crate::tasks::{OwnerKind, TaskOwner};
use crate::trace;
use crate::transition::engine::{
    ConnectionEffect, ConnectionEvent, ConnectionState, InvalidConnectionTransition,
};
use crate::transition::{self, Refusal};
use crate::transport::{Connection, ConnectionWriter};
use goodfetch_protocol::run::RunKind;

/// How many bytes one read takes from the Connection.
const READ_CHUNK: usize = 8 * 1024;

/// How many calls one Connection keeps in flight: one more is refused with
/// `too_many_calls`, and every other message is still read (issue 2312).
const MAX_IN_FLIGHT: usize = 256;

/// How many lines wait for the writer before their senders wait.
const OUTGOING_LINES: usize = 64;

/// How long a Connection the engine closed still reads what its Client
/// sends, waiting for the Client to close too.
const LINGER: Duration = Duration::from_secs(2);

/// What a dispatch record names a method no catalog holds.
const UNKNOWN_METHOD: &str = "(unknown)";

/// What every Connection of an engine dispatches to.
#[derive(Clone)]
pub(crate) struct Dispatch {
    pub(crate) identity: Arc<Identity>,
    pub(crate) handlers: Handlers,
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
    /// The runtime's metrics, which count a call's panic (ADR 0148), and
    /// whose Counted work the Connections' messages add to.
    pub(crate) metrics: Metrics,
}

impl Dispatch {
    /// The engine's Counted work, what its Connections' messages add to
    /// (ADR 0140's E1).
    pub(crate) fn work(&self) -> &CountedWork {
        self.metrics.counted_work()
    }
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
}

impl Access {
    /// The token its `hello` must carry, if any.
    fn token(&self) -> Option<&str> {
        match self {
            Self::Attached(..) => None,
            Self::Daemon(daemon) | Self::Exposed(daemon, _) => Some(daemon.token()),
            Self::PrivateEndpoint(token) => Some(token),
        }
    }

    /// The Daemon it reaches, if any.
    fn daemon(&self) -> Option<Arc<dyn DaemonSide>> {
        match self {
            Self::Daemon(daemon) | Self::Exposed(daemon, _) => Some(Arc::clone(daemon)),
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
        }
    }

    /// Whether a stranger learns nothing from it: a first message that is
    /// no `hello` is closed unanswered on a listener a Client proves
    /// itself to.
    fn guarded(&self) -> bool {
        self.token().is_some()
    }
}

/// The bounds of a Connection's opening on a guarded listener: when it
/// must have said `hello`, and how long its first message may be (#2324).
#[derive(Clone, Copy)]
struct Opening {
    /// None when `timeout.hello` is too long for the clock to reach.
    deadline: Option<Instant>,
    max_size: usize,
}

impl Opening {
    /// The bounds of a Connection reached through `access` and starting
    /// now, as `engine`'s settings set them; none on a listener that takes
    /// no token, such as an Attached engine's stdio.
    async fn of(access: &Access, engine: &EngineContext) -> Option<Self> {
        if !access.guarded() {
            return None;
        }
        let machine = engine.settings().snapshot().await;
        let resolved = machine
            .resolved()
            .inspect_err(|error| {
                tracing::warn!(%error, "the machine's settings cannot be read, so a Connection opens under the default bounds");
            })
            .ok();
        let timeout: Duration = get_or_default(resolved, &TIMEOUT_HELLO);
        let max_size: u64 = get_or_default(resolved, &NETWORK_MAX_HELLO_SIZE);
        Some(Self {
            deadline: engine.clock().now().checked_add(timeout),
            max_size: usize::try_from(max_size)
                .map_or(MAX_MESSAGE_BYTES, |size| size.min(MAX_MESSAGE_BYTES)),
        })
    }
}

/// How a Connection's messages arrive: as lines or as WebSocket messages.
enum Reading {
    Lines {
        reader: Prefixed,
        framer: Framer,
        chunk: Vec<u8>,
    },
    Messages(MessageReader),
}

impl Reading {
    /// Lines from `reader`, its first no longer than `opening`'s bound.
    fn lines(reader: Prefixed, opening: Option<Opening>) -> Self {
        let framer = match opening {
            Some(opening) => Framer::opening(opening.max_size, MAX_MESSAGE_BYTES),
            None => Framer::new(MAX_MESSAGE_BYTES),
        };
        Self::Lines {
            reader,
            framer,
            chunk: vec![0; READ_CHUNK],
        }
    }

    /// Whether the line being read has passed its bound already, before
    /// its end: a WebSocket message is read whole, so never.
    fn skipping(&self) -> bool {
        match self {
            Self::Lines { framer, .. } => framer.skipping(),
            Self::Messages(_) => false,
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

/// How a Connection's messages leave: as lines or as WebSocket messages.
enum Writing {
    Lines(ConnectionWriter),
    Messages(MessageWriter),
}

/// How `connection`, reached through `access`, carries its messages, and
/// its listener; `None` when an Exposed Daemon's WebSocket handshake
/// was refused or failed, which closes it.
async fn carriage(
    connection: Connection,
    access: &Access,
    engine: &EngineContext,
    opening: Option<Opening>,
) -> Option<(Reading, Writing, Listener)> {
    let (mut reader, writer) = connection.split();
    let Access::Exposed(_, authorities) = access else {
        let reader = Prefixed::new(Vec::new(), reader);
        return Some((
            Reading::lines(reader, opening),
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
            Reading::lines(reader, opening),
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
    let metrics = dispatch.metrics.clone();
    let serving = async {
        let opening = Opening::of(&access, &engine).await;
        let carrying = carriage(connection, &access, &engine, opening);
        let clock = engine.clock();
        let carried = match opening.and_then(|opening| opening.deadline) {
            Some(deadline) => tokio::select! {
                biased;
                carried = carrying => carried,
                () = clock.sleep_until(deadline) => {
                    engine
                        .refusals()
                        .refused(access.listener(), RefusalReason::HelloTimedOut)
                        .await;
                    None
                }
            },
            None => carrying.await,
        };
        let Some((reading, writing, listener)) = carried else {
            return;
        };
        serve_carried(
            dispatch,
            Carried {
                reading,
                writing,
                listener,
                opening,
            },
            owner,
            runs,
            events,
            access,
            engine,
        )
        .await;
    };
    // The serving task's watch (ADR 0148): its own panic, such as one
    // while it handles a message, ends only its Connection, which its
    // caller then stops as when its Client went away.
    if AssertUnwindSafe(serving).catch_unwind().await.is_err() {
        metrics.panicked(PanicEnded::Connection);
        tracing::error!(
            owner = OwnerKind::Connection.as_str(),
            crash_report = crash_report::take_written().as_deref(),
            "the task that serves a Connection panicked; the Connection ends"
        );
    }
}

/// How a Connection's messages travel, its listener and the bounds of
/// its opening.
struct Carried {
    reading: Reading,
    writing: Writing,
    listener: Listener,
    opening: Option<Opening>,
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
        opening,
    } = carried;
    let (outgoing, lines) = mpsc::channel(OUTGOING_LINES);
    owner.spawn(write_lines(
        writing,
        lines,
        dispatch.masks.clone(),
        dispatch.work().clone(),
    ));
    let metrics = dispatch.metrics.clone();
    let (completed, completions) = mpsc::channel(MAX_IN_FLIGHT);
    // The watch of its own tasks (ADR 0148): a panic of one is told to the
    // serving loop, which ends the Connection; this ends once that loop
    // stopped listening.
    let (panicked, telling) = (owner.panicked(), completed.clone());
    owner.spawn(async move {
        tokio::select! {
            () = panicked => {
                let _ = telling.send(Told::OwnTaskPanicked).await;
            }
            () = telling.closed() => {}
        }
    });
    let (client, requests) = Client::new();
    let daemon = access.daemon();
    let log = dispatch
        .log_events
        .as_ref()
        .map_or_else(ConnectionLog::none, LogEvents::connection);
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
        form: OnceLock::new(),
        behind_gateway: OnceLock::new(),
        settings: OnceLock::new(),
        drivers: Arc::clone(&dispatch.drivers),
        runs,
        events,
        engine,
        daemon,
        log,
    });
    let mut session = Session {
        dispatch,
        owner,
        outgoing,
        completed,
        completions,
        own_task_panicked: false,
        in_flight: BTreeMap::new(),
        state: ConnectionState::Opening,
        connection,
        requests,
        pending: BTreeMap::new(),
        opened_client: None,
        access,
        listener,
        opening,
    };
    let ended = session.run(&mut reading).await;
    logging.cancel();
    session.cancel_in_flight();
    drop(session);
    owner.wait().await;
    // Every task of its owner ended, so a panic among them was told, even
    // one the serving loop met first as its writer gone (ADR 0148): it
    // counts once, and the Connection ends without lingering.
    let panicked = owner.has_panicked();
    if panicked {
        metrics.panicked(PanicEnded::Connection);
    }
    if !ended && !panicked {
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

/// Sleeps on `clock` until `deadline`, or for ever when there is none.
///
/// # Cancel safety
///
/// Dropping the future loses nothing.
async fn sleep_until(clock: Clock, deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => clock.sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Writes each line whole, or as one WebSocket message, flushing once no
/// other line waits; shuts the Connection's sending side down, or closes
/// its WebSocket, when every sender is gone. Every line passes through
/// `masks` first, as what a call returns or an event carries: no message
/// the Connection sends holds a value a Run resolved (spec #564). Every line
/// encoded for its message counts once on `work`, as the one encode into
/// bytes it was made by, whoever made it; a `$/events` batch copies events
/// counted as they were published (ADR 0140's E1).
async fn write_lines(
    mut writing: Writing,
    mut lines: mpsc::Receiver<Line>,
    masks: Masks,
    work: CountedWork,
) {
    while let Some(line) = lines.recv().await {
        if line.encoded() {
            work.count(Row::MessageEncodes);
        }
        let line = masks.apply_json_line(line.bytes, Scope::Returned);
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
    input: Option<(&'static str, InputSender)>,
}

/// A message read from the Connection, by the event of the engine's table
/// it is.
enum Message {
    Hello(RequestId, Call),
    Request(RequestId, Call),
    Notification(Call),
    Reply {
        id: RequestId,
        answer: Result<Received, ErrorObject>,
    },
    Unreadable(DecodeError),
}

impl Message {
    fn of(incoming: Result<Incoming, DecodeError>) -> Self {
        match incoming {
            Ok(Incoming::Request { id, call }) if call.method() == HELLO => Self::Hello(id, call),
            Ok(Incoming::Request { id, call }) => Self::Request(id, call),
            Ok(Incoming::Notification(notification)) => Self::Notification(notification),
            Ok(Incoming::Reply { id, answer }) => Self::Reply { id, answer },
            Err(error) => Self::Unreadable(error),
        }
    }

    fn event(&self) -> ConnectionEvent {
        match self {
            Self::Hello(..) => ConnectionEvent::Hello,
            Self::Request(..) => ConnectionEvent::Request,
            Self::Notification(_) => ConnectionEvent::Notification,
            Self::Reply { .. } => ConnectionEvent::Reply,
            Self::Unreadable(_) => ConnectionEvent::Unreadable,
        }
    }
}

/// What ended, as the Connection's tasks tell its serving loop.
enum Told {
    /// The call of this id, whose answer is on its way.
    CallEnded(RequestId),
    /// One of the Connection's own tasks, which panicked.
    OwnTaskPanicked,
}

/// Whether the Connection goes on after a message.
enum Flow {
    Continue,
    Close,
}

struct Session<'owner> {
    dispatch: Arc<Dispatch>,
    owner: &'owner TaskOwner,
    outgoing: mpsc::Sender<Line>,
    completed: mpsc::Sender<Told>,
    completions: mpsc::Receiver<Told>,
    /// Whether one of its own tasks panicked, which ends it as when its
    /// Client goes away.
    own_task_panicked: bool,
    in_flight: BTreeMap<RequestId, InFlight>,
    /// Where it is, which only the engine's table changes (ADR 0128).
    state: ConnectionState,
    connection: Arc<ConnectionContext>,
    requests: mpsc::Receiver<Outgoing>,
    pending: BTreeMap<RequestId, Pending>,
    /// On a Daemon, the count of this Client, held while it is open.
    opened_client: Option<Box<dyn Send + Sync>>,
    /// How the Connection reached the engine.
    access: Access,
    /// Its listener in the endpoint table (ADR 0067).
    listener: Listener,
    /// The bounds of its opening, on a guarded listener.
    opening: Option<Opening>,
}

/// A handler request the engine sent, waiting for its reply.
struct Pending {
    method: &'static str,
    reply: oneshot::Sender<Result<Received, ErrorObject>>,
}

impl Session<'_> {
    /// Serves the Connection until the Client's input ended and every call
    /// was answered, until one of its own tasks panicked and its calls
    /// were cancelled, or until the engine closed it; answers whether the
    /// input ended or a task panicked.
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
            ended |= self.own_task_panicked;
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
                // A call's own task may be what panicked, and never
                // ends in `in_flight`: the calls' tasks are waited for as
                // the Connection's owner's, once this returns.
                if self.own_task_panicked {
                    return true;
                }
                if self.in_flight.is_empty() {
                    if let Err(refused) = self.transition(ConnectionEvent::CallsEnded) {
                        let _ = refused.logged();
                    }
                    return true;
                }
            }
            let opening = self.state == ConnectionState::Opening;
            let deadline = self
                .opening
                .and_then(|bounds| bounds.deadline)
                .filter(|_| opening);
            let clock = self.connection.engine.clock();
            tokio::select! {
                biased;
                Some(told) = self.completions.recv() => self.told(told),
                Some(request) = self.requests.recv(), if !opening => {
                    if let Flow::Close = self.send_request(request).await {
                        return ended;
                    }
                }
                more = reading.read(&mut pending), if !ended => {
                    ended = !more;
                    // A first line already past its bound is refused now,
                    // not once its Client ends it.
                    if opening && self.opening.is_some() && reading.skipping() {
                        pending.push(Frame::TooLarge);
                    }
                }
                () = sleep_until(clock, deadline), if deadline.is_some() => {
                    self.refused(RefusalReason::HelloTimedOut).await;
                    let _ = self.rejected();
                    return ended;
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
        let opening_bound = self
            .opening
            .filter(|_| self.state == ConnectionState::Opening)
            .map(|bounds| bounds.max_size);
        let incoming = match frame {
            Frame::TooLarge => Err(DecodeError::too_large()),
            // A WebSocket's first message, read whole, past the opening's
            // bound.
            Frame::Line(line) if opening_bound.is_some_and(|bound| line.len() > bound) => {
                Err(DecodeError::too_large())
            }
            Frame::Line(line) => decode(line),
        };
        let message = Message::of(incoming);
        let effect = match self.transition(message.event()) {
            Ok(effect) => effect,
            Err(refused) => return self.refuse_transition(refused, message).await,
        };
        match (effect, message) {
            (ConnectionEffect::Handshake, Message::Hello(id, call)) => self.open(id, &call).await,
            (ConnectionEffect::Serve, Message::Request(id, call)) => self.request(id, &call).await,
            (ConnectionEffect::Serve, Message::Notification(notification)) => {
                self.notification(notification);
                Flow::Continue
            }
            (ConnectionEffect::Serve, Message::Reply { id, answer }) => {
                self.reply(&id, answer);
                Flow::Continue
            }
            (ConnectionEffect::Answer, Message::Unreadable(error)) => {
                let failure = CallError::protocol(error.failure, error.detail);
                if self
                    .send(failure_line(error.id, &failure, self.form()))
                    .await
                {
                    Flow::Continue
                } else {
                    Flow::Close
                }
            }
            (ConnectionEffect::Refuse, message) => self.refuse(message).await,
            (ConnectionEffect::CancelCalls, _) => {
                self.cancel_in_flight();
                Flow::Continue
            }
            (ConnectionEffect::Nothing, _) => Flow::Continue,
            (
                effect @ (ConnectionEffect::Handshake
                | ConnectionEffect::Serve
                | ConnectionEffect::Answer),
                message,
            ) => {
                tracing::error!(
                    ?effect,
                    event = ?message.event(),
                    "the Connection's table gave a message an effect that is not its own; the Connection closes"
                );
                Flow::Close
            }
        }
    }

    /// Refuses a first message that is no `hello`, then closes: unanswered
    /// on a listener with a token, answered with its error otherwise.
    async fn refuse(&mut self, message: Message) -> Flow {
        if self.access.guarded() {
            let reason = match &message {
                Message::Unreadable(error) if error.failure == ProtocolFailure::MessageTooLarge => {
                    RefusalReason::HelloTooLarge
                }
                _ => RefusalReason::HelloNotFirst,
            };
            self.refused(reason).await;
            return Flow::Close;
        }
        match message {
            Message::Hello(id, call) | Message::Request(id, call) => {
                let error = CallError::protocol(
                    ProtocolFailure::InvalidRequest,
                    "a Connection opens with `hello`",
                );
                record(self.named(call.method()), Some(error.code()));
                self.send(failure_line(Some(id), &error, self.form())).await;
            }
            Message::Unreadable(error) => {
                let failure = CallError::protocol(error.failure, error.detail);
                self.send(failure_line(error.id, &failure, self.form()))
                    .await;
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
        let (id, call) = match (refused.refusal, message) {
            (Refusal::Program(_), Message::Hello(id, call)) => (id, call),
            (Refusal::Program(_) | Refusal::Internal, _) => return Flow::Close,
        };
        let error = CallError::protocol(
            ProtocolFailure::InvalidRequest,
            "a Connection opens with `hello` once",
        );
        record(self.named(call.method()), Some(error.code()));
        if self.send(failure_line(Some(id), &error, self.form())).await {
            Flow::Continue
        } else {
            Flow::Close
        }
    }

    /// Sends a handler request, its line encoded under an id of the
    /// engine's own, or `$/cancel` of one still waiting for its reply.
    async fn send_request(&mut self, request: Outgoing) -> Flow {
        let line = match request {
            Outgoing::Request {
                id,
                method,
                line,
                reply,
            } => {
                self.pending.insert(id, Pending { method, reply });
                line
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
    fn reply(&mut self, id: &RequestId, answer: Result<Received, ErrorObject>) {
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
    async fn open(&mut self, id: RequestId, call: &Call) -> Flow {
        let daemon = self.connection.daemon.clone();
        // A `hello` without params is refused as one with `null`.
        let arrived = Arrived::new(Some(call.params().unwrap_or("null")), self.dispatch.work());
        // One read of the params, in whose pass their `_meta` is found; a
        // `_meta` the engine refuses comes before what else they hold.
        let hello = arrived.params().read::<HelloParams>();
        let (hello, meta) = match arrived.preamble() {
            Ok(preamble) => (hello, preamble.meta),
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
        let span = trace::server_span(HELLO, &id, &trace::Ids::default(), meta.as_ref());
        let answered = match hello {
            Ok(hello) => handshake::answer(
                &self.dispatch.identity,
                self.connection.events.instance(),
                &hello,
            )
            .map(|accepted| {
                let surface = accepted.surface;
                let form = hello.form.unwrap_or(surface.form());
                (accepted, surface, form, hello.log_level, hello.settings)
            }),
            Err(error) => Err(handshake::Refused::Invalid(error)),
        };
        let answered = match answered {
            Ok((accepted, surface, form, log_level, settings)) => self
                .checked_settings(&settings, surface, accepted.behind_gateway)
                .await
                .map(|()| (accepted, surface, form, log_level, settings))
                // A refused setting reads in the form the `hello` declares.
                .map_err(|error| (error, form)),
            Err(refusal) => {
                let handover = match &daemon {
                    Some(daemon) if refusal.by_product_or_version() => daemon.refusing().await,
                    _ => false,
                };
                Err((refusal.into_error(handover), self.form()))
            }
        };
        match answered {
            Ok((mut accepted, surface, form, log_level, settings)) => {
                record(HELLO, None);
                self.connection.log.set(log_level.unwrap_or_default());
                if let Err(refused) = self.transition(ConnectionEvent::Accepted) {
                    let _ = refused.logged();
                    return Flow::Close;
                }
                // A Connection answers one `hello`: the first sets it.
                let _ = self.connection.surface.set(surface);
                let _ = self.connection.form.set(form);
                let _ = self.connection.settings.set(settings);
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
                if let Access::Attached(private_endpoint) = &self.access {
                    accepted
                        .result
                        .private_endpoint
                        .clone_from(private_endpoint);
                }
                let line = success_line(&id, &accepted.result)
                    .expect("invariant: a `hello` reply encodes");
                drop(span);
                if self.send(line).await {
                    Flow::Continue
                } else {
                    Flow::Close
                }
            }
            Err((error, form)) => {
                record(HELLO, Some(error.code()));
                trace::record_failure(&span, error.code(), error.wire_code());
                drop(span);
                self.send(failure_line(Some(id), &error, form)).await;
                self.rejected()
            }
        }
    }

    /// Checks `own`, the Connection's own settings as its `hello` gives
    /// them, from `surface`: each key at the connection scope, then every
    /// layer resolved over them as beneath a Run it starts, so a locked or
    /// capped key is refused at the `hello` rather than at each start (ADR
    /// 0083).
    async fn checked_settings(
        &self,
        own: &Map<String, Value>,
        surface: Surface,
        behind_gateway: bool,
    ) -> Result<(), CallError> {
        if own.is_empty() {
            return Ok(());
        }
        let identity = &self.dispatch.identity;
        let refused = |error: SettingsError| translated(&error, identity, behind_gateway);
        check_scope(own, SettingScope::Connection, identity.product).map_err(refused)?;
        self.connection
            .engine
            .settings()
            .open(SettingsOpening {
                run: None,
                project: None,
                recipe: None,
                settings: own.clone(),
                origin: Some(RunOrigin::Settings),
                acting_for_model: surface.acts_for_model(),
            })
            .await
            .map(drop)
            .map_err(refused)
    }

    /// Its `hello` was refused: it closes.
    fn rejected(&mut self) -> Flow {
        if let Err(refused) = self.transition(ConnectionEvent::Rejected) {
            let _ = refused.logged();
        }
        Flow::Close
    }

    async fn request(&mut self, mut id: RequestId, call: &Call) -> Flow {
        let method = call.method();
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
        } else if let Some(handler) = self.dispatch.handlers.get(method) {
            if handler.operation().reach == Reach::PersonOnly
                && !self
                    .connection
                    .surface
                    .get()
                    .copied()
                    .is_some_and(Surface::is_a_person)
            {
                Some(person_required(method))
            } else {
                let handler = handler.clone();
                if self.in_flight.len() >= MAX_IN_FLIGHT {
                    // Refused last, so a call the engine would never start
                    // keeps its own refusal at the cap.
                    let refused = Arrived::new(call.params(), self.dispatch.work())
                        .preamble()
                        .err();
                    Some(refused.unwrap_or_else(too_many_calls))
                } else {
                    match self.call(id, &handler, call.params()) {
                        Ok(()) => return Flow::Continue,
                        Err((refused, error)) => {
                            id = refused;
                            Some(error)
                        }
                    }
                }
            }
        } else if !self.dispatch.identity.product.contains(Product::Foxhound)
            && foxhound_names::OPERATIONS.contains(&method)
        {
            Some(not_in_product(
                &self.dispatch.identity,
                Requested::Name(method),
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
        record(self.named(method), Some(error.code()));
        if self.send(failure_line(Some(id), &error, self.form())).await {
            Flow::Continue
        } else {
            Flow::Close
        }
    }

    /// Starts `handler` on `params`, a request's params where they are, as a
    /// task of the Connection, in the request's span, the child of the span
    /// the Trace context of their `_meta` names. The handler reads `params`
    /// before it returns, and what dispatch takes of them, their `_meta`
    /// and the IDs the span records, comes from that read's pass.
    ///
    /// # Errors
    ///
    /// The request's id and `invalid_params`, for a `_meta` the engine
    /// refuses: nothing started.
    fn call(
        &mut self,
        id: RequestId,
        handler: &Handler,
        params: Option<&str>,
    ) -> Result<(), (RequestId, CallError)> {
        let token = self.owner.token().child_token();
        let operation = handler.operation();
        let credits = handler.streams().then(Credits::new);
        // Only a call that streams or takes input notifies its Client: any
        // other, most calls, makes no notifier and carries no channels (E1,
        // ADR 0140).
        let (input, channels) = if credits.is_none() && handler.input().is_none() {
            (None, None)
        } else {
            let notifier = CallNotifier::new(id.clone(), self.outgoing.clone());
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
            (input, Some(CallChannels::new(stream, call_input)))
        };
        let context = CallContext::new(
            Arc::clone(&self.connection),
            operation,
            id.clone(),
            token.clone(),
            channels,
        );
        // The call's watch (ADR 0148): a panic in its handler is answered
        // here, in the task the call already has, and never reaches the
        // Connection's owner.
        let arrived = Arrived::new(params, self.dispatch.work());
        let started = handler
            .call(arrived.params(), context)
            .and_then(|answer| Ok((answer, arrived.preamble()?)));
        let (answer, preamble) = match started {
            Ok(started) => started,
            Err(error) => return Err((id, error)),
        };
        let span = trace::server_span(operation.name, &id, &preamble.ids, preamble.meta.as_ref());
        let answer = CallWatch {
            answer: answer.instrument(span.clone()),
            metrics: self.dispatch.metrics.clone(),
        };
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
        let form = self.form();
        // Each call keeps its own task (the audit's E1-4, kept by #1898):
        // the Connection goes on reading while a handler waits, a
        // `$/cancel` reaches the call at once, and [`MAX_IN_FLIGHT`] caps
        // how many there are.
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
                Ok(answered) => {
                    record(operation.name, None);
                    answered.into_line()
                }
                Err(error) => {
                    record(operation.name, Some(error.code()));
                    trace::record_failure(&span, error.code(), error.wire_code());
                    failure_line(Some(id.clone()), &error, form)
                }
            };
            drop(span);
            // Ended before its answer is sent: the Client may reuse the id
            // as soon as the answer arrives, and `still_in_flight` then
            // finds this end waiting.
            let _ = completed.send(Told::CallEnded(id)).await;
            let _ = outgoing.send(line.into()).await;
        });
        Ok(())
    }

    /// Whether the call `id` names is still in flight, once every call that
    /// ended was taken out: a call ends before its answer is sent, so the
    /// id of a call whose answer the Client read is free again (JSON-RPC
    /// 2.0), however the session's loop was scheduled.
    fn still_in_flight(&mut self, id: &RequestId) -> bool {
        while let Ok(told) = self.completions.try_recv() {
            self.told(told);
        }
        self.in_flight.contains_key(id)
    }

    /// Takes in what one of its tasks told: a call that ended gives its
    /// place in flight back, and a panic of one of its own tasks ends the
    /// Connection.
    fn told(&mut self, told: Told) {
        match told {
            Told::CallEnded(id) => {
                self.in_flight.remove(&id);
            }
            Told::OwnTaskPanicked => self.own_task_panicked = true,
        }
    }

    fn notification(&mut self, notification: Call) {
        if notification.method() == CREDIT {
            self.credit(&notification);
            return;
        }
        if notification.method() != CANCEL {
            self.input(notification);
            return;
        }
        let Some(CancelParams { id }) = read_params(&notification) else {
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
    fn input(&mut self, notification: Call) {
        #[derive(serde::Deserialize)]
        struct Named {
            id: RequestId,
        }
        let taker = read_params::<Named>(&notification)
            .and_then(|Named { id }| self.in_flight.get_mut(&id))
            .and_then(|call| call.input.as_mut())
            .filter(|(method, _)| *method == notification.method());
        if let Some((_, sender)) = taker {
            sender.hand_over(notification.into_params());
        } else {
            tracing::debug!("an unknown notification is ignored");
        }
    }

    /// Adds a `$/credit`'s credits to the call it names; one for a call no
    /// longer in flight, or for one that does not stream, is ignored.
    fn credit(&self, notification: &Call) {
        let Some(CreditParams { id, credits }) = read_params(notification) else {
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
        self.outgoing.send(line.into()).await.is_ok()
    }
}

impl Session<'_> {
    /// The form its Client writes names in (ADR 0099), which a failure's
    /// marked names are spelled in; the catalog's own names before the
    /// `hello` declared one.
    fn form(&self) -> Form {
        self.connection.form.get().copied().unwrap_or(Form::Catalog)
    }

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
    /// records it in the Audit log.
    async fn refused(&self, reason: RefusalReason) {
        self.connection
            .engine
            .refusals()
            .refused(self.listener, reason)
            .await;
    }
}

/// A notification's params, read as `P` where they are; none when they
/// are no `P`.
fn read_params<P: serde::de::DeserializeOwned>(notification: &Call) -> Option<P> {
    serde_json::from_str(notification.params()?).ok()
}

/// `person_required`: `method` is a person's answer, which only the Kennel
/// and the CLI's interactive prompt ask for (ADRs 0011, 0041 and 0141).
fn person_required(method: &str) -> CallError {
    CallError::new(
        &PERSON_REQUIRED,
        format!(
            "`{method}` is a person's answer: give it in the Kennel, or in the CLI's interactive prompt"
        ),
        &serde_json::Map::new(),
    )
}

/// `too_many_calls`: the Connection holds [`MAX_IN_FLIGHT`] calls in flight,
/// so one more is refused, the Connection still reading everything else.
fn too_many_calls() -> CallError {
    CallError::new(
        &TOO_MANY_CALLS,
        format!(
            "The Connection already has {MAX_IN_FLIGHT} calls in flight: make the call again once one of them ended"
        ),
        &TooManyCallsDetails {
            max: u64::try_from(MAX_IN_FLIGHT).unwrap_or(u64::MAX),
        },
    )
}

/// A call's answer under its watch (ADR 0148): a panic while its handler
/// is polled is caught there and answered [`engine_panicked`].
///
/// Hand-written rather than `catch_unwind().map(…)`, whose two layers cost
/// every call on E1's path (ADR 0140) more than the one catch they hold.
struct CallWatch<F> {
    answer: F,
    metrics: Metrics,
}

impl<F> Future for CallWatch<F>
where
    F: Future<Output = Result<Answered, CallError>> + Unpin,
{
    type Output = Result<Answered, CallError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let watch = self.get_mut();
        let answer = &mut watch.answer;
        std::panic::catch_unwind(AssertUnwindSafe(|| Pin::new(answer).poll(context)))
            .unwrap_or_else(|_| Poll::Ready(Err(engine_panicked(&watch.metrics))))
    }
}

/// `engine_panicked` for a call whose handler panicked, counted on
/// `metrics` ([`crash_report::engine_panicked`]). Made where the panic was
/// caught, on the thread that panicked.
fn engine_panicked(metrics: &Metrics) -> CallError {
    crash_report::engine_panicked(
        metrics,
        PanicEnded::Call,
        "The engine hit a bug in Goodfetch while it served this call, which ended the call; its Connection goes on",
    )
}
