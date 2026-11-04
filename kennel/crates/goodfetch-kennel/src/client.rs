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
//!   `run.usage` each time [`Wanted::usage`] changes; `dataset.query`
//!   each time [`Wanted::records`] changes, a query no longer wanted
//!   cancelled; and `config.explain` each time [`Wanted::explain`] changes.
//! - **The Resource governor's readings** (`engine.system_info`) only while
//!   [`Wanted::system_info`] asks: the subscription is cancelled and taken
//!   again after the last event read, with or without them, so the
//!   Connection keeps one subscription and loses no event.
//! - **A person's calls** ([`Call`]): each is sent on the Connection as it
//!   is made, and its answer reported; one made while no Daemon is reached
//!   is not sent, and one whose Connection closes before its answer fails.
//!   Add Seeds streams its seeds to `run.add_seeds` within the credits the
//!   call grants, and reports the counts of seeds added and not added.
//!
//! Its parts: this module follows the Daemon from one Connection to the
//! next; `connection` is one Connection, its subscription and its reads;
//! `seeds` streams Add Seeds; `wire` frames the messages and opens the
//! socket; `runtime_files` watches the Daemons' runtime files. A report
//! goes to a [`Reports`] sink, which may make the Client wait while it is
//! full: the Client then reads nothing more from the Connection.
//!
//! The Daemon's token is read from the runtime file at each connection,
//! held in memory only, sent only in `hello`, and never logged: the
//! protocol's `DaemonToken` prints `[REDACTED]`, and no `hello` is logged.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::checkpoint::{RUN_PAUSE, RUN_RESUME, RunResumeParams};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{self, DaemonHealth, DaemonParams, DaemonRuntimeFile};
use goodfetch_protocol::events::Resync;
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::RUN_ADD_SEEDS;
use goodfetch_protocol::foxhound::outcomes::{RUN_RETRY_FAILED, RunRetryFailedParams};
use goodfetch_protocol::foxhound::sql_tab::{DatasetQueryParams, DatasetQueryResult};
use goodfetch_protocol::governor::SystemInfo;
use goodfetch_protocol::jsonrpc::{Message, Request, RequestId, Response};
use goodfetch_protocol::needs_you::{
    NEEDS_YOU_ALLOW_CONSENT_DOMAIN, NEEDS_YOU_DENY_WALL_DOMAIN, NEEDS_YOU_DISMISS,
    NEEDS_YOU_REMOVE_REFETCH_DOMAIN, NeedsYouAllowConsentDomainParams, NeedsYouChange,
    NeedsYouDenyWallDomainParams, NeedsYouDismissParams, NeedsYouRemoveRefetchDomainParams,
};
use goodfetch_protocol::run::{
    RUN_ABORT, RUN_STOP, RunAbortParams, RunAxes, RunId, RunIndexRow, RunListParams, RunParams,
    RunProgress, RunReport, RunUsage, RunWaitParams,
};
use goodfetch_protocol::settings::SettingsExplanation;
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::home::Home;
use crate::reports::Reports;
use crate::tasks::Tasks;
use connection::{Connection, Opening, Pending, Reads};
use runtime_files::RuntimeFiles;
use wire::{request, send};

mod connection;
mod runtime_files;
mod seeds;
mod wire;

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
    /// `run.add_seeds`: adds seeds to a live crawl, from the details
    /// window; its counts are reported as [`Report::SeedsAdded`] before
    /// its answer.
    AddSeeds(SeedsToAdd),
}

/// The seeds a person adds to a live crawl.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedsToAdd {
    /// The crawl's Run.
    pub run_id: RunId,
    /// The seeds, in order.
    pub seeds: Vec<Seed>,
}

/// What Add Seeds did: how many seeds the crawl added, and how many it
/// did not, by reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeedsAdded {
    /// The seeds added.
    pub added: u64,
    /// The seeds not added, counted by their reason as the wire names it,
    /// such as `duplicate`.
    pub not_added: BTreeMap<String, u64>,
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
            Self::AddSeeds(_) => RUN_ADD_SEEDS.name,
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
        generation: Generation,
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
    /// The answer to the query of [`Wanted::records`]: its result, or why
    /// there is none.
    Queried {
        /// The query's generation, which the answer is of.
        generation: Generation,
        /// Its result, or why there is none.
        result: Result<DatasetQueryResult, CallFailure>,
    },
    /// The answer to the read of [`Wanted::explain`]: the settings in
    /// force and where each comes from, or why there are none.
    Explained {
        /// The read's generation, which the answer is of.
        generation: Generation,
        /// The settings, or why there are none.
        result: Result<Box<SettingsExplanation>, CallFailure>,
    },
    /// The Resource governor's latest reading, while
    /// [`Wanted::system_info`] asks for them.
    SystemInfo(Box<SystemInfo>),
    /// What a [`Call::AddSeeds`] did, reported before its answer.
    SeedsAdded {
        /// The crawl's Run.
        run_id: RunId,
        /// The counts.
        added: SeedsAdded,
    },
    /// A call made through [`Follow::calls`] was answered, or failed.
    Answered(Call, Result<(), CallFailure>),
}

/// The reads the State wants the Client to keep in step with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    /// The live Runs' rows are read again each time this changes.
    pub live_runs: Generation,
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
    /// The query of a Run's Records, read again each time this changes; a
    /// query no longer wanted is cancelled.
    pub records: Option<Query>,
    /// The settings in force, read with `config.explain` again each time
    /// this generation changes.
    pub explain: Option<Generation>,
    /// Whether the subscription takes the Resource governor's readings,
    /// `engine.system_info`.
    pub system_info: bool,
}

/// How many times a read was asked for again: a read is made again each
/// time its generation changes, and an answer names the generation it is
/// of, so an answer to a read no longer wanted is told apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    /// The generation after this one.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Moves on to the next generation: the read is to be made again.
    pub fn bump(&mut self) {
        *self = self.next();
    }
}

/// A query of the Runs' Records, through `dataset.query`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// Changed each time the query is to be read again.
    pub generation: Generation,
    /// The statement, its Runs and its row limit.
    pub params: DatasetQueryParams,
}

/// A read of the Run index's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// Changed each time the rows are to be read again.
    pub generation: Generation,
    /// The filter and order.
    pub params: RunListParams,
}

/// A read of a Run's Usage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRead {
    /// The Run.
    pub run_id: RunId,
    /// Changed each time its Usage is to be read again.
    pub generation: Generation,
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
pub async fn follow(tasks: Tasks, follow: Follow, report: impl Reports<Report>) {
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

impl<R: Reports<Report>> Follower<R> {
    /// Follows until the Kennel stops.
    async fn run(&mut self) {
        loop {
            // The watch covers every per-major folder before the look, so
            // no change after the look goes unreported.
            self.runtime_files
                .rewatch(&self.tasks, &self.follow.home)
                .await;
            let keep_daemon_alive = *self.follow.keep_daemon_alive.borrow_and_update();
            let system_info = self.follow.wanted.borrow().system_info;
            let opened = match self.running_daemon().await {
                None => Err(Opening::Unreachable),
                Some(runtime) => Connection::open(&runtime, keep_daemon_alive, system_info)
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
                    self.report
                        .report(Report::Connection(ConnectionChange::Connected(
                            connection.identity.clone(),
                        )))
                        .await;
                    let left = self
                        .serve(connection, &mut reads, &runtime, keep_daemon_alive)
                        .await;
                    tracing::info!(?left, "left the Daemon");
                    for pending in reads.pending.into_values() {
                        let call = match pending {
                            Pending::Call(call) => call,
                            Pending::Seeds(stream) => stream.call,
                            _ => continue,
                        };
                        let lost = CallFailure {
                            code: CONNECTION_LOST.to_owned(),
                            message: "The Connection to the Daemon closed before it answered"
                                .to_owned(),
                        };
                        self.report.report(Report::Answered(call, Err(lost))).await;
                    }
                    continue;
                }
                Err(Opening::Refused(refusal)) => {
                    tracing::warn!(?refusal, "the Daemon refused the Kennel");
                    self.report
                        .report(Report::Connection(ConnectionChange::Refused(refusal)))
                        .await;
                }
                Err(Opening::Unreachable) => {
                    self.report
                        .report(Report::Connection(ConnectionChange::NoDaemon))
                        .await;
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
                        self.report.report(Report::Answered(call, Err(unsent))).await;
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
                        self.report
                            .report(Report::Connection(ConnectionChange::Health(reading)))
                            .await;
                    }
                    connection.read(message, reads, &self.report).await
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
                        connection.call(reads, call).await
                    } else {
                        self.calls_open = false;
                        Ok(())
                    }
                }
                changed = connection.credits.renewals.changed() => {
                    if changed.is_err() {
                        return Left::Lost;
                    }
                    let total = *connection.credits.renewals.borrow_and_update();
                    let renewed = total - connection.credits.credited;
                    if renewed >= RENEWAL {
                        connection.credits.credited = total;
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
