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
//! - **The Daemon's life.** It follows it without polling and without
//!   holding it awake: the platform's file notifications on the runtime
//!   files make it look again when a Daemon starts, stops or hands over to
//!   its major's pick (`runtime_files`), and its `hello` asks the Daemon to
//!   stay alive only when the keep-alive setting is on. Its one poll is
//!   `daemon.health`, on a slow timer, while connected.
//! - **Push, not poll.** Once `hello` is answered it subscribes with
//!   `events.subscribe` and grants [`CREDITS`] credits, renewed as the
//!   events are applied: each [`Batch`] gives its credits back once it is
//!   dropped, and the Client renews them in grants of at least half. A
//!   queue that filled while the Kennel was slow comes back as
//!   `events.resync`, which the Client reports like any event. Each
//!   subscription names a position ahead of any the engine gave, which
//!   starts it with `events.resync` too (ADR 0031): the State starts from
//!   the Daemon's live Runs and Needs you items, and the stream continues
//!   from that state with nothing lost between.
//! - **The reads the view needs.** `run.list` of the live Runs each time
//!   [`Wanted::live_runs`] changes, and of a filter each time
//!   [`Wanted::listing`] changes; `run.progress` for each Run of
//!   [`Wanted::progress`]: read at once, then followed with `after` until
//!   the Run leaves the set, when the read is cancelled, or its progress
//!   is the last; `run.report` of [`Wanted::report`], which a live Run
//!   answers at its end, cancelled once another Run or none is wanted;
//!   and `run.usage` each time [`Wanted::usage`] changes.
//! - **A person's calls** ([`Call`]): each is sent on the Connection as it
//!   is made, and its answer reported; one made while no Daemon is reached
//!   is not sent, and one whose Connection closes before its answer fails.
//!
//! The Daemon's token is read from the runtime file at each connection,
//! held in memory only, sent only in `hello`, and never logged: the
//! protocol's `DaemonToken` prints `[REDACTED]`, and no `hello` is logged.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::pin::Pin;
use std::time::Duration;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use goodfetch_protocol::checkpoint::{RUN_PAUSE, RUN_RESUME, RunResumeParams};
use goodfetch_protocol::connection::{
    CANCEL, CancelParams, DaemonEndpoint, EngineInstance, HELLO, HelloParams, HelloResult,
    PROTOCOL_VERSION_UNSUPPORTED, ProductVersion, ProtocolVersionUnsupportedDetails, Surface,
};
use goodfetch_protocol::daemon::{self, DaemonHealth, DaemonParams, DaemonRuntimeFile};
use goodfetch_protocol::events::{
    CREDIT, CreditParams, EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventEnvelope,
    EventsSubscribeParams, LastRead, Position, RESYNC, Resync,
};
use goodfetch_protocol::foxhound::outcomes::{RUN_RETRY_FAILED, RunRetryFailedParams};
use goodfetch_protocol::jsonrpc::{
    ErrorObject, Message, Notification, Request, RequestId, Response, Version,
};
use goodfetch_protocol::needs_you::{
    NEEDS_YOU_ALLOW_CONSENT_DOMAIN, NEEDS_YOU_CHANGED, NEEDS_YOU_DENY_WALL_DOMAIN,
    NEEDS_YOU_DISMISS, NEEDS_YOU_REMOVE_REFETCH_DOMAIN, NeedsYouAllowConsentDomainParams,
    NeedsYouChange, NeedsYouDenyWallDomainParams, NeedsYouDismissParams,
    NeedsYouRemoveRefetchDomainParams,
};
use goodfetch_protocol::run::{
    Lifecycle, RUN_ABORT, RUN_AXES_CHANGED, RUN_LIST, RUN_PROGRESS_READ, RUN_REPORT, RUN_STOP,
    RUN_USAGE, RunAbortParams, RunAxes, RunId, RunIndexRow, RunList, RunListParams, RunParams,
    RunProgress, RunProgressParams, RunReport, RunUsage, RunWaitParams,
};
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::home::Home;
use crate::tasks::Tasks;
use runtime_files::RuntimeFiles;

mod runtime_files;

/// The id of the Kennel's `hello`, its first request on each Connection.
const HELLO_ID: i64 = 1;

/// The id of its `events.subscribe`, its second.
const SUBSCRIBE_ID: i64 = 2;

/// The id of its first other call, the health poll's included; each next
/// one takes the next.
const FIRST_CALL_ID: i64 = 3;

/// How often the Kennel asks the reached Daemon `daemon.health`, the one
/// poll ADR 0141 allows: slow, so that an idle Kennel costs nothing.
pub const HEALTH_EVERY: Duration = Duration::from_secs(30);

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
    /// The reached Daemon's answer to the health poll, `daemon.health`.
    Health(DaemonHealth),
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

/// A call a person makes through the Kennel, each to an operation the
/// engine admits from the `kennel` surface, person-only ones included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `usage.raise`: sets a Limit to the person's amount.
    RaiseLimit(UsageRaiseParams),
    /// `needs_you.allow_consent_domain`: the next runs accept the domain's
    /// consent walls.
    AllowConsentDomain(NeedsYouAllowConsentDomainParams),
    /// `needs_you.deny_wall_domain`: no run reads past the domain's Walls.
    DenyWallDomain(NeedsYouDenyWallDomainParams),
    /// `needs_you.remove_refetch_domain`: the domain leaves the layer of
    /// `wall.refetch_domains` that lists it.
    RemoveRefetchDomain(NeedsYouRemoveRefetchDomainParams),
    /// `needs_you.dismiss`: hides a Needs you item for every Client.
    Dismiss(NeedsYouDismissParams),
    /// `run.stop`, a Run's control: a Checkpoint, and the Run can resume.
    Stop(RunWaitParams),
    /// `run.abort`, a Run's control: no attempt in flight commits; not
    /// immediate.
    Abort(RunAbortParams),
    /// `run.pause`, a Run's control: a Checkpoint and a stop.
    Pause(RunParams),
    /// `run.resume`, a Run's control, of a Run the engine can run itself.
    Resume(RunResumeParams),
    /// `run.retry_failed`, a Run's control, of a live crawl.
    RetryFailed(RunRetryFailedParams),
}

impl Call {
    /// The operation it calls.
    pub fn method(&self) -> &'static str {
        match self {
            Self::RaiseLimit(_) => USAGE_RAISE.name,
            Self::AllowConsentDomain(_) => NEEDS_YOU_ALLOW_CONSENT_DOMAIN.name,
            Self::DenyWallDomain(_) => NEEDS_YOU_DENY_WALL_DOMAIN.name,
            Self::RemoveRefetchDomain(_) => NEEDS_YOU_REMOVE_REFETCH_DOMAIN.name,
            Self::Dismiss(_) => NEEDS_YOU_DISMISS.name,
            Self::Stop(_) => RUN_STOP.name,
            Self::Abort(_) => RUN_ABORT.name,
            Self::Pause(_) => RUN_PAUSE.name,
            Self::Resume(_) => RUN_RESUME.name,
            Self::RetryFailed(_) => RUN_RETRY_FAILED.name,
        }
    }

    /// The request of id `id` that makes it.
    fn request(&self, id: i64) -> Request {
        let method = self.method();
        match self {
            Self::RaiseLimit(params) => request(id, method, params),
            Self::AllowConsentDomain(params) => request(id, method, params),
            Self::DenyWallDomain(params) => request(id, method, params),
            Self::RemoveRefetchDomain(params) => request(id, method, params),
            Self::Dismiss(params) => request(id, method, params),
            Self::Stop(params) => request(id, method, params),
            Self::Abort(params) => request(id, method, params),
            Self::Pause(params) => request(id, method, params),
            Self::Resume(params) => request(id, method, params),
            Self::RetryFailed(params) => request(id, method, params),
        }
    }
}

/// Why a call did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFailure {
    /// The Error code the engine refused it with, or [`NO_DAEMON`] or
    /// [`CONNECTION_LOST`] when it never got an answer.
    pub code: String,
    /// What went wrong, for people: the engine's Error message, which the
    /// State shows through the Text function.
    pub message: String,
}

/// The code of a call made while no Daemon was reached, which was not
/// sent.
pub const NO_DAEMON: &str = "no_daemon";

/// The code of a call whose Connection was lost before its answer.
pub const CONNECTION_LOST: &str = "connection_lost";

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
    /// The rows of [`Wanted::listing`], as `run.list` answered.
    Listed {
        /// The listing's generation, which the answer is of.
        generation: u64,
        /// The rows, in the listing's order.
        rows: Vec<RunIndexRow>,
    },
    /// The answer to the read of [`Wanted::report`]: the Run's report, or
    /// the Error code the read failed with.
    RunReport {
        /// The Run.
        run_id: RunId,
        /// Its report, or why there is none.
        report: Result<Box<RunReport>, String>,
    },
    /// A Run's Usage, as `run.usage` answered the read of
    /// [`Wanted::usage`].
    Usage {
        /// The Run.
        run_id: RunId,
        /// What it used.
        usage: RunUsage,
    },
    /// A call made through [`Follow::calls`] was answered, or failed.
    Answered(Call, Result<(), CallFailure>),
}

/// The reads the State wants the Client to keep in step with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    /// The live Runs' rows are read again each time this changes.
    pub live_runs: u64,
    /// The Runs whose progress is read and followed.
    pub progress: BTreeSet<RunId>,
    /// The Run index's rows of a filter, read again each time this
    /// changes.
    pub listing: Option<Listing>,
    /// The Run whose report is read while it is wanted: at once for a Run
    /// that ended, at its end for a live one.
    pub report: Option<RunId>,
    /// The Run whose Usage is read, again each time this changes.
    pub usage: Option<UsageRead>,
}

/// A read of the Run index's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// Changed each time the rows are to be read again.
    pub generation: u64,
    /// The filter and order.
    pub params: RunListParams,
}

/// A read of a Run's Usage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRead {
    /// The Run.
    pub run_id: RunId,
    /// Changed each time its Usage is to be read again.
    pub generation: u64,
}

/// What the Client follows the Daemon with.
#[derive(Debug)]
pub struct Follow {
    /// The Goodfetch home, whose per-major folders hold the Daemons'
    /// runtime files.
    pub home: Home,
    /// Asks the Client to look for the Daemon again, as after a "Start
    /// Daemon", where the platform cannot watch the home.
    pub look_again: mpsc::Receiver<()>,
    /// The keep-alive setting: when on, `hello` asks the Daemon to count
    /// the Connection as a Client that keeps it alive; a change reconnects.
    pub keep_daemon_alive: watch::Receiver<bool>,
    /// How often the Client asks the reached Daemon `daemon.health`.
    pub health_every: Duration,
    /// The reads the State wants.
    pub wanted: watch::Receiver<Wanted>,
    /// The calls a person makes: each is sent on the Connection, and its
    /// answer reported as [`Report::Answered`]; one made while no Daemon
    /// is reached is not sent, and fails.
    pub calls: mpsc::Receiver<Call>,
}

/// Follows the Daemon until the Kennel stops: looks for it in the home,
/// connects, subscribes, keeps the reads in step with what is wanted, and
/// reports each change. It looks again whenever the platform's file
/// notifications report a change to a runtime file, which a Daemon that
/// starts or hands over to its major's pick makes, never by polling; and
/// when `look_again` asks. A lost Connection makes it look again at once,
/// and so does a runtime file that now names another Daemon, or a change
/// of the keep-alive setting, which `hello` carries. While connected, it
/// asks the Daemon `daemon.health` every `health_every`, and reports each
/// answer that differs from the last.
///
/// # Cancel safety
/// Dropping the future closes the Connection; nothing else is lost.
pub async fn follow(tasks: Tasks, follow: Follow, report: impl Fn(Report) + Send + Sync + 'static) {
    let mut follower = Follower {
        tasks,
        follow,
        runtime_files: RuntimeFiles::new(),
        report,
        calls_open: true,
    };
    follower.run().await;
}

/// The Client while it follows the Daemon.
struct Follower<R> {
    tasks: Tasks,
    follow: Follow,
    runtime_files: RuntimeFiles,
    report: R,
    /// Whether the calls' channel is still open.
    calls_open: bool,
}

/// Why the Client left a Connection.
#[derive(Debug)]
enum Left {
    /// The Daemon closed it.
    Lost,
    /// The runtime files name another Daemon now.
    Replaced,
    /// The keep-alive setting changed, which only a new `hello` carries.
    KeepAliveChanged,
}

impl<R: Fn(Report)> Follower<R> {
    /// Follows until the Kennel stops.
    async fn run(&mut self) {
        loop {
            // The watch covers every per-major folder before the look, so
            // no change after the look goes unreported.
            self.runtime_files
                .rewatch(&self.tasks, &self.follow.home)
                .await;
            let keep_daemon_alive = *self.follow.keep_daemon_alive.borrow_and_update();
            let opened = match self.running_daemon().await {
                None => Err(Opening::Unreachable),
                Some(runtime) => Connection::open(&runtime, keep_daemon_alive)
                    .await
                    .map(|connection| (connection, runtime)),
            };
            match opened {
                Ok((connection, runtime)) => {
                    tracing::info!(
                        product = connection.identity.product.as_str(),
                        version = %connection.identity.product_version,
                        keep_daemon_alive,
                        "reached the Daemon"
                    );
                    let mut reads = Reads::new(&mut self.follow.wanted);
                    (self.report)(Report::Connection(ConnectionChange::Connected(
                        connection.identity.clone(),
                    )));
                    let left = self
                        .serve(connection, &mut reads, &runtime, keep_daemon_alive)
                        .await;
                    tracing::info!(?left, "left the Daemon");
                    for pending in reads.pending.into_values() {
                        if let Pending::Call(call) = pending {
                            let lost = CallFailure {
                                code: CONNECTION_LOST.to_owned(),
                                message: "The Connection to the Daemon closed before it answered"
                                    .to_owned(),
                            };
                            (self.report)(Report::Answered(call, Err(lost)));
                        }
                    }
                    continue;
                }
                Err(Opening::Refused(refusal)) => {
                    tracing::warn!(?refusal, "the Daemon refused the Kennel");
                    (self.report)(Report::Connection(ConnectionChange::Refused(refusal)));
                }
                Err(Opening::Unreachable) => {
                    (self.report)(Report::Connection(ConnectionChange::NoDaemon));
                }
            }
            if self.wait_for_a_change().await.is_none() {
                return;
            }
        }
    }

    /// Waits, with no Daemon reached, for a reason to look for it again;
    /// none once the Kennel stops. A call made meanwhile is not sent, and
    /// fails.
    async fn wait_for_a_change(&mut self) -> Option<()> {
        loop {
            tokio::select! {
                asked = self.follow.look_again.recv() => return asked,
                () = self.runtime_files.changed() => return Some(()),
                call = self.follow.calls.recv(), if self.calls_open => {
                    if let Some(call) = call {
                        let unsent = CallFailure {
                            code: NO_DAEMON.to_owned(),
                            message: "No Daemon is reached, so nothing was sent".to_owned(),
                        };
                        (self.report)(Report::Answered(call, Err(unsent)));
                    } else {
                        self.calls_open = false;
                    }
                }
            }
        }
    }

    /// The running Daemon's runtime file.
    async fn running_daemon(&self) -> Option<DaemonRuntimeFile> {
        let home = self.follow.home.clone();
        self.tasks.blocking(move || home.running_daemon()).await
    }

    /// Serves `connection`, opened with the runtime file `reached` and
    /// the keep-alive setting `keep_daemon_alive`, until the Client
    /// leaves it: reports the subscription's events and the reads'
    /// answers, keeps the reads in step with what is wanted, renews the
    /// subscription's credits and polls the Daemon's health.
    async fn serve(
        &mut self,
        mut connection: Connection,
        reads: &mut Reads,
        reached: &DaemonRuntimeFile,
        keep_daemon_alive: bool,
    ) -> Left {
        // The first poll comes one period after `hello`, not at once.
        let mut polls = tokio::time::interval(self.follow.health_every);
        polls.reset();
        polls.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut health = Health::default();
        let mut setting_followed = true;
        let mut wanted_followed = true;
        let (applied, mut renewals) = watch::channel(0_u64);
        let mut credited = 0_u64;
        let first = reads.first.take().unwrap_or_default();
        if connection.keep_in_step(reads, &first).await.is_err() {
            return Left::Lost;
        }
        loop {
            let written = tokio::select! {
                line = connection.lines.next_line() => {
                    let Ok(Some(line)) = line else {
                        return Left::Lost;
                    };
                    let message = serde_json::from_str::<Message>(&line);
                    if let Ok(Message::Response(response)) = &message
                        && let Some(reading) = health.answered(response)
                    {
                        (self.report)(Report::Connection(ConnectionChange::Health(reading)));
                    }
                    connection.read(message, reads, &applied, &self.report).await
                }
                _ = polls.tick() => {
                    let id = reads.next_id();
                    send(&mut connection.writer, &health.ask(id)).await
                }
                () = self.runtime_files.changed() => {
                    self.runtime_files.rewatch(&self.tasks, &self.follow.home).await;
                    let found = self.running_daemon().await;
                    if found.is_some_and(|found| !same_daemon(&found, reached)) {
                        return Left::Replaced;
                    }
                    Ok(())
                }
                changed = self.follow.keep_daemon_alive.changed(), if setting_followed => {
                    // A closed channel keeps the setting the Connection has.
                    setting_followed = changed.is_ok();
                    if setting_followed
                        && *self.follow.keep_daemon_alive.borrow_and_update() != keep_daemon_alive
                    {
                        return Left::KeepAliveChanged;
                    }
                    Ok(())
                }
                changed = self.follow.wanted.changed(), if wanted_followed => {
                    // A closed channel keeps the reads the Connection has.
                    wanted_followed = changed.is_ok();
                    if wanted_followed {
                        let now = self.follow.wanted.borrow_and_update().clone();
                        connection.keep_in_step(reads, &now).await
                    } else {
                        Ok(())
                    }
                }
                call = self.follow.calls.recv(), if self.calls_open => {
                    if let Some(call) = call {
                        let id = reads.next_id();
                        tracing::info!(method = call.method(), "a person's call");
                        let sent = send(&mut connection.writer, &call.request(id)).await;
                        reads.pending.insert(id, Pending::Call(call));
                        sent
                    } else {
                        self.calls_open = false;
                        Ok(())
                    }
                }
                changed = renewals.changed() => {
                    if changed.is_err() {
                        return Left::Lost;
                    }
                    let total = *renewals.borrow_and_update();
                    if total - credited >= RENEWAL {
                        let renewed = total - credited;
                        credited = total;
                        connection.credit(renewed).await
                    } else {
                        Ok(())
                    }
                }
            };
            if written.is_err() {
                return Left::Lost;
            }
        }
    }
}

/// Whether two runtime files name the same Daemon: a Daemon's token may
/// change while it runs, its process and endpoint do not.
fn same_daemon(one: &DaemonRuntimeFile, other: &DaemonRuntimeFile) -> bool {
    one.pid == other.pid && one.endpoint == other.endpoint
}

/// The health poll of one Connection.
#[derive(Debug, Default)]
struct Health {
    /// The id of the last request sent, which no other request reuses.
    asked: Option<i64>,
    /// The last answer reported.
    last: Option<DaemonHealth>,
}

impl Health {
    /// The next `daemon.health` request, with the Connection's next id.
    fn ask(&mut self, id: i64) -> Request {
        self.asked = Some(id);
        request(id, daemon::HEALTH.name, &DaemonParams {})
    }

    /// The reading `response` gives, when it answers the last request and
    /// differs from the last reading.
    fn answered(&mut self, response: &Response) -> Option<DaemonHealth> {
        if Some(&response.id) != self.asked.map(RequestId::Number).as_ref() {
            return None;
        }
        let reading = serde_json::from_value::<DaemonHealth>(response.result.clone()).ok()?;
        if self.last.as_ref() == Some(&reading) {
            return None;
        }
        tracing::info!(
            live = reading.live,
            ready = reading.ready,
            unready = ?reading.unready,
            "the Daemon's health"
        );
        self.last = Some(reading.clone());
        Some(reading)
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
    /// Opens the Connection to the Daemon of `runtime` with `hello`,
    /// asking it to stay alive when `keep_daemon_alive` is on, then
    /// subscribes to its events, starting from a resync.
    async fn open(runtime: &DaemonRuntimeFile, keep_daemon_alive: bool) -> Result<Self, Opening> {
        let (reader, mut writer) = connect(&runtime.endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut lines = BufReader::new(reader).lines();
        send(&mut writer, &hello_request(runtime, keep_daemon_alive))
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

    /// Handles one message of the Daemon.
    async fn read(
        &mut self,
        message: Result<Message, serde_json::Error>,
        reads: &mut Reads,
        applied: &watch::Sender<u64>,
        report: &impl Fn(Report),
    ) -> io::Result<()> {
        match message {
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
                Some(id) => self.answered(reads, id, Err(failure.error), report).await,
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
        answer: Result<Value, ErrorObject>,
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
                    Err(error) => tracing::warn!(code = error.data.code, "run.list failed"),
                }
                Ok(())
            }
            Some(Pending::Listed(generation)) => {
                match answer.map(serde_json::from_value::<RunList>) {
                    Ok(Ok(list)) => report(Report::Listed {
                        generation,
                        rows: list.runs,
                    }),
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "a run.list answer the Kennel cannot read");
                    }
                    Err(error) => tracing::warn!(code = error.data.code, "run.list failed"),
                }
                Ok(())
            }
            Some(Pending::Report(run)) => {
                if let Some((reported, reading)) = &mut reads.report
                    && *reported == run
                {
                    *reading = None;
                }
                let answer = answer.map_err(|error| error.data.code).and_then(|value| {
                    serde_json::from_value::<Box<RunReport>>(value).map_err(|error| {
                        tracing::warn!(%error, run_id = %run, "a run.report answer the Kennel cannot read");
                        "invalid_run_report".to_owned()
                    })
                });
                report(Report::RunReport {
                    run_id: run,
                    report: answer,
                });
                Ok(())
            }
            Some(Pending::Usage(run)) => {
                match answer.map(serde_json::from_value::<RunUsage>) {
                    Ok(Ok(usage)) => report(Report::Usage { run_id: run, usage }),
                    Ok(Err(error)) => {
                        tracing::warn!(%error, run_id = %run, "a run.usage answer the Kennel cannot read");
                    }
                    Err(error) => {
                        tracing::warn!(code = error.data.code, run_id = %run, "run.usage failed")
                    }
                }
                Ok(())
            }
            Some(Pending::Call(call)) => {
                let answer = answer.map(drop).map_err(|error| CallFailure {
                    code: error.data.code,
                    message: error.message,
                });
                if let Err(failure) = &answer {
                    tracing::warn!(
                        method = call.method(),
                        code = failure.code,
                        "a person's call failed"
                    );
                }
                report(Report::Answered(call, answer));
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
                    Err(error) => {
                        tracing::warn!(code = error.data.code, run_id = %run, "run.progress failed");
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
        if wanted.listing != reads.listing {
            reads.listing.clone_from(&wanted.listing);
            if let Some(listing) = &wanted.listing {
                let id = reads.next_id();
                send(
                    &mut self.writer,
                    &request(id, RUN_LIST.name, &listing.params),
                )
                .await?;
                reads
                    .pending
                    .insert(id, Pending::Listed(listing.generation));
            }
        }
        if wanted.usage != reads.usage {
            reads.usage.clone_from(&wanted.usage);
            if let Some(usage) = &wanted.usage {
                let id = reads.next_id();
                let params = RunParams {
                    run_id: usage.run_id.clone(),
                };
                send(&mut self.writer, &request(id, RUN_USAGE.name, &params)).await?;
                reads
                    .pending
                    .insert(id, Pending::Usage(usage.run_id.clone()));
            }
        }
        if wanted.report.as_ref() != reads.report.as_ref().map(|(run, _)| run) {
            if let Some((_, Some(id))) = reads.report.take() {
                reads.pending.remove(&id);
                let params = CancelParams {
                    id: RequestId::Number(id),
                };
                send(&mut self.writer, &notification(CANCEL, &params)).await?;
            }
            reads.report = None;
            if let Some(run) = &wanted.report {
                let id = reads.next_id();
                let params = RunParams {
                    run_id: run.clone(),
                };
                send(&mut self.writer, &request(id, RUN_REPORT.name, &params)).await?;
                reads.pending.insert(id, Pending::Report(run.clone()));
                reads.report = Some((run.clone(), Some(id)));
            }
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
    /// The [`Wanted::listing`] last read.
    listing: Option<Listing>,
    /// The [`Wanted::usage`] last read.
    usage: Option<UsageRead>,
    /// The Run whose report is read, with the id of the call that reads
    /// it until it answers.
    report: Option<(RunId, Option<i64>)>,
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
            listing: first.listing.clone(),
            usage: first.usage.clone(),
            report: None,
            first: Some(first),
        }
    }

    /// The Connection's next request id, which no other request takes.
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
    Listed(u64),
    Report(RunId),
    Usage(RunId),
    Progress(RunId),
    /// A person's call.
    Call(Call),
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

/// The Kennel's `hello` to the Daemon of `runtime`, which carries the
/// keep-alive flag only when `keep_daemon_alive` is on.
fn hello_request(runtime: &DaemonRuntimeFile, keep_daemon_alive: bool) -> Request {
    let hello = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Kennel,
        form: None,
        token: Some(runtime.token.expose().to_owned()),
        needs: Some(Product::Collie),
        product_version: kennel_version(),
        log_level: None,
        client_surface: None,
        keep_daemon_alive,
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
