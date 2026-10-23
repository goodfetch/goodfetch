//! The engine's Runs (ADRs 0002, 0003, 0029, 0031, 0036, 0098 and 0102): a
//! registry, an actor of the engine's owner, that starts each Run in its
//! own folder (the per-user, per-major data folder's, or the one
//! `run_state.location` names), takes the writer's lock on the lock file
//! beside its state and the Run's first run epoch, records it in run state
//! and in the Run index, returns the Run a start's Run ID already names, and
//! ends each Run once, with its Run report: `finished` by `run.finish` or by
//! its driver, `stopped` by an abort. Its Lifecycle changes only as the
//! Run's transition table ([`lifecycle`], ADR 0128) says: a late event
//! changes nothing, and a refused one answers its Error code. Every axis
//! change rewrites the Run's
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
//! bodies, and whether `run.finish` let it finish, which a keep-alive crawl
//! waits for (RT-12). The program's own status line about its Run (RT-15)
//! is kept in its run state, which a resume reads back, written into its
//! Run index row and published as `run.progress`. On a Daemon, each Run belongs to the Connection that started it,
//! whose end stops it (ADR 0078).
//!
//! Size: one registry actor, its handles and the commands it answers, kept
//! whole so every change to a Run's state passes through one loop.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::Instrument as _;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{
    RUN_ABORTING, RUN_CHECKPOINTING, RUN_MIGRATING, RUN_STOPPING, RunAborting, RunBinding,
    RunCheckpointing, RunMigrating, RunStopping,
};
use goodfetch_protocol::connection::{ProductVersion, StoppedBy};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::governor::RUN_SHARE;
use goodfetch_protocol::page::BodyHandle;
use goodfetch_protocol::politeness::POLITENESS_SHARING;
use goodfetch_protocol::run::{
    AppliedLimit, AxisReason, Checkpoint, Completeness, EndingError, Lifecycle, NotUnderstoodLimit,
    NothingDone, PointInTime, Presence, ProgressMessage, RUN_IN_USE, RUN_PROGRESS,
    RUN_STATE_DURABILITY, ReachedLimit, RunAxes, RunId, RunIndexRow, RunKind, RunOwner,
    RunProgress, RunReport, RunStarted, RunStorage, RunUsage, RunWarning, StartInput, StartedAs,
    StopReason, Tags,
};
use goodfetch_protocol::secrets::{RUN_STATE_KEY_REFERENCE, TIMEOUT_SECRET};
use goodfetch_protocol::settings::{SettingsLayer, WeakenedSetting};
use goodfetch_protocol::storage::StorageLevel;
use goodfetch_protocol::usage::Limit;
use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

pub use self::import::fuzz_run_state_import;
pub use self::reader::RunReader;
pub use self::reindex::{Reindexed, reindex};
pub use self::report::ReportPart;
pub use self::writer::{Writer, test_writer};

use self::folder::{
    Found, RunRow, canonical_inputs, input_differences, point_in_time, read_found, read_report,
    text, write_end,
};
use self::lifecycle::{InvalidRunTransition, Next, RunEvent};
use self::reader::Following;
use self::report::EndFacts;
pub(crate) use self::slices::Limits;
use self::takeover::TakeOver;
use self::writer::{LockError, take_lock};
use crate::admission::{Admission, Gates};
use crate::audit::AuditLog;
use crate::body;
use crate::checkpoint::{self, CHECKPOINT_FILE, WriteError};
use crate::client::Client;
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::events::Events;
use crate::governor::{Filesystem, Governor, Held, RunShare, StorageSample};
use crate::home::{Home, RunStateLocation, major_name};
use crate::known_certificates::KnownCertificates;
use crate::ledger::{Charged, Charging, Ledger, LedgerError, Refusal, Reservation};
use crate::metrics::Metrics;
use crate::politeness::{Holder, Politeness, PolitenessError, PolitenessOptions, PolitenessTables};
use crate::run_index;
use crate::run_state::install::InstallDatabase;
use crate::run_state::{
    self, Durability, RunState, RunStateError, RunStateOptions, RunStateTables,
};
use crate::secrets::{Carried, Keychain, Purpose, RunSecrets, SecretError, Secrets};
use crate::settings::{Resolved, RunSettings, Settings};
use crate::tasks::TaskOwner;
use crate::trace::Link;
use crate::transition;

/// A Run's state, in its folder.
const RUN_FILE: &str = "run.sqlite";

/// The identity of the machine a Goodfetch home is on, in that home: what a
/// Run bound to its machine records (ADR 0056).
const MACHINE_FILE: &str = "machine-id";

/// Who a Connection acts for: the OS user on a single-user install (ADR
/// 0102), the only caller until accounts arrive (area 12).
pub(crate) const CALLER: &str = "os_user";

/// How many commands wait for the registry before their senders wait.
const COMMANDS: usize = 256;

/// What the registry starts from.
pub(crate) struct RunsSetup {
    /// The engine's environment map, which `env:` Secret references
    /// resolve in.
    pub(crate) environment: BTreeMap<OsString, OsString>,
    /// The OS keychain, which `keychain:` Secret references resolve in.
    pub(crate) keychain: Keychain,
    /// The Goodfetch home.
    pub(crate) home: Home,
    /// The engine's Product version, which names its per-major folder.
    pub(crate) version: ProductVersion,
    /// Where the run folders are.
    pub(crate) location: RunStateLocation,
    /// How run files open.
    pub(crate) run_state: RunStateOptions,
    /// The VFS the Usage ledger and the Audit log open through: the run
    /// state's own, never the install database's, which serves only the run
    /// folders.
    pub(crate) ledger_vfs: Option<String>,
    /// The install database `run_state`'s VFS serves the run files from,
    /// in the per-install layout, kept open while the registry lives.
    #[expect(
        dead_code,
        reason = "held, never read: its VFS serves the run files while it is open"
    )]
    pub(crate) install: Option<Arc<InstallDatabase>>,
    /// The tables the composition adds to every run file.
    pub(crate) tables: Vec<RunStateTables>,
    /// The gate in admission the composition passes in, if any.
    pub(crate) gates: Option<Arc<dyn Gates>>,
    /// How the per-user Politeness file opens.
    pub(crate) politeness: PolitenessOptions,
    /// The tables the composition adds to the Politeness file.
    pub(crate) politeness_tables: Vec<PolitenessTables>,
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
    /// The runtime's metrics, which count the live Runs and their ends.
    pub(crate) metrics: Metrics,
    /// The engine's Resource governor, which each live Run joins.
    pub(crate) governor: Governor,
    /// What the composition's areas add to each Run's report (ADR 0102).
    pub(crate) report_parts: Vec<Arc<dyn ReportPart>>,
    /// The Console's address, when this engine serves one (ADR 0031): each
    /// Run's report names its page there.
    pub(crate) console: Option<String>,
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
    pub weakened: Vec<WeakenedSetting>,
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
        /// The Run, boxed as the larger variant.
        run: Box<Run>,
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
    /// A Limit that covers the Run has nothing left in its period (ADR
    /// 0041).
    #[error("a Limit on {} {} has nothing left", .0.scope.kind, .0.usage_kind)]
    LimitReached(Box<ReachedLimit>),
    /// A Limit flagged *must understand* that this engine cannot evaluate
    /// may cover the Run (ADR 0041).
    #[error("a Limit on {} cannot be evaluated by this engine", .0.scope.kind)]
    LimitNotUnderstood(Box<Limit>),
    /// The Run index refuses this engine: its format is of another major,
    /// or its lowest minor is newer than the engine (ADR 0098), so no Run
    /// starts.
    #[error("the Run index is {format} {found}, which this engine cannot use")]
    Format {
        /// The format's name.
        format: &'static str,
        /// The index's format version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
        /// The Product version to install, when its lowest minor refused
        /// this engine.
        install: Option<String>,
    },
    /// The engine drains (ADR 0049): it takes no Run.
    #[error("the engine drains and takes no Run")]
    Draining {
        /// The Run ID asked for, when the start named one.
        run_id: Option<RunId>,
    },
    /// The Run's transition table refused the start, which only a bug in
    /// Goodfetch causes (ADR 0128).
    #[error("the start of `{run_id}` was refused: {message}")]
    Internal {
        /// The Run ID.
        run_id: RunId,
        /// The refused transition.
        message: String,
    },
    /// The Secret reference a start resolves, its key-encryption key's
    /// (`run_state.key_reference`), gave no value.
    #[error(transparent)]
    Secret(SecretError),
    /// The filesystem its run file would go to is at the `critical`
    /// storage level (ADR 0122).
    #[error("the filesystem of `{}` is at the critical storage level", .0.folder.display())]
    ResourcesExhausted(Box<StorageExhausted>),
    /// The registry has stopped.
    #[error("the engine's Runs have stopped")]
    Stopped,
}

/// A filesystem a Run needs at the `critical` storage level (ADR 0122).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageExhausted {
    /// The Run.
    pub run_id: RunId,
    /// The folder on that filesystem.
    pub folder: PathBuf,
    /// What the governor found of it.
    pub filesystem: Filesystem,
}

/// The worst storage level in `sample` among the filesystems of `folders`,
/// with the first folder found on it.
fn worst<'a>(
    sample: &'a StorageSample,
    folders: [&'a PathBuf; 3],
) -> Option<(&'a PathBuf, &'a Filesystem)> {
    folders
        .into_iter()
        .filter_map(|folder| sample.of(folder).map(|filesystem| (folder, filesystem)))
        .fold(
            None,
            |worst: Option<(&PathBuf, &Filesystem)>, found| match worst {
                Some((_, kept)) if kept.level >= found.1.level => worst,
                _ => Some(found),
            },
        )
}

/// The `storage_low` warning of a Run whose worst filesystem is `storage`
/// (ADR 0122): its folder, its free space and what the Run sheds there.
fn storage_low(storage: &RunStorage) -> RunWarning {
    RunWarning {
        code: goodfetch_protocol::run::warning::STORAGE_LOW.to_owned(),
        message: format!(
            "The filesystem of `{}` had {} bytes free, below `storage.low_free`: the Run records no Run trace video, stores no new Response cache entry and skips its periodic Checkpoints there.",
            storage.folder, storage.free_bytes
        ),
        sessions: Vec::new(),
        folder: Some(storage.folder.clone()),
    }
}

/// Why a Run could not charge Usage.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChargeError {
    /// The Usage ledger refused it: a Limit has too little left, which asks
    /// the Run to stop at its next safe point, or cannot be evaluated, or
    /// the ledger could not be read or written.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// What it charged could not be journaled in its run state.
    #[error(transparent)]
    State(#[from] RunStateError),
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
        /// The Product version to install, when a shared store's lowest
        /// minor refused this engine (ADR 0098).
        install: Option<String>,
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
    /// The Run's transition table refused what was asked, which only a
    /// bug in Goodfetch causes (ADR 0128).
    #[error("an action on `{run_id}` was refused: {message}")]
    Internal {
        /// The Run ID.
        run_id: RunId,
        /// The refused transition.
        message: String,
    },
    /// The engine drains (ADR 0049): it resumes no Run.
    #[error("the engine drains and resumes no Run")]
    Draining(RunId),
    /// A filesystem the Run needs is at the `critical` storage level (ADR
    /// 0122).
    #[error("the filesystem of `{}` is at the critical storage level", .0.folder.display())]
    ResourcesExhausted(Box<StorageExhausted>),
    /// The registry has stopped.
    #[error("the engine's Runs have stopped")]
    Stopped,
}

impl RunError {
    /// What the Run's table refusing an event to `run_id` in `folder`
    /// answers: `run_in_use` when another writer holds it, an internal
    /// error, logged at `error`, when only a bug causes it.
    fn refused(run_id: &RunId, folder: &Path, refused: InvalidRunTransition) -> Self {
        let refused = refused.logged();
        match refused.refusal {
            transition::Refusal::Program(code) if code == RUN_IN_USE.name => Self::InUse {
                run_id: run_id.clone(),
                folder: folder.to_owned(),
            },
            transition::Refusal::Program(_) | transition::Refusal::Internal => Self::Internal {
                run_id: run_id.clone(),
                message: refused.to_string(),
            },
        }
    }

    /// The failure of a Run's state or Checkpoint as read or written for
    /// `run_id` in `folder`.
    fn of_state(run_id: &RunId, folder: &Path, error: RunStateError) -> Self {
        match error {
            RunStateError::Format {
                format,
                found,
                compatibility,
                install,
            } => Self::Format {
                run_id: run_id.clone(),
                format,
                found,
                compatibility,
                install,
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
    Report(watch::Receiver<Option<Ended>>),
    TakeOver(oneshot::Receiver<Result<RunReport, RunError>>),
}

/// How a Run's driver ends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Its work is done: `finished`, `complete` unless the driver gives
    /// reasons part of it was not, such as requests a site held unsent
    /// further away than the Run waits (ADR 0118) or URLs a bound or a cap
    /// refused (ADRs 0093 and 0120): the Run then takes the worst
    /// Completeness they give (ADR 0029), each listed in its report.
    Finished(Vec<CompletenessReason>),
    /// An error in the engine ended it: `failed`, the calls that end it
    /// raising `error` (ADR 0102).
    Failed(EndingError),
    /// It reached the safe point a stop asked for: `stopped`, for the
    /// reason [`Run::stopping`] gave.
    Stopped(StopReason),
}

/// A reason a finished Run's results do not cover its scope (ADRs 0029
/// and 0102): the Completeness it gives the Run, `incomplete` or
/// `unverified`, and the reason its report lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletenessReason {
    /// What it makes the Run.
    pub completeness: Completeness,
    /// Why, as the Run's report states it.
    pub reason: AxisReason,
}

impl CompletenessReason {
    /// `reason`, which leaves the Run `incomplete`.
    pub fn incomplete(reason: AxisReason) -> Self {
        Self {
            completeness: Completeness::Incomplete,
            reason,
        }
    }
}

/// Why a Run ends.
#[derive(Clone, Debug)]
enum End {
    Finished(Vec<CompletenessReason>),
    Failed(EndingError),
    Stopped(StopReason),
}

impl End {
    /// The event of the Run's table it is.
    fn event(&self) -> RunEvent {
        match self {
            Self::Finished(_) => RunEvent::Finish,
            Self::Failed(_) => RunEvent::Fail,
            Self::Stopped(reason) => match reason {
                StopReason::Abort { .. } => RunEvent::Abort,
                StopReason::Pause => RunEvent::Pause,
                StopReason::Migrate => RunEvent::Migrate,
                StopReason::Timeout => RunEvent::Timeout,
                StopReason::Limit(_) => RunEvent::Limit,
                StopReason::LimitNotUnderstood(_) => RunEvent::LimitNotUnderstood,
                StopReason::Drain => RunEvent::Drain,
                StopReason::EngineLost => RunEvent::EngineLost,
                StopReason::Resources { .. } => RunEvent::ResourcesExhausted,
            },
        }
    }
}

/// What the registry is asked. The channel's blocks hold 32 commands
/// each, of the largest one's size, so the two that carry a Run's whole
/// start or report box it: each is sent once per Run.
enum Command {
    Start(
        Box<(Start, RunSecrets, Link)>,
        Option<ConnectionId>,
        oneshot::Sender<Result<Started, StartError>>,
    ),
    Finish(
        RunId,
        Letting,
        oneshot::Sender<Result<watch::Receiver<Option<Ended>>, RunError>>,
    ),
    ProgressMessage(
        RunId,
        ProgressMessage,
        oneshot::Sender<Result<RunProgress, RunError>>,
    ),
    Abort(
        RunId,
        StoppedBy,
        Forwarded,
        oneshot::Sender<Result<Waiting, RunError>>,
    ),
    End(RunId, End),
    Uses(RunId, Product),
    Ended(RunId, Box<Ended>),
    StopAll(oneshot::Sender<Vec<watch::Receiver<Option<Ended>>>>),
    StopConnection(
        ConnectionId,
        oneshot::Sender<Vec<watch::Receiver<Option<Ended>>>>,
    ),
    LiveCount(oneshot::Sender<usize>),
    Health(oneshot::Sender<RegistryHealth>),
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
        oneshot::Sender<Result<watch::Receiver<Option<Ended>>, RunError>>,
    ),
    Resume(
        Box<(Resume, Link)>,
        Option<ConnectionId>,
        oneshot::Sender<Result<Resumed, RunError>>,
    ),
    Machine(oneshot::Sender<Result<String, RunError>>),
    SafeStop(RunId, StoppedBy, oneshot::Sender<Result<Waiting, RunError>>),
    Reader(RunId, oneshot::Sender<RunReader>),
    Drain(oneshot::Sender<Vec<watch::Receiver<Option<Ended>>>>),
    Storage(Arc<StorageSample>),
    StorageFailed(PathBuf),
}

/// What the registry answers about the engine's health (ADR 0050): whether
/// it drains, and the folder its run folders go in.
pub(crate) struct RegistryHealth {
    /// Whether the engine drains, taking no Run (ADR 0049).
    pub(crate) draining: bool,
    /// The folder of the run folders.
    pub(crate) folder: PathBuf,
    /// Whether the Resource governor is at its last resort (ADR 0040).
    pub(crate) governor_last_resort: bool,
}

/// Whether a wait for a Run's end lets it finish (ADR 0102): `run.finish`
/// does, `crawl.serve` only waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Letting {
    /// It lets the Run finish: a Run its program drives ends at once, and
    /// its driver is told, which a keep-alive crawl waits for.
    Finish,
    /// It only waits for the end.
    Wait,
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
    connection: Option<ConnectionId>,
    limits: Arc<Limits>,
    /// The Secret-provider driver, which holds the user's Audit log.
    secrets: Secrets,
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
    admitting: Arc<Admitting>,
    shared: Arc<Shared>,
    owner: Arc<TaskOwner>,
    runs: Runs,
    settings: Option<Arc<RunSettings>>,
    entropy: Arc<tokio::sync::Mutex<Entropy>>,
}

/// What every handle of a live Run shares.
struct Shared {
    /// The reason it was asked to stop at its next safe point, once asked.
    stop: watch::Sender<Option<StopReason>>,
    /// Whether `run.finish` let it finish.
    finish: watch::Sender<bool>,
    /// Its Slices of the Limits that hold it, until it ends.
    charging: tokio::sync::Mutex<Option<Charging>>,
    /// The runtime's metrics.
    metrics: Metrics,
    /// Its `tracing` span, a root of its own (ADR 0132), which each of its
    /// tasks runs in.
    span: tracing::Span,
    /// What it resolves its Secret references through.
    secrets: RunSecrets,
    /// Its share of the Resource governor's admissions.
    governor: RunShare,
    /// Its folder, which the governor samples while the Run lives (ADR
    /// 0122).
    _sampled: Held,
    /// The governor's last storage sample.
    storage: watch::Receiver<Arc<StorageSample>>,
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

    /// The runtime's metrics, which what the Run runs records in (ADR
    /// 0050).
    pub fn metrics(&self) -> &Metrics {
        &self.shared.metrics
    }

    /// Its share of the engine's Resource governor (ADR 0040), through
    /// which it takes each HTTP fetch, browser page and model call it
    /// runs, in turn with the other live Runs, up to its
    /// `governor.run_share`.
    pub fn governor(&self) -> &RunShare {
        &self.shared.governor
    }

    /// Its folder, which holds its state and its Run trace.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The settings it was started with, when its start resolved them.
    pub fn settings(&self) -> Option<&RunSettings> {
        self.settings.as_deref()
    }

    /// What it resolves its Secret references through, holding every value
    /// it resolved.
    pub fn secrets(&self) -> &RunSecrets {
        &self.shared.secrets
    }

    /// The engine's clock.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// An `Entropy` of its own for one of the Run's owners, such as its
    /// driver, forked from the Run's, which the engine's forked when the
    /// Run went live (ADR 0059).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn fork_entropy(&self) -> Entropy {
        self.entropy.lock().await.fork()
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
        let mut stop = self.shared.stop.subscribe();
        let asked = stop
            .wait_for(Option::is_some)
            .await
            .map(|reason| reason.clone());
        match asked {
            Ok(Some(reason)) => reason,
            _ => std::future::pending().await,
        }
    }

    /// Whether `run.finish` let the Run finish: a driver that waits for new
    /// work while it has none, as a keep-alive crawl does, then ends it once
    /// its work is done.
    pub fn is_let_finish(&self) -> bool {
        *self.shared.finish.borrow()
    }

    /// Answers once `run.finish` lets the Run finish.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing; a later call answers the same.
    pub async fn let_finish(&self) {
        let mut finish = self.shared.finish.subscribe();
        if finish.wait_for(|finish| *finish).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Starts the tasks that ask the Run to stop at its next safe point:
    /// once its `timeout` passes; and for a Run its program drives, which
    /// has nothing in flight in the engine, the one that then stops it.
    fn watch_safe_points(&self, timeout: Option<Duration>, ended_by: EndedBy) {
        // A commit its writer could not make stops it from its last
        // acknowledged one, as at `critical` (ADR 0122).
        self.stop_on_failures(self.state.storage_failures(), Some(&self.folder));
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

    /// Asks the Run to stop at its next safe point for `reason`, such as
    /// the resource governor's `resources` (ADRs 0040 and 0122) or its
    /// engine's loss; the first reason asked stays.
    pub fn stop(&self, reason: StopReason) {
        self.stop_at_safe_point(reason);
    }

    /// Hands `warning` over to the Run's report (ADR 0102): kept in its run
    /// state, in the order handed over, so a resume keeps it. A code of the
    /// warning table, or any other, is kept as it is.
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write was queued still writes it.
    pub async fn warn(&self, warning: RunWarning) -> Result<(), RunStateError> {
        self.state
            .write(move |connection| report::warn(connection, &warning))
            .await
    }

    /// Records why the Run did nothing (ADR 0102), as its driver found it:
    /// no seed reached a handler, or a resume found nothing left. Its
    /// report says so.
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write was queued still writes it.
    pub async fn nothing_done(&self, nothing_done: NothingDone) -> Result<(), RunStateError> {
        self.state
            .write(move |connection| report::record_nothing_done(connection, &nothing_done))
            .await
    }

    /// What the Run used so far, as its run state counted it, with each
    /// Limit that applies to it and what is left of it (ADR 0101).
    ///
    /// # Errors
    ///
    /// `Refused` when the Usage ledger cannot be read, `State` when its run
    /// state cannot.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn usage(&self) -> Result<RunUsage, ChargeError> {
        let charging = self.shared.charging.lock().await.clone();
        let now = self.runs.limits.now();
        let limits = match charging {
            Some(charging) => self.owner.blocking(move || charging.applied(now)).await?,
            None => Vec::new(),
        };
        Ok(self
            .state
            .read(move |connection| report::usage(connection, limits))
            .await?)
    }

    /// Asks the Run to stop at its next safe point for `reason`; the first
    /// reason asked stays.
    fn stop_at_safe_point(&self, reason: StopReason) {
        self.shared.stop.send_if_modified(|stop| {
            let unset = stop.is_none();
            if unset {
                *stop = Some(reason);
            }
            unset
        });
    }

    /// Its admission under `settings`: the gate in admission the
    /// composition passes in, opened from what its run file journaled, its
    /// rate cap, and, when the gate names a rule for its keys, the
    /// Politeness `politeness.sharing` says it shares: the per-user
    /// Politeness file for `shared` and `per_daemon`, a file of its own in
    /// memory for `run`. `None` when the composition passes in no gate.
    ///
    /// # Errors
    ///
    /// When its run file cannot be read, or the Politeness file opened.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn admission(
        &self,
        settings: &Resolved,
    ) -> Result<Option<Admission>, AdmissionError> {
        let Some(gates) = self.admitting.gates.clone() else {
            return Ok(None);
        };
        let opening = settings.clone();
        let (wall, now) = (self.clock.wall(), self.clock.now());
        let entropy = self.fork_entropy().await;
        let gate = self
            .state
            .read(move |connection| gates.open(&opening, connection, wall, now, entropy))
            .await?;
        let admission = Admission::new(gate, settings);
        if !admission.shares(settings) {
            return Ok(Some(admission));
        }
        let politeness = if settings.value(POLITENESS_SHARING.name).as_str() == Some("run") {
            Politeness::in_memory(self.admitting.politeness.tables.clone(), &self.owner).await?
        } else {
            let file = self.admitting.politeness.file().await?;
            self.stop_on_failures(
                file.storage_failures(),
                self.admitting.politeness.path.parent(),
            );
            file
        };
        let holder = Holder {
            run_id: self.id.to_string(),
            epoch: self.state.epoch(),
            run_folder: self.folder.clone(),
        };
        Ok(Some(
            admission.sharing(politeness, holder, self.clock, settings),
        ))
    }

    /// Stops the Run at its next safe point, as at `critical`, once
    /// `failures` reports a write of one of its essential files that met a
    /// full disk or an I/O error, naming `folder` (ADR 0122).
    fn stop_on_failures(
        &self,
        mut failures: watch::Receiver<Option<String>>,
        folder: Option<&Path>,
    ) {
        let failing = self.clone();
        let folder = folder.map(|folder| folder.display().to_string());
        self.spawn(async move {
            tokio::select! {
                () = failing.cancellation().cancelled() => {}
                failed = failures.wait_for(Option::is_some) => if failed.is_ok() {
                    failing.stop_at_safe_point(StopReason::Resources {
                        resource: goodfetch_protocol::run::Resource::Storage,
                        folder,
                    });
                },
            }
        });
    }

    /// Its owner, whose tasks and blocking work stop with the Run.
    pub fn owner(&self) -> &TaskOwner {
        &self.owner
    }

    /// Starts `future` as one of the Run's tasks, in its span.
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.owner
            .spawn(future.instrument(self.shared.span.clone()));
    }

    /// Its `tracing` span, whose field is its Run ID: the parent of what its
    /// driver and its pages record (ADR 0132).
    pub fn span(&self) -> &tracing::Span {
        &self.shared.span
    }

    /// Ends the Run as its driver says, once: its tasks stop, and its Run
    /// report is written and given to whoever waits for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the end was sent still ends the Run.
    pub async fn end(&self, ending: Ending) {
        let end = match ending {
            Ending::Finished(reasons) => End::Finished(reasons),
            Ending::Failed(error) => End::Failed(error),
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

    /// Charges `amount` of `usage_kind` against every Limit that holds the
    /// Run, before the Run uses it (ADR 0041): out of its Slices, taking
    /// another Slice in one ledger transaction when they run out, then
    /// journaled in its run state under its run epoch, from which an engine
    /// reclaiming its Slices after a crash charges them. A Run with nothing
    /// left of a Limit, or covered by a Limit flagged *must understand*
    /// that this engine cannot evaluate, is asked to stop at its next safe
    /// point for that Limit, and must not use what it was refused.
    ///
    /// # Errors
    ///
    /// `Refused` when the ledger refused the charge or could not be read or
    /// written, `State` when the run state could not journal it: nothing
    /// may be used then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Slice taken or the charge journaled,
    /// never more charged than the Run used.
    pub async fn charge(&self, usage_kind: &str, amount: u64) -> Result<(), ChargeError> {
        let kind = usage_kind.to_owned();
        self.charged(move |charging, now| {
            charging
                .charge(&kind, amount, now)
                .map(|charged| ((), charged))
        })
        .await
        .map(drop)
    }

    /// Reserves `worst`, the worst case of a cost the Run is about to pay
    /// in `currency`, such as a model call or a paid solve, against every
    /// Limit on `usage_kind` in that currency that holds it (ADR 0041):
    /// taken in one ledger transaction and journaled in its run state
    /// before the Run pays, so that an engine reclaiming its Slices after a
    /// crash charges it at its worst case. A currency no Limit holding the
    /// Run caps marks it *uncapped* in its Run report. A refusal asks the
    /// Run to stop as [`Run::charge`]'s does.
    ///
    /// # Errors
    ///
    /// As [`Run::charge`]: the cost must not be paid then.
    ///
    /// # Cancel safety
    ///
    /// As [`Run::charge`]'s.
    pub async fn reserve(
        &self,
        usage_kind: &str,
        currency: Option<&str>,
        worst: u64,
    ) -> Result<Reservation, ChargeError> {
        let kind = usage_kind.to_owned();
        let currency = currency.map(str::to_owned);
        self.charged(move |charging, now| charging.reserve(&kind, currency.as_deref(), worst, now))
            .await
            .map(Option::unwrap_or_default)
    }

    /// Who allows the Run to pay for `usage_kind` in `currency`, as
    /// [`Charging::allowed_by`] answers (ADR 0045): a person or a managed
    /// Operator policy, or none, when the Run must not pay. A refusal asks
    /// the Run to stop as [`Run::charge`]'s does.
    ///
    /// # Errors
    ///
    /// As [`Run::charge`].
    pub async fn allowed_by(
        &self,
        usage_kind: &str,
        currency: &str,
    ) -> Result<Option<goodfetch_protocol::run::LimitSetter>, ChargeError> {
        let (kind, currency) = (usage_kind.to_owned(), currency.to_owned());
        self.charged(move |charging, _| {
            charging
                .allowed_by(&kind, &currency)
                .map(|setter| (setter, Charged::default()))
        })
        .await
        .map(Option::flatten)
    }

    /// Settles `reservation` at `cost`, what the Run paid, which its worst
    /// case bounds: the rest is the Run's to use again, journaled as such.
    ///
    /// # Errors
    ///
    /// `State` when the run state could not journal it: the reservation
    /// then stays charged at its worst case.
    ///
    /// # Cancel safety
    ///
    /// As [`Run::charge`]'s.
    pub async fn settle(&self, reservation: Reservation, cost: u64) -> Result<(), ChargeError> {
        self.charged(move |charging, _| Ok(((), charging.settle(reservation, cost))))
            .await
            .map(drop)
    }

    /// Runs `change` on the Run's charging under the ledger's blocking
    /// calls, journals what it charged, and asks the Run to stop at its next
    /// safe point when a Limit refused it; answers none once the Run ended.
    async fn charged<T, F>(&self, change: F) -> Result<Option<T>, ChargeError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Charging, jiff::Timestamp) -> Result<(T, Charged), Refusal> + Send + 'static,
    {
        let mut held = self.shared.charging.lock().await;
        let Some(mut charging) = held.take() else {
            return Ok(None);
        };
        let now = self.runs.limits.now();
        let (charging, changed) = self
            .owner
            .blocking(move || {
                let changed = change(&mut charging, now);
                (charging, changed)
            })
            .await;
        let uncapped = charging.uncapped();
        *held = Some(charging);
        match changed {
            Ok((value, journaled)) => {
                if !journaled.used.is_empty() || journaled.counted.is_some() || uncapped {
                    let epoch = self.state.epoch();
                    self.state
                        .write(move |connection| {
                            slices::journal(connection, epoch, &journaled.used, uncapped)?;
                            match &journaled.counted {
                                Some(counted) => report::count(
                                    connection,
                                    &counted.usage_kind,
                                    counted.currency.as_deref(),
                                    counted.delta,
                                ),
                                None => Ok(()),
                            }
                        })
                        .await?;
                }
                Ok(Some(value))
            }
            Err(Refusal::Reached(reached)) => {
                self.stop_at_safe_point(StopReason::Limit(Box::new(ReachedLimit::clone(&reached))));
                Err(Refusal::Reached(reached).into())
            }
            Err(Refusal::NotUnderstood(limit)) => {
                self.stop_at_safe_point(StopReason::LimitNotUnderstood(Box::new(
                    NotUnderstoodLimit {
                        scope: limit.scope.clone(),
                        usage_kind: limit.usage_kind.clone(),
                        currency: limit.currency.clone(),
                        period: limit.period.clone(),
                    },
                )));
                Err(Refusal::NotUnderstood(limit).into())
            }
            Err(Refusal::Ledger(LedgerError::Storage { path, message })) => {
                // The ledger holds every Run's Limits: each Run of the
                // engine stops as at `critical` (ADR 0122).
                let folder = path.parent().map_or_else(|| path.clone(), Path::to_owned);
                let _ = self
                    .runs
                    .commands
                    .send(Command::StorageFailed(folder))
                    .await;
                Err(Refusal::Ledger(LedgerError::Storage { path, message }).into())
            }
            Err(refusal) => Err(refusal.into()),
        }
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

/// The `timeout.secret` of a Run whose settings resolved as `settings`,
/// its default when none did.
fn secret_timeout(settings: Option<&RunSettings>) -> Duration {
    settings
        .and_then(|settings| settings.resolved().duration(TIMEOUT_SECRET.name))
        .unwrap_or(Duration::from_secs(30))
}

impl Runs {
    /// Starts the registry as a task of `owner`, each Run's tasks under a
    /// child of it.
    pub(crate) fn start_registry(mut setup: RunsSetup, owner: &TaskOwner) -> Self {
        let (commands, mailbox) = mpsc::channel(COMMANDS);
        let readers = Arc::new(owner.child());
        let limits = Limits {
            home: setup.home.clone(),
            owner: Arc::clone(&readers),
            ledger: Ledger::at(setup.home.usage_ledger(), setup.ledger_vfs.clone()),
            known_certificates: KnownCertificates::of(&setup.home, setup.ledger_vfs.clone()),
            clock: setup.clock,
            product: setup.product,
            instance: setup.events.instance().as_str().to_owned(),
            daemon: matches!(setup.reached_at, Some(RunOwner::Daemon(_)))
                .then(|| major_name(&setup.version)),
        };
        let secrets = Secrets::new(
            Carried {
                environment: std::mem::take(&mut setup.environment),
                keychain: setup.keychain.clone(),
                home: setup.home.clone(),
            },
            Arc::new(AuditLog::of(&setup.home, setup.ledger_vfs.clone())),
            Arc::clone(&readers),
            setup.clock,
        );
        let runs = Self {
            commands,
            settings: setup.settings.clone(),
            connection: None,
            limits: Arc::new(limits),
            secrets,
            drivers: Arc::new(setup.drivers.clone()),
        };
        let politeness = SharedPoliteness {
            file: Arc::new(tokio::sync::OnceCell::new()),
            path: setup.home.politeness_file(),
            options: setup.politeness.clone(),
            tables: setup.politeness_tables.clone(),
            engine: format!("{:016x}", setup.entropy.next_u64()),
            owner: Arc::new(owner.child()),
        };
        let admitting = Arc::new(Admitting {
            gates: setup.gates.clone(),
            politeness,
        });
        let registry = Registry {
            setup,
            admitting,
            owner: owner.child(),
            ending_owner: owner.child(),
            readers,
            live: BTreeMap::new(),
            ended: BTreeMap::new(),
            runs: runs.downgrade(),
            limits: Arc::clone(&runs.limits),
            secrets: runs.secrets.clone(),
            drivers: Arc::clone(&runs.drivers),
            machine: None,
            draining: false,
            found_ended: std::collections::BTreeSet::new(),
            sampled: Vec::new(),
            unrecorded: BTreeMap::new(),
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

    /// What this engine's Runs charge against: the user's Usage ledger and
    /// the scopes they stand in.
    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The user's Audit log, to which this engine's Runs record their
    /// security events.
    pub(crate) fn audit(&self) -> &AuditLog {
        self.secrets.audit()
    }

    /// The Daemon's Connection these Runs belong to; none on an Attached
    /// engine, whose one Connection is its Client's.
    pub(crate) fn connection(&self) -> Option<ConnectionId> {
        self.connection
    }

    /// The engine's clock, which a call's wait on a Run reads.
    pub fn clock(&self) -> Clock {
        self.limits.clock
    }

    /// The Goodfetch home the engine runs in.
    pub fn home(&self) -> &Home {
        &self.limits.home
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

    /// Begins a Run, or returns the one its Run ID names. A new Run's span
    /// is a root linked to the current span, the starting request's.
    ///
    /// # Errors
    ///
    /// As [`StartError`] says.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the start was sent still starts the Run.
    pub async fn start(&self, start: Start) -> Result<Started, StartError> {
        let secrets = self
            .secrets
            .for_run(secret_timeout(start.settings.as_ref()));
        // Only a reference a layer set resolves here: the default's item,
        // in the OS keychain or the encrypted local file, is the engine's
        // own, made when run state first wraps a data key with it (#617).
        if let Some((reference, layer)) = start.settings.as_ref().and_then(|settings| {
            let resolved = settings.resolved();
            let layer = resolved.get(RUN_STATE_KEY_REFERENCE.name)?.layer;
            (layer != SettingsLayer::Default).then_some(())?;
            Some((
                resolved.secret_reference(RUN_STATE_KEY_REFERENCE.name)?,
                layer,
            ))
        }) {
            // The key-encryption key is used as the Run starts, to wrap its
            // data key (ADR 0015), so it resolves now, before the Run exists.
            secrets
                .resolve(
                    &reference,
                    &Purpose {
                        placeholder: RUN_STATE_KEY_REFERENCE.name,
                        domain: None,
                        domains: &[],
                        layer: Some(layer),
                        run_id: start.run_id.as_ref(),
                    },
                )
                .await
                .map_err(StartError::Secret)?;
        }
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Start(
                Box::new((start, secrets, Link::to_current())),
                self.connection,
                answer,
            ))
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
        self.finish_or_wait(run_id, Letting::Finish).await
    }

    /// Answers the Run report of `run_id` once it ended, without letting it
    /// finish: a keep-alive crawl goes on waiting for new work.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future ends only the wait.
    pub async fn ended(&self, run_id: RunId) -> Result<RunReport, RunError> {
        self.finish_or_wait(run_id, Letting::Wait).await
    }

    async fn finish_or_wait(&self, run_id: RunId, letting: Letting) -> Result<RunReport, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Finish(run_id, letting, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        let report = answered.await.map_err(|_| RunError::Stopped)??;
        wait_for(report).await
    }

    /// Sets the program's own status line about the live Run `run_id`
    /// (RT-15): kept in its run state, written into its Run index row and
    /// published as `run.progress`; answers its progress.
    ///
    /// # Errors
    ///
    /// `NotFound` when this engine runs no Run of this Run ID,
    /// `Unavailable` when its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still sets the message.
    pub async fn set_progress_message(
        &self,
        run_id: RunId,
        message: ProgressMessage,
    ) -> Result<RunProgress, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::ProgressMessage(run_id, message, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)?
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

    /// Stops the Run `run_id` at its next safe point with a Checkpoint,
    /// `stopped` and `incomplete` as an abort by `by`, resumable by its Run
    /// ID, and answers its Run report once it ended (ADR 0102); an ended
    /// Run's report is answered as it is, and a Run no engine runs is
    /// stopped as [`Runs::abort`] stops it.
    ///
    /// # Errors
    ///
    /// As [`Runs::abort`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the stop was sent still stops the Run.
    pub async fn stop_at_safe_point(
        &self,
        run_id: RunId,
        by: StoppedBy,
    ) -> Result<RunReport, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::SafeStop(run_id, by, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        match answered.await.map_err(|_| RunError::Stopped)?? {
            Waiting::Report(report) => wait_for(report).await,
            Waiting::TakeOver(taken) => taken.await.map_err(|_| RunError::Stopped)?,
        }
    }

    /// What the Run `run_id` used, as [`Run::usage`] gives it; an ended
    /// Run's report's.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID, `Unavailable` when its state
    /// or the Usage ledger cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn usage(&self, run_id: RunId) -> Result<RunUsage, RunError> {
        match self.live(run_id.clone()).await {
            Ok(run) => run.usage().await.map_err(|error| RunError::Unavailable {
                folder: run.folder.clone(),
                run_id,
                error: error.to_string(),
            }),
            Err(RunError::NotFound(_)) => self.ended(run_id).await.map(|report| report.usage),
            Err(error) => Err(error),
        }
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
            .send(Command::Resume(
                Box::new((resume, Link::to_current())),
                self.connection,
                answer,
            ))
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

    /// Whether the engine drains and where its run folders go, answered by
    /// the registry itself: `None` once it stopped or stopped answering.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn health(&self) -> Option<RegistryHealth> {
        let (answer, answered) = oneshot::channel();
        self.commands.send(Command::Health(answer)).await.ok()?;
        answered.await.ok()
    }

    /// Drains the engine (ADR 0049): from now on it takes no Run, starting
    /// or resuming, and each live Run stops at its next safe point with a
    /// Checkpoint, `stopped` and resumable by its Run ID; answers once the
    /// engine takes no Run, with the [`Draining`] Runs, whose ends
    /// [`Draining::ended`] waits for.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still drains: the Runs stop,
    /// and only the answer is lost.
    pub(crate) async fn drain(&self) -> Draining {
        let (answer, answered) = oneshot::channel();
        if self.commands.send(Command::Drain(answer)).await.is_err() {
            return Draining(Vec::new());
        }
        Draining(answered.await.unwrap_or_default())
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

/// The Runs a drain stops (ADR 0049).
pub(crate) struct Draining(Vec<watch::Receiver<Option<Ended>>>);

impl Draining {
    /// How many Runs it stops.
    pub(crate) fn count(&self) -> usize {
        self.0.len()
    }

    /// Answers once every one ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the wait.
    pub(crate) async fn ended(self) {
        for report in self.0 {
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

async fn wait_for(mut report: watch::Receiver<Option<Ended>>) -> Result<RunReport, RunError> {
    let ended = report
        .wait_for(Option::is_some)
        .await
        .map_err(|_| RunError::Stopped)?;
    match ended.clone().expect("invariant: the wait ends on an end") {
        Ended::Report(report) => Ok(*report),
        Ended::Unrecorded(exhausted) => Err(RunError::ResourcesExhausted(exhausted)),
    }
}

/// How a Run's end went: its report, or a stop its run file could not
/// record, which leaves it interrupted (ADR 0122).
#[derive(Clone, Debug)]
enum Ended {
    /// Its end and its report are in its run file.
    Report(Box<RunReport>),
    /// The commit recording its stop met a full disk or an I/O error.
    Unrecorded(Box<StorageExhausted>),
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
    weakened: Vec<WeakenedSetting>,
    report: watch::Sender<Option<Ended>>,
    writer: Arc<TaskOwner>,
    lock: Arc<File>,
    connection: Option<ConnectionId>,
    /// The last Checkpoint written of it.
    checkpoint: Option<Checkpoint>,
    /// The program's own status line, once it set one.
    progress_message: Option<ProgressMessage>,
    /// Its Lifecycle, which only the Run's table changes: `running` until
    /// its end is decided, then the Lifecycle it ends in.
    lifecycle: Lifecycle,
    /// The worst storage level among the filesystems it writes to, as its
    /// progress last carried it; none while all are `normal`.
    storage: Option<RunStorage>,
    /// The folders whose `low` level its report carries a warning for.
    warned_low: std::collections::BTreeSet<String>,
}

struct Registry {
    setup: RunsSetup,
    /// The gate in admission and the Politeness the engine's Runs share.
    admitting: Arc<Admitting>,
    owner: TaskOwner,
    ending_owner: TaskOwner,
    /// Where readers of run state run their blocking reads.
    readers: Arc<TaskOwner>,
    live: BTreeMap<RunId, Live>,
    ended: BTreeMap<RunId, RunReport>,
    runs: mpsc::WeakSender<Command>,
    /// What its Runs charge against.
    limits: Arc<Limits>,
    secrets: Secrets,
    drivers: Arc<BTreeMap<RunKind, Driver>>,
    /// The machine's identity, once read.
    machine: Option<String>,
    /// Whether the engine drains, taking no Run (ADR 0049).
    draining: bool,
    /// The Runs a start found finished: a later call that ends one did
    /// nothing, which its report says (ADR 0102).
    found_ended: std::collections::BTreeSet<RunId>,
    /// The engine's folders the governor samples while the registry lives:
    /// the run folders', the Goodfetch home and the temporary folder (ADR
    /// 0122).
    sampled: Vec<Held>,
    /// The Runs whose stop their run file could not record, left
    /// interrupted until a start with their ID resumes them (ADR 0122).
    unrecorded: BTreeMap<RunId, Box<StorageExhausted>>,
}

impl Registry {
    async fn run(mut self, mut mailbox: mpsc::Receiver<Command>) {
        // No command is answered before the reclaim ends, so a Run starts
        // on a ledger holding its major's managed Limits and rid of dead
        // runs' Slices, and an answered Run call says the engine's startup
        // writes are done.
        self.write_managed().await;
        self.reclaim().await;
        self.sample_storage();
        while let Some(command) = mailbox.recv().await {
            match command {
                Command::Start(start, _, answer) if self.draining => {
                    let (start, _, _) = *start;
                    let _ = answer.send(Err(StartError::Draining {
                        run_id: start.run_id,
                    }));
                }
                Command::Resume(resume, _, answer) if self.draining => {
                    let (resume, _) = *resume;
                    let _ = answer.send(Err(RunError::Draining(resume.run_id)));
                }
                Command::Drain(answer) => {
                    let _ = answer.send(self.drain().await);
                }
                Command::Start(start, connection, answer) => {
                    let (start, secrets, link) = *start;
                    let started = self.start(start, secrets, connection, &link).await;
                    let _ = answer.send(started);
                }
                Command::Finish(run_id, letting, answer) => {
                    let _ = answer.send(self.finish(&run_id, letting).await);
                }
                Command::ProgressMessage(run_id, message, answer) => {
                    let _ = answer.send(self.progress_message(&run_id, message).await);
                }
                Command::Abort(run_id, by, forwarded, answer) => {
                    let _ = answer.send(self.abort(&run_id, by, forwarded).await);
                }
                Command::End(run_id, end) => {
                    let _ = self.end_or_report(&run_id, end).await;
                }
                Command::Uses(run_id, product) => self.uses(&run_id, product).await,
                Command::Ended(run_id, ended) => self.ended(run_id, *ended),
                Command::StopAll(answer) => {
                    let ids: Vec<RunId> = self.live.keys().cloned().collect();
                    let _ = answer.send(self.stop_by_client(ids).await);
                }
                // A draining engine leaves each Run to its drain, which a
                // Client's leaving does not abort.
                Command::StopConnection(_, answer) if self.draining => {
                    let _ = answer.send(Vec::new());
                }
                Command::StopConnection(connection, answer) => {
                    let ids = self.runs_of(connection);
                    let _ = answer.send(self.stop_by_client(ids).await);
                }
                Command::LiveCount(answer) => {
                    let _ = answer.send(self.live.len());
                }
                Command::Health(answer) => {
                    let _ = answer.send(self.health());
                }
                Command::Folder(run_id, answer) => {
                    let _ = answer.send(self.folder(&run_id));
                }
                Command::Live(run_id, answer) => {
                    let _ = answer.send(self.live_run(run_id));
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
                    let (resume, link) = *resume;
                    let resumed = self.resume(resume, connection).await;
                    if let Ok(Resumed::New { run, .. }) = &resumed {
                        link.add_to(run.span());
                    }
                    let _ = answer.send(resumed);
                }
                Command::Machine(answer) => {
                    let _ = answer.send(self.machine().await);
                }
                Command::SafeStop(run_id, by, answer) => {
                    let _ = answer.send(self.safe_stop(&run_id, by).await);
                }
                Command::Reader(run_id, answer) => {
                    let _ = answer.send(self.reader(&run_id));
                }
                Command::Storage(sample) => self.storage(&sample).await,
                Command::StorageFailed(folder) => self.storage_failed(&folder),
            }
        }
        self.readers.wait().await;
        self.ending_owner.wait().await;
        self.owner.wait().await;
    }

    /// Has the governor sample the engine's own folders, and hands each of
    /// its storage samples to the registry (ADR 0122).
    fn sample_storage(&mut self) {
        let governor = &self.setup.governor;
        let location = self
            .setup
            .location
            .folder(&self.setup.home, &self.setup.version);
        self.sampled = vec![
            governor.sample(location),
            governor.sample(self.setup.home.root().to_owned()),
            governor.sample(std::env::temp_dir()),
        ];
        let mut samples = governor.storage();
        let registry = self.runs.clone();
        let token = self.owner.token().clone();
        self.owner.spawn(async move {
            loop {
                tokio::select! {
                    () = token.cancelled() => return,
                    changed = samples.changed() => if changed.is_err() { return },
                }
                let sample = Arc::clone(&samples.borrow_and_update());
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                if registry.send(Command::Storage(sample)).await.is_err() {
                    return;
                }
            }
        });
    }

    /// Acts on a storage sample (ADR 0122): each live Run with an essential
    /// write on a filesystem at `critical` (its run file, or the Usage
    /// ledger and the Politeness file in the Goodfetch home) stops at its
    /// next safe point; each Run's progress carries the worst level among
    /// its filesystems when it changes, and its report a `storage_low`
    /// warning for each folder found at `low` or worse.
    async fn storage(&mut self, sample: &StorageSample) {
        let home = self.setup.home.root().to_owned();
        let temporary = std::env::temp_dir();
        let mut changed = Vec::new();
        for live in self.live.values_mut() {
            let Some(run) = &live.run else {
                continue;
            };
            let essential = [&live.folder, &home];
            if let Some(folder) = essential.into_iter().find(|folder| {
                sample
                    .of(folder)
                    .is_some_and(|filesystem| filesystem.level == StorageLevel::Critical)
            }) {
                run.stop(StopReason::Resources {
                    resource: goodfetch_protocol::run::Resource::Storage,
                    folder: Some(folder.display().to_string()),
                });
            }
            let Some((folder, filesystem)) = worst(sample, [&live.folder, &home, &temporary])
            else {
                continue;
            };
            let storage = RunStorage {
                level: filesystem.level,
                folder: folder.display().to_string(),
                free_bytes: filesystem.free,
            };
            let was = live
                .storage
                .as_ref()
                .map_or(StorageLevel::Normal, |storage| storage.level);
            if storage.level != was {
                live.storage = Some(storage.clone());
                changed.push((
                    live.id.clone(),
                    storage.clone(),
                    live.progress_message.clone(),
                ));
            }
            if storage.level >= StorageLevel::Low && live.warned_low.insert(storage.folder.clone())
            {
                let warning = storage_low(&storage);
                let warned = run.clone();
                run.spawn(async move {
                    if let Err(error) = warned.warn(warning).await {
                        tracing::warn!(%error, run_id = %warned.id, "the Run's `storage_low` warning was not kept");
                    }
                });
            }
        }
        for (run_id, storage, message) in changed {
            let progress = RunProgress {
                run_id: run_id.clone(),
                lifecycle: Lifecycle::Running,
                message,
                storage: Some(storage),
            };
            self.setup
                .events
                .publish(&RUN_PROGRESS, Some(run_id.as_str()), &progress)
                .await;
        }
    }

    /// Whether this engine may resume the Run `run_id` that `held` holds in
    /// `folder`: one bound to its machine only there (ADR 0056), one of its
    /// Product only, and none while its folder's filesystem is at `critical`
    /// (ADR 0122).
    async fn may_resume(
        &mut self,
        run_id: &RunId,
        held: &checkpoint::Held,
        folder: &Path,
    ) -> Result<(), RunError> {
        if let Some(RunBinding::Machine { machine }) = &held.binding
            && *machine != self.machine().await?
        {
            return Err(RunError::BoundToMachine(run_id.clone()));
        }
        if !self.setup.product.contains(held.needs) {
            return Err(RunError::NotInProduct {
                run_id: run_id.clone(),
                needs: held.needs,
            });
        }
        match self.critical_at(folder.to_owned()).await {
            Some(filesystem) => Err(RunError::ResourcesExhausted(Box::new(StorageExhausted {
                run_id: run_id.clone(),
                folder: folder.to_owned(),
                filesystem,
            }))),
            None => Ok(()),
        }
    }

    /// Stops every live Run at its next safe point, as at `critical`, after
    /// a write of a file they all depend on, in `folder`, met a full disk or
    /// an I/O error (ADR 0122).
    fn storage_failed(&self, folder: &Path) {
        for run in self.live.values().filter_map(|live| live.run.as_ref()) {
            run.stop(StopReason::Resources {
                resource: goodfetch_protocol::run::Resource::Storage,
                folder: Some(folder.display().to_string()),
            });
        }
    }

    /// The filesystem holding `path` now, when it is at `critical`.
    async fn critical_at(&self, path: PathBuf) -> Option<Filesystem> {
        let governor = self.setup.governor.clone();
        self.owner
            .blocking(move || governor.storage_at(&path))
            .await
            .filter(|filesystem| filesystem.level == StorageLevel::Critical)
    }

    /// Takes no Run from now on, stops each live Run for the drain with a
    /// Checkpoint, and answers the waits for their reports.
    async fn drain(&mut self) -> Vec<watch::Receiver<Option<Ended>>> {
        self.draining = true;
        let ids: Vec<RunId> = self.live.keys().cloned().collect();
        let mut reports = Vec::new();
        for run_id in ids {
            if let Ok(report) = self.stop(&run_id, StopReason::Drain, None).await {
                reports.push(report);
            }
        }
        reports
    }

    /// Stops each of `ids` as aborted by its Client, and answers the waits
    /// for their reports.
    async fn stop_by_client(&mut self, ids: Vec<RunId>) -> Vec<watch::Receiver<Option<Ended>>> {
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

    /// Writes the install Limits the managed Operator policy of the
    /// engine's major names as that major's managed rows, as each engine
    /// does when it starts (ADR 0041); a policy that names none, or none
    /// at all, leaves that major no managed row.
    async fn write_managed(&self) {
        let machine = match self.setup.settings.machine().await {
            Ok(machine) => machine,
            Err(error) => {
                tracing::warn!(%error, "the Operator policy could not be read to write its managed Limits");
                return;
            }
        };
        let limits: Vec<goodfetch_protocol::usage::Limit> = machine
            .operator_policy
            .managed
            .iter()
            .flat_map(|managed| managed.install_limits.iter())
            .map(goodfetch_protocol::usage::ManagedLimit::limit)
            .collect();
        let ledger = self.limits.ledger.clone();
        let major = major_name(&self.setup.version);
        let now = self.limits.now();
        let written = self
            .owner
            .blocking(move || ledger.write_managed(&major, &limits, now))
            .await;
        if let Err(error) = written {
            tracing::warn!(%error, "the Usage ledger could not be opened to write the managed Operator policy's Limits");
        }
    }

    /// Reclaims the Slices of every run whose engine is gone, as each engine
    /// that opens the Usage ledger does (ADR 0041).
    async fn reclaim(&self) {
        let ledger = self.limits.ledger.clone();
        let vfs = self.setup.run_state.vfs.clone();
        let reclaimed = self
            .owner
            .blocking(move || {
                ledger.reclaim(&|holder| slices::probe(holder, RUN_FILE, vfs.as_deref()))
            })
            .await;
        if let Err(error) = reclaimed {
            tracing::warn!(%error, "the Usage ledger could not be opened to reclaim the Slices of dead runs");
        }
    }

    /// Records that the Run `run_id` ended as `ended` says, and wakes the
    /// waits for its report. A stop its run file could not record leaves
    /// the Run interrupted, with no live writer, as after a crash, and a
    /// start with its ID resumes it (ADR 0122).
    fn ended(&mut self, run_id: RunId, ended: Ended) {
        if let Some(live) = self.live.remove(&run_id) {
            // The lock goes before the report, so whoever the report wakes
            // finds the Run's writer gone.
            let Live {
                report: waiters,
                lock,
                kind,
                ..
            } = live;
            drop(lock);
            if let Ended::Report(report) = &ended {
                self.setup.metrics.run_ended(kind, report);
            }
            waiters.send_replace(Some(ended.clone()));
        }
        match ended {
            Ended::Report(report) => {
                self.ended.insert(run_id, *report);
            }
            Ended::Unrecorded(exhausted) => {
                self.unrecorded.insert(run_id, exhausted);
            }
        }
    }

    /// The live Runs that the Client of `connection` started or resumed.
    fn runs_of(&self, connection: ConnectionId) -> Vec<RunId> {
        self.live
            .values()
            .filter(|live| live.connection == Some(connection))
            .map(|live| live.id.clone())
            .collect()
    }

    /// The live Run `run_id`, once it has one.
    fn live_run(&self, run_id: RunId) -> Result<Run, RunError> {
        self.live
            .get(&run_id)
            .and_then(|live| live.run.clone())
            .ok_or(RunError::NotFound(run_id))
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

    /// How a Run's run file opens under its `settings`: in the durability
    /// `run_state.durability` names, `full` when it has none.
    fn opened_with(&self, settings: Option<&RunSettings>) -> RunStateOptions {
        let durability = settings.map_or(Durability::Full, |settings| {
            Durability::named(settings.resolved().value(RUN_STATE_DURABILITY.name))
        });
        RunStateOptions {
            durability,
            ..self.setup.run_state.clone()
        }
    }

    fn folder(&self, run_id: &RunId) -> PathBuf {
        self.setup
            .location
            .folder(&self.setup.home, &self.setup.version)
            .join(run_id.as_str())
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
        secrets: RunSecrets,
        connection: Option<ConnectionId>,
        link: &Link,
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
            if let Err(refused) = lifecycle::apply(found.lifecycle, RunEvent::Start) {
                let folder = self.folder(&run_id);
                return Err(match RunError::refused(&run_id, &folder, refused) {
                    RunError::InUse { run_id, folder } => StartError::InUse { run_id, folder },
                    other => StartError::Internal {
                        run_id,
                        message: other.to_string(),
                    },
                });
            }
            if found.lifecycle == Lifecycle::Finished {
                self.found_ended.insert(run_id.clone());
            }
            return Ok(Started::Existing(RunStarted {
                run_id,
                started_as: StartedAs::Existing,
                lifecycle: found.lifecycle,
                needs: found.needs,
            }));
        }
        let location = self
            .setup
            .location
            .folder(&self.setup.home, &self.setup.version);
        if let Some(filesystem) = self.critical_at(location.clone()).await {
            return Err(StartError::ResourcesExhausted(Box::new(StorageExhausted {
                run_id,
                folder: location,
                filesystem,
            })));
        }
        let started = self
            .begin(run_id, start, inputs, secrets, connection)
            .await?;
        if let Started::New(run) = &started {
            link.add_to(run.span());
        }
        Ok(started)
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
                lifecycle: live.lifecycle,
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

    /// Whether a new Run placed at `placement` may start: every Limit that
    /// holds it has something left, and none it cannot be placed outside of
    /// is one this engine cannot evaluate (ADR 0041). A ledger that cannot
    /// be read refuses it.
    async fn may_start(
        &self,
        run_id: &RunId,
        connection: Option<ConnectionId>,
    ) -> Result<(), StartError> {
        let (ledger, now) = (self.limits.ledger.clone(), self.limits.now());
        let checked = self.limits.placement(connection);
        let may_start = self
            .owner
            .blocking(move || Charging::may_start(&ledger, &checked, now))
            .await;
        match may_start {
            Ok(()) => Ok(()),
            Err(Refusal::Reached(reached)) => Err(StartError::LimitReached(reached)),
            Err(Refusal::NotUnderstood(limit)) => Err(StartError::LimitNotUnderstood(limit)),
            Err(Refusal::Ledger(error)) => Err(StartError::Unavailable {
                run_id: run_id.clone(),
                folder: self.limits.ledger.path().to_owned(),
                error: error.to_string(),
            }),
        }
    }

    async fn begin(
        &mut self,
        run_id: RunId,
        start: Start,
        inputs: String,
        secrets: RunSecrets,
        connection: Option<ConnectionId>,
    ) -> Result<Started, StartError> {
        let folder = self.folder(&run_id);
        let unavailable = |error: String| StartError::Unavailable {
            run_id: run_id.clone(),
            folder: folder.clone(),
            error,
        };
        self.check_index().await?;
        self.may_start(&run_id, connection).await?;
        let lock = self.lock(&run_id, &folder).await?;
        let writer = Arc::new(self.owner.child());
        let state = RunState::open(
            folder.join(RUN_FILE),
            &self.opened_with(start.settings.as_ref()),
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
            weakened: start.weakened.clone(),
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
            run: Some(self.new_run(&run_id, &folder, state, connection, start.settings, secrets)),
            folder,
            kind: start.kind,
            needs: start.needs,
            tags: start.tags,
            inputs,
            ended_by: start.ended_by,
            started_at,
            policy_version: start.policy_version,
            weakened: start.weakened,
            report: watch::Sender::new(None),
            writer,
            lock: Arc::new(lock),
            connection,
            checkpoint: None,
            progress_message: None,
            lifecycle: Lifecycle::Running,
            storage: None,
            warned_low: std::collections::BTreeSet::new(),
        };
        let row = self.index_row(&live, Lifecycle::Running, None);
        self.write_index(row).await;
        let run = live.run.clone().expect("invariant: a Run begins live");
        run.watch_safe_points(start.timeout, start.ended_by);
        self.setup.metrics.run_live(live.kind);
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
            progress_message: live
                .progress_message
                .as_ref()
                .map(|message| message.as_str().to_owned()),
        }
    }

    /// Refuses a start when the Run index's marker refuses this engine
    /// (ADR 0098): an engine that finds its lowest minor raised past itself
    /// takes no new Run. An index that cannot be opened stops no Run; its
    /// row is restored by `runs reindex`.
    async fn check_index(&self) -> Result<(), StartError> {
        let path = self.setup.home.run_index(&self.setup.version);
        let version = self.setup.version.clone();
        match self
            .owner
            .blocking(move || run_index::check(&path, &version))
            .await
        {
            Err(RunStateError::Format {
                format,
                found,
                compatibility,
                install,
            }) => Err(StartError::Format {
                format,
                found,
                compatibility,
                install,
            }),
            Err(error) => {
                tracing::warn!(%error, "the Run index cannot be read; the Run starts without it");
                Ok(())
            }
            Ok(()) => Ok(()),
        }
    }

    async fn write_index(&self, row: RunIndexRow) {
        let path = self.setup.home.run_index(&self.setup.version);
        let version = self.setup.version.clone();
        if let Err(error) = self
            .owner
            .blocking(move || run_index::write(&path, &version, &row))
            .await
        {
            tracing::warn!(%error, "the Run index row was not written; `runs reindex` restores it");
        }
    }

    async fn finish(
        &mut self,
        run_id: &RunId,
        letting: Letting,
    ) -> Result<watch::Receiver<Option<Ended>>, RunError> {
        let Some(live) = self.live.get(run_id) else {
            return self.ended_report(run_id).await;
        };
        match (live.ended_by, letting) {
            (EndedBy::Finish, Letting::Finish) => {
                self.end_or_report(run_id, End::Finished(Vec::new())).await
            }
            (EndedBy::Driver, Letting::Finish) => {
                if let Some(run) = &live.run {
                    run.shared.finish.send_replace(true);
                }
                Ok(live.report.subscribe())
            }
            (_, Letting::Wait) => Ok(live.report.subscribe()),
        }
    }

    /// Sets the program's status line about the live Run `run_id`, as
    /// [`Runs::set_progress_message`] says.
    async fn progress_message(
        &mut self,
        run_id: &RunId,
        message: ProgressMessage,
    ) -> Result<RunProgress, RunError> {
        let Some(state) = self
            .live
            .get(run_id)
            .and_then(|live| live.run.as_ref())
            .map(|run| run.state.clone())
        else {
            return Err(RunError::NotFound(run_id.clone()));
        };
        let text = message.as_str().to_owned();
        state
            .write(move |connection| {
                connection
                    .execute("UPDATE run SET progress_message = ?1", [text])
                    .map(drop)
            })
            .await
            .map_err(|error| RunError::of_state(run_id, &self.folder(run_id), error))?;
        if let Some(live) = self.live.get_mut(run_id) {
            live.progress_message = Some(message.clone());
        }
        if let Some(live) = self.live.get(run_id) {
            let row = self.index_row(live, Lifecycle::Running, None);
            self.write_index(row).await;
        }
        let progress = RunProgress {
            run_id: run_id.clone(),
            lifecycle: Lifecycle::Running,
            message: Some(message),
            storage: self.live.get(run_id).and_then(|live| live.storage.clone()),
        };
        self.setup
            .events
            .publish(&RUN_PROGRESS, Some(run_id.as_str()), &progress)
            .await;
        Ok(progress)
    }

    /// What it answers about the engine's health.
    fn health(&self) -> RegistryHealth {
        RegistryHealth {
            draining: self.draining,
            governor_last_resort: self.setup.governor.at_last_resort(),
            folder: self
                .setup
                .location
                .folder(&self.setup.home, &self.setup.version),
        }
    }

    /// A handle to the registry for a Run of `connection`.
    /// The live Run `run_id` as its driver holds it, writing `state` in
    /// `folder`, under its resolved `settings`, resolving its Secret
    /// references through `secrets`.
    fn new_run(
        &mut self,
        run_id: &RunId,
        folder: &Path,
        state: RunState,
        connection: Option<ConnectionId>,
        settings: Option<RunSettings>,
        secrets: RunSecrets,
    ) -> Run {
        let epoch = state.epoch();
        let share = settings
            .as_ref()
            .and_then(|settings| settings.resolved().count(RUN_SHARE.name))
            .unwrap_or(100);
        Run {
            id: run_id.clone(),
            folder: folder.to_owned(),
            state,
            clock: self.setup.clock,
            admitting: Arc::clone(&self.admitting),
            shared: Arc::new(Shared {
                stop: watch::Sender::new(None),
                finish: watch::Sender::new(false),
                charging: tokio::sync::Mutex::new(Some(self.limits.charging(
                    run_id,
                    folder,
                    epoch,
                    self.limits.placement(connection),
                ))),
                metrics: self.setup.metrics.clone(),
                span: tracing::info_span!(parent: None, "run", run_id = %run_id),
                secrets,
                governor: self.setup.governor.join(share),
                _sampled: self.setup.governor.sample(folder.to_owned()),
                storage: self.setup.governor.storage(),
            }),
            owner: Arc::new(self.owner.child()),
            runs: self.handle(connection),
            settings: settings.map(Arc::new),
            entropy: Arc::new(tokio::sync::Mutex::new(self.setup.entropy.fork())),
        }
    }

    fn handle(&self, connection: Option<ConnectionId>) -> Runs {
        Runs {
            commands: self
                .runs
                .upgrade()
                .expect("invariant: the registry runs while a handle sends to it"),
            settings: self.setup.settings.clone(),
            connection,
            limits: Arc::clone(&self.limits),
            secrets: self.secrets.clone(),
            drivers: Arc::clone(&self.drivers),
        }
    }

    /// Ends the live Run `run_id` with `end`, once, or answers the report
    /// of an ended one.
    async fn end_or_report(
        &mut self,
        run_id: &RunId,
        end: End,
    ) -> Result<watch::Receiver<Option<Ended>>, RunError> {
        self.end_with(run_id, end, None).await
    }

    /// [`Registry::end_or_report`], a stop writing its Checkpoint to
    /// `location` rather than where the engine keeps them.
    async fn end_with(
        &mut self,
        run_id: &RunId,
        end: End,
        location: Option<PathBuf>,
    ) -> Result<watch::Receiver<Option<Ended>>, RunError> {
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
        let live = &self.live[run_id];
        let next = match lifecycle::apply(live.lifecycle, end.event()) {
            Ok(Next::To(next)) => next,
            Ok(Next::Unchanged) => return Ok(live.report.subscribe()),
            Err(refused) => return Err(RunError::refused(run_id, &live.folder, refused)),
        };
        let stopping = match &end {
            End::Stopped(cause) => Some(cause.clone()),
            End::Finished(_) | End::Failed(_) => None,
        };
        if let Some(cause) = &stopping {
            announce_stop(&events, run_id, cause).await;
        }
        let live = self
            .live
            .get_mut(run_id)
            .expect("invariant: the Run was live a line above");
        live.lifecycle = next;
        let receiver = live.report.subscribe();
        let run = live
            .run
            .take()
            .expect("invariant: a running Run keeps its writer until its end is decided");
        let checkpoint_to = stopping.map(|_| CheckpointTo {
            destination: location.unwrap_or_else(|| live.folder.join(CHECKPOINT_FILE)),
            vfs,
            nonce,
            events: events.clone(),
            governor: self.setup.governor.clone(),
        });
        let console_url = self
            .setup
            .console
            .as_ref()
            .map(|console| format!("{}/runs/{run_id}", console.trim_end_matches('/')));
        let closing = Closing {
            run,
            writer: Arc::clone(&live.writer),
            lock: Arc::clone(&live.lock),
            started_at: live.started_at.clone(),
            policy_version: live.policy_version.clone(),
            weakened: live.weakened.clone(),
            clock,
            end,
            lifecycle: next,
            checkpoint_to,
            last_checkpoint: live.checkpoint.clone(),
            parts: self.setup.report_parts.clone(),
            console_url,
            governor: self.setup.governor.clone(),
        };
        let recording = Recording {
            index: self.setup.home.run_index(&self.setup.version),
            version: self.setup.version.clone(),
            row,
            events,
            registry: self.runs.clone(),
        };
        self.ending_owner.spawn(recording.end(closing));
        Ok(receiver)
    }

    /// The report of a Run this engine ended, or whose state its folder
    /// holds.
    async fn report_of(&self, run_id: &RunId) -> Result<watch::Receiver<Option<Ended>>, RunError> {
        if let Some(report) = self.ended.get(run_id) {
            return Ok(
                watch::Sender::new(Some(Ended::Report(Box::new(report.clone())))).subscribe(),
            );
        }
        if let Some(exhausted) = self.unrecorded.get(run_id) {
            return Ok(watch::Sender::new(Some(Ended::Unrecorded(exhausted.clone()))).subscribe());
        }
        let file = self.folder(run_id).join(RUN_FILE);
        let vfs = self.setup.run_state.vfs.clone();
        let report = self
            .owner
            .blocking(move || read_report(&file, vfs.as_deref()))
            .await;
        match report {
            Ok(Some(report)) => {
                Ok(watch::Sender::new(Some(Ended::Report(Box::new(report)))).subscribe())
            }
            Ok(None) | Err(_) => Err(RunError::NotFound(run_id.clone())),
        }
    }

    /// The report of an ended Run as [`Registry::report_of`] gives it, saying
    /// the call did nothing when a start found the Run finished (ADR 0102).
    async fn ended_report(
        &self,
        run_id: &RunId,
    ) -> Result<watch::Receiver<Option<Ended>>, RunError> {
        let answered = self.report_of(run_id).await?;
        if !self.found_ended.contains(run_id) {
            return Ok(answered);
        }
        let mut report = answered.borrow().clone();
        if let Some(Ended::Report(report)) = &mut report {
            report.nothing_done = Some(NothingDone::AlreadyEnded {
                ended_at: report.ended_at.clone(),
            });
        }
        Ok(watch::Sender::new(report).subscribe())
    }

    /// Asks a live Run of this engine's to stop at its next safe point as an
    /// abort by `by`, answers the report of an ended one, and stops one no
    /// engine of its own runs as an abort does.
    async fn safe_stop(&mut self, run_id: &RunId, by: StoppedBy) -> Result<Waiting, RunError> {
        if let Some(live) = self.live.get(run_id) {
            if let Some(run) = &live.run {
                run.stop_at_safe_point(StopReason::Abort { by });
            }
            return Ok(Waiting::Report(live.report.subscribe()));
        }
        if self.ended.contains_key(run_id) || self.found_ended.contains(run_id) {
            return self.ended_report(run_id).await.map(Waiting::Report);
        }
        self.abort(run_id, by, Forwarded::No).await
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
            version: self.setup.version.clone(),
            events: self.setup.events.clone(),
            forwarder: Arc::clone(&self.setup.forwarder),
            owner: Arc::new(self.ending_owner.child()),
            parts: self.setup.report_parts.clone(),
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
        let state = match self.live.get(&run_id) {
            Some(live) => live.lifecycle,
            None => match self.existing(&run_id).await {
                Ok(Some(found)) => found.lifecycle,
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
            },
        };
        if let Err(refused) = lifecycle::apply(state, RunEvent::Checkpoint) {
            let _ = answer.send(Err(RunError::refused(&run_id, &folder, refused)));
            return;
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
            governor: self.setup.governor.clone(),
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
    ) -> Result<watch::Receiver<Option<Ended>>, RunError> {
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
        // A Run this engine writes is answered as it is.
        if let Some(live) = self.live.get(&run_id) {
            return Ok(Resumed::Existing(RunStarted {
                run_id,
                started_as: StartedAs::Existing,
                lifecycle: live.lifecycle,
                needs: live.needs,
            }));
        }
        let held = self.held(&run_id, checkpoint.as_deref()).await?;
        let folder = self.folder(&run_id);
        self.may_resume(&run_id, &held, &folder).await?;
        let lifecycle = match lifecycle::apply(held.lifecycle, RunEvent::Resume) {
            Ok(Next::To(next)) => next,
            Ok(Next::Unchanged) => {
                return Ok(Resumed::Existing(RunStarted {
                    run_id,
                    started_as: StartedAs::Existing,
                    lifecycle: held.lifecycle,
                    needs: held.needs,
                }));
            }
            Err(refused) => return Err(RunError::refused(&run_id, &folder, refused)),
        };
        let options = self.opened_with(settings.as_ref());
        let secrets = self.secrets.for_run(secret_timeout(settings.as_ref()));
        let (lock, writer, state) = self
            .reopen(&run_id, held.lifecycle, checkpoint, &options)
            .await?;
        let recorded = self.recorded(&run_id, &folder).await?;
        self.ended.remove(&run_id);
        self.unrecorded.remove(&run_id);
        let live = Live {
            id: run_id.clone(),
            run: Some(self.new_run(&run_id, &folder, state, connection, settings, secrets)),
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
            weakened: recorded.weakened,
            report: watch::Sender::new(None),
            writer,
            lock: Arc::new(lock),
            connection,
            checkpoint: None,
            progress_message: recorded
                .progress_message
                .as_deref()
                .and_then(ProgressMessage::parse),
            lifecycle,
            storage: None,
            warned_low: std::collections::BTreeSet::new(),
        };
        let row = self.index_row(&live, lifecycle, None);
        self.write_index(row).await;
        let run = live.run.clone().expect("invariant: a Run resumes live");
        run.watch_safe_points(timeout, live.ended_by);
        let (kind, needs) = (live.kind, live.needs);
        self.setup.metrics.run_live(kind);
        self.live.insert(run_id.clone(), live);
        self.setup
            .events
            .publish_axes(RunAxes {
                run_id,
                lifecycle,
                verdict: None,
                completeness: None,
            })
            .await;
        if let Some(interval) = checkpoint_interval {
            run.spawn(periodic_checkpoints(run.clone(), interval));
        }
        Ok(Resumed::New {
            run: Box::new(run),
            kind,
            needs,
        })
    }

    /// What the run file of `run_id` in `folder` records of the Run.
    async fn recorded(&self, run_id: &RunId, folder: &Path) -> Result<folder::Recorded, RunError> {
        let (read, vfs) = (folder.join(RUN_FILE), self.setup.run_state.vfs.clone());
        self.owner
            .blocking(move || folder::read_recorded(&read, vfs.as_deref()))
            .await
            .map_err(|error| RunError::Unavailable {
                run_id: run_id.clone(),
                folder: folder.to_owned(),
                error: error.to_string(),
            })?
            .ok_or_else(|| RunError::NotFound(run_id.clone()))
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

    /// Takes the writer role of `run_id`, `held` as its state says: its
    /// lock, which another writer's holding refuses as the Run's table says
    /// (`WriterFound`), the Checkpoint `checkpoint` made its run file when
    /// given, then the next run epoch, under which its writer opens and
    /// marks it running again.
    async fn reopen(
        &mut self,
        run_id: &RunId,
        held: Lifecycle,
        checkpoint: Option<PathBuf>,
        options: &RunStateOptions,
    ) -> Result<(File, Arc<TaskOwner>, RunState), RunError> {
        let folder = self.folder(run_id);
        let file = folder.join(RUN_FILE);
        let lock = self
            .lock(run_id, &folder)
            .await
            .map_err(|error| match error {
                StartError::InUse { run_id, folder } => {
                    match lifecycle::apply(held, RunEvent::WriterFound) {
                        Err(refused) => RunError::refused(&run_id, &folder, refused),
                        Ok(_) => RunError::InUse { run_id, folder },
                    }
                }
                StartError::Unavailable {
                    run_id,
                    folder,
                    error,
                } => RunError::Unavailable {
                    run_id,
                    folder,
                    error,
                },
                StartError::Internal { run_id, message } => RunError::Internal { run_id, message },
                StartError::Format {
                    format,
                    found,
                    compatibility,
                    install,
                } => RunError::Format {
                    run_id: run_id.clone(),
                    format,
                    found,
                    compatibility,
                    install,
                },
                StartError::ResourcesExhausted(exhausted) => {
                    RunError::ResourcesExhausted(exhausted)
                }
                StartError::Conflict { .. }
                | StartError::LimitReached(_)
                | StartError::LimitNotUnderstood(_)
                | StartError::Draining { .. }
                | StartError::Secret(_)
                | StartError::Stopped => RunError::Stopped,
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
        let (taking, tables, next) = (options.clone(), self.setup.tables.clone(), file.clone());
        let epoch = self
            .owner
            .blocking(move || run_state::next_epoch(&next, &taking, &tables, after))
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
        let state = RunState::open(file, options, self.setup.tables.clone(), epoch, &writer)
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

/// Announces that the Run `run_id` stops for `cause` (RT-14): migrating or
/// aborting when it is, then stopping.
async fn announce_stop(events: &Events, run_id: &RunId, cause: &StopReason) {
    if *cause == StopReason::Migrate {
        let migrating = RunMigrating {
            run_id: run_id.clone(),
        };
        events
            .publish(&RUN_MIGRATING, Some(run_id.as_str()), &migrating)
            .await;
    }
    if let StopReason::Abort { by } = cause {
        let aborting = RunAborting {
            run_id: run_id.clone(),
            by: *by,
        };
        events
            .publish(&RUN_ABORTING, Some(run_id.as_str()), &aborting)
            .await;
    }
    let stopping = RunStopping {
        run_id: run_id.clone(),
        cause: cause.clone(),
    };
    events
        .publish(&RUN_STOPPING, Some(run_id.as_str()), &stopping)
        .await;
}

/// Writes a Checkpoint of a Run every `interval` of its clock until it
/// stops; one that fails is logged and the next is tried, and one onto
/// a filesystem at the `low` storage level or worse is skipped, as the run
/// file still holds the state (ADR 0122).
async fn periodic_checkpoints(run: Run, interval: Duration) {
    loop {
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => return,
            () = run.clock.sleep(interval) => {}
        }
        let low = run
            .shared
            .storage
            .borrow()
            .of(&run.folder)
            .is_some_and(|filesystem| filesystem.level >= StorageLevel::Low);
        if low {
            tracing::debug!(run_id = %run.id, "a periodic Checkpoint was skipped: its filesystem is low on space");
            continue;
        }
        if let Err(error) = run.runs.checkpoint(run.id.clone(), None).await {
            tracing::warn!(%error, run_id = %run.id, "a periodic Checkpoint was not written");
            if let RunError::WriteFailed {
                folder, message, ..
            } = &error
            {
                let warning = RunWarning {
                    code: goodfetch_protocol::run::warning::FILE_WRITE_FAILED.to_owned(),
                    message: format!("A periodic Checkpoint could not be written ({message})."),
                    sessions: Vec::new(),
                    folder: Some(folder.display().to_string()),
                };
                if let Err(error) = run.warn(warning).await {
                    tracing::warn!(%error, run_id = %run.id, "the warning of a failed periodic Checkpoint was not kept");
                }
            }
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
    /// What tells whether its destination's filesystem is at `critical`.
    governor: Governor,
}

impl Taking {
    /// Writes it: a live Run's, of `live_epoch`, announced first; an
    /// interrupted Run's under its writer lock. A destination on a
    /// filesystem at the `critical` storage level fails at once with a
    /// full disk, before a byte is written (ADR 0122).
    async fn take(self, live_epoch: Option<u64>) -> Result<Checkpoint, RunError> {
        let (governor, destination) = (self.governor.clone(), self.destination.clone());
        let critical = self
            .tasks
            .blocking(move || governor.storage_at(&destination))
            .await
            .is_some_and(|filesystem| filesystem.level == StorageLevel::Critical);
        if critical {
            return Err(RunError::WriteFailed {
                run_id: self.run_id,
                message: format!(
                    "the filesystem of `{}` is at the critical storage level",
                    self.destination.display()
                ),
                folder: self.folder,
                reason: "storage_full",
                os_code: None,
            });
        }
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

/// Gives back what is left of the Slices `run` holds, under `writer`'s
/// blocking work, and answers the Limits that then apply to it, with what
/// is left of each (ADR 0101).
async fn given_back(run: &Run, writer: &TaskOwner) -> Vec<AppliedLimit> {
    let Some(mut charging) = run.shared.charging.lock().await.take() else {
        return Vec::new();
    };
    let now = run.runs.limits.now();
    writer
        .blocking(move || {
            if let Err(error) = charging.give_back() {
                tracing::warn!(%error, "the Run's Slices were not given back; the next engine to open the Usage ledger reclaims them");
            }
            charging.applied(now)
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "the Limits that apply to the Run could not be read; its report goes without them");
            Vec::new()
        })
}

/// Where a stopping Run's Checkpoint goes, and what writes it.
struct CheckpointTo {
    destination: PathBuf,
    vfs: Option<String>,
    nonce: u64,
    events: Events,
    governor: Governor,
}

/// What a stop of the Run `run_id` in `folder` that its run file could not
/// record answers its waiting call with (ADR 0122): its folder's filesystem
/// as `governor` reads it now, under `writer`'s blocking work, or a full one
/// when it cannot be read.
async fn unrecorded(
    run_id: RunId,
    folder: PathBuf,
    governor: &Governor,
    writer: &TaskOwner,
) -> Box<StorageExhausted> {
    let (governor, probed) = (governor.clone(), folder.clone());
    let filesystem = writer
        .blocking(move || governor.storage_at(&probed))
        .await
        .unwrap_or_else(|| Filesystem {
            folders: vec![folder.clone()],
            level: StorageLevel::Critical,
            free: 0,
            limit: None,
        });
    Box::new(StorageExhausted {
        run_id,
        folder,
        filesystem,
    })
}

/// Where a Run's end is recorded beside its run file: its Run index row,
/// its axes on the event stream, and the registry.
struct Recording {
    index: PathBuf,
    version: ProductVersion,
    row: RunIndexRow,
    events: Events,
    registry: mpsc::WeakSender<Command>,
}

impl Recording {
    /// Ends `closing`'s Run, then records its end; a stop its run file could
    /// not record goes to the registry alone, its Run index row left as it
    /// was, as after a crash (ADR 0122).
    async fn end(self, closing: Closing) {
        let run_id = closing.run.id.clone();
        let report = match closing.end().await {
            Ended::Report(report) => *report,
            unrecorded @ Ended::Unrecorded(_) => {
                if let Some(registry) = self.registry.upgrade() {
                    let _ = registry
                        .send(Command::Ended(run_id, Box::new(unrecorded)))
                        .await;
                }
                return;
            }
        };
        let ended = RunIndexRow {
            lifecycle: report.lifecycle,
            verdict: Some(report.verdict),
            completeness: Some(report.completeness),
            ended_at: Some(report.ended_at.clone()),
            owner: None,
            ..self.row
        };
        if let Err(error) = run_index::write(&self.index, &self.version, &ended) {
            tracing::warn!(%error, "the Run index row was not written; `runs reindex` restores it");
        }
        self.events
            .publish_axes(RunAxes {
                run_id: run_id.clone(),
                lifecycle: report.lifecycle,
                verdict: Some(report.verdict),
                completeness: Some(report.completeness),
            })
            .await;
        if let Some(registry) = self.registry.upgrade() {
            let _ = registry
                .send(Command::Ended(
                    run_id,
                    Box::new(Ended::Report(Box::new(report))),
                ))
                .await;
        }
    }
}

/// A Run on its way to its end.
struct Closing {
    run: Run,
    writer: Arc<TaskOwner>,
    lock: Arc<File>,
    started_at: PointInTime,
    policy_version: Option<String>,
    weakened: Vec<WeakenedSetting>,
    clock: Clock,
    end: End,
    /// The Lifecycle the Run's table gave its end.
    lifecycle: Lifecycle,
    /// A stop's Checkpoint, written once its end is.
    checkpoint_to: Option<CheckpointTo>,
    /// The last Checkpoint written before.
    last_checkpoint: Option<Checkpoint>,
    /// What the composition's areas add to its report.
    parts: Vec<Arc<dyn ReportPart>>,
    /// Its page in the Console, when the engine serves one.
    console_url: Option<String>,
    /// What reads the filesystem of a stop its run file could not record.
    governor: Governor,
}

impl Closing {
    /// Stops the Run's tasks, gives back its Slices, writes its end into
    /// run state with its report, assembled there (ADR 0102), a stop's
    /// Checkpoint after it, closes its writer and frees its lock. An end
    /// whose commit meets a full disk or an I/O error is unrecorded (ADR
    /// 0122).
    async fn end(self) -> Ended {
        let Self {
            run,
            writer,
            lock,
            started_at,
            policy_version,
            weakened,
            clock,
            end,
            lifecycle,
            checkpoint_to,
            last_checkpoint,
            parts,
            console_url,
            governor,
        } = self;
        run.owner.stop().await;
        let (stop_reason, error, finished) = match end {
            End::Finished(reasons) => (None, None, reasons),
            End::Failed(error) => (None, Some(error), Vec::new()),
            End::Stopped(reason) => (Some(reason), None, Vec::new()),
        };
        let limits = given_back(&run, &writer).await;
        let facts = EndFacts {
            run_id: run.id.clone(),
            lifecycle,
            stop_reason,
            started_at,
            ended_at: point_in_time(&clock),
            policy_version,
            weakened,
            settings: run
                .settings()
                .map(|settings| settings.resolved().explain(&[], Vec::new()).settings),
            checkpoint: None,
            console_url,
            limits,
            error,
            finished,
        };
        let state = run.state.clone();
        let (run_id, folder) = (run.id.clone(), run.folder.clone());
        drop(run);
        let bare = facts.bare();
        let ended = state
            .write(move |connection| {
                let report = report::assemble(connection, &facts, &parts)?;
                write_end(connection, &report)?;
                Ok(report)
            })
            .await;
        let written = ended.is_ok();
        let mut report = match ended {
            Ok(report) => report,
            Err(RunStateError::Storage { message, .. }) => {
                tracing::warn!(%message, run_id = %run_id, "the Run's stop was not recorded: its run file met a full disk or an I/O error, and it is left interrupted");
                drop(state);
                let exhausted = unrecorded(run_id, folder, &governor, &writer).await;
                writer.wait().await;
                drop(lock);
                return Ended::Unrecorded(exhausted);
            }
            Err(error) => {
                tracing::warn!(%error, "the Run's end was not written to its run state");
                bare
            }
        };
        report.checkpoint = last_checkpoint;
        if let Some(to) = checkpoint_to
            && written
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
                governor: to.governor,
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
        Ended::Report(Box::new(report))
    }
}

mod folder;
mod import;
pub mod lifecycle;
pub mod outcome;
mod reader;
mod reindex;
mod report;
mod slices;
mod takeover;
mod writer;

/// Why a Run's admission did not open.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdmissionError {
    /// Its run file could not be read.
    #[error(transparent)]
    RunState(#[from] RunStateError),
    /// The Politeness it shares could not be opened.
    #[error(transparent)]
    Politeness(#[from] PolitenessError),
}

/// What a Run's admission opens from: the gate in admission the
/// composition passes in, if any, and the Politeness the engine's Runs
/// share.
struct Admitting {
    gates: Option<Arc<dyn Gates>>,
    politeness: SharedPoliteness,
}

/// The per-user Politeness file the engine's Runs share, opened the first
/// time one shares it (ADR 0053), as one engine of the user.
struct SharedPoliteness {
    file: Arc<tokio::sync::OnceCell<Politeness>>,
    path: PathBuf,
    options: PolitenessOptions,
    tables: Vec<PolitenessTables>,
    engine: String,
    owner: Arc<TaskOwner>,
}

impl SharedPoliteness {
    /// The file, opened now if it is not yet.
    async fn file(&self) -> Result<Politeness, PolitenessError> {
        self.file
            .get_or_try_init(|| {
                Politeness::open(
                    self.path.clone(),
                    &self.options,
                    self.tables.clone(),
                    holder_gone,
                    self.engine.clone(),
                    &self.owner,
                )
            })
            .await
            .cloned()
    }
}

/// Whether the Run in `folder` that took Permits under `epoch` is gone
/// (ADRs 0041 and 0053): its writer lock free, or held while its run file
/// holds a later epoch. It blocks.
fn holder_gone(folder: &Path, epoch: u64) -> bool {
    match test_writer(folder) {
        Ok(Writer::None) => true,
        Ok(Writer::Live) => {
            crate::run_state::epoch_of(&folder.join(RUN_FILE)).is_some_and(|found| found > epoch)
        }
        Err(_) => false,
    }
}
