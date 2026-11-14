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
//! Run index row and published as `run.progress`. A Run's progress (ADR
//! 0102) changes as its registry and its driver change it, a Human pause
//! beginning or ending included; a read follows it while this engine works
//! the Run and reads run state otherwise, and a read of a Run another
//! engine works goes to that engine, the owner the Run index names. On a Daemon, each Run belongs to the Connection that started it,
//! whose end stops it (ADR 0078).
//!
//! Size: one registry actor, its handles and the commands it answers, kept
//! whole so every change to a Run's state passes through one loop.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::Instrument as _;

use goodfetch_protocol::audit::{Actor, CERTIFICATE_EXCEPTIONS_USED};
use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::checkpoint::{
    RUN_ABORTING, RUN_CHECKPOINTING, RUN_MIGRATING, RUN_STOPPING, RunAborting, RunBinding,
    RunCheckpointing, RunMigrating, RunStopping,
};
use goodfetch_protocol::connection::{ProductVersion, StoppedBy};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::governor::{RUN_SHARE, SystemInfo};
use goodfetch_protocol::jsonrpc::ErrorObject;
use goodfetch_protocol::page::BodyHandle;
use goodfetch_protocol::politeness::POLITENESS_SHARING;
use goodfetch_protocol::run::{
    AppliedLimit, AxisReason, Checkpoint, Completeness, EndingError, HumanPause, Lifecycle,
    LimitScope, MAX_PROGRESS_MESSAGE_CHARACTERS, NotUnderstoodLimit, NothingDone, PointInTime,
    Presence, ProgressMessage, RUN_IN_USE, RUN_STATE_BATCH_BYTES, RUN_STATE_BATCH_INTERVAL,
    RUN_STATE_BATCH_PAGES, RUN_STATE_BATCH_PER_ATTEMPT, RUN_STATE_BATCH_SYNC,
    RUN_STATE_COMMIT_INTERVAL, ReachedLimit, RunAxes, RunId, RunIndexRow, RunKind, RunListParams,
    RunOwner, RunProgress, RunReport, RunStarted, RunStorage, RunUsage, RunWarning, RunWriter,
    StartInput, StartedAs, StopReason, Tags,
};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    KEY_REFERENCE_IN_KEYCHAIN, RUN_STATE_KEY_REFERENCE, SecretFailure, TIMEOUT_SECRET,
};
use goodfetch_protocol::security::SECURITY_REDACTION;
use goodfetch_protocol::settings::{
    ExplainedSetting, SettingsLayer, WALL_REFETCH_DOMAINS, WeakenedSetting,
};
use goodfetch_protocol::storage::{SampledFolder, StorageLevel};
use goodfetch_protocol::usage::Limit;
use rusqlite::Connection;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

pub use self::clean_up::{Answering, Chosen, CleanUpVersions, Fallback, Holding, RulesContext};
pub use self::import::fuzz_run_state_import;
pub use self::progress::{PartValue, PauseHeld, ProgressPart, TypedPart, TypedParts};
pub use self::reader::{RunFileOpener, RunReader};
pub use self::reindex::{Reindexed, reindex};
pub use self::report::ReportPart;
pub use self::writer::{Writer, test_writer};

use self::ended::{EndedHere, EndedRuns};
use self::folder::{
    Found, RunRow, canonical_inputs, input_differences, masked_tags, read_found, read_report, text,
    write_end,
};
use self::lifecycle::{InvalidRunTransition, Next, RunEvent};
use self::progress::{Progress, Progressing};
use self::reader::Following;
use self::report::EndFacts;
pub(crate) use self::slices::Limits;
use self::takeover::TakeOver;
use self::writer::{LockError, retried, take_lock, try_lock};
use crate::CallError;
use crate::admission::{Admission, Gates};
use crate::audit::{AuditLog, Origin};
use crate::body;
use crate::checkpoint::{self, CHECKPOINT_FILE, WriteError};
use crate::clock::Clock;
use crate::controls::Controls;
use crate::endpoint::Refusals;
use crate::engine_context::{CALLER, EngineContext};
use crate::entropy::Entropy;
use crate::events::Events;
use crate::governor::{
    Filesystem, Governor, Held, LastResort, RunShare, StorageSample, last_resort_run,
};
use crate::home::RunStateLocation;
use crate::known_certificates::KnownCertificates;
use crate::ledger::{
    Change, Charged, Charging, Counted, LedgerError, Refusal, Reservation, SliceId, Unmade,
};
use crate::local_addresses::{self, Rotation};
use crate::mask::{Masks, Redaction, Scope};
use crate::metrics::{FolderStorage, Metrics};
use crate::politeness::{Holder, Politeness, PolitenessError, PolitenessOptions, PolitenessTables};
use crate::robots_txt::{Answer, Copies, Fetched, NotRead, OwnCopies, RunCopies};
use crate::run_files::{self, FileKind, KeepError, Kept, KeptExport};
use crate::run_index::{self, RunIndex};
use crate::run_key::{self, KeyError, RunKey, Sealed, StoredKey};
use crate::run_state::install::InstallDatabase;
use crate::run_state::{
    self, BatchSync, Durability, RUN_FILE, ReservedWrite, RunState, RunStateError, RunStateOptions,
    RunStateTables, SecretColumn, Syncing,
};
use crate::run_trace;
use crate::sandbox_probe::SandboxProbe;
use crate::secrets::{Purpose, RunSecrets, SecretError, Secrets, is_engine_key};
use crate::settings::{Resolved, RunSettings, get_or_default};
use crate::staging::AttemptWrites;
use crate::tasks::{OwnerKind, TaskOwner};
use crate::trace::Link;
use crate::transition;

/// How often a report read of a Run another engine writes, whose owner did
/// not answer it, reads the Run's file again for its end (ADR 0102).
const REPORT_POLL: Duration = Duration::from_secs(1);

/// The identity of the machine a Goodfetch home is on, in that home: what a
/// Run bound to its machine records (ADR 0056).
const MACHINE_FILE: &str = "machine-id";

/// How many commands wait for the registry before their senders wait.
const COMMANDS: usize = 256;

/// What the registry starts from.
pub(crate) struct RunsSetup {
    /// What the engine gives its Runs: its home, clock and settings, and
    /// the user's Audit log, known-certificates store and Usage ledger.
    pub(crate) engine: EngineContext,
    /// The engine's Secret-provider driver, which each Run resolves its
    /// Secret references through.
    pub(crate) secrets: Secrets,
    /// Every live Run's resolved values, which each Run's own join.
    pub(crate) masks: Masks,
    /// The engine's Product version, which names its per-major folder.
    pub(crate) version: ProductVersion,
    /// Where the run folders are.
    pub(crate) location: RunStateLocation,
    /// How run files open.
    pub(crate) run_state: RunStateOptions,
    /// The install database `run_state`'s VFS serves the run files from,
    /// in the per-install layout, kept open while the registry lives.
    pub(crate) install: Option<Arc<InstallDatabase>>,
    /// The tables the composition adds to every run file.
    pub(crate) tables: Vec<RunStateTables>,
    /// How the Runs of each kind the composition registers a driver for go.
    pub(crate) driving: BTreeMap<RunKind, Driving>,
    /// The gate in admission the composition passes in, if any.
    pub(crate) gates: Option<Arc<dyn Gates>>,
    /// How the per-user Politeness file opens.
    pub(crate) politeness: PolitenessOptions,
    /// The tables the composition adds to the Politeness file.
    pub(crate) politeness_tables: Vec<PolitenessTables>,
    /// The engine's entropy, from which Run IDs draw.
    pub(crate) entropy: Entropy,
    /// The engine's event stream, which each axis change reaches.
    pub(crate) events: Events,
    /// The engine's Product, which a Run must not need more than for the
    /// engine to take its writer role (ADR 0098).
    pub(crate) product: Product,
    /// Where other engines reach this one, which the Run index names as
    /// the owner of its live Runs: a Daemon's endpoint, or an Attached
    /// engine's private endpoint; none when it has neither.
    pub(crate) reached_at: Option<RunOwner>,
    /// How a live action reaches the owner the Run index names.
    pub(crate) forwarder: Arc<dyn Forwarder>,
    /// The runtime's metrics, which count the live Runs and their ends.
    pub(crate) metrics: Metrics,
    /// The engine's Resource governor, which each live Run joins.
    pub(crate) governor: Governor,
    /// What the composition's areas add to each Run's report (ADR 0102).
    pub(crate) report_parts: Vec<Arc<dyn ReportPart>>,
    /// What the composition's areas add to each Run's progress from its
    /// committed state (ADR 0102).
    pub(crate) progress_parts: Vec<Arc<dyn ProgressPart>>,
    /// The composition's Clean-up rules, which choose the version each new
    /// Run holds to (ADR 0047), if it has any.
    pub(crate) clean_up: Option<Arc<dyn CleanUpVersions>>,
    /// The switchable controls the composition registered, which each new
    /// Run reports when its settings switch one off.
    pub(crate) controls: Controls,
}

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

    /// Asks the engine at `owner`, as a Client that needs `needs`, for the
    /// read `method` with `params` of a Run it works (ADR 0102), its
    /// blocking work under `tasks`, and answers what it answered: its
    /// result, or the failure it answered; `None` when it did not answer.
    fn read<'a>(
        &'a self,
        owner: &'a RunOwner,
        needs: Product,
        method: &'static str,
        params: Value,
        tasks: &'a TaskOwner,
    ) -> ReadAnswer<'a>;
}

/// What a read forwarded to a Run's owner answers: its result, or the
/// failure it answered; `None` when it did not answer.
pub(crate) type ReadAnswer<'a> =
    Pin<Box<dyn Future<Output = Option<Result<Value, ErrorObject>>> + Send + 'a>>;

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

/// How the Runs of a kind go, which the contribution that registers the
/// kind's driver says (ADR 0099): the engine names no kind's own settings
/// or habits. A kind no contribution registers goes as [`Driving::default`]
/// says: a Run its program drives.
#[derive(Clone, Copy)]
pub struct Driving {
    /// Who ends a Run of the kind that a resume starts again.
    pub ended_by: EndedBy,
    /// Whether its claims defer their charges (ADR 0138's note of #1926):
    /// its start then declares their bound in its own synced commit rather
    /// than its first claim's (#2117).
    pub defers_charges: bool,
    /// The setting that says whether its Run trace keeps a Provenance log
    /// (ADR 0138); none when it always does.
    pub provenance_log: Option<&'static Setting>,
}

impl Default for Driving {
    fn default() -> Self {
        Self {
            ended_by: EndedBy::Finish,
            defers_charges: false,
            provenance_log: None,
        }
    }
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
    /// Whether `run.retry_failed` reopens it, the Run's event
    /// `RetryFailed`, which also returns a `finished` Run to `running`,
    /// rather than a resume (ADR 0102).
    pub retry_failed: bool,
}

impl Resume {
    /// The Run's event a resume is: `RetryFailed` for `run.retry_failed`'s,
    /// else `Resume`.
    fn event(retry_failed: bool) -> RunEvent {
        if retry_failed {
            RunEvent::RetryFailed
        } else {
            RunEvent::Resume
        }
    }
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
        /// The settings its report listed as weakened before this resume,
        /// which a switch the resume's settings turn off is reported beyond
        /// (ADR 0016).
        listed: Vec<WeakenedSetting>,
    },
    /// The Run is live in this engine, or finished: answered as it is.
    Existing(RunStarted),
}

impl Resumed {
    /// The Run `run_id`, answered as it is: at `lifecycle`, needing `needs`.
    fn existing(run_id: RunId, lifecycle: Lifecycle, needs: Product) -> Self {
        Self::Existing(RunStarted {
            run_id,
            started_as: StartedAs::Existing,
            lifecycle,
            needs,
        })
    }
}

/// What a start found.
#[expect(
    clippy::large_enum_variant,
    reason = "a start answers once and its caller takes the Run out at once: boxing it would allocate on every start for no saving"
)]
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
    /// This engine is discarding the Run of this Run ID: its discard holds
    /// its writer lock until it ends.
    #[error("the Run `{run_id}` is being discarded")]
    Discarding {
        /// The Run ID.
        run_id: RunId,
        /// Its folder.
        folder: PathBuf,
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
    /// The composition refused the start's settings, such as a version of
    /// the Clean-up rules it cannot have (ADR 0047).
    #[error("the start was refused: {0}")]
    Refused(Box<CallError>),
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
    folders: &[&'a PathBuf],
) -> Option<(&'a PathBuf, &'a Filesystem)> {
    folders
        .iter()
        .copied()
        .filter_map(|folder| sample.of(folder).map(|filesystem| (folder, filesystem)))
        .fold(
            None,
            |worst: Option<(&PathBuf, &Filesystem)>, found| match worst {
                Some((_, kept)) if kept.level >= found.1.level => worst,
                _ => Some(found),
            },
        )
}

/// The engine's own folders the governor samples while the registry lives
/// (ADR 0122), and what each holds.
struct EngineFolders {
    /// The folder of the filesystem holding run state: the per-major
    /// folder, which holds the install database, in the per-install
    /// layout, the run folders' otherwise.
    run_state: PathBuf,
    /// The run folders' folder, which holds each Run's lock, its page
    /// bodies and its default Checkpoint in either layout.
    runs: PathBuf,
    /// The Goodfetch home, which holds the Usage ledger and the Politeness
    /// file.
    home: PathBuf,
    /// The temporary folder.
    temporary: PathBuf,
}

impl EngineFolders {
    /// Each folder the runtime's metrics give series of, with what it
    /// holds (ADR 0050).
    fn labelled(&self) -> [(SampledFolder, &PathBuf); 3] {
        [
            (SampledFolder::RunState, &self.run_state),
            (SampledFolder::Home, &self.home),
            (SampledFolder::Temp, &self.temporary),
        ]
    }

    /// Every folder sampled, each once.
    fn sampled(&self) -> Vec<PathBuf> {
        let mut sampled = Vec::new();
        for folder in [&self.run_state, &self.runs, &self.home, &self.temporary] {
            if !sampled.contains(folder) {
                sampled.push(folder.clone());
            }
        }
        sampled
    }

    /// The folders of the engine's essential writes, run state's first: no
    /// Run goes on while one's filesystem is at `critical`, so none starts
    /// and the Daemon is not ready (ADRs 0050 and 0122).
    fn essential(&self) -> Vec<PathBuf> {
        vec![self.run_state.clone(), self.home.clone()]
    }
}

/// The first of `folders` whose filesystem `governor` reads at `critical`
/// now, with it. It blocks: run it under a blocking policy.
pub(crate) fn critical_among(
    governor: &Governor,
    folders: impl IntoIterator<Item = PathBuf>,
) -> Option<(PathBuf, Filesystem)> {
    folders.into_iter().find_map(|folder| {
        let filesystem = governor
            .storage_at(&folder)
            .filter(Filesystem::is_critical)?;
        Some((folder, filesystem))
    })
}

/// The `storage_low` warning of a Run whose worst filesystem is `storage`
/// (ADR 0122): its folder, its free space and what the Run sheds there.
/// The code of the warning a Run resumed after an unclean end hands over
/// when its reclaimed Slices may have been charged past what it used (ADR
/// 0138's note of #1926).
pub(crate) const LIMIT_MAY_BE_OVER_CHARGED: &str = "limit_may_be_over_charged";

/// Hands over [`LIMIT_MAY_BE_OVER_CHARGED`] once for `run`, resumed after an
/// unclean end, when an earlier run epoch charged a Limit in writes a power
/// cut could have taken: the reclaim then charged their Slices past its
/// journal, so a Limit may be over-charged, never under-charged.
async fn warn_over_charged(run: &Run) {
    let epoch = run.state.epoch();
    let beyond = run
        .state
        .read(move |connection| slices::reclaimed_beyond(connection, epoch))
        .await;
    let message = match beyond {
        Ok(Some(slices::OverCharge::WholeSlices)) => "the Run ended uncleanly and its charges were answered before they were synced, in a Run-state batch or at a `run_state.batch.sync` other than `each`, so the Usage ledger charged the whole of its earlier Slices: its Limits may be over-charged by what it did not use of them".to_owned(),
        Ok(Some(slices::OverCharge::UpTo(requests))) => format!(
            "the Run ended uncleanly, so the Usage ledger charged its earlier Slices up to {requests} past what its journal held, for the requests that may have been in flight: its Limits may be over-charged by as much"
        ),
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "whether the Run's Limits may be over-charged could not be read");
            return;
        }
    };
    let warning = RunWarning {
        code: LIMIT_MAY_BE_OVER_CHARGED.to_owned(),
        message,
        sessions: Vec::new(),
        folder: None,
    };
    if let Err(error) = run.warn_once(warning).await {
        tracing::warn!(%error, run_id = %run.id(), "the Run's possible over-charge could not be handed over");
    }
}

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

/// What a Run charged with [`Run::charge_unjournaled`] that no journal
/// holds yet: the Slices its charges took from, and what they counted in
/// the Run's Usage, which [`Run::write_charged`] journals.
#[derive(Debug, Default)]
pub struct UnjournaledCharges {
    slices: Vec<SliceId>,
    counted: Vec<Counted>,
}

impl UnjournaledCharges {
    /// Whether no charge is held.
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty() && self.counted.is_empty()
    }

    /// Adds `charged`: each Slice it touched once, and its counts summed
    /// with those of the same kind, currency and part.
    fn add(&mut self, charged: Charged) {
        for (slice, _) in charged.used {
            if !self.slices.contains(&slice) {
                self.slices.push(slice);
            }
        }
        for counted in charged.counted {
            match self.counted.iter_mut().find(|held| {
                held.usage_kind == counted.usage_kind
                    && held.currency == counted.currency
                    && held.part == counted.part
            }) {
                Some(held) => held.delta += counted.delta,
                None => self.counted.push(counted),
            }
        }
    }
}

/// What a Run reserved ahead of a part of a Usage kind it is about to use,
/// such as a Session's proxy bytes under its proxy kind (ADRs 0021 and
/// 0041): the amount it grants, and that amount's cost when priced, each a
/// reservation settled at what was used ([`Run::settle_part`]).
#[derive(Debug)]
#[must_use = "an allowance is settled once its use is known"]
pub struct Allowance {
    granted: u64,
    amount: Reservation,
    price: Option<Reservation>,
    part: String,
}

impl Allowance {
    /// The amount it grants.
    pub fn granted(&self) -> u64 {
        self.granted
    }
}

/// What a live Run resolves its Secret references through, and its data
/// key once unwrapped.
struct Keys {
    secrets: RunSecrets,
    key: Option<RunKey>,
}

/// A resumed Run's data key, unwrapped, with the wrapped key its file held.
struct Unwrapped {
    stored: StoredKey,
    key: RunKey,
}

/// Whether a resolution of the key-encryption key makes the engine's own
/// key where it is missing: a Run's first seal does, a resume never, as a
/// new key would open no data key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Making {
    Yes,
    No,
}

/// Why a Run has no data key.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunKeyError {
    /// The key-encryption key gave no value.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The data key could not be made.
    #[error(transparent)]
    Key(#[from] KeyError),
    /// The wrapped data key could not be written.
    #[error(transparent)]
    State(#[from] RunStateError),
}

/// Why the Run index could not be listed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ListError {
    /// The registry has stopped.
    #[error("the engine's Runs have stopped")]
    Stopped,
    /// The Run index's marker refuses this engine (ADRs 0066 and 0098).
    #[error("the Run index is {format} {found}, which this engine cannot read")]
    Format {
        /// The format's name.
        format: &'static str,
        /// The index's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
        /// The Product version to install, when its lowest minor refused
        /// this engine.
        install: Option<String>,
    },
    /// The Run index cannot be read.
    #[error("the Run index cannot be read: {0}")]
    Unreadable(String),
}

impl From<RunStateError> for ListError {
    fn from(error: RunStateError) -> Self {
        match error {
            RunStateError::Format {
                format,
                found,
                compatibility,
                install,
            } => Self::Format {
                format,
                found,
                compatibility,
                install,
            },
            other => Self::Unreadable(other.to_string()),
        }
    }
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
    /// The key-encryption key of a Run's data key gave no value, as a
    /// resume resolved it.
    #[error("the key-encryption key of `{run_id}` gave no value: {error}")]
    Secret {
        /// The Run ID.
        run_id: RunId,
        /// The resolution's failure.
        error: Box<SecretError>,
    },
    /// The key-encryption key `run_state.key_reference` names does not
    /// open the Run's data key.
    #[error("the key-encryption key `{reference}` does not open the data key of `{run_id}`")]
    KeyMismatch {
        /// The Run ID.
        run_id: RunId,
        /// The reference the resume resolved.
        reference: SecretReference,
        /// The reference of the key that wrapped the data key.
        wrapped_by: String,
    },
    /// A filesystem the Run needs is at the `critical` storage level (ADR
    /// 0122).
    #[error("the filesystem of `{}` is at the critical storage level", .0.folder.display())]
    ResourcesExhausted(Box<StorageExhausted>),
    /// This engine still runs the Run, so it is not discarded.
    #[error("the Run `{0}` is live")]
    Live(RunId),
    /// The Run has no live writer and has not ended, so it has no report
    /// (ADR 0102).
    #[error("the Run `{0}` is interrupted")]
    Interrupted(RunId),
    /// This engine is discarding the Run: its discard holds its writer
    /// lock until it ends, and nothing of it is read meanwhile.
    #[error("the Run `{run_id}` is being discarded")]
    Discarding {
        /// The Run ID.
        run_id: RunId,
        /// Its folder.
        folder: PathBuf,
    },
    /// The engine that works the Run answered a read forwarded to it with
    /// this failure (ADR 0102).
    #[error("the Run's owner answered `{}`", .0.data.code)]
    Forwarded(Box<ErrorObject>),
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

/// What a progress read follows: a live Run's changes, or none for one
/// this engine does not run, its reader, its composition's parts and the
/// engine's clock.
struct Followed {
    changes: Option<watch::Receiver<Progressing>>,
    reader: RunReader,
    parts: Vec<Arc<dyn ProgressPart>>,
    clock: Clock,
}

/// Where a read of a Run this engine does not run looks for the engine
/// that does (ADR 0102).
struct Located {
    folder: PathBuf,
    index: RunIndex,
    reached_at: Option<RunOwner>,
    forwarder: Arc<dyn Forwarder>,
    tasks: Arc<TaskOwner>,
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
                StopReason::EnginePanicked => RunEvent::EnginePanicked,
                StopReason::Resources { .. } => RunEvent::ResourcesExhausted,
            },
        }
    }
}

/// Why the engine stops every live Run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cause {
    /// A drain was asked for, or an Attached engine's program went away.
    Asked,
    /// A part every Run shares panicked (ADR 0148): each Run's report
    /// carries the warning `engine_panicked`.
    Panic,
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
    StopAll(Cause, oneshot::Sender<Vec<watch::Receiver<Option<Ended>>>>),
    StopConnection(ConnectionId, oneshot::Sender<ConnectionEnded>),
    Outlive(ConnectionId),
    LiveCount(oneshot::Sender<usize>),
    Health(oneshot::Sender<RegistryHealth>),
    Folder(RunId, oneshot::Sender<PathBuf>),
    Governor(oneshot::Sender<Governor>),
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
        Box<(Resume, Link, Keys, Option<StoredKey>)>,
        Option<ConnectionId>,
        oneshot::Sender<Result<Resumed, RunError>>,
    ),
    StoredKey(
        RunId,
        Option<PathBuf>,
        oneshot::Sender<Result<Option<StoredKey>, RunError>>,
    ),
    Machine(oneshot::Sender<Result<String, RunError>>),
    GenerateId(oneshot::Sender<RunId>),
    SafeStop(RunId, StoppedBy, oneshot::Sender<Result<Waiting, RunError>>),
    Reader(RunId, oneshot::Sender<Result<RunReader, RunError>>),
    Progress(RunId, oneshot::Sender<Result<Followed, RunError>>),
    Locate(RunId, oneshot::Sender<Result<Option<Located>, RunError>>),
    Index(oneshot::Sender<RunIndex>),
    Discard(RunId, oneshot::Sender<Result<(), RunError>>),
    Discarded(Box<Erased>, oneshot::Sender<Result<(), RunError>>),
    Drain(Cause, oneshot::Sender<Vec<watch::Receiver<Option<Ended>>>>),
    Storage(Arc<StorageSample>),
    StorageFailed(PathBuf),
    LastResort(LastResort),
}

/// What the registry answers about the engine's health (ADR 0050): whether
/// it drains, the folder its run folders go in, and the folders of its
/// essential writes, whose filesystems readiness reads afresh.
pub(crate) struct RegistryHealth {
    /// Whether the engine drains, taking no Run (ADR 0049).
    pub(crate) draining: bool,
    /// The folder of the run folders.
    pub(crate) folder: PathBuf,
    /// Whether the Resource governor is at its last resort (ADR 0040).
    pub(crate) governor_last_resort: bool,
    /// The folders of the essential writes: run state's and the Goodfetch
    /// home (ADR 0122).
    pub(crate) essential: Vec<PathBuf>,
    /// What reads their filesystems now.
    pub(crate) governor: Governor,
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
    /// What the engine gives its Runs.
    engine: EngineContext,
    connection: Option<ConnectionId>,
    limits: Arc<Limits>,
    /// The Secret-provider driver, which each Run resolves through.
    secrets: Secrets,
    /// The composition's Clean-up rules, which choose the version each new
    /// Run holds to (ADR 0047), if it has any.
    clean_up: Option<Arc<CleanUpRules>>,
    /// The switchable controls each new Run reports.
    controls: Arc<Controls>,
    /// The runtime's metrics.
    metrics: Metrics,
    /// The owner of the registry's own task, whose panic stops the engine
    /// at once (ADR 0148).
    registry: Arc<TaskOwner>,
}

/// The composition's Clean-up rules, and the engine's Product version, the
/// release whose code applies them.
struct CleanUpRules {
    versions: Arc<dyn CleanUpVersions>,
    product_version: ProductVersion,
}

impl CleanUpRules {
    /// Those of `setup`, if its composition has any.
    fn of(setup: &RunsSetup) -> Option<Arc<Self>> {
        let versions = setup.clean_up.clone()?;
        Some(Arc::new(Self {
            versions,
            product_version: setup.version.clone(),
        }))
    }
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
    /// Its Slices of the Limits that hold it, until it ends: taken in the
    /// ledger's blocking calls and as the Run is made, and tried, never
    /// waited for, by a change that is a step in memory (a charge or a
    /// reservation its Slices cover, a settlement), which calls nothing
    /// that blocks; never held across an await.
    #[expect(
        clippy::disallowed_types,
        reason = "the ledger's calls block: one Mutex guards a Run's charging, taken inside them, or tried without waiting, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
    )]
    charging: std::sync::Mutex<Option<Charging>>,
    /// The runtime's metrics.
    metrics: Metrics,
    /// Its `tracing` span, a root of its own (ADR 0132), which each of its
    /// tasks runs in.
    span: tracing::Span,
    /// What it resolves its Secret references through.
    secrets: RunSecrets,
    /// Its data key, once made or unwrapped.
    key: tokio::sync::OnceCell<RunKey>,
    /// Its share of the Resource governor's admissions.
    governor: RunShare,
    /// Its folder, which the governor samples while the Run lives (ADR
    /// 0122).
    _sampled: Held,
    /// The governor's last storage sample.
    storage: watch::Receiver<Arc<StorageSample>>,
    /// Its progress (ADR 0102).
    progress: Progress,
    /// The Resource governor's last sample.
    system_info: watch::Receiver<Option<SystemInfo>>,
    /// The Politeness its own code shares outside admission, once it asked
    /// for it.
    politeness: tokio::sync::OnceCell<Politeness>,
    /// Its own copies of the robots.txt files its program's navigation
    /// read, one per origin (ADRs 0086 and 0098).
    robots_txt: RunCopies,
    /// The copy a crawl's Run reads instead, once its crawl shared one.
    crawls_robots_txt: std::sync::OnceLock<Arc<dyn Copies>>,
    /// Where its crawl's attempts stage the writes made through a
    /// handler's `ctx.run`, once its crawl registered it.
    attempt_writes: std::sync::OnceLock<Arc<dyn AttemptWrites>>,
    /// Its Run trace's Provenance log, which holds its file (ADR 0140's E5).
    provenance: run_trace::Provenance,
}

impl Shared {
    /// Its charging, locked: called inside the ledger's blocking calls,
    /// and as the Run is made, before anything else can hold it.
    fn charging(&self) -> std::sync::MutexGuard<'_, Option<Charging>> {
        self.charging
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Its charging, when nothing else holds it: for a step in memory,
    /// outside the blocking calls, which never waits for the lock. A
    /// ledger call holds it, a claim's journal as it is queued, and
    /// another task's step in memory on a runtime of several threads.
    fn charging_if_free(&self) -> Option<std::sync::MutexGuard<'_, Option<Charging>>> {
        match self.charging.try_lock() {
            Ok(held) => Some(held),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
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

    /// `plaintext` sealed under its data key for the row `row` of the secret
    /// column `column`: the only value its owner writes there. Its first
    /// call makes the data key.
    ///
    /// # Errors
    ///
    /// As [`RunKeyError`] says.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future of the caller that makes the key abandons the
    /// making, and the next caller makes one again. A drop once the
    /// wrapped key's write was queued may leave it stored: the next making
    /// then takes that key rather than store another.
    pub async fn seal(
        &self,
        column: &SecretColumn,
        row: &str,
        plaintext: &[u8],
    ) -> Result<Sealed, RunKeyError> {
        let key = self.key().await?;
        Ok(Sealed::new(key.seal(
            column.table,
            column.column,
            row,
            plaintext,
        )?))
    }

    /// The plaintext of `sealed`, read from the row `row` of the secret
    /// column `column`, in this Run or after its resume. It never makes a
    /// data key: a Run with none held, unwrapped at its resume or stored
    /// by a making whose caller was dropped, sealed nothing, so nothing it
    /// holds opens.
    ///
    /// # Errors
    ///
    /// `Key` when the value was sealed for another place or another Run,
    /// or changed since, or when the Run has no data key.
    ///
    /// # Cancel safety
    ///
    /// Dropping its future abandons the read of the stored key, which
    /// writes nothing; the next call reads it again.
    pub async fn open(
        &self,
        column: &SecretColumn,
        row: &str,
        sealed: &Sealed,
    ) -> Result<Zeroizing<Vec<u8>>, RunKeyError> {
        let key = self
            .shared
            .key
            .get_or_try_init(|| self.stored_key())
            .await?;
        Ok(key.open(column.table, column.column, row, sealed.as_bytes())?)
    }

    /// Its data key, which seals and opens the fields of its secret
    /// columns, made at its first use.
    async fn key(&self) -> Result<&RunKey, RunKeyError> {
        self.shared.key.get_or_try_init(|| self.make_key()).await
    }

    /// The data key its run state holds wrapped, which a making whose
    /// caller was dropped stored without the Run taking it, unwrapped by
    /// the key-encryption key its settings name; it never makes one, nor
    /// the engine's default key-encryption key.
    async fn stored_key(&self) -> Result<RunKey, RunKeyError> {
        let stored = self
            .state
            .read(run_key::stored)
            .await?
            .ok_or(KeyError::DoesNotOpen)?;
        let kek = if let Some(kek) = self.secrets().held_for(RUN_STATE_KEY_REFERENCE.name).await {
            kek
        } else {
            let (reference, layer) = self.runs.key_reference(self.settings()).await;
            self.runs
                .key_encryption_key(self.secrets(), &reference, layer, &self.id, Making::No)
                .await?
        };
        Ok(RunKey::unwrap(self.id.clone(), &stored.wrapped, &kek)?)
    }

    /// Makes its data key, stored only wrapped by the key-encryption key
    /// its settings name, which resolves now unless its start resolved it;
    /// the engine's own default key is made where it is missing (#616).
    async fn make_key(&self) -> Result<RunKey, RunKeyError> {
        let (reference, layer) = self.runs.key_reference(self.settings()).await;
        let kek = match self.secrets().held_for(RUN_STATE_KEY_REFERENCE.name).await {
            Some(kek) => kek,
            None => {
                self.runs
                    .key_encryption_key(self.secrets(), &reference, layer, &self.id, Making::Yes)
                    .await?
            }
        };
        let key = RunKey::generate(self.id.clone())?;
        let stored = StoredKey {
            wrapped: key.wrap(&kek)?,
            key_reference: reference.to_string(),
        };
        // A making whose caller was dropped once its write was queued may
        // have stored a key the Run never took: that one is the Run's key,
        // and this making's is not stored (the check and the write are one
        // job of the writer, so no second key can slip between them).
        let earlier = self
            .state
            .write(move |connection| match run_key::stored(connection)? {
                Some(earlier) => Ok(Some(earlier)),
                None => run_key::store(connection, &stored).map(|()| None),
            })
            .await?;
        match earlier {
            Some(earlier) => Ok(RunKey::unwrap(self.id.clone(), &earlier.wrapped, &kek)?),
            None => Ok(key),
        }
    }

    /// The engine's clock.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// The engine's sandbox probe, whose answer each browser launch reads.
    pub fn sandbox_probe(&self) -> &SandboxProbe {
        self.runs.engine.sandbox_probe()
    }

    /// Shows `pause`, a Human pause waiting on one of its pages (ADR 0092),
    /// in its progress until the handle answered ends or is dropped: its
    /// beginning and its end each give a new value.
    pub fn human_pause(&self, pause: HumanPause) -> PauseHeld {
        self.shared.progress.pause(pause)
    }

    /// Records in the Run index that the Run met a consent wall on the
    /// registrable domain `domain` and left it, so "Needs you" offers the
    /// domain for `consent.domains` (ADR 0048).
    ///
    /// # Errors
    ///
    /// When the Run index cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write began still records it.
    pub async fn record_consent_wall(&self, domain: String) -> Result<(), RunStateError> {
        let at = self.point_in_time();
        self.runs
            .engine
            .run_index()
            .record_consent_wall(domain, self.id.clone(), at)
            .await
    }

    /// Records in the Run index that the Run's refetch without the site's
    /// stored data stopped on the registrable domain `domain` for `cause`,
    /// the Block cause, the `401` or `403` status or the login page that
    /// answered it, so "Needs you" lists the domain with the layer of
    /// `wall.refetch_domains` that lists it (ADR 0052). The Run's first
    /// stop on a domain is kept.
    ///
    /// # Errors
    ///
    /// When the Run index cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write began still records it.
    pub async fn record_refetch_stop(
        &self,
        domain: String,
        cause: String,
    ) -> Result<(), RunStateError> {
        let at = self.point_in_time();
        let stop = run_index::RefetchStopped {
            domain,
            first_stopped: at.clone(),
            last_run: self.id.clone(),
            cause,
            project_file: self
                .settings()
                .and_then(|settings| settings.project_setting(WALL_REFETCH_DOMAINS.name))
                .map(str::to_owned),
        };
        self.runs
            .engine
            .run_index()
            .record_refetch_stop(stop, at)
            .await
    }

    /// Sets the part `name` its kind of Run adds to its progress, such as a
    /// crawl's `waiting` (ADR 0102): a new value when it changed.
    pub fn set_progress_part(&self, name: &str, part: Value) {
        self.shared.progress.set_part(name, part);
    }

    /// Sets the part `name` its kind of Run adds to its progress to
    /// `part`, which it keeps typed, such as a crawl's `in_flight`: a new
    /// value, whose reader has the part encoded. The caller says so only
    /// when the part changed.
    pub fn set_typed_progress_part(&self, name: &'static str, part: TypedPart) {
        self.shared.progress.set_typed_part(name, part);
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

    /// Whether the Run was asked to stop at its next safe point.
    pub(crate) fn is_stopping(&self) -> bool {
        self.shared.stop.borrow().is_some()
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
        let warning = self.masked(warning);
        self.state
            .write(move |connection| report::warn(connection, &warning))
            .await
    }

    /// `warning` with every value a Run resolved masked, as what the engine
    /// keeps.
    fn masked(&self, mut warning: RunWarning) -> RunWarning {
        if let Cow::Owned(message) = self
            .secrets()
            .mask()
            .masks()
            .apply(&warning.message, Scope::Kept)
        {
            warning.message = message;
        }
        warning
    }

    /// Hands `warning` over to the Run's report unless it holds a warning
    /// of the same code for the same folder, or with no folder, already,
    /// such as one an earlier run epoch handed over before a resume.
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a write already queued: the
    /// warning may still be handed over.
    pub async fn warn_once(&self, warning: RunWarning) -> Result<(), RunStateError> {
        let warning = self.masked(warning);
        self.state
            .write(move |connection| {
                if report::warned(connection, &warning.code, warning.folder.as_deref())? {
                    return Ok(());
                }
                report::warn(connection, &warning)
            })
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
        let shared = Arc::clone(&self.shared);
        let now = self.runs.limits.now();
        let limits = self
            .owner
            .blocking(move || {
                shared
                    .charging()
                    .as_mut()
                    .map(|charging| charging.applied(now))
                    .transpose()
            })
            .await?
            .unwrap_or_default();
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
    /// rate cap, and, when the gate names a rule that paces its keys, the
    /// Politeness `politeness.sharing` says it shares: the per-user
    /// Politeness file for `shared` and `per_daemon`, a file of its own in
    /// memory for `run`. A rule that paces nothing opens none (ADR 0138).
    /// `None` when the composition passes in no gate.
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
        // The one Politeness the Run shares, its proxies' limits included.
        let (politeness, holder) = self.politeness(settings).await?;
        Ok(Some(
            admission.sharing(politeness, holder, self.clock, settings),
        ))
    }

    /// The Politeness its own code shares for keys outside admission, such
    /// as a proxy's (ADRs 0053 and 0088), and the holder its Permits name:
    /// the per-user Politeness file, or, when `settings` set
    /// `politeness.sharing` to `run`, one of its own in memory. Each ask
    /// answers the one its first opened.
    ///
    /// # Errors
    ///
    /// When the Politeness file cannot be opened.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn politeness(
        &self,
        settings: &Resolved,
    ) -> Result<(Politeness, Holder), PolitenessError> {
        let own = settings.get::<&str>(&POLITENESS_SHARING) == "run";
        let politeness = self
            .shared
            .politeness
            .get_or_try_init(|| async {
                if own {
                    // The engine's owner, not the Run's: the Run keeps this
                    // handle while it ends, and its owner waits for its
                    // tasks, the file's actor among them.
                    return Politeness::in_memory(
                        &self.admitting.politeness.options,
                        self.admitting.politeness.tables.clone(),
                        &self.admitting.politeness.owner,
                    )
                    .await;
                }
                let file = self.admitting.politeness.file().await?;
                self.stop_on_failures(
                    file.storage_failures(),
                    self.admitting.politeness.path.parent(),
                );
                Ok(file)
            })
            .await?
            .clone();
        let holder = Holder {
            run_id: self.id.to_string(),
            epoch: self.state.epoch(),
            run_folder: self.folder.clone(),
        };
        Ok((politeness, holder))
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

    /// Where it reads robots.txt: its own copies, one fetch per origin
    /// while it lives, apart from the copy Foxhound shares across a user's
    /// runs (ADRs 0086 and 0098); on a crawl's Run, once its crawl shared
    /// one ([`Run::share_robots_txt`]), that shared copy instead.
    pub fn robots_txt(&self) -> Arc<dyn Copies> {
        match self.shared.crawls_robots_txt.get() {
            Some(shared) => Arc::clone(shared),
            None => Arc::new(OwnCopies {
                copies: self.shared.robots_txt.clone(),
                state: self.state.clone(),
            }),
        }
    }

    /// The robots.txt at `location` as its own copies hold it, sending
    /// `fetch` only when no reader of the Run read it before: what its
    /// program's own navigation and fetches read (ADRs 0086 and 0098),
    /// never the copy a crawl shares, kept in its run state.
    ///
    /// # Errors
    ///
    /// [`NotRead`] when `fetch` answered it, which nothing keeps.
    pub async fn own_robots_txt<Fut>(
        &self,
        location: &url::Url,
        fetch: Fut,
    ) -> Result<Arc<Fetched>, NotRead>
    where
        Fut: Future<Output = Result<Answer, NotRead>>,
    {
        self.shared
            .robots_txt
            .copy(&self.state, location, fetch)
            .await
    }

    /// Reads robots.txt from the copies `copies` makes from now on, in
    /// place of its own: a crawl's Run reads the copy shared across the
    /// user's runs (ADR 0086). The first call makes them; a later one
    /// changes nothing and makes none.
    pub fn share_robots_txt(&self, copies: impl FnOnce() -> Arc<dyn Copies>) {
        self.shared.crawls_robots_txt.get_or_init(copies);
    }

    /// Stages the writes a crawl attempt's handler makes through its
    /// `ctx.run` in `writes` from now on (ADR 0093): a crawl's Run
    /// registers its staging once, as its delivery opens. A later call
    /// changes nothing.
    pub fn stage_attempt_writes_through(&self, writes: Arc<dyn AttemptWrites>) {
        self.shared.attempt_writes.get_or_init(|| writes);
    }

    /// Where its attempts stage their writes, once its crawl registered
    /// it; none on a Run with no crawl.
    pub fn attempt_writes(&self) -> Option<Arc<dyn AttemptWrites>> {
        self.shared.attempt_writes.get().map(Arc::clone)
    }

    /// The user's known-certificates store, where its exceptions pin
    /// (ADR 0042).
    pub fn known_certificates(&self) -> KnownCertificates {
        self.runs.engine.known_certificates().clone()
    }

    /// The user's Audit log, which records its security events.
    pub fn audit_log(&self) -> AuditLog {
        self.runs.engine.audit().clone()
    }

    /// Where a listener it opens records its refusals: the user's Audit
    /// log (`endpoint`).
    pub fn refusals(&self) -> Refusals {
        self.runs.engine.refusals()
    }

    /// The person it acts for, whose partition of the user's stores it
    /// reads: the OS user until accounts exist (ADR 0102).
    pub fn person(&self) -> &'static str {
        CALLER
    }

    /// Its owner, whose tasks and blocking work stop with the Run.
    pub fn owner(&self) -> &TaskOwner {
        &self.owner
    }

    /// Records `entry`, one Provenance entry of the Run, such as of one of
    /// its pages or of a request a fetch tier sent for it, as a line of its
    /// Run trace's Provenance log (ADR 0047), masked by its `Mask` and
    /// written through its blocking policy to the file the Run holds.
    ///
    /// # Errors
    ///
    /// When the file cannot be opened or written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it began.
    pub async fn provenance(&self, entry: serde_json::Value) -> std::io::Result<()> {
        self.shared
            .provenance
            .record(
                &self.owner,
                self.secrets().mask(),
                self.metrics().counted_work(),
                entry,
            )
            .await
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
    pub async fn store_body(&self, bytes: bytes::Bytes) -> std::io::Result<BodyHandle> {
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

    /// Keeps the file `source` reads for the program, as the Run's file of
    /// `kind` named `name` (ADRs 0085 and 0116): written whole into its
    /// folder, named by its SHA-256, then recorded in its run state, so it
    /// outlives the page or Session that gave it, travels in its
    /// Checkpoints and goes only with the Run. While the Run folder's
    /// filesystem is at the `critical` storage level it fails at once,
    /// writing nothing, so the reserve stays for the Runs' stops (ADR
    /// 0122).
    ///
    /// # Errors
    ///
    /// `Storage` when the folder cannot take the file, `State` when its
    /// record cannot be committed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a hidden temporary file beside the
    /// kept files, or a kept file no record names, never a partial one.
    pub async fn keep_file<R>(
        &self,
        kind: FileKind,
        name: String,
        source: R,
    ) -> Result<Kept, KeepError>
    where
        R: std::io::Read + Send + 'static,
    {
        let critical = self
            .shared
            .storage
            .borrow()
            .of(&self.folder)
            .is_some_and(Filesystem::is_critical);
        if critical {
            return Err(KeepError::Storage {
                reason: "storage_full",
                os_code: None,
                message: format!(
                    "the filesystem of `{}` is at the critical storage level",
                    self.folder.display()
                ),
            });
        }
        let folder = self.folder.clone();
        let mut source = source;
        let staged = self
            .owner
            .blocking(move || run_files::stage(&folder, &mut source))
            .await
            .map_err(|error| KeepError::io(&error, "keep the file in the Run's folder"))?;
        let (sha256, size) = (staged.sha256.clone(), staged.size);
        let recorded = sha256.clone();
        // Placed in the write that records it, so a forget of the same
        // bytes ordered before it deletes no file the record names.
        let position = self
            .state
            .write(move |connection| {
                if let Err(error) = staged.place() {
                    return Ok(Err(error));
                }
                run_files::record(connection, kind, &name, &recorded, size).map(Ok)
            })
            .await?
            .map_err(|error| KeepError::io(&error, "keep the file in the Run's folder"))?;
        Ok(Kept {
            position,
            sha256,
            size,
        })
    }

    /// The wall time now, as a point in time of the Run's (ADR 0102).
    pub fn point_in_time(&self) -> PointInTime {
        self.clock.point_in_time()
    }

    /// The wall time `wait` from now, as a point in time of the Run's, such
    /// as when a domain is next admitted (ADR 0102).
    pub fn point_in_time_after(&self, wait: Duration) -> PointInTime {
        self.clock.point_in_time_after(wait)
    }

    /// What the engine's Resource governor read and decided at its last
    /// sample, once it sampled (ADR 0040): what a Run's progress says of
    /// its concurrency.
    pub fn system_info(&self) -> Option<SystemInfo> {
        self.shared.system_info.borrow().clone()
    }

    /// Charges `amount` of `usage_kind` against every Limit that holds the
    /// Run, before the Run uses it (ADR 0041): out of its Slices, taking
    /// another Slice in one ledger transaction when they run out, then
    /// journaled in its run state under its run epoch, from which an engine
    /// reclaiming its Slices after a crash charges them. A Run with nothing
    /// left of a Limit, or covered by a Limit flagged *must understand*
    /// that this engine cannot evaluate, is asked to stop at its next safe
    /// point for that Limit, and must not use what it was refused. A charge
    /// its Slices cover is a step in memory, made on the task that asks
    /// with no blocking call (ADR 0140's E3); one that needs the ledger
    /// runs under the Run's blocking policy, as does one that finds the
    /// Run's charging held.
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
        self.charged(move |change| change.charge(&kind, amount).map(|charged| ((), charged)))
            .await
            .map(drop)
    }

    /// Charges `amount` of `usage_kind` as [`Run::charge`] does, and counts
    /// it under `part` of the kind in the Run's Usage, such as the proxy
    /// kind of `proxy_bytes` (ADR 0021), in the same run-state write; with
    /// `price`, a currency and a cost already paid, its cost too, reserved
    /// as [`Run::reserve`] does at that cost: both in one ledger
    /// transaction, so a Limit that refuses one charges neither.
    ///
    /// # Errors
    ///
    /// As [`Run::charge`].
    ///
    /// # Cancel safety
    ///
    /// As [`Run::charge`]'s.
    pub async fn charge_part(
        &self,
        usage_kind: &str,
        part: &str,
        amount: u64,
        price: Option<(&str, u64)>,
    ) -> Result<(), ChargeError> {
        let (kind, part) = (usage_kind.to_owned(), part.to_owned());
        let price = price.map(|(currency, cost)| (currency.to_owned(), cost));
        self.charged(move |change| {
            let priced = match &price {
                Some((currency, cost)) => Some(change.reserve(&kind, Some(currency), *cost)?),
                None => None,
            };
            let mut charged = match change.charge(&kind, amount) {
                Ok(charged) => charged,
                Err(unmade) => {
                    if let Some((reservation, _)) = priced {
                        // The price is given back: nothing was used.
                        let _ = change.settle(reservation, 0);
                    }
                    return Err(unmade);
                }
            };
            for counted in &mut charged.counted {
                counted.part = Some(part.clone());
            }
            if let Some((reservation, priced)) = priced {
                // A cost already paid is its own worst case: it stays
                // charged as reserved.
                drop(reservation);
                charged.used.extend(priced.used);
                charged.counted.extend(priced.counted);
            }
            Ok(((), charged))
        })
        .await
        .map(drop)
    }

    /// Charges `amount` of `usage_kind` as [`Run::charge`] does, without
    /// journaling it: what it charged joins `charges`, which
    /// [`Run::write_charged`] journals in the write that uses it, so the
    /// journal and that write commit as one job of the run state's writer.
    ///
    /// # Errors
    ///
    /// `Refused` as [`Run::charge`]'s, which asks the Run to stop as its
    /// does; nothing joins `charges` then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Slice taken, or the charge made and
    /// not in `charges`: charged and never journaled, before anything used
    /// it, so never more charged than the Run used.
    pub async fn charge_unjournaled(
        &self,
        charges: &mut UnjournaledCharges,
        usage_kind: &str,
        amount: u64,
    ) -> Result<(), ChargeError> {
        let now = self.runs.limits.now();
        let kind = usage_kind.to_owned();
        // A charge the Run's Slices cover is a step in memory, made here
        // with no blocking task (ADR 0140's E3; E3-3 of the audit #1882).
        let made = self
            .on_charging(
                (),
                move |charging, ()| {
                    charging
                        .charge_held(usage_kind, amount, now)
                        .map(Ok)
                        .ok_or(())
                },
                move |charging, ()| charging.charge(&kind, amount, now),
            )
            .await;
        match made {
            None => Ok(()),
            Some(Ok(journaled)) => {
                charges.add(journaled);
                Ok(())
            }
            Some(Err(refusal)) => Err(self.refused(refusal).await),
        }
    }

    /// Queues `work` as one write of the Run's state that first journals
    /// `charges` under its run epoch, as [`Run::charge`] journals each:
    /// with what the Run has now used of each Slice they took from, read
    /// under its charging's lock as the write takes its place in the queue,
    /// so journals reach the run state in the order of the changes. A Run
    /// whose charging ended journals nothing. Answers what `work` returned
    /// once the write's commit is acknowledged, before a sync of its own:
    /// it is an attempt's claim, which a power cut may take, and the
    /// writer bounds what such writes charge by [`DEFERRED_CHARGES`] (ADR
    /// 0138's note of #1926).
    ///
    /// [`DEFERRED_CHARGES`]: crate::run_state::DEFERRED_CHARGES
    ///
    /// # Errors
    ///
    /// `State` when `work` or its commit failed, or the run state's writer
    /// stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the write is queued does not withdraw it.
    pub async fn write_charged<T, F>(
        &self,
        charges: UnjournaledCharges,
        work: F,
    ) -> Result<T, ChargeError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let place = self.state.reserve().await?;
        let shared = Arc::clone(&self.shared);
        let epoch = self.state.epoch();
        let written = self
            .owner
            .blocking(move || {
                let held = shared.charging();
                let at_most = charges
                    .counted
                    .iter()
                    .map(|counted| u64::try_from(counted.delta).unwrap_or(0))
                    .fold(0_u64, u64::saturating_add);
                let journaled = held.as_ref().map(|charging| {
                    let UnjournaledCharges { slices, counted } = charges;
                    (
                        Charged {
                            used: charging.used_of(&slices),
                            counted,
                        },
                        charging.uncapped(),
                    )
                });
                place.write_with(Syncing::Deferred { charged: at_most }, move |connection| {
                    if let Some((journaled, uncapped)) = journaled
                        && (!journaled.used.is_empty() || !journaled.counted.is_empty() || uncapped)
                    {
                        journal(connection, epoch, &journaled, uncapped)?;
                    }
                    work(connection)
                })
            })
            .await;
        Ok(written.await?)
    }

    /// Reserves ahead an amount of `usage_kind` the Run is about to use, as
    /// [`Run::reserve`] reserves a worst case, counted under `part` of the
    /// kind, and with `currency` its `cost` too, in one ledger transaction:
    /// `wanted` when every Limit holding the Run allows it, else the
    /// largest of its halves down to `least` that they allow, so a large
    /// allowance that a nearly spent Limit refuses gives way to what is
    /// left of it, and only `least`'s refusal refuses it and asks the Run
    /// to stop as [`Run::charge`]'s does. What the Run then uses of it is
    /// settled with [`Run::settle_part`], so its Usage counts what it used:
    /// an amount charged before it is used, with one ledger and journal
    /// write per allowance instead of one per use (the deep review's
    /// 599-12). None once the Run ended.
    ///
    /// # Errors
    ///
    /// As [`Run::charge`]: none of it may be used then.
    ///
    /// # Cancel safety
    ///
    /// As [`Run::charge`]'s: an allowance dropped unsettled stays charged
    /// in full.
    pub async fn reserve_part(
        &self,
        usage_kind: &str,
        part: &str,
        currency: Option<&str>,
        cost: impl Fn(u64) -> u64 + Send + 'static,
        wanted: u64,
        least: u64,
    ) -> Result<Option<Allowance>, ChargeError> {
        let (kind, part) = (usage_kind.to_owned(), part.to_owned());
        let currency = currency.map(str::to_owned);
        self.charged(move |change| {
            let reserve = |change: &mut Change<'_>, amount: u64| {
                let cost = cost(amount);
                let (reserved, mut charged) = change.reserve(&kind, None, amount)?;
                for counted in &mut charged.counted {
                    counted.part = Some(part.clone());
                }
                let price = match &currency {
                    Some(currency) => match change.reserve(&kind, Some(currency), cost) {
                        Ok((price, priced)) => {
                            charged.used.extend(priced.used);
                            charged.counted.extend(priced.counted);
                            Some(price)
                        }
                        Err(unmade) => {
                            // Nothing was used: the amount is given back.
                            let _ = change.settle(reserved, 0);
                            return Err(unmade);
                        }
                    },
                    None => None,
                };
                let allowance = Allowance {
                    granted: amount,
                    amount: reserved,
                    price,
                    part: part.clone(),
                };
                Ok((allowance, charged))
            };
            let least = least.min(wanted);
            let mut amount = wanted;
            loop {
                match reserve(change, amount) {
                    // Only the ledger says a Limit refuses the amount: a
                    // smaller allowance is no step in memory.
                    Err(Unmade::Refused(Refusal::Reached(_))) if amount > least => {
                        amount = (amount / 2).max(least);
                    }
                    reserved => break reserved,
                }
            }
        })
        .await
    }

    /// Settles `allowance` at `used` of its amount and `cost` of its
    /// price, which its grant bounds: the rest is the Run's to use again,
    /// and its Usage then counts what it used.
    ///
    /// # Errors
    ///
    /// `State` when the run state could not journal it: the allowance then
    /// stays charged in full.
    ///
    /// # Cancel safety
    ///
    /// As [`Run::charge`]'s.
    pub async fn settle_part(
        &self,
        allowance: Allowance,
        used: u64,
        cost: u64,
    ) -> Result<(), ChargeError> {
        self.settled(move |charging| {
            let Allowance {
                amount,
                price,
                part,
                ..
            } = allowance;
            let mut settled = charging.settle(amount, used);
            for counted in &mut settled.counted {
                counted.part = Some(part.clone());
            }
            if let Some(price) = price {
                let priced = charging.settle(price, cost);
                settled.used.extend(priced.used);
                settled.counted.extend(priced.counted);
            }
            settled
        })
        .await
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
        self.charged(move |change| change.reserve(&kind, currency.as_deref(), worst))
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
        self.charged(move |change| {
            change
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
        self.settled(move |charging| charging.settle(reservation, cost))
            .await
    }

    /// Makes `change` on the Run's charging, journals what it charged, and
    /// asks the Run to stop at its next safe point when a Limit refused it;
    /// answers none once the Run ended. A change that is a step in memory
    /// ([`Charging::change_held`]) is made where the Run asks for it, with
    /// no blocking call (ADR 0140's E3); one that needs the ledger is made
    /// again, whole, in a blocking call ([`Charging::change`]), so
    /// `change` may run twice. Either way the journal is queued under the
    /// charging's lock ([`Run::on_charging`]), so journals reach the run
    /// state in the order of the changes.
    async fn charged<T, F>(&self, change: F) -> Result<Option<T>, ChargeError>
    where
        T: Send + 'static,
        F: Fn(&mut Change<'_>) -> Result<(T, Charged), Unmade> + Send + 'static,
    {
        let place = self.state.reserve().await?;
        let now = self.runs.limits.now();
        let epoch = self.state.epoch();
        let changed = self
            .on_charging(
                (change, place),
                move |charging, (change, place)| match charging.change_held(now, &change) {
                    Some((value, charged)) => {
                        Ok(Ok((value, journaled(charging, place, epoch, charged))))
                    }
                    None => Err((change, place)),
                },
                move |charging, (change, place)| {
                    charging
                        .change(now, &change)
                        .map(|(value, charged)| (value, journaled(charging, place, epoch, charged)))
                },
            )
            .await;
        match changed {
            None => Ok(None),
            Some(Ok((value, journaling))) => {
                if let Some(journaling) = journaling {
                    journaling.await?;
                }
                Ok(Some(value))
            }
            Some(Err(refusal)) => Err(self.refused(refusal).await),
        }
    }

    /// Makes `settle`, a settlement, on the Run's charging and journals
    /// what it gave back: always a step in memory, made where the Run asks
    /// for it unless its charging is held ([`Run::on_charging`]). A Run
    /// that ended settles nothing.
    async fn settled<F>(&self, settle: F) -> Result<(), ChargeError>
    where
        F: FnOnce(&mut Charging) -> Charged + Send + 'static,
    {
        let place = self.state.reserve().await?;
        let epoch = self.state.epoch();
        let settled = move |charging: &mut Charging, (settle, place): (F, ReservedWrite)| {
            let charged = settle(charging);
            journaled(charging, place, epoch, charged)
        };
        let journaling = self
            .on_charging(
                (settle, place),
                move |charging, settlement| Ok(settled(charging, settlement)),
                settled,
            )
            .await;
        if let Some(journaling) = journaling.flatten() {
            journaling.await?;
        }
        Ok(())
    }

    /// Makes one change of the Run's charging, under the charging's lock:
    /// `held` where the Run asks for it, with no blocking call, when
    /// nothing else holds the charging; `ledger` in a blocking call, under
    /// the Run's blocking policy, when `held` hands `change` back because
    /// it is no step in memory, and when the charging is held: by a ledger
    /// call, by a claim's journal, or by another task's step in memory.
    /// The lock is tried and never waited for where the Run asks, and
    /// never held across an await. None once the Run ended.
    async fn on_charging<C, T>(
        &self,
        change: C,
        held: impl FnOnce(&mut Charging, C) -> Result<T, C> + Send,
        ledger: impl FnOnce(&mut Charging, C) -> T + Send + 'static,
    ) -> Option<T>
    where
        C: Send + 'static,
        T: Send + 'static,
    {
        let change = match self.shared.charging_if_free() {
            Some(mut free) => match free.as_mut().map(|charging| held(charging, change)) {
                None => return None,
                Some(Ok(made)) => return Some(made),
                Some(Err(change)) => change,
            },
            None => change,
        };
        let shared = Arc::clone(&self.shared);
        self.owner
            .blocking(move || {
                shared
                    .charging()
                    .as_mut()
                    .map(|charging| ledger(charging, change))
            })
            .await
    }

    /// What `refusal`, a Limit's or the ledger's, does to the Run: a Limit
    /// with nothing left, or one this engine cannot evaluate, asks it to
    /// stop at its next safe point; a ledger that cannot be stored stops
    /// every Run of the engine (ADR 0122).
    async fn refused(&self, refusal: Refusal) -> ChargeError {
        match refusal {
            Refusal::Reached(reached) => {
                self.stop_at_safe_point(StopReason::Limit(Box::new(ReachedLimit::clone(&reached))));
                Refusal::Reached(reached).into()
            }
            Refusal::NotUnderstood(limit) => {
                self.stop_at_safe_point(StopReason::LimitNotUnderstood(Box::new(
                    NotUnderstoodLimit {
                        scope: limit.scope.clone(),
                        usage_kind: limit.usage_kind.clone(),
                        currency: limit.currency.clone(),
                        period: limit.period.clone(),
                    },
                )));
                Refusal::NotUnderstood(limit).into()
            }
            Refusal::Ledger(LedgerError::Storage { path, message }) => {
                // The ledger holds every Run's Limits: each Run of the
                // engine stops as at `critical` (ADR 0122).
                let folder = path.parent().map_or_else(|| path.clone(), Path::to_owned);
                let _ = self
                    .runs
                    .commands
                    .send(Command::StorageFailed(folder))
                    .await;
                Refusal::Ledger(LedgerError::Storage { path, message }).into()
            }
            refusal => refusal.into(),
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
/// its catalog default when none did.
fn secret_timeout(settings: Option<&RunSettings>) -> Duration {
    get_or_default(settings.map(RunSettings::resolved), &TIMEOUT_SECRET)
}

/// What a Run whose settings resolved as `settings` resolves its Secret
/// references through: each provider the composition builds for each Run
/// built from them, bounded by its `timeout.secret` and masked by its
/// `security.redaction`.
fn run_secrets(secrets: &Secrets, settings: Option<&RunSettings>) -> RunSecrets {
    let (timeout, redaction) = (secret_timeout(settings), redaction(settings));
    match settings {
        Some(settings) => secrets.for_run_under(settings.resolved(), timeout, redaction),
        None => secrets.for_run_with(timeout, redaction),
    }
}

/// Whether a Run whose settings resolved as `settings` masks its values in
/// its kept outputs: its `security.redaction`, the catalog default, on,
/// when none resolved.
fn redaction(settings: Option<&RunSettings>) -> Redaction {
    if get_or_default(settings.map(RunSettings::resolved), &SECURITY_REDACTION) {
        Redaction::On
    } else {
        Redaction::Off
    }
}

impl Runs {
    /// Starts the registry as the task of an owner of its own under
    /// `owner`, which `owner`'s stop waits for, each Run's tasks under a
    /// child of `owner`, once `startup`, what the engine writes as it
    /// starts, ends: it answers no command before.
    pub(crate) fn start_registry(
        mut setup: RunsSetup,
        startup: Pin<Box<dyn Future<Output = ()> + Send>>,
        owner: &TaskOwner,
    ) -> Self {
        let (commands, mailbox) = mpsc::channel(COMMANDS);
        let readers = Arc::clone(setup.engine.owner());
        let limits = Limits {
            owner: Arc::clone(&readers),
            ledger: setup.engine.ledger().clone(),
            work: setup.metrics.counted_work().clone(),
            clock: setup.engine.clock(),
            product: setup.product,
            instance: setup.events.instance().as_str().to_owned(),
            daemon: matches!(setup.reached_at, Some(RunOwner::Daemon(_)))
                .then(|| setup.version.line().to_string()),
        };
        let runs = Self {
            commands,
            engine: setup.engine.clone(),
            connection: None,
            limits: Arc::new(limits),
            secrets: setup.secrets.clone(),
            clean_up: CleanUpRules::of(&setup),
            controls: Arc::new(setup.controls.clone()),
            metrics: setup.metrics.clone(),
            registry: Arc::new(owner.part("Run registry")),
        };
        let politeness = SharedPoliteness {
            file: Arc::new(tokio::sync::OnceCell::new()),
            path: setup.engine.home().politeness_file(),
            options: setup.politeness.clone(),
            tables: setup.politeness_tables.clone(),
            engine: format!("{:016x}", setup.entropy.next_u64()),
            owner: Arc::new(owner.part("Politeness store")),
        };
        let admitting = Arc::new(Admitting {
            gates: setup.gates.clone(),
            politeness,
        });
        let registry = Registry {
            setup,
            admitting,
            owner: owner.part("Run registry's shared work"),
            for_one_run: owner.child_for(OwnerKind::Run),
            ending_owner: owner.child_for(OwnerKind::Run),
            itself: Arc::clone(&runs.registry),
            readers,
            live: BTreeMap::new(),
            ended: EndedRuns::new(),
            runs: runs.downgrade(),
            limits: Arc::clone(&runs.limits),
            machine: None,
            draining: false,
            outliving: std::collections::BTreeSet::new(),
            found_ended: std::collections::BTreeSet::new(),
            sampled: Vec::new(),
            discarding: std::collections::BTreeSet::new(),
            unrecorded: BTreeMap::new(),
            memory_stop: MemoryStop::Idle,
        };
        runs.registry.spawn(registry.run(mailbox, startup));
        // `owner`'s stop waits for the registry, as for one of its tasks,
        // while the registry's panic is told on an owner of its own.
        let registry = Arc::clone(&runs.registry);
        owner.spawn(async move { registry.wait().await });
        runs
    }

    fn downgrade(&self) -> mpsc::WeakSender<Command> {
        self.commands.downgrade()
    }

    /// The runtime's metrics, which what a start does before its Run exists
    /// records in (ADR 0050).
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// The scope of the Client connection these Runs stand in, as the
    /// Usage ledger names it.
    pub(crate) fn connection_scope(&self) -> LimitScope {
        self.limits.connection(self.connection)
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
    pub async fn start(&self, mut start: Start) -> Result<Started, StartError> {
        // A binding is never ignored (ADR 0044): local addresses that give
        // no Session an address refuse the Run before it begins.
        if let Some(rotation) = start
            .settings
            .as_ref()
            .and_then(|settings| Rotation::of(settings.resolved()))
        {
            self.limits
                .owner
                .blocking(move || rotation.check(&local_addresses::System))
                .await
                .map_err(|unavailable| StartError::Refused(Box::new(unavailable.into())))?;
        }
        let mut fallback = None;
        if start.clean_up_digest.is_none()
            && let Some(rules) = &self.clean_up
        {
            let chosen = rules
                .versions
                .choose(self.rules_context(rules, start.settings.as_ref()))
                .await
                .map_err(|refused| StartError::Refused(Box::new(refused)))?;
            start.clean_up_digest = Some(chosen.digest);
            fallback = chosen.fallback;
        }
        let settings = start.settings.clone();
        let started = self.start_chosen(start).await;
        if let (Ok(Started::New(run)), Some(rules)) = (&started, &self.clean_up) {
            rules
                .versions
                .held(
                    run,
                    Holding::Started { fallback },
                    self.rules_context(rules, settings.as_ref()),
                )
                .await;
        }
        started
    }

    /// What the Clean-up rules `rules` are told of a Run whose start
    /// resolved `settings`.
    fn rules_context<'a>(
        &'a self,
        rules: &'a CleanUpRules,
        settings: Option<&'a RunSettings>,
    ) -> RulesContext<'a> {
        RulesContext {
            settings,
            home: self.engine.home(),
            clock: self.engine.clock(),
            product: self.limits.product,
            product_version: &rules.product_version,
            owner: &self.limits.owner,
        }
    }

    /// [`Runs::start`] once its Clean-up rules are chosen: a new Run whose
    /// settings switch a control off reports each switch once (ADR 0016),
    /// and with `security.audit_chain` off its entries, from its key's use
    /// on, are written unchained, a start with no Run ID of its own taking
    /// its generated one first so that its key's use names it; a start that
    /// answers an existing Run, or fails, reports nothing.
    async fn start_chosen(&self, mut start: Start) -> Result<Started, StartError> {
        let switched = start
            .settings
            .as_ref()
            .map(|settings| self.controls.switched_off(settings.resolved()))
            .unwrap_or_default();
        if start.run_id.is_none() && switched.unchains() {
            let (answer, answered) = oneshot::channel();
            self.commands
                .send(Command::GenerateId(answer))
                .await
                .map_err(|_| StartError::Stopped)?;
            start.run_id = Some(answered.await.map_err(|_| StartError::Stopped)?);
        }
        let audit = self.engine.audit().clone();
        let chosen = start.run_id.clone().filter(|_| switched.unchains());
        let newly_unchained = chosen.clone().is_some_and(|run_id| audit.unchain(run_id));
        let started = self.start_reported(start).await;
        match &started {
            Ok(Started::New(run)) => {
                if switched.unchains() {
                    audit.unchain(run.id().clone());
                }
                let run_id = run.id().clone();
                let now = self.limits.clock.timestamp();
                let reported = switched.report(&audit, &run_id, now).await;
                if let Err(error) = reported {
                    tracing::error!(
                        run_id = %run.id(),
                        %error,
                        "a security control the Run switched off could not be recorded in the Audit log"
                    );
                }
            }
            _ => {
                if let Some(run_id) = chosen.filter(|_| newly_unchained) {
                    audit.rechain(&run_id);
                }
            }
        }
        started
    }

    /// [`Runs::start_chosen`] before its switched-off controls are reported.
    async fn start_reported(&self, start: Start) -> Result<Started, StartError> {
        let secrets = run_secrets(&self.secrets, start.settings.as_ref());
        // Only a reference a layer set resolves here: the default's item,
        // in the OS keychain or the encrypted local file, is the engine's
        // own, made when run state first wraps a data key with it (#617).
        if let Some((reference, layer)) = layer_set_key_reference(start.settings.as_ref()) {
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
        let message = match self.secrets.masks().apply(message.as_str(), Scope::Kept) {
            Cow::Borrowed(_) => message,
            Cow::Owned(masked) => ProgressMessage::parse(
                &masked
                    .chars()
                    .take(MAX_PROGRESS_MESSAGE_CHARACTERS)
                    .collect::<String>(),
            )
            .expect("invariant: a message cut to the most characters it holds parses"),
        };
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
    /// The stop is asked when this is called, before its future is first
    /// polled: dropping the future ends only the wait (#1743).
    pub fn stop_at_safe_point(
        &self,
        run_id: RunId,
        by: StoppedBy,
    ) -> impl Future<Output = Result<RunReport, RunError>> + Send + use<> {
        let (answer, answered) = oneshot::channel();
        self.send_now(Command::SafeStop(run_id, by, answer));
        async move {
            match answered.await.map_err(|_| RunError::Stopped)?? {
                Waiting::Report(report) => wait_for(report).await,
                Waiting::TakeOver(taken) => taken.await.map_err(|_| RunError::Stopped)?,
            }
        }
    }

    /// Hands `command` to the registry now, whatever becomes of the caller:
    /// while the registry's mailbox is full, a task of the engine's own
    /// sends it. A registry that stopped drops it, and with it its answer.
    fn send_now(&self, command: Command) {
        match self.commands.try_send(command) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
            Err(mpsc::error::TrySendError::Full(command)) => {
                let commands = self.commands.clone();
                self.limits.owner.spawn(async move {
                    let _ = commands.send(command).await;
                });
            }
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
        // With `security.audit_chain` off, the resumed Run's entries, from
        // its key's use on, are written unchained (ADR 0016); a resume
        // that takes no epoch chains them again.
        let switched = resume
            .settings
            .as_ref()
            .map(|settings| self.controls.switched_off(settings.resolved()))
            .unwrap_or_default();
        let (audit, run_id) = (self.engine.audit().clone(), resume.run_id.clone());
        let newly_unchained = switched.unchains() && audit.unchain(run_id.clone());
        let resumed = self.resume_unchained(resume).await;
        match &resumed {
            // Each control it switches off that its report did not list
            // yet is reported as a start's are (ADR 0016).
            Ok(Resumed::New { listed, .. }) => {
                let switched = switched.beyond(listed);
                let now = self.limits.clock.timestamp();
                let reported = switched.report(&audit, &run_id, now).await;
                if let Err(error) = reported {
                    tracing::error!(
                        run_id = %run_id,
                        %error,
                        "a security control the resume switched off could not be recorded in the Audit log"
                    );
                }
            }
            _ if newly_unchained => {
                audit.rechain(&run_id);
            }
            _ => {}
        }
        resumed
    }

    /// [`Runs::resume`] once its entries' chaining is set.
    async fn resume_unchained(&self, resume: Resume) -> Result<Resumed, RunError> {
        let secrets = run_secrets(&self.secrets, resume.settings.as_ref());
        let unwrapped = self.unwrapped_key(&resume, &secrets).await?;
        let (stored, key) = match unwrapped {
            Some(Unwrapped { stored, key }) => (Some(stored), Some(key)),
            None => (None, None),
        };
        let settings = resume.settings.clone();
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Resume(
                Box::new((resume, Link::to_current(), Keys { secrets, key }, stored)),
                self.connection,
                answer,
            ))
            .await
            .map_err(|_| RunError::Stopped)?;
        let resumed = answered.await.map_err(|_| RunError::Stopped)?;
        if let (Ok(Resumed::New { run, .. }), Some(rules)) = (&resumed, &self.clean_up) {
            rules
                .versions
                .held(
                    run,
                    Holding::Resumed,
                    self.rules_context(rules, settings.as_ref()),
                )
                .await;
        }
        resumed
    }

    /// The data key of the Run `resume` resumes, unwrapped by the
    /// key-encryption key its settings name, resolved through `secrets`,
    /// when its run file or Checkpoint holds one: the key comes from the
    /// caller's reference, never from the file (ADR 0015). It resolves
    /// before the registry takes the resume, so a slow provider holds no
    /// other Run back.
    async fn unwrapped_key(
        &self,
        resume: &Resume,
        secrets: &RunSecrets,
    ) -> Result<Option<Unwrapped>, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::StoredKey(
                resume.run_id.clone(),
                resume.checkpoint.clone(),
                answer,
            ))
            .await
            .map_err(|_| RunError::Stopped)?;
        let stored = answered.await.map_err(|_| RunError::Stopped)??;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let run_id = &resume.run_id;
        let (reference, layer) = self.key_reference(resume.settings.as_ref()).await;
        let kek = self
            .key_encryption_key(secrets, &reference, layer, run_id, Making::No)
            .await
            .map_err(|error| RunError::Secret {
                run_id: run_id.clone(),
                error: Box::new(error),
            })?;
        let key = RunKey::unwrap(run_id.clone(), &stored.wrapped, &kek).map_err(|_| {
            RunError::KeyMismatch {
                run_id: run_id.clone(),
                reference,
                wrapped_by: stored.key_reference.clone(),
            }
        })?;
        Ok(Some(Unwrapped { stored, key }))
    }

    /// The Secret reference of the key-encryption key of a Run whose
    /// settings resolved as `settings`, and the layer that set it: this
    /// host's default where none did, or where a layer cleared it, asked
    /// for here, so the OS keychain opens only once a Run first needs its
    /// key, not as its settings open (issue 2204).
    async fn key_reference(
        &self,
        settings: Option<&RunSettings>,
    ) -> (SecretReference, SettingsLayer) {
        if let Some((reference, layer)) = layer_set_key_reference(settings) {
            return (reference, layer);
        }
        let default = self
            .engine
            .settings()
            .host_defaults()
            .await
            .of(&RUN_STATE_KEY_REFERENCE);
        let reference = default
            .as_str()
            .and_then(|text| SecretReference::parse(text).ok())
            .unwrap_or_else(|| {
                SecretReference::parse(KEY_REFERENCE_IN_KEYCHAIN)
                    .expect("invariant: the keychain's default is a Secret reference")
            });
        (reference, SettingsLayer::Default)
    }

    /// The key-encryption key `reference`, set by `layer`, gives for the
    /// Run `run_id`, resolved through `secrets`. With `Making::Yes`, the
    /// engine's own key ([`is_engine_key`]) is made first where it is
    /// missing.
    async fn key_encryption_key(
        &self,
        secrets: &RunSecrets,
        reference: &SecretReference,
        layer: SettingsLayer,
        run_id: &RunId,
        making: Making,
    ) -> Result<SecretString, SecretError> {
        let purpose = Purpose {
            placeholder: RUN_STATE_KEY_REFERENCE.name,
            domain: None,
            domains: &[],
            layer: Some(layer),
            run_id: Some(run_id),
        };
        match secrets.resolve(reference, &purpose).await {
            Err(SecretError::Unavailable {
                failure: SecretFailure::Missing | SecretFailure::NotConfigured,
                ..
            }) if making == Making::Yes && is_engine_key(reference) => {
                self.secrets.make_key(reference).await?;
                secrets.resolve(reference, &purpose).await
            }
            resolved => resolved,
        }
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
    pub async fn folder(&self, run_id: RunId) -> Result<PathBuf, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Folder(run_id, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)
    }

    /// Keeps what `source` reads as an Export of the Run `run_id` (ADRs
    /// 0103, 0116 and 0122): written whole into its folder's `exports/`,
    /// named by its SHA-256, through a hidden temporary file, synced, then
    /// renamed, so a failed write leaves no unfinished file. No run state
    /// records it, so a Run that ended keeps one too. While the Run
    /// folder's filesystem is at the `critical` storage level, as read now,
    /// it fails at once, writing nothing.
    ///
    /// # Errors
    ///
    /// `Storage` when the folder cannot take the file or is at `critical`,
    /// or when `source` fails; `State` never.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a hidden temporary file beside the
    /// Exports, never a partial one.
    pub async fn keep_export<R>(&self, run_id: RunId, source: R) -> Result<KeptExport, KeepError>
    where
        R: std::io::Read + Send + 'static,
    {
        let stopped = || KeepError::Storage {
            reason: "other",
            os_code: None,
            message: "the engine's Runs have stopped".to_owned(),
        };
        let folder = self.folder(run_id).await.map_err(|_| stopped())?;
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Governor(answer))
            .await
            .map_err(|_| stopped())?;
        let governor = answered.await.map_err(|_| stopped())?;
        let (sampled, owner) = (folder.clone(), Arc::clone(&self.limits.owner));
        let critical = owner
            .blocking(move || governor.storage_at(&sampled))
            .await
            .is_some_and(|filesystem| filesystem.is_critical());
        if critical {
            return Err(KeepError::Storage {
                reason: "storage_full",
                os_code: None,
                message: format!(
                    "the filesystem of `{}` is at the critical storage level",
                    folder.display()
                ),
            });
        }
        let mut source = source;
        let (sha256, size) = owner
            .blocking(move || run_files::store_export(&folder, &mut source))
            .await
            .map_err(|error| KeepError::io(&error, "write the Export in the Run's folder"))?;
        Ok(KeptExport { sha256, size })
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
        answered.await.map_err(|_| RunError::Stopped)?
    }

    /// The progress of the Run `run_id` (ADR 0102): without `after`, as it
    /// is now; given `after`, the position of the value a reader last read,
    /// the latest value after it, once a change comes, so the values in
    /// between are coalesced. A live Run of this engine's is read as its
    /// registry and driver hold it, with the parts its composition reads
    /// from its committed state; any other from its run file, the last
    /// value from here, `writer` as a reader's test of its writer lock
    /// finds it, which never takes the lock.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID here, `Unavailable` when its
    /// state cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn progress(
        &self,
        run_id: RunId,
        after: Option<u64>,
    ) -> Result<RunProgress, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Progress(run_id.clone(), answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        let Followed {
            changes,
            mut reader,
            parts,
            clock,
        } = answered.await.map_err(|_| RunError::Stopped)??;
        let Some(mut changes) = changes else {
            let folder = reader.folder().to_owned();
            let at = clock.point_in_time();
            return reader
                .read(move |connection| {
                    let writer = match test_writer(&folder) {
                        Ok(Writer::Live) => RunWriter::Live,
                        Ok(Writer::None) | Err(_) => RunWriter::Absent,
                    };
                    let mut progress = progress::committed(connection, writer, at)?;
                    if let Some(progress) = &mut progress {
                        progress::fill(&parts, connection, &TypedParts::default(), progress);
                    }
                    Ok(progress)
                })
                .await?
                .ok_or(RunError::NotFound(run_id));
        };
        let value = loop {
            let ready = {
                let current = changes.borrow_and_update();
                (current.last || after.is_none_or(|after| current.position() > after))
                    .then(|| current.unencoded(&run_id, clock.point_in_time()))
            };
            if let Some(value) = ready {
                break value;
            }
            if changes.changed().await.is_err() {
                // The registry stopped with the Run live: nothing more
                // comes from here.
                break changes
                    .borrow()
                    .unencoded(&run_id, clock.point_in_time())
                    .last();
            }
        };
        let unfilled = value.clone();
        match reader
            .read(move |connection| Ok(value.filled(&parts, connection)))
            .await
        {
            Ok(value) => Ok(value),
            Err(error) => {
                tracing::warn!(%error, %run_id, "the Run's committed parts of its progress could not be read");
                Ok(unfilled.encoded())
            }
        }
    }

    /// The report of the Run `run_id` (ADR 0102): at once for one that
    /// ended, and at its end for a live one, which the read never ends; a
    /// Run another engine still writes is read again from its run file
    /// every [`REPORT_POLL`] until it ended.
    ///
    /// # Errors
    ///
    /// `NotFound` when no Run has this Run ID here, `Interrupted` when it
    /// has no live writer and has not ended, `Unavailable` when its state
    /// cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn report(&self, run_id: RunId) -> Result<RunReport, RunError> {
        loop {
            match self.ended(run_id.clone()).await {
                Err(RunError::NotFound(_)) => {}
                ended => return ended,
            }
            let mut reader = self.reader(run_id.clone()).await?;
            let folder = reader.folder().to_owned();
            let (recorded, writer) = reader
                .read(move |connection| {
                    let recorded: i64 =
                        connection.query_row("SELECT count(*) FROM run", [], |row| row.get(0))?;
                    Ok((recorded > 0, test_writer(&folder).ok()))
                })
                .await?;
            match (recorded, writer) {
                (false, _) => return Err(RunError::NotFound(run_id)),
                (true, Some(Writer::Live)) => self.engine.clock().sleep(REPORT_POLL).await,
                (true, _) => return Err(RunError::Interrupted(run_id)),
            }
        }
    }

    /// Reads `method` with `params` of the Run `run_id` through the engine
    /// that works it (ADR 0102), when that is another engine, the owner
    /// the Run index names while a reader's test finds its writer lock
    /// held, reached as a Client that needs `needs`: its answer, or `None`
    /// when this engine runs the Run, no engine does, or the owner did not
    /// answer, for the read to go on here, from the Run's committed state.
    ///
    /// # Errors
    ///
    /// `Forwarded` with the failure the owner answered.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn through_owner(
        &self,
        run_id: &RunId,
        needs: Product,
        method: &'static str,
        params: Value,
    ) -> Result<Option<Value>, RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Locate(run_id.clone(), answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        let Some(located) = answered.await.map_err(|_| RunError::Stopped)?? else {
            return Ok(None);
        };
        let Located {
            folder,
            index,
            reached_at,
            forwarder,
            tasks,
        } = located;
        let live = tasks
            .blocking(move || test_writer(&folder).ok() == Some(Writer::Live))
            .await;
        let owner = if live {
            index
                .read(run_id.clone())
                .await
                .ok()
                .flatten()
                .and_then(|row| row.owner)
        } else {
            None
        };
        let Some(owner) = owner.filter(|owner| Some(owner) != reached_at.as_ref()) else {
            return Ok(None);
        };
        match forwarder.read(&owner, needs, method, params, &tasks).await {
            Some(Ok(answer)) => Ok(Some(answer)),
            Some(Err(failure)) => Err(RunError::Forwarded(Box::new(failure))),
            None => Ok(None),
        }
    }

    /// The rows of the Run index that `params` names, in its order (ADRs
    /// 0031, 0036 and 0117), each presence kept, Watch runs left out unless
    /// it asks for them. The index is read off the registry.
    ///
    /// # Errors
    ///
    /// `Stopped` when the registry has stopped, and the index's own when it
    /// cannot be read or its marker refuses this engine.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn list(&self, params: RunListParams) -> Result<Vec<RunIndexRow>, ListError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Index(answer))
            .await
            .map_err(|_| ListError::Stopped)?;
        let index = answered.await.map_err(|_| ListError::Stopped)?;
        let rows = index.rows().await?;
        Ok(list::select(rows, &params))
    }

    /// Discards the Run `run_id` for good, so that nothing of it is left
    /// behind (ADRs 0001, 0002, 0015 and 0036): its wrapped data key is
    /// overwritten and its run folder removed, in the per-install layout
    /// its files deleted from the install database, and its Run index row
    /// marked *erased*. A Run already discarded is answered as discarded.
    ///
    /// # Errors
    ///
    /// `Live` when this engine runs it, `InUse` when another engine holds
    /// its writer lock, `NotFound` when no Run has this Run ID, and
    /// `Unavailable` when its state cannot be erased.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still discards the Run.
    pub async fn discard(&self, run_id: RunId) -> Result<(), RunError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Discard(run_id, answer))
            .await
            .map_err(|_| RunError::Stopped)?;
        answered.await.map_err(|_| RunError::Stopped)?
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
    /// Connections' Runs go on. Answers the ends of the live Runs it
    /// started that outlive it instead, as a CLI command's do, which the
    /// Daemon waits for before it may stop idle.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still stops them.
    pub(crate) async fn stop_connection(&self, connection: ConnectionId) -> Outliving {
        let (answer, answered) = oneshot::channel();
        if self
            .commands
            .send(Command::StopConnection(connection, answer))
            .await
            .is_err()
        {
            return Outliving::default();
        }
        let ConnectionEnded {
            stopping,
            outliving,
        } = answered.await.unwrap_or_default();
        for report in stopping {
            let _ = wait_for(report).await;
        }
        outliving
    }

    /// Lets every Run the Daemon's Connection of these Runs starts or
    /// resumes go on once that Connection ends, as a CLI command's does:
    /// each command is a Connection of its own, and a Run outlives the
    /// command that started it (ADRs 0024 and 0088). Nothing on an
    /// Attached engine, whose Runs end with it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the Connection's Runs to stop with it.
    pub(crate) async fn outlive_connection(&self) {
        if let Some(connection) = self.connection {
            let _ = self.commands.send(Command::Outlive(connection)).await;
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
        self.drain_for(Cause::Asked).await
    }

    /// Drains the engine as [`Runs::drain`] does, because a part every Run
    /// shares panicked (ADR 0148): each Run's report carries the warning
    /// `engine_panicked`.
    ///
    /// # Cancel safety
    ///
    /// As [`Runs::drain`]'s.
    pub(crate) async fn drain_on_panic(&self) -> Draining {
        self.drain_for(Cause::Panic).await
    }

    async fn drain_for(&self, cause: Cause) -> Draining {
        let (answer, answered) = oneshot::channel();
        if self
            .commands
            .send(Command::Drain(cause, answer))
            .await
            .is_err()
        {
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
        self.stop_all_for(Cause::Asked).await.ended().await;
    }

    /// Stops every live Run as [`Runs::stop_all`] does, because a part
    /// every Run shares panicked on an Attached engine (ADR 0148): each is
    /// `stopped` for the engine's panic, never as aborted by its Client,
    /// and its report carries the warning `engine_panicked`. It waits for
    /// their ends only until `grace` resolves, as the part that panicked
    /// may be what a Run's end, or the registry, waits on: a Run not ended
    /// by then stays as it is, resumable from its run file.
    ///
    /// # Cancel safety
    ///
    /// As [`Runs::stop_all`]'s.
    pub(crate) async fn stop_all_on_panic(&self, grace: impl Future<Output = ()>) -> PanicStop {
        let mut grace = std::pin::pin!(grace);
        let stopping = tokio::select! {
            stopping = self.stop_all_for(Cause::Panic) => stopping,
            () = &mut grace => return PanicStop::Unanswered,
        };
        tokio::select! {
            () = stopping.ended() => PanicStop::Ended,
            () = grace => PanicStop::NotEnded(stopping.not_ended()),
        }
    }

    async fn stop_all_for(&self, cause: Cause) -> Draining {
        let (answer, answered) = oneshot::channel();
        if self
            .commands
            .send(Command::StopAll(cause, answer))
            .await
            .is_err()
        {
            return Draining(Vec::new());
        }
        Draining(answered.await.unwrap_or_default())
    }

    /// Whether the registry's own task panicked (ADR 0148).
    pub(crate) fn registry_panicked(&self) -> bool {
        self.registry.has_panicked()
    }
}

/// How an Attached engine's stop on a panic left its Runs
/// ([`Runs::stop_all_on_panic`]).
#[derive(Debug)]
pub(crate) enum PanicStop {
    /// Each Run it stopped ended in time.
    Ended,
    /// This many Runs it stopped had not ended when the grace passed.
    NotEnded(usize),
    /// The registry had not answered the stop when the grace passed.
    Unanswered,
}

/// The Runs a drain (ADR 0049), or a stop of every live Run, stops.
pub(crate) struct Draining(Vec<watch::Receiver<Option<Ended>>>);

impl Draining {
    /// How many Runs it stops.
    pub(crate) fn count(&self) -> usize {
        self.0.len()
    }

    /// How many of them have not ended yet.
    pub(crate) fn not_ended(&self) -> usize {
        self.0
            .iter()
            .filter(|report| report.borrow().is_none())
            .count()
    }

    /// Answers once every one ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the wait.
    pub(crate) async fn ended(&self) {
        for report in self.0.iter().cloned() {
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

/// What the end of a Daemon's Connection leaves: the ends of the Runs that
/// stop with it and of those that outlive it.
#[derive(Default)]
struct ConnectionEnded {
    stopping: Vec<watch::Receiver<Option<Ended>>>,
    outliving: Outliving,
}

/// The live Runs that outlived the Daemon's Connection that started them.
#[derive(Default)]
pub(crate) struct Outliving(Vec<watch::Receiver<Option<Ended>>>);

impl Outliving {
    /// Whether no Run outlived it.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Resolves once each of them has ended, or the registry stopped.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe: it only waits.
    pub(crate) async fn ended(self) {
        for mut report in self.0 {
            let _ = report.wait_for(Option::is_some).await;
        }
    }
}

async fn wait_for(mut report: watch::Receiver<Option<Ended>>) -> Result<RunReport, RunError> {
    let ended = report
        .wait_for(Option::is_some)
        .await
        .map_err(|_| RunError::Stopped)?;
    match ended.clone().expect("invariant: the wait ends on an end") {
        Ended::Report(report) | Ended::Unwritten(report) => Ok(*report),
        Ended::Unrecorded(exhausted) => Err(RunError::ResourcesExhausted(exhausted)),
    }
}

/// How a Run's end went: its report, or a stop its run file could not
/// record, which leaves it interrupted (ADR 0122).
#[derive(Clone, Debug)]
enum Ended {
    /// Its end and its report are in its run file.
    Report(Box<RunReport>),
    /// Its end could not be written to its run file for another reason
    /// than its storage: this report, with nothing read from its state, is
    /// the only one there is.
    Unwritten(Box<RunReport>),
    /// The commit recording its stop met a full disk or an I/O error.
    Unrecorded(Box<StorageExhausted>),
}

impl Ended {
    /// Its report, unless its stop went unrecorded.
    fn report(&self) -> Option<&RunReport> {
        match self {
            Self::Report(report) | Self::Unwritten(report) => Some(report),
            Self::Unrecorded(_) => None,
        }
    }
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
    /// How many storage samples the governor had begun when it went live.
    /// Its start or resume read its filesystems afresh before then (ADR
    /// 0122), so a sample numbered up to it, read before that, says
    /// nothing of it: such a sample, read at `critical` and handed over
    /// after space was freed, would stop the Run its resume let go on.
    sampled_after: u64,
    /// Its progress, which its readers follow (ADR 0102).
    progress: Progress,
}

/// What a discard, which runs beside the registry, needs of it.
struct Erasing {
    /// Under whose blocking policy it runs.
    owner: TaskOwner,
    /// What its retries wait on.
    clock: Clock,
    entropy: Entropy,
    /// The VFS run files open through.
    vfs: Option<String>,
    /// The install database, in the per-install layout.
    install: Option<Arc<InstallDatabase>>,
    /// The Run index, as this engine opened it.
    index: RunIndex,
}

/// Why the run file a crash left at `off` of the Run `run_id` was not
/// discarded.
struct UncleanDiscard {
    run_id: RunId,
    folder: PathBuf,
    error: discard::DiscardError,
}

/// The error a start of the Run answers.
impl From<UncleanDiscard> for StartError {
    fn from(unclean: UncleanDiscard) -> Self {
        let UncleanDiscard {
            run_id,
            folder,
            error,
        } = unclean;
        match error {
            discard::DiscardError::Held => StartError::InUse { run_id, folder },
            discard::DiscardError::State(error) => StartError::Unavailable {
                run_id,
                folder,
                error: error.to_string(),
            },
            discard::DiscardError::Io(error) => StartError::Unavailable {
                run_id,
                folder,
                error,
            },
        }
    }
}

/// The error a resume of the Run answers.
impl From<UncleanDiscard> for RunError {
    fn from(unclean: UncleanDiscard) -> Self {
        let UncleanDiscard {
            run_id,
            folder,
            error,
        } = unclean;
        match error {
            discard::DiscardError::Held => RunError::InUse { run_id, folder },
            discard::DiscardError::State(error) => RunError::of_state(&run_id, &folder, error),
            discard::DiscardError::Io(error) => RunError::Unavailable {
                run_id,
                folder,
                error,
            },
        }
    }
}

/// How a discard went.
struct Erased {
    run_id: RunId,
    /// Whether the Run's state is gone, so that the registry forgets its
    /// end.
    state_gone: bool,
    /// What its caller is answered.
    answered: Result<(), RunError>,
}

impl Erasing {
    /// Erases the Run `run_id` in `folder` (ADRs 0001 and 0036): its state
    /// under its writer lock, then its Run index row, marked *erased*.
    async fn erase(mut self, run_id: RunId, folder: PathBuf) -> Erased {
        let discarding = discard::Discarding {
            owner: &self.owner,
            clock: &self.clock,
            entropy: &mut self.entropy,
        };
        let discarded = discard::discard(
            discarding,
            run_id.as_str(),
            &folder,
            self.vfs.as_deref(),
            self.install.as_ref(),
        )
        .await
        .map_err(|error| match error {
            discard::DiscardError::Held => RunError::InUse {
                run_id: run_id.clone(),
                folder: folder.clone(),
            },
            discard::DiscardError::State(error) => RunError::of_state(&run_id, &folder, error),
            discard::DiscardError::Io(error) => RunError::Unavailable {
                run_id: run_id.clone(),
                folder: folder.clone(),
                error,
            },
        });
        let discarded = match discarded {
            Ok(discarded) => discarded,
            Err(error) => {
                return Erased {
                    run_id,
                    state_gone: false,
                    answered: Err(error),
                };
            }
        };
        let listed = self
            .index
            .erase(run_id.clone())
            .await
            .map_err(|error| RunError::of_state(&run_id, &folder, error));
        let answered = match listed {
            Ok(false) if discarded == discard::Discarded::Absent => {
                Err(RunError::NotFound(run_id.clone()))
            }
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        };
        Erased {
            run_id,
            state_gone: true,
            answered,
        }
    }
}

struct Registry {
    setup: RunsSetup,
    /// The gate in admission and the Politeness the engine's Runs share.
    admitting: Arc<Admitting>,
    /// The owner of the work it does for every Run, such as following the
    /// governor's samples, whose panic stops the engine (ADR 0148).
    owner: TaskOwner,
    /// The owner of the work it does for one Run, such as publishing its
    /// progress or erasing its state, whose panic ends no other Run and
    /// not the engine (ADR 0148).
    for_one_run: TaskOwner,
    /// The owner of each Run's end: recording it, its takeover and its
    /// last Checkpoint, of one Run too.
    ending_owner: TaskOwner,
    /// The owner of its own task, which each handle carries.
    itself: Arc<TaskOwner>,
    /// Where readers of run state run their blocking reads.
    readers: Arc<TaskOwner>,
    live: BTreeMap<RunId, Live>,
    /// What it keeps of the Runs this engine ended: nothing of most
    /// (`runs::ended`).
    ended: EndedRuns,
    runs: mpsc::WeakSender<Command>,
    /// What its Runs charge against.
    limits: Arc<Limits>,
    /// The machine's identity, once read.
    machine: Option<String>,
    /// Whether the engine drains, taking no Run (ADR 0049).
    draining: bool,
    /// The Daemon's Connections whose Runs go on once they end: the CLI's,
    /// one per command.
    outliving: std::collections::BTreeSet<ConnectionId>,
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
    /// Where the governor's last resort stands: it stops one Run at a time.
    memory_stop: MemoryStop,
    /// The Runs being discarded, beside the registry: no start, resume or
    /// read of one is taken until its discard ends.
    discarding: std::collections::BTreeSet<RunId>,
}

/// Where the governor's last resort stands (ADR 0040): it stops one Run at
/// a time, and measures again before it stops another.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MemoryStop {
    /// No Run is being stopped for it.
    Idle,
    /// This Run is being stopped for it.
    Stopping(RunId),
    /// The Run it stopped ended: only a sample past this number, read
    /// wholly after that end, may stop another.
    After(u64),
}

/// What a resumed Run's report lists as weakened: `listed`, what it listed,
/// joined by the settings the resume's own `settings` weaken, recorded in
/// `state` when they add any, so that the report says what ran (ADR 0016).
async fn joined_weakened(
    listed: &[WeakenedSetting],
    settings: Option<&RunSettings>,
    state: &RunState,
) -> Result<Vec<WeakenedSetting>, RunStateError> {
    let mut weakened = listed.to_vec();
    for setting in settings
        .map(|settings| settings.resolved().weakened())
        .unwrap_or_default()
    {
        if !weakened.iter().any(|kept| kept.key == setting.key) {
            weakened.push(setting);
        }
    }
    if weakened.len() > listed.len() {
        let recording = weakened.clone();
        state
            .write(move |connection| folder::record_weakened(connection, &recording))
            .await?;
    }
    Ok(weakened)
}

impl Registry {
    async fn run(
        mut self,
        mut mailbox: mpsc::Receiver<Command>,
        startup: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) {
        // No command is answered before the reclaim ends, so a Run starts
        // on a ledger holding its major's managed Limits and rid of dead
        // runs' Slices, and an answered Run call says the engine's startup
        // writes are done.
        startup.await;
        self.reclaim().await;
        self.follow_governor();
        while let Some(command) = mailbox.recv().await {
            if let Some(command) = self.unless_discarding(command) {
                self.answer(command).await;
            }
        }
        self.readers.wait().await;
        self.ending_owner.wait().await;
        self.for_one_run.wait().await;
        self.owner.wait().await;
    }

    /// Handles `command`, one of a Run the engine is not discarding.
    async fn answer(&mut self, command: Command) {
        match command {
            Command::Start(start, _, answer) if self.draining => {
                let (start, _, _) = *start;
                let _ = answer.send(Err(StartError::Draining {
                    run_id: start.run_id,
                }));
            }
            Command::Resume(resume, _, answer) if self.draining => {
                let (resume, _, _, _) = *resume;
                let _ = answer.send(Err(RunError::Draining(resume.run_id)));
            }
            Command::Drain(cause, answer) => {
                let _ = answer.send(self.drain(cause).await);
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
            Command::StopAll(cause, answer) => {
                let ids: Vec<RunId> = self.live.keys().cloned().collect();
                let stopped = match cause {
                    Cause::Asked => self.stop_by_client(ids).await,
                    Cause::Panic => self.stop_on_panic(ids).await,
                };
                let _ = answer.send(stopped);
            }
            Command::StopConnection(connection, answer) => {
                let ended = self.connection_ended(connection).await;
                let _ = answer.send(ended);
            }
            Command::Outlive(connection) => _ = self.outliving.insert(connection),
            Command::LiveCount(answer) => _ = answer.send(self.live.len()),
            Command::Health(answer) => {
                let _ = answer.send(self.health());
            }
            Command::Folder(run_id, answer) => _ = answer.send(self.folder(&run_id)),
            Command::Governor(answer) => _ = answer.send(self.setup.governor.clone()),
            Command::Live(run_id, answer) => _ = answer.send(self.live_run(run_id)),
            Command::Checkpoint(run_id, location, answer) => {
                self.checkpoint(run_id, location, answer).await;
            }
            Command::Checkpointed(run_id, checkpoint) => {
                if let Some(live) = self.live.get_mut(&run_id) {
                    let time = checkpoint.time.clone();
                    live.progress
                        .change(|progressing| progressing.checkpoint_at = Some(time));
                    live.checkpoint = Some(*checkpoint);
                }
            }
            Command::Stop(run_id, reason, location, answer) => {
                let _ = answer.send(self.stop(&run_id, reason, location).await);
            }
            Command::GenerateId(answer) => _ = answer.send(self.generated_id()),
            Command::StoredKey(run_id, checkpoint, answer) => {
                let _ = answer.send(self.stored_key(&run_id, checkpoint.as_deref()).await);
            }
            Command::Resume(resume, connection, answer) => {
                let (resume, link, keys, stored) = *resume;
                let resumed = self.resume(resume, keys, stored, connection).await;
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
            Command::Reader(run_id, answer) => _ = answer.send(Ok(self.reader(&run_id))),
            Command::Progress(run_id, answer) => _ = answer.send(Ok(self.followed(&run_id))),
            Command::Locate(run_id, answer) => _ = answer.send(Ok(self.locate(&run_id))),
            Command::Index(answer) => _ = answer.send(self.setup.engine.run_index().clone()),
            Command::Discard(run_id, answer) => self.discard(run_id, answer),
            Command::Discarded(erased, answer) => self.discarded(*erased, answer),
            Command::Storage(sample) => self.storage(&sample),
            Command::StorageFailed(folder) => self.storage_failed(&folder),
            Command::LastResort(last_resort) => self.last_resort(last_resort),
        }
    }

    /// Has the governor sample the engine's own folders, and hands each of
    /// its storage samples to the registry (ADR 0122).
    fn sample_storage(&mut self) {
        self.sampled = self
            .setup
            .governor
            .sample_together(self.engine_folders().sampled());
        let mut samples = self.setup.governor.storage();
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

    /// Hands the governor's storage samples (ADR 0122) and whether each
    /// sample found it at its last resort (ADR 0040) to the registry.
    fn follow_governor(&mut self) {
        self.sample_storage();
        self.follow_last_resort();
    }

    /// Hands each of the governor's samples, whether it found it at its
    /// last resort, to the registry (ADR 0040).
    fn follow_last_resort(&self) {
        let mut samples = self.setup.governor.last_resort();
        let registry = self.runs.clone();
        let token = self.owner.token().clone();
        self.owner.spawn(async move {
            loop {
                tokio::select! {
                    () = token.cancelled() => return,
                    changed = samples.changed() => if changed.is_err() { return },
                }
                let last_resort = *samples.borrow_and_update();
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                if registry
                    .send(Command::LastResort(last_resort))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Acts on a sample at the governor's last resort (ADR 0040): the live
    /// Run with the largest footprint, never one a person holds a page of,
    /// checkpoints and stops at its next safe point with
    /// `resource: "memory"`; while it stops, and until a sample read wholly
    /// after its end, no other Run is stopped.
    fn last_resort(&mut self, last_resort: LastResort) {
        match self.memory_stop {
            MemoryStop::Stopping(_) => return,
            MemoryStop::After(after) if last_resort.sample <= after => return,
            MemoryStop::After(_) | MemoryStop::Idle => self.memory_stop = MemoryStop::Idle,
        }
        if !last_resort.reached {
            return;
        }
        let candidates = self.live.values().filter_map(|live| {
            let run = live.run.as_ref().filter(|run| !run.is_stopping())?;
            Some(((live.id.clone(), run), run.governor().weight()))
        });
        let Some((run_id, run)) = last_resort_run(candidates) else {
            return;
        };
        tracing::warn!(
            run_id = %run_id,
            "memory stayed critical at the governor's floor: the Run with the largest footprint stops"
        );
        run.stop(StopReason::Resources {
            resource: goodfetch_protocol::run::Resource::Memory,
            folder: None,
        });
        self.memory_stop = MemoryStop::Stopping(run_id);
    }

    /// Acts on a storage sample (ADR 0122): each live Run with an essential
    /// write on a filesystem at `critical` (its run file, or the Usage
    /// ledger and the Politeness file in the Goodfetch home) stops at its
    /// next safe point; each Run's progress carries the worst level among
    /// its filesystems when it changes, and its report a `storage_low`
    /// warning for each folder found at `low` or worse; a Run ignores a
    /// sample begun before it went live, which its start or resume's fresh
    /// read supersedes. The runtime's metrics record the filesystem of run
    /// state (the install database's in the per-install layout, the run
    /// folders' otherwise), of the Goodfetch home and of the temporary
    /// folder, each labelled by what it holds, so one filesystem holding
    /// several gives each its series.
    fn storage(&mut self, sample: &StorageSample) {
        let folders = self.engine_folders();
        self.setup.metrics.storage(
            folders
                .labelled()
                .into_iter()
                .filter_map(|(holds, folder)| {
                    let filesystem = sample.of(folder)?;
                    Some(FolderStorage {
                        holds,
                        level: filesystem.level,
                        free: filesystem.free,
                        free_inodes: filesystem.free_inodes,
                    })
                }),
        );
        let (home, temporary) = (&folders.home, &folders.temporary);
        let install = self
            .setup
            .install
            .as_ref()
            .map(|install| install.folder().to_owned());
        for live in self.live.values_mut() {
            let Some(run) = &live.run else {
                continue;
            };
            if sample.number <= live.sampled_after {
                continue;
            }
            let run_state = install.as_ref().unwrap_or(&live.folder);
            let essential = [run_state, home];
            if let Some(folder) = essential
                .into_iter()
                .find(|folder| sample.of(folder).is_some_and(Filesystem::is_critical))
            {
                run.stop(StopReason::Resources {
                    resource: goodfetch_protocol::run::Resource::Storage,
                    folder: Some(folder.display().to_string()),
                });
            }
            let Some((folder, filesystem)) =
                worst(sample, &[&live.folder, run_state, home, temporary])
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
                let carried = storage.clone();
                live.progress
                    .change(|progressing| progressing.storage = Some(carried));
            }
            if storage.level >= StorageLevel::Low && live.warned_low.insert(storage.folder.clone())
            {
                // Once per folder, whichever run epoch met it first (ADR
                // 0122): a resume's starts empty in memory.
                let warning = storage_low(&storage);
                let warned = run.clone();
                run.spawn(async move {
                    if let Err(error) = warned.warn_once(warning).await {
                        tracing::warn!(%error, run_id = %warned.id, "the Run's `storage_low` warning was not kept");
                    }
                });
            }
        }
    }

    /// Whether this engine may resume the Run `run_id` that `held` holds in
    /// `folder`: one bound to its machine only there (ADR 0056), one of its
    /// Product only, and none while the filesystem of its run state or of
    /// the Goodfetch home is at `critical` (ADR 0122).
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
        let essential = vec![
            self.run_state_of(folder),
            self.setup.engine.home().root().to_owned(),
        ];
        match self.critical_among(essential).await {
            Some((folder, filesystem)) => {
                Err(RunError::ResourcesExhausted(Box::new(StorageExhausted {
                    run_id: run_id.clone(),
                    folder,
                    filesystem,
                })))
            }
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

    /// The first of `folders` whose filesystem is at `critical` now, read
    /// afresh rather than from the last sample (ADR 0122), with it.
    async fn critical_among(&self, folders: Vec<PathBuf>) -> Option<(PathBuf, Filesystem)> {
        let governor = self.setup.governor.clone();
        self.owner
            .blocking(move || critical_among(&governor, folders))
            .await
    }

    /// Takes no Run from now on, stops each live Run for the drain with a
    /// Checkpoint, warned first when `cause` is a panic, and answers the
    /// waits for their reports.
    async fn drain(&mut self, cause: Cause) -> Vec<watch::Receiver<Option<Ended>>> {
        self.draining = true;
        let ids: Vec<RunId> = self.live.keys().cloned().collect();
        let mut reports = Vec::new();
        for run_id in ids {
            if cause == Cause::Panic {
                self.warn_engine_panicked(&run_id).await;
            }
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

    /// Takes no Run from now on, stops each of `ids` for its Attached
    /// engine's panic (ADR 0148), its report warned first, and answers the
    /// waits for their reports.
    async fn stop_on_panic(&mut self, ids: Vec<RunId>) -> Vec<watch::Receiver<Option<Ended>>> {
        self.draining = true;
        let mut reports = Vec::new();
        for run_id in ids {
            self.warn_engine_panicked(&run_id).await;
            let end = End::Stopped(StopReason::EnginePanicked);
            if let Ok(report) = self.end_or_report(&run_id, end).await {
                reports.push(report);
            }
        }
        reports
    }

    /// Hands the warning `engine_panicked` over to the report of the live
    /// Run `run_id`, which the engine stops as it stops on a panic in a
    /// part every Run shares (ADR 0148).
    async fn warn_engine_panicked(&self, run_id: &RunId) {
        let Some(run) = self.live.get(run_id).and_then(|live| live.run.as_ref()) else {
            return;
        };
        let warning = RunWarning {
            code: goodfetch_protocol::run::warning::ENGINE_PANICKED.to_owned(),
            message: "The engine stopped the Run as it stopped on a bug in a part every Run shares; a start with its Run ID resumes it.".to_owned(),
            sessions: Vec::new(),
            folder: None,
        };
        if let Err(error) = run.warn_once(warning).await {
            tracing::warn!(%error, %run_id, "the Run's `engine_panicked` warning was not kept");
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
        if self.memory_stop == MemoryStop::Stopping(run_id.clone()) {
            // The sample after the latest may have been read before this
            // end: the governor measures again from the one after it.
            let latest = self.setup.governor.last_resort().borrow().sample;
            self.memory_stop = MemoryStop::After(latest + 1);
        }
        if let Some(live) = self.live.remove(&run_id) {
            // The lock goes before the report, so whoever the report wakes
            // finds the Run's writer gone.
            let Live {
                report: waiters,
                lock,
                kind,
                progress,
                ..
            } = live;
            drop(lock);
            progress.change(|progressing| {
                if let Some(report) = ended.report() {
                    progressing.lifecycle = report.lifecycle;
                    progressing.stop_reason.clone_from(&report.stop_reason);
                }
                progressing.writer = RunWriter::Absent;
                progressing.last = true;
            });
            self.setup.metrics.run_ended(kind, ended.report());
            waiters.send_replace(Some(ended.clone()));
        }
        match ended {
            Ended::Report(report) => self.ended.in_run_file(run_id, report.checkpoint),
            Ended::Unwritten(report) => self.ended.unwritten(run_id, report),
            Ended::Unrecorded(exhausted) => {
                self.unrecorded.insert(run_id, exhausted);
            }
        }
    }

    /// The live Runs that stop as the Daemon's Connection `connection`
    /// ends: those its Client started or resumed, unless they outlive it,
    /// as a CLI command's do, or the engine drains, leaving each Run to its
    /// drain, which a Client's leaving does not abort.
    fn stopping_with(&mut self, connection: ConnectionId) -> Vec<RunId> {
        if self.outliving.remove(&connection) || self.draining {
            return Vec::new();
        }
        self.runs_of(connection)
    }

    /// Ends the Daemon's Connection `connection`: stops the live Runs that
    /// stop with it, and answers their ends and those of the Runs that
    /// outlive it, unless the engine drains, whose drain ends them.
    async fn connection_ended(&mut self, connection: ConnectionId) -> ConnectionEnded {
        let outlives = self.outliving.contains(&connection) && !self.draining;
        let stopping = self.stopping_with(connection);
        let stopping = self.stop_by_client(stopping).await;
        let outliving = if outlives {
            self.runs_of(connection)
                .iter()
                .filter_map(|run_id| self.live.get(run_id))
                .map(|live| live.report.subscribe())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        ConnectionEnded {
            stopping,
            outliving: Outliving(outliving),
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

    /// What a progress read of the Run `run_id` follows.
    fn followed(&self, run_id: &RunId) -> Followed {
        Followed {
            changes: self.live.get(run_id).map(|live| live.progress.subscribe()),
            reader: self.reader(run_id),
            parts: self.setup.progress_parts.clone(),
            clock: self.setup.engine.clock(),
        }
    }

    /// Where a read of the Run `run_id` that this engine does not run
    /// looks for the engine that does; `None` when this engine runs it.
    fn locate(&self, run_id: &RunId) -> Option<Located> {
        if self.live.contains_key(run_id) {
            return None;
        }
        Some(Located {
            folder: self.folder(run_id),
            index: self.setup.engine.run_index().clone(),
            reached_at: self.setup.reached_at.clone(),
            forwarder: Arc::clone(&self.setup.forwarder),
            tasks: Arc::clone(&self.readers),
        })
    }

    fn reader(&self, run_id: &RunId) -> RunReader {
        let following = self.live.get(run_id).map(|live| Following {
            commits: live.run.as_ref().map(|run| run.state.commits()),
            report: live.report.subscribe(),
            readers: live.run.as_ref().map(|run| run.state.readers()),
        });
        RunReader::new(
            run_id.clone(),
            self.folder(run_id),
            self.setup.run_state.vfs.clone(),
            Arc::clone(&self.readers),
            following,
            self.setup.engine.clock(),
        )
    }

    /// Discards the Run `run_id` beside the registry, which answers every
    /// other call meanwhile (ADR 0050), and has the registry send `answer`
    /// how it went once it ended: a Run this engine runs is refused, and so
    /// is one already being discarded.
    fn discard(&mut self, run_id: RunId, answer: oneshot::Sender<Result<(), RunError>>) {
        if self.live.contains_key(&run_id) {
            let _ = answer.send(Err(RunError::Live(run_id)));
            return;
        }
        let folder = self.folder(&run_id);
        if !self.discarding.insert(run_id.clone()) {
            let _ = answer.send(Err(RunError::Discarding { run_id, folder }));
            return;
        }
        let erasing = Erasing {
            owner: self.for_one_run.child(),
            clock: self.setup.engine.clock(),
            entropy: self.setup.entropy.fork(),
            vfs: self.setup.run_state.vfs.clone(),
            install: self.setup.install.clone(),
            index: self.setup.engine.run_index().clone(),
        };
        let registry = self.runs.clone();
        self.for_one_run.spawn(async move {
            let erased = erasing.erase(run_id, folder).await;
            if let Some(registry) = registry.upgrade() {
                let _ = registry
                    .send(Command::Discarded(Box::new(erased), answer))
                    .await;
            }
        });
    }

    /// Takes the end of a discard: the Run is no longer being discarded,
    /// and once its state is gone the registry forgets its end too.
    fn discarded(&mut self, erased: Erased, answer: oneshot::Sender<Result<(), RunError>>) {
        let Erased {
            run_id,
            state_gone,
            answered,
        } = erased;
        self.discarding.remove(&run_id);
        if state_gone {
            self.ended.forget(&run_id);
            self.found_ended.remove(&run_id);
            self.unrecorded.remove(&run_id);
        }
        let _ = answer.send(answered);
    }

    /// `command`, unless it starts, resumes, ends or reads a Run being
    /// discarded, which it then answers as refused with `Discarding`.
    fn unless_discarding(&self, command: Command) -> Option<Command> {
        let run_id = match &command {
            Command::Start(start, _, _) => start.0.run_id.as_ref(),
            Command::Resume(resume, _, _) => Some(&resume.0.run_id),
            Command::Finish(run_id, ..)
            | Command::Abort(run_id, ..)
            | Command::SafeStop(run_id, ..)
            | Command::Stop(run_id, ..)
            | Command::Checkpoint(run_id, ..)
            | Command::StoredKey(run_id, ..)
            | Command::Reader(run_id, _)
            | Command::Progress(run_id, _)
            | Command::Locate(run_id, _) => Some(run_id),
            _ => None,
        };
        let Some(run_id) = run_id.filter(|run_id| self.discarding.contains(*run_id)) else {
            return Some(command);
        };
        let (run_id, folder) = (run_id.clone(), self.folder(run_id));
        let refused = || RunError::Discarding {
            run_id: run_id.clone(),
            folder: folder.clone(),
        };
        match command {
            Command::Start(_, _, answer) => {
                let _ = answer.send(Err(StartError::Discarding {
                    run_id: run_id.clone(),
                    folder: folder.clone(),
                }));
            }
            Command::Resume(_, _, answer) => _ = answer.send(Err(refused())),
            Command::Finish(_, _, answer) | Command::Stop(_, _, _, answer) => {
                _ = answer.send(Err(refused()));
            }
            Command::Abort(_, _, _, answer) | Command::SafeStop(_, _, answer) => {
                _ = answer.send(Err(refused()));
            }
            Command::Checkpoint(_, _, answer) => _ = answer.send(Err(refused())),
            Command::StoredKey(_, _, answer) => _ = answer.send(Err(refused())),
            Command::Reader(_, answer) => _ = answer.send(Err(refused())),
            Command::Progress(_, answer) => _ = answer.send(Err(refused())),
            Command::Locate(_, answer) => _ = answer.send(Err(refused())),
            other => return Some(other),
        }
        None
    }

    /// How a Run's run file opens under its `settings`: at the Durability
    /// level `run_state.durability` names, `full` when it has none.
    fn opened_with(&self, settings: Option<&RunSettings>) -> RunStateOptions {
        RunStateOptions {
            durability: durability_of(settings),
            clock: self.setup.engine.clock(),
            ..self.setup.run_state.clone()
        }
    }

    fn folder(&self, run_id: &RunId) -> PathBuf {
        self.setup
            .location
            .folder(self.setup.engine.home(), &self.setup.version)
            .join(run_id.as_str())
    }

    /// The Run's folder as the Run index records it: relative to the
    /// per-major folder when inside it.
    fn location(&self, folder: &Path) -> String {
        location(
            &self.setup.engine.home().per_major(&self.setup.version),
            folder,
        )
    }

    fn now(&self) -> PointInTime {
        self.setup.engine.clock().point_in_time()
    }

    /// A lowercase ULID from the clock and the entropy (ADR 0059).
    fn generated_id(&mut self) -> RunId {
        let milliseconds = self
            .setup
            .engine
            .clock()
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
        mut start: Start,
        secrets: RunSecrets,
        connection: Option<ConnectionId>,
        link: &Link,
    ) -> Result<Started, StartError> {
        let run_id = match &start.run_id {
            Some(run_id) => run_id.clone(),
            None => self.generated_id(),
        };
        // Its tags and inputs pass through the `Mask` before run state and
        // the Run index keep them (spec #564), as every writer does.
        let given = canonical_inputs(&start.inputs);
        let mut kept = start.inputs.clone();
        for value in kept.values_mut() {
            self.setup.masks.apply_value(value, Scope::Kept);
        }
        let inputs = canonical_inputs(&kept);
        start.tags = masked_tags(&self.setup.masks, &start.tags);
        // A run file a crash left at `off` is discarded, never read: the Run
        // starts again (ADR 0138).
        self.discard_unclean(&run_id, start.settings.as_ref())
            .await?;
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
            // A value that left the masks since the Run kept its inputs
            // masked is no difference: an input differs when it differs
            // both masked and as given.
            let unmasked = input_differences(&found.inputs, &given);
            differences.extend(
                input_differences(&found.inputs, &inputs)
                    .into_iter()
                    .filter(|input| unmasked.contains(input)),
            );
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
        let essential = self.engine_folders().essential();
        if let Some((folder, filesystem)) = self.critical_among(essential).await {
            return Err(StartError::ResourcesExhausted(Box::new(StorageExhausted {
                run_id,
                folder,
                filesystem,
            })));
        }
        let started = self
            .begin(
                run_id,
                start,
                inputs,
                Keys { secrets, key: None },
                connection,
            )
            .await?;
        if let Started::New(run) = &started {
            link.add_to(run.span());
        }
        Ok(started)
    }

    /// Discards the state of the Run `run_id`, which this engine does not
    /// run, when its run file is one a crash left at `off` (ADR 0138), the
    /// Run's Durability level now being the one its `settings` name: under
    /// its writer lock, as a discard does, so another engine's live Run is
    /// never touched.
    async fn discard_unclean(
        &mut self,
        run_id: &RunId,
        settings: Option<&RunSettings>,
    ) -> Result<(), UncleanDiscard> {
        if self.live.contains_key(run_id) {
            return Ok(());
        }
        let folder = self.folder(run_id);
        let discarding = discard::Discarding {
            owner: &self.owner,
            clock: &self.setup.engine.clock(),
            entropy: &mut self.setup.entropy,
        };
        let discarded = discard::discard_unclean(
            discarding,
            run_id.as_str(),
            &folder,
            self.setup.run_state.vfs.as_deref(),
            self.setup.install.as_ref(),
            durability_of(settings),
        )
        .await
        .map_err(|error| UncleanDiscard {
            run_id: run_id.clone(),
            folder,
            error,
        })?;
        if discarded {
            tracing::warn!(run_id = %run_id, "the Run's run file was left by a crash at `run_state.durability = off`: it was discarded, and the Run starts again");
            self.ended.forget(run_id);
            self.found_ended.remove(run_id);
            self.unrecorded.remove(run_id);
        }
        Ok(())
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
    /// engine may hold while it runs there; one found held, perhaps only by
    /// a reader's test, is retried as every writer retries it (ADR 0031).
    async fn lock(&mut self, run_id: &RunId, folder: &Path) -> Result<File, StartError> {
        let (owner, clock) = (&self.owner, &self.setup.engine.clock());
        let taken = match try_lock(owner, folder).await {
            Ok(Some(lock)) => Ok(Some(lock)),
            // Only a retry draws from the engine's `Entropy`.
            Ok(None) => retried(clock, &mut self.setup.entropy, || try_lock(owner, folder)).await,
            Err(error) => Err(error),
        };
        match taken {
            Ok(Some(lock)) => Ok(lock),
            Ok(None) => Err(StartError::InUse {
                run_id: run_id.clone(),
                folder: folder.to_path_buf(),
            }),
            Err(error) => Err(StartError::Unavailable {
                run_id: run_id.clone(),
                folder: folder.to_path_buf(),
                error,
            }),
        }
    }

    /// The charging of the new Run `run_id`, started on `connection`, its
    /// state in `folder`, once it may start: every Limit that holds it has
    /// something left, and none it cannot be placed outside of is one this
    /// engine cannot evaluate (ADR 0041). A ledger that cannot be read
    /// refuses it. The Run then holds the Limits this read, on the
    /// connection it opened, so its charges open nothing (ADR 0140's E3).
    async fn may_start(
        &self,
        run_id: &RunId,
        folder: &Path,
        connection: Option<ConnectionId>,
    ) -> Result<Charging, StartError> {
        let now = self.limits.now();
        let mut charging =
            self.limits
                .charging(run_id, folder, 1, self.limits.placement(connection));
        let may_start = self
            .owner
            .blocking(move || charging.may_start(now).map(|()| charging))
            .await;
        match may_start {
            Ok(charging) => Ok(charging),
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
        keys: Keys,
        connection: Option<ConnectionId>,
    ) -> Result<Started, StartError> {
        let folder = self.folder(&run_id);
        let unavailable = |error: String| StartError::Unavailable {
            run_id: run_id.clone(),
            folder: folder.clone(),
            error,
        };
        self.check_index().await?;
        let charging = self.may_start(&run_id, &folder, connection).await?;
        let lock = self.lock(&run_id, &folder).await?;
        let writer = Arc::new(self.owner.child_for(OwnerKind::Run));
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
            .write_with(self.start_syncing(start.kind), move |connection| {
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
        let run = self.new_run(&run_id, start.kind, state, connection, start.settings, keys);
        *run.shared.charging() = Some(charging);
        let live = Live {
            id: run_id.clone(),
            progress: run.shared.progress.clone(),
            run: Some(run),
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
            sampled_after: self.setup.governor.storage_samples_begun(),
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
            stop_reason: report.and_then(|report| report.stop_reason.clone()),
            replayed: report.is_some_and(RunReport::replayed),
        }
    }

    /// Refuses a start when the Run index's marker refuses this engine
    /// (ADR 0098): an engine that finds its lowest minor raised past itself
    /// takes no new Run. An index that cannot be opened stops no Run; its
    /// row is restored by `runs reindex`.
    async fn check_index(&self) -> Result<(), StartError> {
        match self.setup.engine.run_index().check().await {
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
        if let Err(error) = self.setup.engine.run_index().write(row).await {
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
        let Some(live) = self.live.get(run_id) else {
            return Err(RunError::NotFound(run_id.clone()));
        };
        let row = self.index_row(live, Lifecycle::Running, None);
        self.write_index(row).await;
        let Some(live) = self.live.get(run_id) else {
            return Err(RunError::NotFound(run_id.clone()));
        };
        live.progress
            .change(|progressing| progressing.message = Some(message));
        Ok(live.progress.subscribe().borrow().value(run_id, self.now()))
    }

    /// What it answers about the engine's health.
    fn health(&self) -> RegistryHealth {
        let folders = self.engine_folders();
        RegistryHealth {
            draining: self.draining,
            governor_last_resort: self.setup.governor.at_last_resort(),
            essential: folders.essential(),
            governor: self.setup.governor.clone(),
            folder: folders.runs,
        }
    }

    /// The engine's own folders the governor samples.
    fn engine_folders(&self) -> EngineFolders {
        let runs = self
            .setup
            .location
            .folder(self.setup.engine.home(), &self.setup.version);
        EngineFolders {
            run_state: self
                .setup
                .install
                .as_ref()
                .map_or_else(|| runs.clone(), |install| install.folder().to_owned()),
            runs,
            home: self.setup.engine.home().root().to_owned(),
            temporary: std::env::temp_dir(),
        }
    }

    /// The folder of the filesystem holding the run state of the Run whose
    /// run folder is `folder`: the install database's in the per-install
    /// layout, its run folder's otherwise (ADRs 0002 and 0122).
    fn run_state_of(&self, folder: &Path) -> PathBuf {
        self.setup
            .install
            .as_ref()
            .map_or_else(|| folder.to_owned(), |install| install.folder().to_owned())
    }

    /// How the Runs of `kind` go, as the contribution that registers the
    /// kind's driver says.
    fn driving(&self, kind: RunKind) -> Driving {
        self.setup.driving.get(&kind).copied().unwrap_or_default()
    }

    /// How the start of a Run of `kind` commits: synced, and declaring how
    /// its charges reach the run file when its claims defer them.
    fn start_syncing(&self, kind: RunKind) -> Syncing {
        if self.driving(kind).defers_charges {
            Syncing::Declaring
        } else {
            Syncing::Required
        }
    }

    /// Reads the Limits that hold `run` as it resumes, on the connection
    /// its charging keeps, as a new Run's start does (ADR 0140's E3). A
    /// read that fails is left to its first charge, which reads again and
    /// refuses as a charge does.
    async fn read_limits(&self, run: &Run) {
        let shared = Arc::clone(&run.shared);
        let now = self.limits.now();
        self.owner
            .blocking(move || {
                if let Some(charging) = shared.charging().as_mut()
                    && let Err(refusal) = charging.resume(now)
                {
                    tracing::debug!(%refusal, "the resumed Run's Limits could not be read; its first charge reads them again");
                }
            })
            .await;
    }

    /// The live Run `run_id`, of `kind`, as its driver holds it, writing
    /// `state` in its folder, under its resolved `settings`, resolving its
    /// Secret references through `keys`. Its charging has read nothing
    /// yet: a new Run takes the one its start read its Limits with, and a
    /// resumed one reads them next.
    fn new_run(
        &mut self,
        run_id: &RunId,
        kind: RunKind,
        state: RunState,
        connection: Option<ConnectionId>,
        settings: Option<RunSettings>,
        keys: Keys,
    ) -> Run {
        let folder = &self.folder(run_id);
        let epoch = state.epoch();
        let progress = self.progress(run_id, Progressing::live(epoch, Lifecycle::Running, None));
        let resolved = settings.as_ref().map(RunSettings::resolved);
        let share = get_or_default(resolved, &RUN_SHARE);
        // A kind whose contribution names no setting for it logs its
        // Provenance (ADR 0138).
        let logs_provenance = self
            .driving(kind)
            .provenance_log
            .is_none_or(|setting| get_or_default(resolved, setting));
        Run {
            id: run_id.clone(),
            folder: folder.to_owned(),
            state,
            clock: self.setup.engine.clock(),
            admitting: Arc::clone(&self.admitting),
            shared: Arc::new(Shared {
                stop: watch::Sender::new(None),
                finish: watch::Sender::new(false),
                #[expect(
                    clippy::disallowed_types,
                    reason = "the Run's charging, the Mutex its field declares (ADR 0060)"
                )]
                charging: std::sync::Mutex::new(Some(self.limits.charging(
                    run_id,
                    folder,
                    epoch,
                    self.limits.placement(connection),
                ))),
                metrics: self.setup.metrics.clone(),
                span: tracing::info_span!(parent: None, "run", run_id = %run_id),
                secrets: keys.secrets,
                key: tokio::sync::OnceCell::new_with(keys.key),
                governor: self.setup.governor.join(share),
                _sampled: self.setup.governor.sample(folder.to_owned()),
                storage: self.setup.governor.storage(),
                progress,
                system_info: self.setup.governor.system_info(),
                politeness: tokio::sync::OnceCell::new(),
                robots_txt: RunCopies::default(),
                crawls_robots_txt: std::sync::OnceLock::new(),
                attempt_writes: std::sync::OnceLock::new(),
                provenance: run_trace::Provenance::logging(folder, logs_provenance),
            }),
            owner: Arc::new(self.owner.child_for(OwnerKind::Run)),
            runs: self.handle(connection),
            settings: settings.map(Arc::new),
            entropy: Arc::new(tokio::sync::Mutex::new(self.setup.entropy.fork())),
        }
    }

    /// The progress of the live Run `run_id`, as `start` is, whose every
    /// change goes out as `run.progress` (ADR 0102).
    fn progress(&self, run_id: &RunId, start: Progressing) -> Progress {
        let progress = Progress::new(start);
        self.for_one_run.spawn(progress::publish(
            run_id.clone(),
            progress.subscribe(),
            self.setup.events.clone(),
            self.setup.engine.clock(),
        ));
        progress
    }

    fn handle(&self, connection: Option<ConnectionId>) -> Runs {
        Runs {
            commands: self
                .runs
                .upgrade()
                .expect("invariant: the registry runs while a handle sends to it"),
            engine: self.setup.engine.clone(),
            connection,
            limits: Arc::clone(&self.limits),
            secrets: self.setup.secrets.clone(),
            clean_up: CleanUpRules::of(&self.setup),
            controls: Arc::new(self.setup.controls.clone()),
            metrics: self.setup.metrics.clone(),
            registry: Arc::clone(&self.itself),
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
        let clock = self.setup.engine.clock();
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
            governor: self.setup.governor.clone(),
        };
        let recording = Recording {
            index: self.setup.engine.run_index().clone(),
            row,
            events,
            registry: self.runs.clone(),
        };
        self.ending_owner.spawn(recording.end(closing));
        Ok(receiver)
    }

    /// The report of a Run whose state its folder holds, with the last
    /// Checkpoint this engine wrote of it when this engine ended it, or the
    /// report of one this engine ended without writing its end.
    async fn report_of(&self, run_id: &RunId) -> Result<watch::Receiver<Option<Ended>>, RunError> {
        let checkpoint = match self.ended.get(run_id) {
            Some(EndedHere::Unwritten(report)) => {
                return Ok(watch::Sender::new(Some(Ended::Unwritten(report.clone()))).subscribe());
            }
            Some(EndedHere::Checkpointed(checkpoint)) => Some(checkpoint.clone()),
            None => None,
        };
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
            Ok(Some(mut report)) => {
                if checkpoint.is_some() {
                    report.checkpoint = checkpoint;
                }
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
        if let Some(Ended::Report(report) | Ended::Unwritten(report)) = &mut report {
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
        if self.ended.contains(run_id) || self.found_ended.contains(run_id) {
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
        if self.live.contains_key(run_id) || self.ended.contains(run_id) {
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
            clock: self.setup.engine.clock(),
            entropy: self.setup.entropy.fork(),
            index: self.setup.engine.run_index().clone(),
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
            clock: self.setup.engine.clock(),
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
        let root = self.setup.engine.home().root().to_owned();
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
                folder: self.setup.engine.home().root().to_owned(),
                error: error.to_string(),
            })?;
        self.machine = Some(machine.clone());
        Ok(machine)
    }

    /// The wrapped data key of the Run `run_id` that this engine does not
    /// run, in the Checkpoint `checkpoint` or else its run file here.
    async fn stored_key(
        &self,
        run_id: &RunId,
        checkpoint: Option<&Path>,
    ) -> Result<Option<StoredKey>, RunError> {
        if self.live.contains_key(run_id) {
            return Ok(None);
        }
        Ok(self.held(run_id, checkpoint).await?.key)
    }

    /// What [`Self::held`] reads, refused when its wrapped data key is not
    /// `stored`, the one a resume unwrapped before the registry took it.
    async fn held_with_key(
        &self,
        run_id: &RunId,
        checkpoint: Option<&Path>,
        stored: Option<StoredKey>,
    ) -> Result<checkpoint::Held, RunError> {
        let held = self.held(run_id, checkpoint).await?;
        if held.key != stored {
            return Err(RunError::Unavailable {
                run_id: run_id.clone(),
                folder: self.folder(run_id),
                error: "its data key changed while it resumed; resume it again".to_owned(),
            });
        }
        Ok(held)
    }

    /// Resumes an interrupted Run, as [`Runs::resume`] says.
    async fn resume(
        &mut self,
        resume: Resume,
        keys: Keys,
        stored: Option<StoredKey>,
        connection: Option<ConnectionId>,
    ) -> Result<Resumed, RunError> {
        let Resume {
            run_id,
            checkpoint,
            checkpoint_interval,
            timeout,
            settings,
            retry_failed,
        } = resume;
        // A Run this engine writes is answered as it is.
        if let Some(live) = self.live.get(&run_id) {
            return Ok(Resumed::existing(run_id, live.lifecycle, live.needs));
        }
        // A run file a crash left at `off` is discarded, never read: the Run
        // is then not found, and starts again (ADR 0138).
        self.discard_unclean(&run_id, settings.as_ref()).await?;
        let held = self
            .held_with_key(&run_id, checkpoint.as_deref(), stored)
            .await?;
        let folder = self.folder(&run_id);
        self.may_resume(&run_id, &held, &folder).await?;
        let lifecycle = match lifecycle::apply(held.lifecycle, Resume::event(retry_failed)) {
            Ok(Next::To(next)) => next,
            Ok(Next::Unchanged) => {
                return Ok(Resumed::existing(run_id, held.lifecycle, held.needs));
            }
            Err(refused) => return Err(RunError::refused(&run_id, &folder, refused)),
        };
        let options = self.opened_with(settings.as_ref());
        let (lock, writer, state) = self
            .reopen(&run_id, held.lifecycle, checkpoint, &options)
            .await?;
        let recorded = self.recorded(&run_id, &folder).await?;
        self.ended.forget(&run_id);
        self.unrecorded.remove(&run_id);
        let progress_message = recorded
            .progress_message
            .as_deref()
            .and_then(ProgressMessage::parse);
        let listed = recorded.weakened;
        let weakened = joined_weakened(&listed, settings.as_ref(), &state)
            .await
            .map_err(|error| RunError::of_state(&run_id, &folder, error))?;
        let run = self.new_run(&run_id, recorded.kind, state, connection, settings, keys);
        self.read_limits(&run).await;
        if held.lifecycle == Lifecycle::Running {
            warn_over_charged(&run).await;
        }
        run.shared
            .progress
            .resumed(lifecycle, progress_message.clone());
        let live = Live {
            id: run_id.clone(),
            progress: run.shared.progress.clone(),
            run: Some(run),
            folder,
            kind: recorded.kind,
            needs: recorded.needs,
            tags: recorded.tags,
            inputs: recorded.inputs,
            ended_by: self.driving(recorded.kind).ended_by,
            started_at: recorded.started_at,
            policy_version: recorded.policy_version,
            weakened,
            report: watch::Sender::new(None),
            writer,
            lock: Arc::new(lock),
            connection,
            checkpoint: None,
            progress_message,
            lifecycle,
            storage: None,
            warned_low: std::collections::BTreeSet::new(),
            sampled_after: self.setup.governor.storage_samples_begun(),
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
            listed,
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
                StartError::Discarding { run_id, folder } => {
                    RunError::Discarding { run_id, folder }
                }
                StartError::Conflict { .. }
                | StartError::LimitReached(_)
                | StartError::LimitNotUnderstood(_)
                | StartError::Draining { .. }
                | StartError::Secret(_)
                | StartError::Refused(_)
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
        let writer = Arc::new(self.owner.child_for(OwnerKind::Run));
        let state = RunState::open(file, options, self.setup.tables.clone(), epoch, &writer)
            .await
            .map_err(|error| RunError::of_state(run_id, &folder, error))?;
        state
            .write(|connection| {
                connection
                    .execute(
                        "UPDATE run SET lifecycle = ?1, stop_reason = NULL, verdict = NULL,
                         completeness = NULL, ended_at = NULL",
                        [Lifecycle::Running.as_str()],
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
            .is_some_and(|filesystem| filesystem.is_critical());
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
            time: self.clock.point_in_time(),
        })
    }
}

/// Queues in `place`, a place in a Run's run state's queue, the journal
/// under `epoch` of what a change of its `charging` charged, when there is
/// something to journal; answers the future of its commit. Called under
/// the charging's lock, so journals are queued in the order of the
/// changes.
fn journaled(
    charging: &Charging,
    place: ReservedWrite,
    epoch: u64,
    charged: Charged,
) -> Option<impl Future<Output = Result<(), RunStateError>> + Send + 'static + use<>> {
    let uncapped = charging.uncapped();
    (!charged.used.is_empty() || !charged.counted.is_empty() || uncapped)
        .then(|| place.write(move |connection| journal(connection, epoch, &charged, uncapped)))
}

/// Journals in a Run's run state, under `epoch`, what a charge took from
/// each Slice and what it counted, and that the Run is *uncapped* once it is.
fn journal(
    connection: &rusqlite::Connection,
    epoch: u64,
    journaled: &Charged,
    uncapped: bool,
) -> rusqlite::Result<()> {
    slices::journal(connection, epoch, &journaled.used, uncapped)?;
    for counted in &journaled.counted {
        report::count(
            connection,
            &counted.usage_kind,
            counted.currency.as_deref(),
            counted.delta,
        )?;
        if let Some(part) = &counted.part {
            report::count_part(connection, &counted.usage_kind, part, counted.delta)?;
        }
    }
    Ok(())
}

/// Gives back what is left of the Slices `run` holds, under `writer`'s
/// blocking work, and answers the Limits that then apply to it, with what
/// is left of each (ADR 0101).
async fn given_back(run: &Run, writer: &TaskOwner) -> Vec<AppliedLimit> {
    let shared = Arc::clone(&run.shared);
    let now = run.runs.limits.now();
    writer
        .blocking(move || {
            let Some(mut charging) = shared.charging().take() else {
                return Ok(Vec::new());
            };
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

/// Records the certificate exceptions `run` used in the Audit log, once as
/// it ends, by host and kind with the key each presented (ADR 0042):
/// nothing when it reached no host through one. A failure to record is
/// logged, and the Run ends all the same.
async fn exceptions_used(run: &Run, clock: Clock) {
    let used = run.secrets().exception_hosts().used().await;
    if used.is_empty() {
        return;
    }
    let hosts: Vec<Value> = used
        .into_iter()
        .map(|(host, key, kind)| {
            json!({ "host": host, "kind": kind.as_str(), "spki_sha256": key.to_string() })
        })
        .collect();
    let origin = Origin {
        actor: Actor::Engine,
        surface: None,
        layer: None,
        run_id: Some(run.id.clone()),
    };
    let recorded = run
        .runs
        .engine
        .audit()
        .record(
            &origin,
            &CERTIFICATE_EXCEPTIONS_USED,
            json!({ "hosts": hosts }),
            clock.timestamp(),
        )
        .await;
    if let Err(error) = recorded {
        tracing::warn!(run_id = %run.id, %error, "the Audit log cannot record the certificate exceptions the Run used");
    }
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
            free_inodes: None,
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
    index: RunIndex,
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
        let ended = closing.end().await;
        let Some(report) = ended.report() else {
            if let Some(registry) = self.registry.upgrade() {
                let _ = registry.send(Command::Ended(run_id, Box::new(ended))).await;
            }
            return;
        };
        let row = RunIndexRow {
            lifecycle: report.lifecycle,
            verdict: Some(report.verdict),
            completeness: Some(report.completeness),
            ended_at: Some(report.ended_at.clone()),
            owner: None,
            stop_reason: report.stop_reason.clone(),
            replayed: report.replayed(),
            ..self.row
        };
        if let Err(error) = self.index.write(row).await {
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
            let _ = registry.send(Command::Ended(run_id, Box::new(ended))).await;
        }
    }
}

/// The settings `run`'s report keeps, explained. The host's default of
/// `run_state.key_reference` shows only once the Run holds its data key,
/// when the host has answered it; before, the catalog's shows, since
/// asking for it here would open the OS keychain (issue 2204).
fn kept_settings(run: &Run) -> Option<Vec<ExplainedSetting>> {
    let mut settings = run.settings()?.resolved().explain(&[], Vec::new()).settings;
    if run.shared.key.get().is_some()
        && let Some(known) = run.runs.engine.settings().known_host_defaults()
        && let Some(default) = settings.iter_mut().find(|explained| {
            explained.key == RUN_STATE_KEY_REFERENCE.name
                && explained.layer == SettingsLayer::Default
        })
    {
        default.value = known.of(&RUN_STATE_KEY_REFERENCE);
    }
    Some(settings)
}

/// The Secret reference of the key-encryption key that a layer of
/// `settings` set, and that layer; none where the default stands.
fn layer_set_key_reference(
    settings: Option<&RunSettings>,
) -> Option<(SecretReference, SettingsLayer)> {
    let resolved = settings?.resolved();
    let layer = resolved.effective(&RUN_STATE_KEY_REFERENCE)?.layer;
    (layer != SettingsLayer::Default).then_some(())?;
    Some((
        resolved.get::<Option<SecretReference>>(&RUN_STATE_KEY_REFERENCE)?,
        layer,
    ))
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
    /// What reads the filesystem of a stop its run file could not record.
    governor: Governor,
}

impl Closing {
    /// Stops the Run's tasks, gives back its Slices, writes its end into
    /// run state with its report, assembled there (ADR 0102), as its
    /// writer's last write, a stop's Checkpoint after it, waits for its
    /// writer and frees its lock. An end whose commit meets a full disk or
    /// an I/O error is unrecorded (ADR 0122).
    ///
    /// The end waits for the Run's tasks and for the writes queued before
    /// its record, never for the handles on the Run left anywhere else: its
    /// record closes the writer, which then refuses each of their later
    /// writes as `Closed`, so none can keep the Run from ending or reach
    /// the run file after its end (#1763, #1736).
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
            governor,
        } = self;
        run.owner.stop().await;
        let (stop_reason, error, finished) = match end {
            End::Finished(reasons) => (None, None, reasons),
            End::Failed(error) => (None, Some(error), Vec::new()),
            End::Stopped(reason) => (Some(reason), None, Vec::new()),
        };
        let limits = given_back(&run, &writer).await;
        exceptions_used(&run, clock).await;
        let facts = EndFacts {
            run_id: run.id.clone(),
            lifecycle,
            stop_reason,
            started_at,
            ended_at: clock.point_in_time(),
            policy_version,
            weakened,
            settings: kept_settings(&run),
            checkpoint: None,
            limits,
            error,
            finished,
        };
        let state = run.state.clone();
        let (run_id, folder) = (run.id.clone(), run.folder.clone());
        // The Run's values stay in the engine's masks until its report,
        // which its error and warnings may hold, is masked and kept.
        let mask = run.secrets().mask().clone();
        drop(run);
        let bare = report::masked(facts.bare(), mask.masks());
        let ended = state
            .write_last(move |connection| {
                let report =
                    report::masked(report::assemble(connection, &facts, &parts)?, mask.masks());
                write_end(connection, &report)?;
                Ok(report)
            })
            .await;
        let written = ended.is_ok();
        // A report its run file holds, or the bare one, the only one there is.
        let (mut report, kept): (_, fn(Box<RunReport>) -> Ended) = match ended {
            Ok(report) => (report, Ended::Report),
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
                (bare, Ended::Unwritten)
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
        kept(Box::new(report))
    }
}

mod clean_up;
mod discard;
mod ended;
mod folder;
mod import;
pub mod lifecycle;
mod list;
pub mod outcome;
mod progress;
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

/// How a Run's `settings` write its run file: `run_state.durability`'s
/// preset as the resolver unfolded it into the `run_state.batch.*` keys,
/// with `run_state.commit_interval` at `sync: interval` (ADR 0142); `batch`
/// when it has none.
fn durability_of(settings: Option<&RunSettings>) -> Durability {
    settings.map_or_else(Durability::default, |settings| {
        let resolved = settings.resolved();
        let interval = resolved.get(&RUN_STATE_COMMIT_INTERVAL);
        Durability {
            per_attempt: resolved.get(&RUN_STATE_BATCH_PER_ATTEMPT),
            pages: resolved.get(&RUN_STATE_BATCH_PAGES),
            interval: resolved.get(&RUN_STATE_BATCH_INTERVAL),
            bytes: resolved.get(&RUN_STATE_BATCH_BYTES),
            sync: BatchSync::named(resolved.get(&RUN_STATE_BATCH_SYNC), interval).expect(
                "invariant: the settings resolver lets only `run_state.batch.sync`'s choices through",
            ),
        }
    })
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
