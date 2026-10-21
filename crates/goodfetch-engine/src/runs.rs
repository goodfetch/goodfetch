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
//! folders ([`reindex`]). A [`RunReader`] reads a Run's committed state
//! without its writer lock, following this engine's writer while it works.
//!
//! A Checkpoint of a Run (ADRs 0002, 0102 and 0116) is written on
//! `run.checkpoint`, every `checkpoint.interval` of its clock, and after the
//! end of every stop: an abort, a pause, a migration. A resume takes the
//! writer role of an interrupted Run from its run file or from a Checkpoint
//! made its run file, at the run epoch past both (ADR 0003), refuses a Run
//! bound to another machine (ADR 0056), and hands the Run to the driver of
//! its kind again. Stops and Checkpoints are announced on the event stream
//! (RT-14).
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
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{
    RUN_ABORTING, RUN_CHECKPOINTING, RUN_MIGRATING, RUN_STOPPING, RunAborting, RunBinding,
    RunCheckpointing, RunMigrating, RunStopping,
};
use goodfetch_protocol::connection::{ProductVersion, StoppedBy};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::page::BodyHandle;
use goodfetch_protocol::run::{
    Checkpoint, Completeness, Lifecycle, PointInTime, Presence, RunAxes, RunId, RunIndexRow,
    RunKind, RunOwner, RunReport, RunStarted, StartInput, StartedAs, StopReason, Tags, Verdict,
};
use goodfetch_protocol::settings::SettingLimit;
use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

pub use self::reader::RunReader;
pub use self::reindex::{Reindexed, reindex};
pub use self::writer::{Writer, test_writer};

use self::folder::{
    Found, RunRow, canonical_inputs, input_differences, point_in_time, read_found, read_report,
    text, write_end,
};
use self::reader::Following;
use self::takeover::TakeOver;
use self::writer::{LockError, take_lock};
use crate::admission::{Admission, Gates};
use crate::body;
use crate::checkpoint::{self, CHECKPOINT_FILE, WriteError};
use crate::client::Client;
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::events::Events;
use crate::home::{Home, RunStateLocation};
use crate::run_index;
use crate::run_state::{self, RunState, RunStateError, RunStateOptions, RunStateTables};
use crate::settings::{Resolved, RunSettings, Settings};
use crate::tasks::TaskOwner;

/// A Run's state, in its folder.
const RUN_FILE: &str = "run.sqlite";

/// The identity of the machine a Goodfetch home is on, in that home: what a
/// Run bound to its machine records (ADR 0056).
const MACHINE_FILE: &str = "machine-id";

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
    /// The gate in admission the composition passes in, if any.
    pub(crate) gates: Option<Arc<dyn Gates>>,
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
    /// the owner of its live Runs: a Daemon's endpoint, or an Attached
    /// engine's private endpoint; none when it has neither.
    pub(crate) reached_at: Option<RunOwner>,
    /// How a live action reaches the owner the Run index names.
    pub(crate) forwarder: Arc<dyn Forwarder>,
    /// The driver of each kind of Run that has one, which a resume starts
    /// again.
    pub(crate) drivers: BTreeMap<RunKind, Driver>,
}

/// What drives a kind of Run, such as Foxhound's crawl's delivery, given
/// the Run, the Client of the Connection that resumed it and its settings:
/// a resume starts it again on the Run's committed state.
pub type Driver =
    Arc<dyn Fn(Run, Client, RunSettings) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// How a live action reaches the engine that owns a Run (ADR 0031).
pub(crate) trait Forwarder: Send + Sync {
    /// Asks the engine at `owner` to abort `run_id` as its owner, its
    /// blocking work under `tasks`, and answers the Run report it answered;
    /// `None` when it did not answer, answered `run_not_owned`, or failed.
    fn abort<'a>(
        &'a self,
        owner: &'a RunOwner,
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
    /// What binds it to a machine or a Browser host (ADR 0056), which a
    /// resume elsewhere checks.
    pub binding: Option<RunBinding>,
    /// The digest of the Clean-up rules it holds to, which a resume keeps
    /// (ADR 0047).
    pub clean_up_digest: Option<String>,
    /// How often it writes a Checkpoint, from `checkpoint.interval`.
    pub checkpoint_interval: Option<Duration>,
    /// Its `timeout.run`, none when unset: once it passes, counted from
    /// this start, the Run stops at its next safe point (ADR 0080).
    pub timeout: Option<Duration>,
    /// Its resolved settings, which its pages and Sessions open under.
    pub settings: Option<RunSettings>,
}

/// What a resume asks for.
pub struct Resume {
    /// The Run's ID.
    pub run_id: RunId,
    /// The Checkpoint it resumes from, or none for its run file.
    pub checkpoint: Option<PathBuf>,
    /// How often it writes a Checkpoint from now on.
    pub checkpoint_interval: Option<Duration>,
    /// Its `timeout.run` from now on, none when unset, counted from the
    /// resume on a fresh clock (ADR 0080).
    pub timeout: Option<Duration>,
    /// Its resolved settings, which its pages and Sessions open under.
    pub settings: Option<RunSettings>,
}

/// What a resume found.
pub enum Resumed {
    /// It took the next run epoch of an interrupted Run, which goes on.
    New {
        /// The Run.
        run: Run,
        /// Its kind, whose driver drives it.
        kind: RunKind,
        /// The Product it needs.
        needs: Product,
    },
    /// The Run is live in this engine, or finished: answered as it is.
    Existing(RunStarted),
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
    /// Another engine holds the Run's writer lock.
    #[error("another engine holds the writer lock of `{run_id}`")]
    InUse {
        /// The Run ID.
        run_id: RunId,
        /// The Run's folder.
        folder: PathBuf,
    },
    /// The Run's state or Checkpoint is of a format version this engine
    /// cannot read (ADR 0066).
    #[error("the state of `{run_id}` is {format} {found}, which this engine cannot read")]
    Format {
        /// The Run ID.
        run_id: RunId,
        /// The format's name.
        format: &'static str,
        /// The file's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
    /// The Run is bound to another machine (ADR 0056).
    #[error("the Run `{0}` is bound to another machine")]
    BoundToMachine(RunId),
    /// A Checkpoint could not be written (ADR 0116); no file is left.
    #[error("a Checkpoint of `{run_id}` could not be written: {message}")]
    WriteFailed {
        /// The Run ID.
        run_id: RunId,
        /// The Run's folder.
        folder: PathBuf,
        /// Why, as `io::ErrorKind`'s name in `snake_case`.
        reason: &'static str,
        /// The OS's own name for it, when known.
        os_code: Option<&'static str>,
        /// What failed.
        message: String,
    },
    /// The registry has stopped.
    #[error("the engine's Runs have stopped")]
    Stopped,
}

impl RunError {
    /// The failure of a Run's state or Checkpoint as read or written for
    /// `run_id` in `folder`.
    fn of_state(run_id: &RunId, folder: &Path, error: RunStateError) -> Self {
        match error {
            RunStateError::Format {
                format,
                found,
                compatibility,
            } => Self::Format {
                run_id: run_id.clone(),
                format,
                found,
                compatibility,
            },
            other => Self::Unavailable {
                run_id: run_id.clone(),
                folder: folder.to_owned(),
                error: other.to_string(),
            },
        }
    }

    /// The failure of a Checkpoint written for `run_id` in `folder`.
    fn of_write(run_id: &RunId, folder: &Path, error: WriteError) -> Self {
        match error {
            WriteError::Storage {
                reason,
                os_code,
                message,
            } => Self::WriteFailed {
                run_id: run_id.clone(),
                folder: folder.to_owned(),
                reason,
                os_code,
                message,
            },
            WriteError::Source(error) => Self::of_state(run_id, folder, error),
        }
    }
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
    /// It reached the safe point a stop asked for: `stopped`, for the
    /// reason [`Run::stopping`] gave.
    Stopped(StopReason),
}

/// Why a Run ends.
#[derive(Clone, Copy, Debug)]
enum End {
    Finished,
    Failed,
    Stopped(StopReason),
}

/// What the registry is asked. The channel's blocks hold 32 commands
/// each, of the largest one's size, so the two that carry a Run's whole
/// start or report box it: each is sent once per Run.
enum Command {
    Start(
        Box<Start>,
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
    Ended(RunId, Box<RunReport>),
    StopAll(oneshot::Sender<Vec<watch::Receiver<Option<RunReport>>>>),
    StopConnection(
        ConnectionId,
        oneshot::Sender<Vec<watch::Receiver<Option<RunReport>>>>,
    ),
    LiveCount(oneshot::Sender<usize>),
    Folder(RunId, oneshot::Sender<PathBuf>),
    Live(RunId, oneshot::Sender<Result<Run, RunError>>),
    Checkpoint(
        RunId,
        Option<PathBuf>,
        oneshot::Sender<Result<Checkpoint, RunError>>,
    ),
    Checkpointed(RunId, Box<Checkpoint>),
    Stop(
        RunId,
        StopReason,
        Option<PathBuf>,
        oneshot::Sender<Result<watch::Receiver<Option<RunReport>>, RunError>>,
    ),
    Resume(
        Box<Resume>,
        Option<ConnectionId>,
        oneshot::Sender<Result<Resumed, RunError>>,
    ),
    Machine(oneshot::Sender<Result<String, RunError>>),
    Reader(RunId, oneshot::Sender<RunReader>),
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
#[derive(Clone)]
pub struct Runs {
    commands: mpsc::Sender<Command>,
    settings: Settings,
    home: Arc<Home>,
    connection: Option<ConnectionId>,
    drivers: Arc<BTreeMap<RunKind, Driver>>,
}

impl std::fmt::Debug for Runs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Runs")
            .field("connection", &self.connection)
            .finish_non_exhaustive()
    }
}

/// A live Run, as its driver holds it.
#[derive(Clone)]
pub struct Run {
    id: RunId,
    folder: PathBuf,
    state: RunState,
    clock: Clock,
    gates: Option<Arc<dyn Gates>>,
    stop: Arc<watch::Sender<Option<StopReason>>>,
    owner: Arc<TaskOwner>,
    runs: Runs,
    settings: Option<Arc<RunSettings>>,
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

    /// Its folder, which holds its state and its Run trace.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The settings it was started with, when its start resolved them.
    pub fn settings(&self) -> Option<&RunSettings> {
        self.settings.as_deref()
    }

    /// The engine's clock.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// Cancelled when the Run stops at once, by an abort: its driver
    /// abandons each attempt in flight.
    pub fn cancellation(&self) -> &CancellationToken {
        self.owner.token()
    }

    /// Answers once something asks the Run to stop at its next safe point,
    /// such as its `timeout.run` passing, with the reason it stops for: its
    /// driver then admits nothing new, lets each attempt in flight commit
    /// or be abandoned under its lease, and ends it with
    /// [`Ending::Stopped`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing; a later call answers the same.
    pub async fn stopping(&self) -> StopReason {
        let mut stop = self.stop.subscribe();
        let asked = stop.wait_for(Option::is_some).await.map(|reason| *reason);
        match asked {
            Ok(Some(reason)) => reason,
            _ => std::future::pending().await,
        }
    }

    /// Starts the tasks that ask the Run to stop at its next safe point:
    /// once its `timeout` passes; and for a Run its program drives, which
    /// has nothing in flight in the engine, the one that then stops it.
    fn watch_safe_points(&self, timeout: Option<Duration>, ended_by: EndedBy) {
        if let Some(timeout) = timeout {
            let timed = self.clone();
            self.spawn(async move {
                tokio::select! {
                    () = timed.cancellation().cancelled() => {}
                    () = timed.clock.sleep(timeout) => timed.stop_at_safe_point(StopReason::Timeout),
                }
            });
        }
        if ended_by == EndedBy::Finish {
            let stopped = self.clone();
            self.spawn(async move {
                tokio::select! {
                    () = stopped.cancellation().cancelled() => {}
                    reason = stopped.stopping() => stopped.end(Ending::Stopped(reason)).await,
                }
            });
        }
    }

    /// Asks the Run to stop at its next safe point for `reason`; the first
    /// reason asked stays.
    fn stop_at_safe_point(&self, reason: StopReason) {
        self.stop.send_if_modified(|stop| {
            let unset = stop.is_none();
            if unset {
                *stop = Some(reason);
            }
            unset
        });
    }

    /// Its admission under `settings`: the gate in admission the
    /// composition passes in, opened from what its run file journaled, and
    /// its rate cap; `None` when the composition passes in no gate.
    ///
    /// # Errors
    ///
    /// When its run file cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn admission(&self, settings: &Resolved) -> Result<Option<Admission>, RunStateError> {
        let Some(gates) = self.gates.clone() else {
            return Ok(None);
        };
        let opening = settings.clone();
        let (wall, now) = (self.clock.wall(), self.clock.now());
        let gate = self
            .state
            .read(move |connection| gates.open(&opening, connection, wall, now))
            .await?;
        Ok(Some(Admission::new(gate, settings)))
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
            Ending::Stopped(reason) => End::Stopped(reason),
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

    /// The wall time now, as a point in time of the Run's (ADR 0102).
    pub fn point_in_time(&self) -> PointInTime {
        point_in_time(&self.clock)
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
            home: Arc::new(setup.home.clone()),
            connection: None,
            drivers: Arc::new(setup.drivers.clone()),
        };
        let registry = Registry {
            setup,
            owner: owner.child(),
            ending_owner: owner.child(),
            readers: Arc::new(owner.child()),
            live: BTreeMap::new(),
            ended: BTreeMap::new(),
            runs: runs.downgrade(),
            drivers: Arc::clone(&runs.drivers),
            machine: None,
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

    /// The Goodfetch home the engine runs in.
    pub fn home(&self) -> &Home {
        &self.home
    }

    /// The live Run of `run_id`, as its driver holds it.
    ///
    /// # Errors
    ///
    /// `NotFound` when no live Run has this Run ID, such as one that ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn live(&self, run_id: RunId) -> Result<Run, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Live(run_id, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)?
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
            .send(Command::Start(Box::new(start), self.connection, answer))
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

    /// Writes a Checkpoint of the Run `run_id` to `location`, or where the
    /// engine keeps its Checkpoints, its own folder, through SQLite's
    /// online backup, and answers it once its file is complete (ADRs 0002,
    /// 0102 and 0116). A live Run's is announced as `run.checkpointing`; an
    /// interrupted Run's is written under its writer lock.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID, `Busy` when another engine
    /// writes an interrupted one, `WriteFailed` when the file cannot be
    /// written, `Unavailable` when its state cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the checkpoint was sent still writes it.
    pub async fn checkpoint(
        &self,
        run_id: RunId,
        location: Option<PathBuf>,
    ) -> Result<Checkpoint, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Checkpoint(run_id, location, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)?
    }

    /// Stops the live Run `run_id` at its next safe point for `reason`, a
    /// pause or a migration, `stopped` and resumable, with a Checkpoint at
    /// `location` or where the engine keeps them, and answers its Run
    /// report, which names that Checkpoint; an ended Run's report is
    /// answered as it is.
    ///
    /// # Errors
    ///
    /// `NotFound` when this engine runs no Run of this Run ID and none
    /// ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the stop was sent still stops the Run.
    pub async fn stop(
        &self,
        run_id: RunId,
        reason: StopReason,
        location: Option<PathBuf>,
    ) -> Result<RunReport, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Stop(run_id, reason, location, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        wait_for(answered.await.map_err(|_| RunError::Stopped)??).await
    }

    /// Resumes an interrupted Run from its run file, or from the
    /// Checkpoint `resume` names, which becomes its run file here: takes its
    /// writer lock and the next run epoch, so an earlier writer commits
    /// nothing more, and keeps what its state recorded, its Clean-up rules
    /// digest included (ADRs 0002, 0003, 0047 and 0102). A Run live in this
    /// engine, or finished, is answered as it is. The Run belongs to the
    /// resuming Connection, whose driver of its kind the caller starts.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID here and no Checkpoint is
    /// given, `InUse` when another engine writes it, `Format` for a state
    /// of a version this engine cannot read, `BoundToMachine` for a Run
    /// bound to another machine, `NotInProduct` for a Run that needs a
    /// Product this engine is not, `WriteFailed` when the Checkpoint cannot
    /// be made its run file, `Unavailable` when its state cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the resume was sent still resumes the Run.
    pub async fn resume(&self, resume: Resume) -> Result<Resumed, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Resume(Box::new(resume), self.connection, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)?
    }

    /// The driver of Runs of `kind`, when the composition has one.
    pub fn driver(&self, kind: RunKind) -> Option<Driver> {
        self.drivers.get(&kind).cloned()
    }

    /// The identity of the machine this engine's Goodfetch home is on,
    /// which a Run bound to its machine records (ADR 0056): drawn once and
    /// kept in the home.
    ///
    /// # Errors
    ///
    /// `Unavailable` when the home cannot keep it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn machine(&self) -> Result<String, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Machine(answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)?
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

    /// A reader of the committed state of the Run `run_id`, which never
    /// takes its writer lock: it follows the Run while this engine's writer
    /// works it, and reads what is committed otherwise.
    ///
    /// # Errors
    ///
    /// `Stopped` when the registry has stopped; whether the Run exists is
    /// the reader's first read's to say.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn reader(&self, run_id: RunId) -> Result<RunReader, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Reader(run_id, answer))
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
    /// The last Checkpoint written of it.
    checkpoint: Option<Checkpoint>,
}

struct Registry {
    setup: RunsSetup,
    owner: TaskOwner,
    ending_owner: TaskOwner,
    /// Where readers of run state run their blocking reads.
    readers: Arc<TaskOwner>,
    live: BTreeMap<RunId, Live>,
    ended: BTreeMap<RunId, RunReport>,
    runs: mpsc::WeakSender<Command>,
    drivers: Arc<BTreeMap<RunKind, Driver>>,
    /// The machine's identity, once read.
    machine: Option<String>,
}

impl Registry {
    async fn run(mut self, mut mailbox: mpsc::Receiver<Command>) {
        while let Some(command) = mailbox.recv().await {
            match command {
                Command::Start(start, connection, answer) => {
                    let started = self.start(*start, connection).await;
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
                        waiters.send_replace(Some(RunReport::clone(&report)));
                    }
                    self.ended.insert(run_id, *report);
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
                Command::Live(run_id, answer) => {
                    let live = self
                        .live
                        .get(&run_id)
                        .and_then(|live| live.run.clone())
                        .ok_or(RunError::NotFound(run_id));
                    let _ = answer.send(live);
                }
                Command::Checkpoint(run_id, location, answer) => {
                    self.checkpoint(run_id, location, answer).await;
                }
                Command::Checkpointed(run_id, checkpoint) => {
                    if let Some(live) = self.live.get_mut(&run_id) {
                        live.checkpoint = Some(*checkpoint);
                    }
                }
                Command::Stop(run_id, reason, location, answer) => {
                    let _ = answer.send(self.stop(&run_id, reason, location).await);
                }
                Command::Resume(resume, connection, answer) => {
                    let _ = answer.send(self.resume(*resume, connection).await);
                }
                Command::Machine(answer) => {
                    let _ = answer.send(self.machine().await);
                }
                Command::Reader(run_id, answer) => {
                    let _ = answer.send(self.reader(&run_id));
                }
            }
        }
        self.readers.wait().await;
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

    fn reader(&self, run_id: &RunId) -> RunReader {
        let following = self.live.get(run_id).map(|live| Following {
            commits: live.run.as_ref().map(|run| run.state.commits()),
            report: live.report.subscribe(),
        });
        RunReader::new(
            run_id.clone(),
            self.folder(run_id),
            self.setup.run_state.vfs.clone(),
            Arc::clone(&self.readers),
            following,
        )
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
            binding: start.binding.as_ref().map(|binding| {
                serde_json::to_string(binding).expect("invariant: a binding encodes")
            }),
            clean_up_digest: start.clean_up_digest.clone(),
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
            run: Some(self.new_run(&run_id, &folder, state, connection, start.settings)),
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
            checkpoint: None,
        };
        let row = self.index_row(&live, Lifecycle::Running, None);
        self.write_index(row).await;
        let run = live.run.clone().expect("invariant: a Run begins live");
        run.watch_safe_points(start.timeout, start.ended_by);
        self.live.insert(run_id.clone(), live);
        if let Some(interval) = start.checkpoint_interval {
            run.spawn(periodic_checkpoints(run.clone(), interval));
        }
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
                None => self.setup.reached_at.clone(),
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

    /// A handle to the registry for a Run of `connection`.
    /// The live Run `run_id` as its driver holds it, writing `state` in
    /// `folder`, under its resolved `settings`.
    fn new_run(
        &self,
        run_id: &RunId,
        folder: &Path,
        state: RunState,
        connection: Option<ConnectionId>,
        settings: Option<RunSettings>,
    ) -> Run {
        Run {
            id: run_id.clone(),
            folder: folder.to_owned(),
            state,
            clock: self.setup.clock,
            gates: self.setup.gates.clone(),
            stop: Arc::new(watch::Sender::new(None)),
            owner: Arc::new(self.owner.child()),
            runs: self.handle(connection),
            settings: settings.map(Arc::new),
        }
    }

    fn handle(&self, connection: Option<ConnectionId>) -> Runs {
        Runs {
            commands: self
                .runs
                .upgrade()
                .expect("invariant: the registry runs while a handle sends to it"),
            settings: self.setup.settings.clone(),
            home: Arc::new(self.setup.home.clone()),
            connection,
            drivers: Arc::clone(&self.drivers),
        }
    }

    /// Ends the live Run `run_id` with `end`, once, or answers the report
    /// of an ended one.
    async fn end_or_report(
        &mut self,
        run_id: &RunId,
        end: End,
    ) -> Result<watch::Receiver<Option<RunReport>>, RunError> {
        self.end_with(run_id, end, None).await
    }

    /// [`Registry::end_or_report`], a stop writing its Checkpoint to
    /// `location` rather than where the engine keeps them.
    async fn end_with(
        &mut self,
        run_id: &RunId,
        end: End,
        location: Option<PathBuf>,
    ) -> Result<watch::Receiver<Option<RunReport>>, RunError> {
        let Some(row) = self
            .live
            .get(run_id)
            .map(|live| self.index_row(live, Lifecycle::Running, None))
        else {
            return self.report_of(run_id).await;
        };
        let clock = self.setup.clock;
        let nonce = self.setup.entropy.next_u64();
        let vfs = self.setup.run_state.vfs.clone();
        let events = self.setup.events.clone();
        let stopping = match end {
            End::Stopped(cause) if self.live[run_id].run.is_some() => Some(cause),
            _ => None,
        };
        if let Some(cause) = stopping {
            if let StopReason::Abort { by } = cause {
                let aborting = RunAborting {
                    run_id: run_id.clone(),
                    by,
                };
                events
                    .publish(&RUN_ABORTING, Some(run_id.as_str()), &aborting)
                    .await;
            }
            let stopping = RunStopping {
                run_id: run_id.clone(),
                cause,
            };
            events
                .publish(&RUN_STOPPING, Some(run_id.as_str()), &stopping)
                .await;
        }
        let live = self
            .live
            .get_mut(run_id)
            .expect("invariant: the Run was live a line above");
        let receiver = live.report.subscribe();
        if let Some(run) = live.run.take() {
            let checkpoint_to = stopping.map(|_| CheckpointTo {
                destination: location.unwrap_or_else(|| live.folder.join(CHECKPOINT_FILE)),
                vfs,
                nonce,
                events: events.clone(),
            });
            let closing = Closing {
                run,
                writer: Arc::clone(&live.writer),
                lock: Arc::clone(&live.lock),
                started_at: live.started_at.clone(),
                policy_version: live.policy_version.clone(),
                weakened_by: live.weakened_by.clone(),
                clock,
                end,
                checkpoint_to,
                last_checkpoint: live.checkpoint.clone(),
            };
            let index = self.setup.home.run_index(&self.setup.version);
            let registry = self.runs.clone();
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
                    let _ = registry.send(Command::Ended(run_id, Box::new(report))).await;
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

    /// Writes a Checkpoint of `run_id` in a task of its own, which answers
    /// `answer` once its file is complete.
    async fn checkpoint(
        &mut self,
        run_id: RunId,
        location: Option<PathBuf>,
        answer: oneshot::Sender<Result<Checkpoint, RunError>>,
    ) {
        let folder = self.folder(&run_id);
        let live_epoch = self
            .live
            .get(&run_id)
            .and_then(|live| live.run.as_ref())
            .map(|run| run.state.epoch());
        if live_epoch.is_none() {
            match self.existing(&run_id).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    let _ = answer.send(Err(RunError::NotFound(run_id)));
                    return;
                }
                Err(error) => {
                    let _ = answer.send(Err(RunError::Unavailable {
                        run_id,
                        folder,
                        error: error.to_string(),
                    }));
                    return;
                }
            }
        }
        let taking = Taking {
            destination: location.unwrap_or_else(|| folder.join(CHECKPOINT_FILE)),
            run_id,
            folder,
            vfs: self.setup.run_state.vfs.clone(),
            nonce: self.setup.entropy.next_u64(),
            clock: self.setup.clock,
            events: self.setup.events.clone(),
            tasks: Arc::new(self.ending_owner.child()),
        };
        let registry = self.runs.clone();
        self.ending_owner.spawn(async move {
            let taken = taking.take(live_epoch).await;
            if let (Ok(checkpoint), Some(registry)) = (&taken, registry.upgrade()) {
                let checkpointed =
                    Command::Checkpointed(checkpoint.run_id.clone(), Box::new(checkpoint.clone()));
                let _ = registry.send(checkpointed).await;
            }
            let _ = answer.send(taken);
        });
    }

    /// Stops the live Run `run_id` for `reason` with a Checkpoint at
    /// `location`, or answers an ended one's report.
    async fn stop(
        &mut self,
        run_id: &RunId,
        reason: StopReason,
        location: Option<PathBuf>,
    ) -> Result<watch::Receiver<Option<RunReport>>, RunError> {
        let live = self.live.get(run_id).is_some_and(|live| live.run.is_some());
        if live && reason == StopReason::Migrate {
            let migrating = RunMigrating {
                run_id: run_id.clone(),
            };
            self.setup
                .events
                .publish(&RUN_MIGRATING, Some(run_id.as_str()), &migrating)
                .await;
        }
        self.end_with(run_id, End::Stopped(reason), location).await
    }

    /// The machine's identity, read from the home or drawn and kept there
    /// the first time.
    async fn machine(&mut self) -> Result<String, RunError> {
        if let Some(machine) = &self.machine {
            return Ok(machine.clone());
        }
        let root = self.setup.home.root().to_owned();
        let drawn = format!(
            "{:016x}{:016x}",
            self.setup.entropy.next_u64(),
            self.setup.entropy.next_u64()
        );
        let machine = self
            .owner
            .blocking(move || folder::machine_identity(&root.join(MACHINE_FILE), &drawn))
            .await
            .map_err(|error| RunError::Unavailable {
                run_id: RunId::parse("machine").expect("invariant: `machine` is a Run ID"),
                folder: self.setup.home.root().to_owned(),
                error: error.to_string(),
            })?;
        self.machine = Some(machine.clone());
        Ok(machine)
    }

    /// Resumes an interrupted Run, as [`Runs::resume`] says.
    async fn resume(
        &mut self,
        resume: Resume,
        connection: Option<ConnectionId>,
    ) -> Result<Resumed, RunError> {
        let Resume {
            run_id,
            checkpoint,
            checkpoint_interval,
            timeout,
            settings,
        } = resume;
        if let Some(live) = self.live.get(&run_id) {
            return Ok(Resumed::Existing(RunStarted {
                run_id,
                started_as: StartedAs::Existing,
                lifecycle: Lifecycle::Running,
                needs: live.needs,
            }));
        }
        let held = self.held(&run_id, checkpoint.as_deref()).await?;
        if let Some(RunBinding::Machine { machine }) = &held.binding
            && *machine != self.machine().await?
        {
            return Err(RunError::BoundToMachine(run_id));
        }
        if !self.setup.product.contains(held.needs) {
            return Err(RunError::NotInProduct {
                run_id,
                needs: held.needs,
            });
        }
        if held.lifecycle == Lifecycle::Finished {
            return Ok(Resumed::Existing(RunStarted {
                run_id,
                started_as: StartedAs::Existing,
                lifecycle: Lifecycle::Finished,
                needs: held.needs,
            }));
        }
        let (lock, writer, state) = self.reopen(&run_id, checkpoint).await?;
        let folder = self.folder(&run_id);
        let (read, vfs) = (folder.join(RUN_FILE), self.setup.run_state.vfs.clone());
        let recorded = self
            .owner
            .blocking(move || folder::read_recorded(&read, vfs.as_deref()))
            .await
            .map_err(|error| RunError::Unavailable {
                run_id: run_id.clone(),
                folder: folder.clone(),
                error: error.to_string(),
            })?
            .ok_or_else(|| RunError::NotFound(run_id.clone()))?;
        self.ended.remove(&run_id);
        let live = Live {
            id: run_id.clone(),
            run: Some(self.new_run(&run_id, &folder, state, connection, settings)),
            folder,
            kind: recorded.kind,
            needs: recorded.needs,
            tags: recorded.tags,
            inputs: recorded.inputs,
            ended_by: match recorded.kind {
                RunKind::Collie => EndedBy::Finish,
                RunKind::Crawl => EndedBy::Driver,
            },
            started_at: recorded.started_at,
            policy_version: recorded.policy_version,
            weakened_by: recorded.weakened_by,
            report: watch::Sender::new(None),
            writer,
            lock: Arc::new(lock),
            connection,
            checkpoint: None,
        };
        let row = self.index_row(&live, Lifecycle::Running, None);
        self.write_index(row).await;
        let run = live.run.clone().expect("invariant: a Run resumes live");
        run.watch_safe_points(timeout, live.ended_by);
        let (kind, needs) = (live.kind, live.needs);
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
        if let Some(interval) = checkpoint_interval {
            run.spawn(periodic_checkpoints(run.clone(), interval));
        }
        Ok(Resumed::New { run, kind, needs })
    }

    /// What the Checkpoint `checkpoint`, or else the run file of `run_id`
    /// here, holds of the Run, refusing a version this engine cannot read.
    async fn held(
        &self,
        run_id: &RunId,
        checkpoint: Option<&Path>,
    ) -> Result<checkpoint::Held, RunError> {
        let folder = self.folder(run_id);
        let held = if let Some(checkpoint) = checkpoint {
            let path = checkpoint.to_owned();
            self.owner
                .blocking(move || checkpoint::read(&checkpoint::open_checkpoint(&path)?, &path))
                .await
        } else {
            let found = self
                .existing(run_id)
                .await
                .map_err(|error| RunError::Unavailable {
                    run_id: run_id.clone(),
                    folder: folder.clone(),
                    error: error.to_string(),
                })?;
            if found.is_none() {
                return Err(RunError::NotFound(run_id.clone()));
            }
            let (path, vfs) = (folder.join(RUN_FILE), self.setup.run_state.vfs.clone());
            self.owner
                .blocking(move || {
                    checkpoint::read(&run_state::open_reader(&path, vfs.as_deref())?, &path)
                })
                .await
        }
        .map_err(|error| RunError::of_state(run_id, &folder, error))?;
        if held.run_id != run_id.as_str() {
            return Err(RunError::Unavailable {
                run_id: run_id.clone(),
                folder,
                error: format!("the Checkpoint holds the Run `{}`", held.run_id),
            });
        }
        Ok(held)
    }

    /// Takes the writer role of `run_id` here: its lock, the Checkpoint
    /// `checkpoint` made its run file when given, then the next run epoch,
    /// under which its writer opens and marks it running again.
    async fn reopen(
        &mut self,
        run_id: &RunId,
        checkpoint: Option<PathBuf>,
    ) -> Result<(File, Arc<TaskOwner>, RunState), RunError> {
        let folder = self.folder(run_id);
        let file = folder.join(RUN_FILE);
        let lock = self
            .lock(run_id, &folder)
            .await
            .map_err(|error| match error {
                StartError::InUse { run_id, folder } => RunError::InUse { run_id, folder },
                StartError::Unavailable {
                    run_id,
                    folder,
                    error,
                } => RunError::Unavailable {
                    run_id,
                    folder,
                    error,
                },
                StartError::Conflict { .. } | StartError::Stopped => RunError::Stopped,
            })?;
        let mut after = 0;
        if let Some(checkpoint) = checkpoint {
            // The run epoch of a run file of this Run already here, which the
            // Checkpoint replaces: the resume's goes past it too.
            if let Ok(Some(_)) = self.existing(run_id).await
                && let Ok(here) = self.held(run_id, None).await
            {
                after = here.run_epoch;
            }
            let (into, vfs) = (file.clone(), self.setup.run_state.vfs.clone());
            self.owner
                .blocking(move || checkpoint::install(&checkpoint, &into, vfs.as_deref()))
                .await
                .map_err(|error| RunError::of_write(run_id, &folder, error))?;
        }
        let (options, tables, next) = (
            self.setup.run_state.clone(),
            self.setup.tables.clone(),
            file.clone(),
        );
        let epoch = self
            .owner
            .blocking(move || run_state::next_epoch(&next, &options, &tables, after))
            .await
            .map_err(|error| RunError::of_state(run_id, &folder, error))?;
        tracing::info!(
            target: "goodfetch::lease",
            transition = "take_epoch",
            run_id = %run_id,
            run_epoch = epoch,
            "took the Run's writer role to resume it"
        );
        let writer = Arc::new(self.owner.child());
        let state = RunState::open(
            file,
            &self.setup.run_state,
            self.setup.tables.clone(),
            epoch,
            &writer,
        )
        .await
        .map_err(|error| RunError::of_state(run_id, &folder, error))?;
        state
            .write(|connection| {
                connection
                    .execute(
                        "UPDATE run SET lifecycle = 'running', stop_reason = NULL, verdict = NULL,
                         completeness = NULL, ended_at = NULL",
                        [],
                    )
                    .map(drop)
            })
            .await
            .map_err(|error| RunError::of_state(run_id, &folder, error))?;
        Ok((lock, writer, state))
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

/// Writes a Checkpoint of a Run every `interval` of its clock until it
/// stops; one that fails is logged and the next is tried (ADR 0122).
async fn periodic_checkpoints(run: Run, interval: Duration) {
    loop {
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => return,
            () = run.clock.sleep(interval) => {}
        }
        if let Err(error) = run.runs.checkpoint(run.id.clone(), None).await {
            tracing::warn!(%error, run_id = %run.id, "a periodic Checkpoint was not written");
        }
    }
}

/// A Checkpoint being written of a Run, live or interrupted.
struct Taking {
    run_id: RunId,
    folder: PathBuf,
    destination: PathBuf,
    vfs: Option<String>,
    nonce: u64,
    clock: Clock,
    events: Events,
    tasks: Arc<TaskOwner>,
}

impl Taking {
    /// Writes it: a live Run's, of `live_epoch`, announced first; an
    /// interrupted Run's under its writer lock.
    async fn take(self, live_epoch: Option<u64>) -> Result<Checkpoint, RunError> {
        if let Some(run_epoch) = live_epoch {
            let checkpointing = RunCheckpointing {
                run_id: self.run_id.clone(),
                run_epoch,
            };
            self.events
                .publish(
                    &RUN_CHECKPOINTING,
                    Some(self.run_id.as_str()),
                    &checkpointing,
                )
                .await;
        }
        let (folder, destination, vfs, nonce) = (
            self.folder.clone(),
            self.destination.clone(),
            self.vfs.clone(),
            self.nonce,
        );
        let written = self
            .tasks
            .blocking(move || {
                let _lock = match live_epoch {
                    Some(_) => None,
                    None => match take_lock(&folder) {
                        Ok(lock) => Some(lock),
                        Err(LockError::Held) => return Ok(None),
                        Err(LockError::Io(error)) => {
                            return Err(WriteError::Source(RunStateError::Sqlite {
                                path: folder,
                                message: error,
                            }));
                        }
                    },
                };
                checkpoint::write(&folder.join(RUN_FILE), vfs.as_deref(), &destination, nonce)
                    .map(Some)
            })
            .await
            .map_err(|error| RunError::of_write(&self.run_id, &self.folder, error))?;
        let Some(written) = written else {
            return Err(RunError::Busy {
                run_id: self.run_id,
                folder: self.folder,
            });
        };
        Ok(Checkpoint {
            run_id: self.run_id,
            location: self.destination.display().to_string(),
            size: written.size,
            digest: written.digest,
            run_epoch: written.run_epoch,
            journal_position: written.journal_position,
            time: point_in_time(&self.clock),
        })
    }
}

/// Where a stopping Run's Checkpoint goes, and what writes it.
struct CheckpointTo {
    destination: PathBuf,
    vfs: Option<String>,
    nonce: u64,
    events: Events,
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
    /// A stop's Checkpoint, written once its end is.
    checkpoint_to: Option<CheckpointTo>,
    /// The last Checkpoint written before.
    last_checkpoint: Option<Checkpoint>,
}

impl Closing {
    /// Stops the Run's tasks, writes its end into run state, a stop's
    /// Checkpoint after it, closes its writer and frees its lock.
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
            checkpoint_to,
            last_checkpoint,
        } = self;
        run.owner.stop().await;
        let (lifecycle, stop_reason, completeness) = match end {
            End::Finished => (Lifecycle::Finished, None, Completeness::Complete),
            End::Failed => (Lifecycle::Failed, None, Completeness::Incomplete),
            End::Stopped(reason) => (Lifecycle::Stopped, Some(reason), Completeness::Incomplete),
        };
        let mut report = RunReport {
            run_id: run.id.clone(),
            lifecycle,
            stop_reason,
            verdict: Verdict::Verified,
            completeness,
            started_at,
            ended_at: point_in_time(&clock),
            policy_version,
            weakened_by,
            checkpoint: last_checkpoint,
        };
        let written = report.clone();
        let state = run.state.clone();
        let (run_id, folder) = (run.id.clone(), run.folder.clone());
        drop(run);
        let ended = state
            .write(move |connection| write_end(connection, &written))
            .await;
        if let Err(error) = &ended {
            tracing::warn!(%error, "the Run's end was not written to its run state");
        }
        if let Some(to) = checkpoint_to
            && ended.is_ok()
        {
            let taking = Taking {
                run_id,
                destination: to.destination,
                vfs: to.vfs,
                nonce: to.nonce,
                clock,
                events: to.events,
                tasks: Arc::clone(&writer),
                folder,
            };
            match taking.take(Some(state.epoch())).await {
                Ok(checkpoint) => report.checkpoint = Some(checkpoint),
                Err(error) => {
                    tracing::warn!(%error, "the stopping Run's Checkpoint was not written; its run file resumes it");
                }
            }
        }
        drop(state);
        writer.wait().await;
        drop(lock);
        report
    }
}

mod folder;
pub mod outcome;
mod reader;
mod reindex;
mod takeover;
mod writer;
