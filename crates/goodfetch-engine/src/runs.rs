//! The engine's Runs (ADRs 0002, 0003, 0029, 0031, 0036, 0098 and 0102): a
//! registry, an actor of the engine's owner, that starts each Run in its
//! own folder (the per-user, per-major data folder's, or the one
//! `run_state.location` names), takes the writer's lock on the lock file
//! beside its state and the Run's first run epoch, records it in run state
//! and in the Run index, returns the Run a start's Run ID already names, and
//! ends each Run once, with its Run report: `finished` by `run.finish` or by
//! its driver, `stopped` by an abort. Every axis change rewrites the Run's
//! Run index row, then is published as `run.axes_changed`, a lossless event
//! that a full subscriber's queue makes wait (ADR 0024).
//!
//! An abort of a Run no engine runs takes its writer role (ADR 0031): the
//! writer lock, then the next run epoch, then the end. When the lock is
//! held, the abort goes to the owner the Run index names, and when that
//! owner does not take it, the lock is retried with backoff, since a
//! reader's test may hold it for a moment (the `writer` module), before the
//! abort fails as busy. `runs reindex` rebuilds the Run index from the run
//! folders ([`reindex`]).
//!
//! A Run's driver, such as Foxhound's crawl, holds a [`Run`]: its run
//! state's writer, its clock, its cancellation, its tasks and its page
//! bodies. On a Daemon, each Run belongs to the Connection that started it,
//! whose end stops it (ADR 0078).
//!
//! Size: one registry actor, its handles and the commands it answers, kept
//! whole so every change to a Run's state passes through one loop.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{ProductVersion, StoppedBy};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::page::BodyHandle;
use goodfetch_protocol::run::{
    Completeness, Lifecycle, PointInTime, Presence, RunAxes, RunId, RunIndexRow, RunKind,
    RunReport, RunStarted, StartInput, StartedAs, StopReason, Tags, Verdict,
};
use goodfetch_protocol::settings::SettingLimit;
use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

pub use self::reindex::{Reindexed, reindex};
pub use self::writer::{Writer, test_writer};

use self::folder::{
    Found, RunRow, canonical_inputs, input_differences, point_in_time, read_found, read_report,
    text, write_end,
};
use self::takeover::TakeOver;
use self::writer::{LockError, take_lock};
use crate::body;
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::events::Events;
use crate::home::{Home, RunStateLocation};
use crate::run_index;
use crate::run_state::{RunState, RunStateOptions, RunStateTables};
use crate::settings::Settings;
use crate::tasks::TaskOwner;

/// A Run's state, in its folder.
const RUN_FILE: &str = "run.sqlite";

/// Who a Connection acts for: the OS user on a single-user install (ADR
/// 0102), the only caller until accounts arrive (area 12).
const CALLER: &str = "os_user";

/// How many commands wait for the registry before their senders wait.
const COMMANDS: usize = 256;

/// What the registry starts from.
pub(crate) struct RunsSetup {
    /// The Goodfetch home.
    pub(crate) home: Home,
    /// The engine's Product version, which names its per-major folder.
    pub(crate) version: ProductVersion,
    /// Where the run folders are.
    pub(crate) location: RunStateLocation,
    /// How run files open.
    pub(crate) run_state: RunStateOptions,
    /// The tables the composition adds to every run file.
    pub(crate) tables: Vec<RunStateTables>,
    /// The engine's clock.
    pub(crate) clock: Clock,
    /// The engine's entropy, from which Run IDs draw.
    pub(crate) entropy: Entropy,
    /// The engine's event stream, which each axis change reaches.
    pub(crate) events: Events,
    /// The engine's settings, which each Run resolves its own under.
    pub(crate) settings: Settings,
    /// The engine's Product, which a Run must not need more than for the
    /// engine to take its writer role (ADR 0098).
    pub(crate) product: Product,
    /// Where other engines reach this one, which the Run index names as
    /// the owner of its live Runs: a Daemon's endpoint, none on an Attached
    /// engine.
    pub(crate) endpoint: Option<DaemonEndpoint>,
    /// How a live action reaches the owner the Run index names.
    pub(crate) forwarder: Arc<dyn Forwarder>,
}

/// How a live action reaches the engine that owns a Run (ADR 0031).
pub(crate) trait Forwarder: Send + Sync {
    /// Asks the engine at `owner` to abort `run_id` as its owner, its
    /// blocking work under `tasks`, and answers the Run report it answered;
    /// `None` when it did not answer, answered `run_not_owned`, or failed.
    fn abort<'a>(
        &'a self,
        owner: &'a DaemonEndpoint,
        run_id: &'a RunId,
        tasks: &'a TaskOwner,
    ) -> Pin<Box<dyn Future<Output = Option<RunReport>> + Send + 'a>>;
}

/// Writes a new Run's first rows, beside the engine's own, in the
/// transaction that records the Run.
pub type Initialize = Box<dyn FnOnce(&Connection) -> rusqlite::Result<()> + Send>;

/// Who ends a Run that finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndedBy {
    /// `run.finish` ends it at once: a Run its program drives.
    Finish,
    /// Its driver ends it once its work is done, and `run.finish` waits for
    /// that: a crawl.
    Driver,
}

/// What a start asks for.
pub struct Start {
    /// The Run ID the caller chose, or none for a generated one.
    pub run_id: Option<RunId>,
    /// The Run's tags.
    pub tags: Tags,
    /// What kind of Run it is.
    pub kind: RunKind,
    /// The Product the Run needs from its start.
    pub needs: Product,
    /// The inputs a later start under the same Run ID must match, each
    /// named by the input it is.
    pub inputs: BTreeMap<StartInput, Value>,
    /// Who ends it.
    pub ended_by: EndedBy,
    /// Its first rows of the composition's tables.
    pub initialize: Option<Initialize>,
    /// The version of the Operator policy its settings resolved under,
    /// which its Run report names (ADR 0028).
    pub policy_version: Option<String>,
    /// The settings it runs with that weaken a guarantee or switch off a
    /// costly control, which its Run report is marked with (ADR 0028).
    pub weakened_by: Vec<SettingLimit>,
}

/// What a start found.
pub enum Started {
    /// It began a new Run, which its driver now drives.
    New(Run),
    /// Its Run ID names a Run of the same inputs, returned as it is.
    Existing(RunStarted),
}

/// Why a start failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StartError {
    /// The Run ID names a Run of other inputs.
    #[error("the Run ID `{run_id}` names a Run of other inputs")]
    Conflict {
        /// The Run ID.
        run_id: RunId,
        /// Each input that differs.
        differences: Vec<StartInput>,
    },
    /// Another engine holds the Run's writer lock.
    #[error("another engine holds the writer lock of `{run_id}`")]
    InUse {
        /// The Run ID.
        run_id: RunId,
        /// The Run's folder.
        folder: PathBuf,
    },
    /// The Run's state cannot be read, created or written.
    #[error("the state of `{run_id}` is unavailable: {error}")]
    Unavailable {
        /// The Run ID.
        run_id: RunId,
        /// The Run's folder.
        folder: PathBuf,
        /// What failed.
        error: String,
    },
    /// The registry has stopped.
    #[error("the engine's Runs have stopped")]
    Stopped,
}

/// Why a Run could not be finished or aborted.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunError {
    /// No Run has this Run ID.
    #[error("no Run has the Run ID `{0}`")]
    NotFound(RunId),
    /// Another engine held the Run's writer lock through every retry, and
    /// the owner the Run index names did not take the action.
    #[error("another engine holds the writer lock of `{run_id}`")]
    Busy {
        /// The Run ID.
        run_id: RunId,
        /// The Run's folder.
        folder: PathBuf,
    },
    /// The action came forwarded to this engine as the Run's owner, and
    /// this engine does not own it.
    #[error("this engine does not own the Run `{0}`")]
    NotOwned(RunId),
    /// The Run needs a Product this engine is not, so it does not take its
    /// writer role (ADR 0098).
    #[error("the Run `{run_id}` needs {needs:?}")]
    NotInProduct {
        /// The Run ID.
        run_id: RunId,
        /// The Product it needs.
        needs: Product,
    },
    /// The Run's state cannot be read or written.
    #[error("the state of `{run_id}` is unavailable: {error}")]
    Unavailable {
        /// The Run ID.
        run_id: RunId,
        /// The Run's folder.
        folder: PathBuf,
        /// What failed.
        error: String,
    },
    /// The registry has stopped.
    #[error("the engine's Runs have stopped")]
    Stopped,
}

/// What an abort waits for: a Run this engine ends, or one whose writer
/// role it takes.
enum Waiting {
    Report(watch::Receiver<Option<RunReport>>),
    TakeOver(oneshot::Receiver<Result<RunReport, RunError>>),
}

/// How a Run's driver ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Its work is done: `finished`.
    Finished,
    /// An error in the engine ended it: `failed`.
    Failed,
}

/// Why a Run ends.
#[derive(Clone, Copy, Debug)]
enum End {
    Finished,
    Failed,
    Stopped(StopReason),
}

enum Command {
    Start(
        Start,
        Option<ConnectionId>,
        oneshot::Sender<Result<Started, StartError>>,
    ),
    Finish(
        RunId,
        oneshot::Sender<Result<watch::Receiver<Option<RunReport>>, RunError>>,
    ),
    Abort(
        RunId,
        StoppedBy,
        Forwarded,
        oneshot::Sender<Result<Waiting, RunError>>,
    ),
    End(RunId, End),
    Uses(RunId, Product),
    Ended(RunId, RunReport),
    StopAll(oneshot::Sender<Vec<watch::Receiver<Option<RunReport>>>>),
    StopConnection(
        ConnectionId,
        oneshot::Sender<Vec<watch::Receiver<Option<RunReport>>>>,
    ),
    LiveCount(oneshot::Sender<usize>),
    Folder(RunId, oneshot::Sender<PathBuf>),
}

/// Whether an abort came forwarded from another engine, which counts on
/// this one to own the Run (ADR 0031).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarded {
    /// A Client asked this engine.
    No,
    /// Another engine forwarded it to this one as the Run's owner.
    Yes,
}

/// The Connection of a Daemon a Run belongs to (ADR 0078): its end stops
/// the Runs it started.
pub(crate) type ConnectionId = u64;

/// The engine's Runs: a handle to its registry, which any task may clone.
#[derive(Clone, Debug)]
pub struct Runs {
    commands: mpsc::Sender<Command>,
    settings: Settings,
    connection: Option<ConnectionId>,
}

/// A live Run, as its driver holds it.
#[derive(Clone)]
pub struct Run {
    id: RunId,
    folder: PathBuf,
    state: RunState,
    clock: Clock,
    owner: Arc<TaskOwner>,
    runs: Runs,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Run")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Run {
    /// Its Run ID.
    pub fn id(&self) -> &RunId {
        &self.id
    }

    /// Its run state's writer.
    pub fn state(&self) -> &RunState {
        &self.state
    }

    /// The engine's clock.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// Cancelled when the Run stops: its driver stops at its next safe
    /// point.
    pub fn cancellation(&self) -> &CancellationToken {
        self.owner.token()
    }

    /// Its owner, whose tasks and blocking work stop with the Run.
    pub fn owner(&self) -> &TaskOwner {
        &self.owner
    }

    /// Starts `future` as one of the Run's tasks.
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.owner.spawn(future);
    }

    /// Ends the Run as its driver says, once: its tasks stop, and its Run
    /// report is written and given to whoever waits for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the end was sent still ends the Run.
    pub async fn end(&self, ending: Ending) {
        let end = match ending {
            Ending::Finished => End::Finished,
            Ending::Failed => End::Failed,
        };
        let _ = self
            .runs
            .commands
            .send(Command::End(self.id.clone(), end))
            .await;
    }

    /// Stores `bytes` as one of the Run's page bodies, a file in its folder
    /// named by their digest, and answers its handle, which a Client on the
    /// same machine reads directly and any other through `page.body`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a temporary file beside the bodies,
    /// never a partial body.
    pub async fn store_body(&self, bytes: Vec<u8>) -> std::io::Result<BodyHandle> {
        let folder = self.folder.clone();
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (body, file) = self
            .owner
            .blocking(move || body::store(&folder, &bytes))
            .await?;
        Ok(BodyHandle {
            run_id: self.id.clone(),
            body,
            path: file.display().to_string(),
            size,
        })
    }

    /// Records that the Run uses an entry of `product`'s, so that from now
    /// on it needs the larger of its Product and `product` (ADR 0098).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still records it.
    pub async fn uses(&self, product: Product) {
        let _ = self
            .runs
            .commands
            .send(Command::Uses(self.id.clone(), product))
            .await;
    }
}

impl Runs {
    /// Starts the registry as a task of `owner`, each Run's tasks under a
    /// child of it.
    pub(crate) fn start_registry(setup: RunsSetup, owner: &TaskOwner) -> Self {
        let (commands, mailbox) = mpsc::channel(COMMANDS);
        let runs = Self {
            commands,
            settings: setup.settings.clone(),
            connection: None,
        };
        let registry = Registry {
            setup,
            owner: owner.child(),
            ending_owner: owner.child(),
            live: BTreeMap::new(),
            ended: BTreeMap::new(),
            runs: runs.downgrade(),
        };
        owner.spawn(registry.run(mailbox));
        runs
    }

    fn downgrade(&self) -> mpsc::WeakSender<Command> {
        self.commands.downgrade()
    }

    /// The engine's settings, which a start resolves its Run's under.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Begins a Run, or returns the one its Run ID names.
    ///
    /// # Errors
    ///
    /// As [`StartError`] says.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the start was sent still starts the Run.
    pub async fn start(&self, start: Start) -> Result<Started, StartError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Start(start, self.connection, answer))
            .await
            .map_err(|_| StartError::Stopped)?;
        answered.await.map_err(|_| StartError::Stopped)?
    }

    /// Lets the Run `run_id` finish, and answers its Run report once it
    /// ended: a Run its program drives ends at once, `finished` and
    /// `complete`; one its driver ends ends when its driver says.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future ends only the wait.
    pub async fn finish(&self, run_id: RunId) -> Result<RunReport, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Finish(run_id, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        let report = answered.await.map_err(|_| RunError::Stopped)??;
        wait_for(report).await
    }

    /// Stops the Run `run_id` at its next safe point, `stopped` and
    /// `incomplete`, and answers its Run report; an ended Run's report is
    /// answered as it is. A Run whose engine died is stopped by this one,
    /// which takes its writer role; a Run another engine writes is stopped
    /// by that engine, the owner the Run index names. A `forwarded` abort
    /// stops only a Run this engine owns.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID, `Busy` when another engine
    /// keeps its writer lock and its owner did not stop it, `NotOwned` for a
    /// forwarded abort of a Run this engine does not own, `NotInProduct`
    /// when the Run needs a Product this engine is not, `Unavailable` when
    /// its state cannot be read or written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the abort was sent still stops the Run.
    pub async fn abort(
        &self,
        run_id: RunId,
        by: StoppedBy,
        forwarded: Forwarded,
    ) -> Result<RunReport, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Abort(run_id, by, forwarded, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        match answered.await.map_err(|_| RunError::Stopped)?? {
            Waiting::Report(report) => wait_for(report).await,
            Waiting::TakeOver(taken) => taken.await.map_err(|_| RunError::Stopped)?,
        }
    }

    /// The folder of the Run `run_id`, whether or not it exists.
    ///
    /// # Errors
    ///
    /// `Stopped` when the registry has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn folder(&self, run_id: RunId) -> Result<PathBuf, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Folder(run_id, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)
    }

    /// The same Runs, each Run started through them belonging to the
    /// Daemon's Connection `connection`.
    pub(crate) fn on_connection(&self, connection: ConnectionId) -> Self {
        Self {
            connection: Some(connection),
            ..self.clone()
        }
    }

    /// Stops every live Run the Connection `connection` started, as aborted
    /// by its Client (ADR 0078), and waits for their ends; the other
    /// Connections' Runs go on.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still stops them.
    pub(crate) async fn stop_connection(&self, connection: ConnectionId) {
        let (answer, answered) = oneshot::channel();
        if self
            .commands
            .send(Command::StopConnection(connection, answer))
            .await
            .is_err()
        {
            return;
        }
        for report in answered.await.unwrap_or_default() {
            let _ = wait_for(report).await;
        }
    }

    /// How many Runs are live; none once the registry has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn live_count(&self) -> usize {
        let (answer, answered) = oneshot::channel();
        if self
            .commands
            .send(Command::LiveCount(answer))
            .await
            .is_err()
        {
            return 0;
        }
        answered.await.unwrap_or(0)
    }

    /// Stops every live Run as aborted by its Client, as an Attached engine
    /// does when its Connection ends (ADR 0078), and waits for their ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still stops them.
    pub(crate) async fn stop_all(&self) {
        let (answer, answered) = oneshot::channel();
        if self.commands.send(Command::StopAll(answer)).await.is_err() {
            return;
        }
        for report in answered.await.unwrap_or_default() {
            let _ = wait_for(report).await;
        }
    }
}

/// A Run's folder as the Run index records it: relative to the per-major
/// folder `per_major` when inside it, absolute otherwise.
fn location(per_major: &Path, folder: &Path) -> String {
    folder
        .strip_prefix(per_major)
        .unwrap_or(folder)
        .to_string_lossy()
        .replace('\\', "/")
}

async fn wait_for(mut report: watch::Receiver<Option<RunReport>>) -> Result<RunReport, RunError> {
    let ended = report
        .wait_for(Option::is_some)
        .await
        .map_err(|_| RunError::Stopped)?;
    Ok(ended.clone().expect("invariant: the wait ends on a report"))
}

/// What the registry keeps of a live Run; its [`Run`] goes to its end once
/// it ends, so the end is the last to hold its writer.
struct Live {
    id: RunId,
    run: Option<Run>,
    folder: PathBuf,
    kind: RunKind,
    needs: Product,
    tags: Tags,
    inputs: String,
    ended_by: EndedBy,
    started_at: PointInTime,
    policy_version: Option<String>,
    weakened_by: Vec<SettingLimit>,
    report: watch::Sender<Option<RunReport>>,
    writer: Arc<TaskOwner>,
    lock: Arc<File>,
    connection: Option<ConnectionId>,
}

struct Registry {
    setup: RunsSetup,
    owner: TaskOwner,
    ending_owner: TaskOwner,
    live: BTreeMap<RunId, Live>,
    ended: BTreeMap<RunId, RunReport>,
    runs: mpsc::WeakSender<Command>,
}

impl Registry {
    async fn run(mut self, mut mailbox: mpsc::Receiver<Command>) {
        while let Some(command) = mailbox.recv().await {
            match command {
                Command::Start(start, connection, answer) => {
                    let started = self.start(start, connection).await;
                    let _ = answer.send(started);
                }
                Command::Finish(run_id, answer) => {
                    let _ = answer.send(self.finish(&run_id).await);
                }
                Command::Abort(run_id, by, forwarded, answer) => {
                    let _ = answer.send(self.abort(&run_id, by, forwarded).await);
                }
                Command::End(run_id, end) => {
                    let _ = self.end_or_report(&run_id, end).await;
                }
                Command::Uses(run_id, product) => self.uses(&run_id, product).await,
                Command::Ended(run_id, report) => {
                    if let Some(live) = self.live.remove(&run_id) {
                        // The lock goes before the report, so whoever the
                        // report wakes finds the Run's writer gone.
                        let Live {
                            report: waiters,
                            lock,
                            ..
                        } = live;
                        drop(lock);
                        waiters.send_replace(Some(report.clone()));
                    }
                    self.ended.insert(run_id, report);
                }
                Command::StopAll(answer) => {
                    let ids: Vec<RunId> = self.live.keys().cloned().collect();
                    let _ = answer.send(self.stop_by_client(ids).await);
                }
                Command::StopConnection(connection, answer) => {
                    let ids: Vec<RunId> = self
                        .live
                        .values()
                        .filter(|live| live.connection == Some(connection))
                        .map(|live| live.id.clone())
                        .collect();
                    let _ = answer.send(self.stop_by_client(ids).await);
                }
                Command::LiveCount(answer) => {
                    let _ = answer.send(self.live.len());
                }
                Command::Folder(run_id, answer) => {
                    let _ = answer.send(self.folder(&run_id));
                }
            }
        }
        self.ending_owner.wait().await;
        self.owner.wait().await;
    }

    /// Stops each of `ids` as aborted by its Client, and answers the waits
    /// for their reports.
    async fn stop_by_client(&mut self, ids: Vec<RunId>) -> Vec<watch::Receiver<Option<RunReport>>> {
        let mut reports = Vec::new();
        for run_id in ids {
            let end = End::Stopped(StopReason::Abort {
                by: StoppedBy::Client,
            });
            if let Ok(report) = self.end_or_report(&run_id, end).await {
                reports.push(report);
            }
        }
        reports
    }

    fn folder(&self, run_id: &RunId) -> PathBuf {
        let runs = match &self.setup.location {
            RunStateLocation::PerMajor => self.setup.home.runs(&self.setup.version),
            RunStateLocation::Folder(folder) => folder.clone(),
        };
        runs.join(run_id.as_str())
    }

    /// The Run's folder as the Run index records it: relative to the
    /// per-major folder when inside it.
    fn location(&self, folder: &Path) -> String {
        location(&self.setup.home.per_major(&self.setup.version), folder)
    }

    fn now(&self) -> PointInTime {
        point_in_time(&self.setup.clock)
    }

    /// A lowercase ULID from the clock and the entropy (ADR 0059).
    fn generated_id(&mut self) -> RunId {
        let milliseconds = self
            .setup
            .clock
            .wall()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        let random = (u128::from(self.setup.entropy.next_u64()) << 64
            | u128::from(self.setup.entropy.next_u64()))
            & ((1_u128 << 80) - 1);
        let ulid = ulid::Ulid::from_parts(milliseconds, random);
        RunId::parse(&ulid.to_string().to_lowercase())
            .expect("invariant: a lowercase ULID is a Run ID")
    }

    async fn start(
        &mut self,
        start: Start,
        connection: Option<ConnectionId>,
    ) -> Result<Started, StartError> {
        let run_id = match &start.run_id {
            Some(run_id) => run_id.clone(),
            None => self.generated_id(),
        };
        let inputs = canonical_inputs(&start.inputs);
        if let Some(found) = self.existing(&run_id).await? {
            let mut differences = Vec::new();
            if found.needs != start.needs {
                differences.push(StartInput::Product);
            }
            if found.kind != start.kind {
                differences.push(StartInput::Kind);
            }
            if found.caller != CALLER {
                differences.push(StartInput::Caller);
            }
            differences.extend(input_differences(&found.inputs, &inputs));
            if !differences.is_empty() {
                return Err(StartError::Conflict {
                    run_id,
                    differences,
                });
            }
            return Ok(Started::Existing(RunStarted {
                run_id,
                started_as: StartedAs::Existing,
                lifecycle: found.lifecycle,
                needs: found.needs,
            }));
        }
        self.begin(run_id, start, inputs, connection).await
    }

    /// The Run of `run_id` this engine runs or ran, or whose state its
    /// folder holds.
    async fn existing(&self, run_id: &RunId) -> Result<Option<Found>, StartError> {
        if let Some(live) = self.live.get(run_id) {
            return Ok(Some(Found {
                kind: live.kind,
                needs: live.needs,
                caller: CALLER.to_owned(),
                inputs: live.inputs.clone(),
                lifecycle: Lifecycle::Running,
            }));
        }
        let folder = self.folder(run_id);
        let file = folder.join(RUN_FILE);
        let vfs = self.setup.run_state.vfs.clone();
        let found = self
            .owner
            .blocking(move || read_found(&file, vfs.as_deref()))
            .await;
        found.map_err(|error| StartError::Unavailable {
            run_id: run_id.clone(),
            folder,
            error,
        })
    }

    /// Takes the lock of the Run of `run_id` in `folder`, which no other
    /// engine may hold while it runs there.
    async fn lock(&self, run_id: &RunId, folder: &Path) -> Result<File, StartError> {
        let lock_folder = folder.to_path_buf();
        self.owner
            .blocking(move || take_lock(&lock_folder))
            .await
            .map_err(|error| match error {
                LockError::Held => StartError::InUse {
                    run_id: run_id.clone(),
                    folder: folder.to_path_buf(),
                },
                LockError::Io(error) => StartError::Unavailable {
                    run_id: run_id.clone(),
                    folder: folder.to_path_buf(),
                    error,
                },
            })
    }

    async fn begin(
        &mut self,
        run_id: RunId,
        start: Start,
        inputs: String,
        connection: Option<ConnectionId>,
    ) -> Result<Started, StartError> {
        let folder = self.folder(&run_id);
        let unavailable = |error: String| StartError::Unavailable {
            run_id: run_id.clone(),
            folder: folder.clone(),
            error,
        };
        let lock = self.lock(&run_id, &folder).await?;
        let writer = Arc::new(self.owner.child());
        let state = RunState::open(
            folder.join(RUN_FILE),
            &self.setup.run_state,
            self.setup.tables.clone(),
            1,
            &writer,
        )
        .await
        .map_err(|error| unavailable(error.to_string()))?;
        let started_at = self.now();
        let row = RunRow {
            run_id: run_id.clone(),
            kind: start.kind,
            needs: start.needs,
            epoch: 1,
            inputs: inputs.clone(),
            started_at: started_at.clone(),
            policy_version: start.policy_version.clone(),
            weakened_by: start.weakened_by.clone(),
        };
        let tags = start.tags.clone();
        let initialize = start.initialize;
        state
            .write(move |connection| {
                row.insert(connection)?;
                for (position, tag) in tags.iter().enumerate() {
                    connection.execute(
                        "INSERT INTO tags (position, tag) VALUES (?1, ?2)",
                        (i64::try_from(position).unwrap_or(i64::MAX), tag.as_str()),
                    )?;
                }
                match initialize {
                    Some(initialize) => initialize(connection),
                    None => Ok(()),
                }
            })
            .await
            .map_err(|error| unavailable(error.to_string()))?;
        let live = Live {
            id: run_id.clone(),
            run: Some(Run {
                id: run_id.clone(),
                folder: folder.clone(),
                state,
                clock: self.setup.clock,
                owner: Arc::new(self.owner.child()),
                runs: Runs {
                    commands: self
                        .runs
                        .upgrade()
                        .expect("invariant: the registry runs while a handle sends to it"),
                    settings: self.setup.settings.clone(),
                    connection,
                },
            }),
            folder,
            kind: start.kind,
            needs: start.needs,
            tags: start.tags,
            inputs,
            ended_by: start.ended_by,
            started_at,
            policy_version: start.policy_version,
            weakened_by: start.weakened_by,
            report: watch::Sender::new(None),
            writer,
            lock: Arc::new(lock),
            connection,
        };
        let row = self.index_row(&live, Lifecycle::Running, None);
        self.write_index(row).await;
        let run = live.run.clone().expect("invariant: a Run begins live");
        self.live.insert(run_id.clone(), live);
        self.setup
            .events
            .publish_axes(RunAxes {
                run_id,
                lifecycle: Lifecycle::Running,
                verdict: None,
                completeness: None,
            })
            .await;
        Ok(Started::New(run))
    }

    fn index_row(
        &self,
        live: &Live,
        lifecycle: Lifecycle,
        report: Option<&RunReport>,
    ) -> RunIndexRow {
        RunIndexRow {
            run_id: live.id.clone(),
            location: self.location(&live.folder),
            kind: live.kind,
            needs: live.needs,
            tags: live.tags.clone(),
            lifecycle,
            verdict: report.map(|report| report.verdict),
            completeness: report.map(|report| report.completeness),
            started_at: live.started_at.clone(),
            ended_at: report.map(|report| report.ended_at.clone()),
            owner: match report {
                Some(_) => None,
                None => self.setup.endpoint.clone(),
            },
            presence: Presence::Present,
        }
    }

    async fn write_index(&self, row: RunIndexRow) {
        let path = self.setup.home.run_index(&self.setup.version);
        if let Err(error) = self
            .owner
            .blocking(move || run_index::write(&path, &row))
            .await
        {
            tracing::warn!(%error, "the Run index row was not written; `runs reindex` restores it");
        }
    }

    async fn finish(
        &mut self,
        run_id: &RunId,
    ) -> Result<watch::Receiver<Option<RunReport>>, RunError> {
        match self.live.get(run_id).map(|live| live.ended_by) {
            Some(EndedBy::Driver) => Ok(self.live[run_id].report.subscribe()),
            Some(EndedBy::Finish) => self.end_or_report(run_id, End::Finished).await,
            None => self.report_of(run_id).await,
        }
    }

    /// Ends the live Run `run_id` with `end`, once, or answers the report
    /// of an ended one.
    async fn end_or_report(
        &mut self,
        run_id: &RunId,
        end: End,
    ) -> Result<watch::Receiver<Option<RunReport>>, RunError> {
        let Some(row) = self
            .live
            .get(run_id)
            .map(|live| self.index_row(live, Lifecycle::Running, None))
        else {
            return self.report_of(run_id).await;
        };
        let clock = self.setup.clock;
        let live = self
            .live
            .get_mut(run_id)
            .expect("invariant: the Run was live a line above");
        let receiver = live.report.subscribe();
        if let Some(run) = live.run.take() {
            let closing = Closing {
                run,
                writer: Arc::clone(&live.writer),
                lock: Arc::clone(&live.lock),
                started_at: live.started_at.clone(),
                policy_version: live.policy_version.clone(),
                weakened_by: live.weakened_by.clone(),
                clock,
                end,
            };
            let index = self.setup.home.run_index(&self.setup.version);
            let registry = self.runs.clone();
            let events = self.setup.events.clone();
            let run_id = run_id.clone();
            self.ending_owner.spawn(async move {
                let report = closing.end().await;
                let ended = RunIndexRow {
                    lifecycle: report.lifecycle,
                    verdict: Some(report.verdict),
                    completeness: Some(report.completeness),
                    ended_at: Some(report.ended_at.clone()),
                    owner: None,
                    ..row
                };
                if let Err(error) = run_index::write(&index, &ended) {
                    tracing::warn!(%error, "the Run index row was not written; `runs reindex` restores it");
                }
                events
                    .publish_axes(RunAxes {
                        run_id: run_id.clone(),
                        lifecycle: report.lifecycle,
                        verdict: Some(report.verdict),
                        completeness: Some(report.completeness),
                    })
                    .await;
                if let Some(registry) = registry.upgrade() {
                    let _ = registry.send(Command::Ended(run_id, report)).await;
                }
            });
        }
        Ok(receiver)
    }

    /// The report of a Run this engine ended, or whose state its folder
    /// holds.
    async fn report_of(
        &self,
        run_id: &RunId,
    ) -> Result<watch::Receiver<Option<RunReport>>, RunError> {
        if let Some(report) = self.ended.get(run_id) {
            return Ok(watch::Sender::new(Some(report.clone())).subscribe());
        }
        let file = self.folder(run_id).join(RUN_FILE);
        let vfs = self.setup.run_state.vfs.clone();
        let report = self
            .owner
            .blocking(move || read_report(&file, vfs.as_deref()))
            .await;
        match report {
            Ok(Some(report)) => Ok(watch::Sender::new(Some(report)).subscribe()),
            Ok(None) | Err(_) => Err(RunError::NotFound(run_id.clone())),
        }
    }

    /// Stops a live Run of this engine's, answers the report of one it
    /// ended, or, unless the abort came forwarded, takes the writer role of
    /// one no engine of its own runs, in a task of its own, since a held
    /// lock is retried for about a second.
    async fn abort(
        &mut self,
        run_id: &RunId,
        by: StoppedBy,
        forwarded: Forwarded,
    ) -> Result<Waiting, RunError> {
        if self.live.contains_key(run_id) || self.ended.contains_key(run_id) {
            return self
                .end_or_report(run_id, End::Stopped(StopReason::Abort { by }))
                .await
                .map(Waiting::Report);
        }
        if forwarded == Forwarded::Yes {
            return Err(RunError::NotOwned(run_id.clone()));
        }
        let folder = self.folder(run_id);
        let take_over = TakeOver {
            run_id: run_id.clone(),
            location: self.location(&folder),
            folder,
            by,
            product: self.setup.product,
            run_state: self.setup.run_state.clone(),
            tables: self.setup.tables.clone(),
            clock: self.setup.clock,
            entropy: self.setup.entropy.fork(),
            index: self.setup.home.run_index(&self.setup.version),
            events: self.setup.events.clone(),
            forwarder: Arc::clone(&self.setup.forwarder),
            owner: Arc::new(self.ending_owner.child()),
        };
        let (answer, answered) = oneshot::channel();
        self.ending_owner.spawn(async move {
            let _ = answer.send(take_over.abort().await);
        });
        Ok(Waiting::TakeOver(answered))
    }

    async fn uses(&mut self, run_id: &RunId, product: Product) {
        let Some(live) = self.live.get_mut(run_id) else {
            return;
        };
        let Some(run) = &live.run else {
            return;
        };
        if live.needs.contains(product) {
            return;
        }
        live.needs = product;
        let needs = text(&product);
        let state = run.state.clone();
        if let Err(error) = state
            .write(move |connection| {
                connection
                    .execute("UPDATE run SET needs = ?1", [needs])
                    .map(drop)
            })
            .await
        {
            tracing::warn!(%error, "the Product the Run needs was not recorded");
        }
        let row = self.index_row(&self.live[run_id], Lifecycle::Running, None);
        self.write_index(row).await;
    }
}

/// A Run on its way to its end.
struct Closing {
    run: Run,
    writer: Arc<TaskOwner>,
    lock: Arc<File>,
    started_at: PointInTime,
    policy_version: Option<String>,
    weakened_by: Vec<SettingLimit>,
    clock: Clock,
    end: End,
}

impl Closing {
    /// Stops the Run's tasks, writes its end into run state, closes its
    /// writer and frees its lock.
    async fn end(self) -> RunReport {
        let Self {
            run,
            writer,
            lock,
            started_at,
            policy_version,
            weakened_by,
            clock,
            end,
        } = self;
        run.owner.stop().await;
        let (lifecycle, stop_reason, completeness) = match end {
            End::Finished => (Lifecycle::Finished, None, Completeness::Complete),
            End::Failed => (Lifecycle::Failed, None, Completeness::Incomplete),
            End::Stopped(reason) => (Lifecycle::Stopped, Some(reason), Completeness::Incomplete),
        };
        let report = RunReport {
            run_id: run.id.clone(),
            lifecycle,
            stop_reason,
            verdict: Verdict::Verified,
            completeness,
            started_at,
            ended_at: point_in_time(&clock),
            policy_version,
            weakened_by,
        };
        let written = report.clone();
        let state = run.state.clone();
        drop(run);
        if let Err(error) = state
            .write(move |connection| write_end(connection, &written))
            .await
        {
            tracing::warn!(%error, "the Run's end was not written to its run state");
        }
        drop(state);
        writer.wait().await;
        drop(lock);
        report
    }
}

mod folder;
pub mod outcome;
mod reindex;
mod takeover;
mod writer;
