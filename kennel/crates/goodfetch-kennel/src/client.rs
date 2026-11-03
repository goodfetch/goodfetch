//! The Kennel's Client of the Daemon (ADR 0141), over `goodfetch-protocol`'s
//! types alone: it reads the running Daemon's runtime file in the Goodfetch
//! home, opens its socket, says `hello` as the `kennel` surface needing
//! Collie, which both Products contain, and reads the Daemon's Product from
//! the answer. It frames JSON-RPC by line. It is the one module that could
//! later move into a shared client crate.
//!
//! Its interface is what it reports, [`Report`]: the Connection's changes,
//! the subscription's events, and the answers to the reads the State wants,
//! [`Wanted`], which it keeps in step with.
//!
//! - **Push, not poll.** Once `hello` is answered it subscribes with
//!   `events.subscribe` and grants [`CREDITS`] credits, renewed as the
//!   events are applied: each [`Batch`] gives its credits back once it is
//!   dropped, and the Client renews them in grants of at least half. A
//!   queue that filled while the Kennel was slow comes back as
//!   `events.resync`, which the Client reports like any event. The first
//!   subscription names a position ahead of any the engine gave, which
//!   starts it with `events.resync` too (ADR 0031): the State starts from
//!   the Daemon's live Runs and Needs you items, and the stream continues
//!   from that state with nothing lost between.
//! - **The reads the view needs.** `run.list` of the live Runs each time
//!   [`Wanted::live_runs`] changes, and `run.progress` for each Run of
//!   [`Wanted::progress`]: read at once, then followed with `after` until
//!   the Run leaves the set, when the read is cancelled, or its progress
//!   is the last.
//!
//! The Daemon's token is read from the runtime file at each connection,
//! held in memory only, sent only in `hello`, and never logged: the
//! protocol's `DaemonToken` prints `[REDACTED]`, and no `hello` is logged.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::pin::Pin;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use goodfetch_protocol::connection::{
    CANCEL, CancelParams, DaemonEndpoint, EngineInstance, HELLO, HelloParams, HelloResult,
    PROTOCOL_VERSION_UNSUPPORTED, ProductVersion, ProtocolVersionUnsupportedDetails, Surface,
};
use goodfetch_protocol::daemon::DaemonRuntimeFile;
use goodfetch_protocol::events::{
    CREDIT, CreditParams, EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventEnvelope,
    EventsSubscribeParams, LastRead, Position, RESYNC, Resync,
};
use goodfetch_protocol::jsonrpc::{
    ErrorObject, Message, Notification, Request, RequestId, Version,
};
use goodfetch_protocol::needs_you::{NEEDS_YOU_CHANGED, NeedsYouChange};
use goodfetch_protocol::run::{
    Lifecycle, RUN_AXES_CHANGED, RUN_LIST, RUN_PROGRESS_READ, RunAxes, RunId, RunIndexRow, RunList,
    RunListParams, RunProgress, RunProgressParams,
};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::sync::{mpsc, watch};

use crate::home::Home;
use crate::tasks::Tasks;

/// The id of the Kennel's `hello`, its first request on each Connection.
const HELLO_ID: i64 = 1;

/// The id of its `events.subscribe`, its second.
const SUBSCRIBE_ID: i64 = 2;

/// The id of its first other call; each next one takes the next.
const FIRST_CALL_ID: i64 = 3;

/// How many events the subscription may hold unapplied: the engine's queue
/// of lossless events per subscription holds 64 before it falls behind to
/// `events.resync`.
pub const CREDITS: u32 = 64;

/// How many applied events the Client gives back at once, at the least.
const RENEWAL: u64 = CREDITS as u64 / 2;

/// Who the reached Daemon is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonIdentity {
    /// Its Product, by which the State hides Foxhound's views from a
    /// Collie Daemon.
    pub product: Product,
    /// Its Product version.
    pub product_version: ProductVersion,
}

/// Why a running Daemon refused the Kennel's `hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// It speaks another protocol version.
    ProtocolVersion {
        /// The version it speaks.
        engine: ProtocolVersion,
        /// The version this Kennel needs.
        needs: ProtocolVersion,
    },
    /// It is handing over to its major's newer version (ADR 0098).
    HandOver,
    /// Another refusal, by its Error code.
    Other {
        /// The Error code.
        code: String,
    },
}

/// A change of the Kennel's Connection to the Daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionChange {
    /// No Daemon runs, or the one that ran is gone.
    NoDaemon,
    /// A Daemon answered `hello`.
    Connected(DaemonIdentity),
    /// A Daemon runs and refused the Kennel.
    Refused(Refusal),
}

/// An event of the Daemon's subscription that the Kennel reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonEvent {
    /// The current state, from which the stream continues: at the start of
    /// each subscription, and after the Kennel's queue filled.
    Resync(Resync),
    /// A Run started or ended (`run.axes_changed`).
    RunAxes(RunAxes),
    /// The Needs you list changed (`needs_you.changed`).
    NeedsYou(NeedsYouChange),
}

/// Events of the subscription, in order. Dropping the batch, once its
/// events are applied, gives their credits back.
pub struct Batch {
    /// The events the Kennel reads; the others were skipped.
    pub events: Vec<DaemonEvent>,
    /// The credits the batch took.
    applied: Applied,
}

impl Batch {
    /// The batch's events; dropping the second part gives their credits
    /// back.
    pub fn into_parts(self) -> (Vec<DaemonEvent>, Applied) {
        (self.events, self.applied)
    }
}

impl fmt::Debug for Batch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Batch")
            .field("events", &self.events)
            .field("credits", &self.applied.credits)
            .finish()
    }
}

/// The credits of events taken: given back to the subscription when
/// dropped.
#[derive(Debug)]
pub struct Applied {
    credits: u64,
    to: watch::Sender<u64>,
}

impl Drop for Applied {
    fn drop(&mut self) {
        let credits = self.credits;
        self.to.send_modify(|applied| *applied += credits);
    }
}

/// What the Client reports.
#[derive(Debug)]
pub enum Report {
    /// The Connection changed.
    Connection(ConnectionChange),
    /// Events of the subscription.
    Events(Batch),
    /// The live Runs' rows of the Run index, as `run.list` answered.
    LiveRuns(Vec<RunIndexRow>),
    /// A Run's progress, as `run.progress` answered.
    Progress(RunProgress),
}

/// The reads the State wants the Client to keep in step with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    /// The live Runs' rows are read again each time this changes.
    pub live_runs: u64,
    /// The Runs whose progress is read and followed.
    pub progress: BTreeSet<RunId>,
}

/// Follows the Daemon until the Kennel stops: looks for it in `home`,
/// connects, subscribes, keeps the reads in step with `wanted`, and reports
/// each change. Once no Daemon answers, it looks again only when
/// `look_again` asks (after a "Start Daemon"); a lost Connection makes it
/// look again at once.
///
/// # Cancel safety
/// Dropping the future closes the Connection; nothing else is lost.
pub async fn follow(
    tasks: Tasks,
    home: Home,
    mut look_again: mpsc::Receiver<()>,
    mut wanted: watch::Receiver<Wanted>,
    report: impl Fn(Report) + Send + Sync + 'static,
) {
    loop {
        let lookup = home.clone();
        let found = tasks.blocking(move || lookup.running_daemon()).await;
        let reached = match found {
            None => None,
            Some(runtime) => match Connection::open(&runtime).await {
                Ok(connection) => Some(connection),
                Err(Opening::Unreachable) => None,
                Err(Opening::Refused(refusal)) => {
                    tracing::warn!(?refusal, "the Daemon refused the Kennel");
                    report(Report::Connection(ConnectionChange::Refused(refusal)));
                    if look_again.recv().await.is_none() {
                        return;
                    }
                    continue;
                }
            },
        };
        if let Some(connection) = reached {
            tracing::info!(
                product = connection.identity.product.as_str(),
                version = %connection.identity.product_version,
                "reached the Daemon"
            );
            let reads = Reads::new(&mut wanted);
            report(Report::Connection(ConnectionChange::Connected(
                connection.identity.clone(),
            )));
            connection.serve(reads, &mut wanted, &report).await;
            if wanted.has_changed().is_err() {
                // The Kennel is quitting: nothing more is wanted.
                return;
            }
            tracing::info!("lost the Daemon");
        } else {
            report(Report::Connection(ConnectionChange::NoDaemon));
            if look_again.recv().await.is_none() {
                return;
            }
        }
    }
}

/// Why opening a Connection failed.
#[derive(Debug)]
enum Opening {
    /// Nothing answered at the endpoint, or it closed before answering.
    Unreachable,
    /// The Daemon refused the Kennel.
    Refused(Refusal),
}

type Reader = Pin<Box<dyn AsyncRead + Send>>;
type Writer = Pin<Box<dyn AsyncWrite + Send>>;

/// An open Connection, after its `hello` and its subscription.
struct Connection {
    lines: Lines<BufReader<Reader>>,
    writer: Writer,
    identity: DaemonIdentity,
}

impl Connection {
    /// Opens the Connection to the Daemon of `runtime` with `hello`, then
    /// subscribes to its events, starting from a resync.
    async fn open(runtime: &DaemonRuntimeFile) -> Result<Self, Opening> {
        let (reader, mut writer) = connect(&runtime.endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut lines = BufReader::new(reader).lines();
        send(&mut writer, &hello_request(runtime))
            .await
            .map_err(|_| Opening::Unreachable)?;
        let accepted = loop {
            let Ok(Some(line)) = lines.next_line().await else {
                return Err(Opening::Unreachable);
            };
            match serde_json::from_str::<Message>(&line) {
                Ok(Message::Response(response)) if response.id == RequestId::Number(HELLO_ID) => {
                    break serde_json::from_value::<HelloResult>(response.result).map_err(
                        |_| {
                            Opening::Refused(Refusal::Other {
                                code: "invalid_hello_result".to_owned(),
                            })
                        },
                    )?;
                }
                // An error whose id is null answers a message the Daemon
                // could not read, which can only be `hello`.
                Ok(Message::Error(failure))
                    if failure.id.is_none() || failure.id == Some(RequestId::Number(HELLO_ID)) =>
                {
                    return Err(Opening::Refused(refusal(failure.error)));
                }
                Ok(_) | Err(_) => {}
            }
        };
        let mut connection = Self {
            lines,
            writer,
            identity: DaemonIdentity {
                product: accepted.product,
                product_version: accepted.product_version,
            },
        };
        connection
            .subscribe(accepted.engine_instance)
            .await
            .map_err(|_| Opening::Unreachable)?;
        Ok(connection)
    }

    /// Subscribes to the events of the engine `instance`, from a resync,
    /// and grants the subscription its credits.
    async fn subscribe(&mut self, instance: EngineInstance) -> io::Result<()> {
        let params = EventsSubscribeParams {
            // A position ahead of any the engine gave starts the
            // subscription with `events.resync` (ADR 0031).
            after: Some(LastRead {
                engine_instance: instance,
                position: Position(u64::MAX),
            }),
            system_info: false,
        };
        send(
            &mut self.writer,
            &request(SUBSCRIBE_ID, EVENTS_SUBSCRIBE.name, &params),
        )
        .await?;
        self.credit(u64::from(CREDITS)).await
    }

    /// Grants the subscription `credits` more events.
    async fn credit(&mut self, credits: u64) -> io::Result<()> {
        let params = CreditParams {
            id: RequestId::Number(SUBSCRIBE_ID),
            credits: u32::try_from(credits).unwrap_or(u32::MAX),
        };
        send(&mut self.writer, &notification(CREDIT, &params)).await
    }

    /// Reports the subscription's events and the reads' answers, and keeps
    /// the reads in step with `wanted`, until the Daemon closes the
    /// Connection.
    async fn serve(
        mut self,
        mut reads: Reads,
        wanted: &mut watch::Receiver<Wanted>,
        report: &impl Fn(Report),
    ) {
        let (applied, mut renewals) = watch::channel(0_u64);
        let mut credited = 0_u64;
        let first = reads.first.take().unwrap_or_default();
        if self.keep_in_step(&mut reads, &first).await.is_err() {
            return;
        }
        loop {
            let outcome = tokio::select! {
                line = self.lines.next_line() => match line {
                    Ok(Some(line)) => self.read(&line, &mut reads, &applied, report).await,
                    Ok(None) | Err(_) => return,
                },
                changed = wanted.changed() => {
                    if changed.is_err() {
                        // The Kennel is quitting: nothing more is wanted.
                        return;
                    }
                    let now = wanted.borrow_and_update().clone();
                    self.keep_in_step(&mut reads, &now).await
                }
                changed = renewals.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let total = *renewals.borrow_and_update();
                    if total - credited >= RENEWAL {
                        let renewed = total - credited;
                        credited = total;
                        self.credit(renewed).await
                    } else {
                        Ok(())
                    }
                }
            };
            if outcome.is_err() {
                return;
            }
        }
    }

    /// Reads one line of the Daemon.
    async fn read(
        &mut self,
        line: &str,
        reads: &mut Reads,
        applied: &watch::Sender<u64>,
        report: &impl Fn(Report),
    ) -> io::Result<()> {
        match serde_json::from_str::<Message>(line) {
            Ok(Message::Notification(notification)) if notification.method == EVENTS => {
                let Some(batch) = notification
                    .params
                    .and_then(|params| serde_json::from_value::<EventBatch>(params).ok())
                else {
                    tracing::warn!("an event batch the Kennel cannot read");
                    return Ok(());
                };
                if batch.id == RequestId::Number(SUBSCRIBE_ID) {
                    events(batch.events, applied, report);
                }
                Ok(())
            }
            Ok(Message::Response(response)) => {
                self.answered(reads, response.id, Ok(response.result), report)
                    .await
            }
            Ok(Message::Error(failure)) => match failure.id {
                Some(id) => {
                    self.answered(reads, id, Err(failure.error.data.code), report)
                        .await
                }
                None => Ok(()),
            },
            Ok(Message::Request(_) | Message::Notification(_)) | Err(_) => Ok(()),
        }
    }

    /// Handles the answer to the call `id`.
    async fn answered(
        &mut self,
        reads: &mut Reads,
        id: RequestId,
        answer: Result<Value, String>,
        report: &impl Fn(Report),
    ) -> io::Result<()> {
        let RequestId::Number(id) = id else {
            return Ok(());
        };
        match reads.pending.remove(&id) {
            None => Ok(()),
            Some(Pending::LiveRuns) => {
                match answer.map(serde_json::from_value::<RunList>) {
                    Ok(Ok(list)) => report(Report::LiveRuns(list.runs)),
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "a run.list answer the Kennel cannot read");
                    }
                    Err(code) => tracing::warn!(code, "run.list failed"),
                }
                Ok(())
            }
            Some(Pending::Progress(run)) => {
                let progress = match answer.map(serde_json::from_value::<RunProgress>) {
                    Ok(Ok(progress)) => progress,
                    Ok(Err(error)) => {
                        tracing::warn!(%error, run_id = %run, "a run.progress answer the Kennel cannot read");
                        reads.followed.insert(run, Followed::Ended);
                        return Ok(());
                    }
                    Err(code) => {
                        tracing::warn!(code, run_id = %run, "run.progress failed");
                        reads.followed.insert(run, Followed::Ended);
                        return Ok(());
                    }
                };
                let next = (!progress.last).then_some(progress.position);
                report(Report::Progress(progress));
                if let Some(position) = next {
                    self.read_progress(reads, run, Some(position)).await
                } else {
                    reads.followed.insert(run, Followed::Ended);
                    Ok(())
                }
            }
        }
    }

    /// Sends the reads `wanted` asks for and cancels those it no longer
    /// does.
    async fn keep_in_step(&mut self, reads: &mut Reads, wanted: &Wanted) -> io::Result<()> {
        if wanted.live_runs != reads.live_runs {
            reads.live_runs = wanted.live_runs;
            let id = reads.next_id();
            let params = RunListParams {
                lifecycle: Some(Lifecycle::Running),
                ..RunListParams::default()
            };
            send(&mut self.writer, &request(id, RUN_LIST.name, &params)).await?;
            reads.pending.insert(id, Pending::LiveRuns);
        }
        let dropped: Vec<RunId> = reads
            .followed
            .keys()
            .filter(|run| !wanted.progress.contains(*run))
            .cloned()
            .collect();
        for run in dropped {
            if let Some(Followed::Reading(id)) = reads.followed.remove(&run) {
                reads.pending.remove(&id);
                let params = CancelParams {
                    id: RequestId::Number(id),
                };
                send(&mut self.writer, &notification(CANCEL, &params)).await?;
            }
        }
        for run in &wanted.progress {
            if !reads.followed.contains_key(run) {
                self.read_progress(reads, run.clone(), None).await?;
            }
        }
        Ok(())
    }

    /// Reads `run`'s progress: at once, or after the value at `after`.
    async fn read_progress(
        &mut self,
        reads: &mut Reads,
        run: RunId,
        after: Option<u64>,
    ) -> io::Result<()> {
        let id = reads.next_id();
        let params = RunProgressParams {
            run_id: run.clone(),
            after,
        };
        send(
            &mut self.writer,
            &request(id, RUN_PROGRESS_READ.name, &params),
        )
        .await?;
        reads.pending.insert(id, Pending::Progress(run.clone()));
        reads.followed.insert(run, Followed::Reading(id));
        Ok(())
    }
}

/// The reads of one Connection.
struct Reads {
    next_id: i64,
    /// The calls waiting for their answer.
    pending: BTreeMap<i64, Pending>,
    /// The Runs whose progress is followed.
    followed: BTreeMap<RunId, Followed>,
    /// The [`Wanted::live_runs`] last read.
    live_runs: u64,
    /// What was wanted as the Connection opened.
    first: Option<Wanted>,
}

impl Reads {
    /// The reads of a new Connection, which reads the live Runs' rows only
    /// once `wanted` asks again.
    fn new(wanted: &mut watch::Receiver<Wanted>) -> Self {
        let first = wanted.borrow_and_update().clone();
        Self {
            next_id: FIRST_CALL_ID,
            pending: BTreeMap::new(),
            followed: BTreeMap::new(),
            live_runs: first.live_runs,
            first: Some(first),
        }
    }

    fn next_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// What a call waiting for its answer reads.
#[derive(Debug)]
enum Pending {
    LiveRuns,
    Progress(RunId),
}

/// Where a followed Run's progress is.
#[derive(Debug)]
enum Followed {
    /// The call with this id reads it.
    Reading(i64),
    /// Its last value came, or it cannot be read.
    Ended,
}

/// Reports the events of a batch the Kennel reads, and gives back at once
/// the credits of a batch with none.
fn events(envelopes: Vec<EventEnvelope>, applied: &watch::Sender<u64>, report: &impl Fn(Report)) {
    let credits = u64::try_from(envelopes.len()).unwrap_or(u64::MAX);
    let events: Vec<DaemonEvent> = envelopes.into_iter().filter_map(decoded).collect();
    let applied = Applied {
        credits,
        to: applied.clone(),
    };
    if !events.is_empty() {
        report(Report::Events(Batch { events, applied }));
    }
}

/// The event `envelope` carries, if the Kennel reads it.
fn decoded(envelope: EventEnvelope) -> Option<DaemonEvent> {
    let EventEnvelope { event, data, .. } = envelope;
    let decoded = if event == RESYNC.name {
        serde_json::from_value(data).map(DaemonEvent::Resync)
    } else if event == RUN_AXES_CHANGED.name {
        serde_json::from_value(data).map(DaemonEvent::RunAxes)
    } else if event == NEEDS_YOU_CHANGED.name {
        serde_json::from_value(data).map(DaemonEvent::NeedsYou)
    } else {
        return None;
    };
    decoded
        .inspect_err(|error| tracing::warn!(event, %error, "an event the Kennel cannot read"))
        .ok()
}

/// The Kennel's `hello` to the Daemon of `runtime`.
fn hello_request(runtime: &DaemonRuntimeFile) -> Request {
    let hello = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Kennel,
        form: None,
        token: Some(runtime.token.expose().to_owned()),
        needs: Some(Product::Collie),
        product_version: kennel_version(),
        log_level: None,
        client_surface: None,
        keep_daemon_alive: false,
    };
    request(HELLO_ID, HELLO, &hello)
}

/// The request `id` of `method` with `params`.
fn request(id: i64, method: &str, params: &impl Serialize) -> Request {
    Request {
        jsonrpc: Version,
        id: RequestId::Number(id),
        method: method.to_owned(),
        params: Some(serde_json::to_value(params).expect("invariant: a request's params encode")),
    }
}

/// The notification `method` with `params`.
fn notification(method: &str, params: &impl Serialize) -> Notification {
    Notification {
        jsonrpc: Version,
        method: method.to_owned(),
        params: Some(
            serde_json::to_value(params).expect("invariant: a notification's params encode"),
        ),
    }
}

/// The Kennel's own version, which its `hello` names (ADR 0141).
fn kennel_version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version")
}

/// What a refused `hello` says.
fn refusal(error: ErrorObject) -> Refusal {
    let details = Value::Object(error.data.details);
    if details.get("handover").and_then(Value::as_bool) == Some(true) {
        return Refusal::HandOver;
    }
    if error.data.code == PROTOCOL_VERSION_UNSUPPORTED.name
        && let Ok(details) = serde_json::from_value::<ProtocolVersionUnsupportedDetails>(details)
    {
        return Refusal::ProtocolVersion {
            engine: details.engine_protocol_version,
            needs: PROTOCOL_VERSION,
        };
    }
    Refusal::Other {
        code: error.data.code,
    }
}

/// Writes `message` as one line.
async fn send(writer: &mut Writer, message: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).expect("invariant: a message encodes");
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Opens the Daemon's socket: a Unix socket on macOS and Linux, loopback
/// TCP on Windows (ADR 0024).
#[expect(
    clippy::disallowed_types,
    reason = "the Kennel's Client is the one module of the Kennel that opens a socket, its Connection to the local Daemon (ADR 0141)"
)]
async fn connect(endpoint: &DaemonEndpoint) -> io::Result<(Reader, Writer)> {
    match endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            let (reader, writer) = tokio::net::UnixStream::connect(path).await?.into_split();
            Ok((Box::pin(reader), Box::pin(writer)))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a Unix socket on a host that has none",
        )),
        DaemonEndpoint::Tcp(address) => {
            let stream = tokio::net::TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let (reader, writer) = stream.into_split();
            Ok((Box::pin(reader), Box::pin(writer)))
        }
    }
}
