//! A Run's own SQLite file (ADRs 0002, 0003 and 0060): bundled SQLite in WAL
//! mode, `synchronous=FULL` with `fullfsync` on macOS, so an acknowledged
//! commit survives a power cut, and one writer, an actor of the run's owner
//! that commits every write its mailbox holds in one transaction. Each
//! commit is accepted only under the writer's run epoch: a writer whose
//! epoch the file no longer holds commits nothing. A commit that meets a full
//! disk or an I/O error (`SQLITE_FULL`, `SQLITE_IOERR`, and the
//! `SQLITE_CORRUPT` SQLite reports for a read that met EIO) at any of its
//! writes is rolled back with `ROLLBACK`, as SQLite advises, so the file is
//! the same after any of them; the writes queued after the failing one never
//! run, since SQLite may have ended the transaction already, and nothing of
//! the commit is acknowledged, and the writer
//! says so to whoever watches its storage failures, which stop its Run
//! (ADR 0122). The file opens through a
//! VFS named by a value, none in production, the crash VFS in tests. Each
//! acknowledged commit moves a counter on, which a reader of the file's
//! committed state waits on for more, never holding the writer back; its
//! reads run beside the writer, on a bounded group of read-only
//! connections it keeps ([`readers`], ADR 0138). The writer stops once every handle on it is gone, or once its last write
//! commits, as a Run's end writes its record: whatever any handle queues
//! after that is refused (#1763). Beside those writes it takes Run-state
//! batches, rows memory already decided ([`batch`], ADR 0142).
//!
//! The file holds its format's `major.minor` (ADR 0066), the Run's own row
//! and tags, and the tables each contribution of the composition adds, such
//! as a crawl's work items.
//!
//! Its settings keep a page's writes small (ADR 0138): savepoint journals in
//! memory (`temp_store=MEMORY`), `journal_position` written once per group
//! commit, and no checkpoint on the writer: its connection runs none of its
//! own, and a checkpointer, an actor of the same owner on a connection of its
//! own, runs a `PASSIVE` checkpoint once the WAL passes
//! [`WAL_CHECKPOINT_AFTER_BYTES`], which never takes the writer's lock. No
//! connection of the writer's copies the WAL into the file as it closes: a
//! clean stop's last write syncs the WAL, which stays beside the file
//! (#2117).
//!
//! Its own syncs fold into the commits it acknowledges (#2117): a new
//! file's opening commits its tables unsynced below `off`, and the first
//! synced commit covers them; that commit also declares the bound of the
//! charges its deferred commits may hold ([`DEFERRED_CHARGES`]).
//!
//! Size: one concern, the run file's writer on its one connection: how it
//! opens, queues jobs and commits each group, which read side by side.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::{Compatibility, RUN_STATE};
use goodfetch_protocol::run::{
    DURABILITY_FAST, DURABILITY_FULL, DURABILITY_OFF, DURABILITY_PERIODIC,
};
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;
use crate::metrics::Metrics;
use crate::sqlite_store::{
    self, Access, Counting, File, Folder, FormatRefusal, FullFsync, OpenError, Opening, Schema,
    Synchronous, Wait, WalOnClose,
};
use crate::tasks::TaskOwner;
use wal_checkpointer::{WalCheckpointer, note_wal_pages, open_wal_checkpointer};

mod batch;
pub mod install;
mod readers;
pub mod run_row;
mod wal_checkpointer;

pub use batch::{BatchRows, BatchStatement, BatchWriter, SealRequest, SealedBatch, Seals};
pub(crate) use readers::RunStateReaders;

/// How many writes wait for the writer before their callers wait.
const MAILBOX: usize = 1024;

/// How long the writer's switch of a new file to WAL waits while another
/// connection holds the file.
const SWITCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The statements the writer's connection keeps prepared: more than every
/// statement a crawl runs on its run file, the engine's and each
/// contribution's (about 200 in all: the Frontier's admission and
/// `rules::learn`, the Lease, the outcome, the charge), so its cache never
/// evicts one a later page runs again (ADR 0138). rusqlite's default, 16,
/// holds fewer than one enqueue with a parent and `learn` run.
const STATEMENTS: usize = 256;

/// A Run's state's file in its run folder.
pub(crate) use goodfetch_protocol::home::RUN_FILE;

/// The engine's own tables, which every run file holds: the Run's own row
/// in `run`, which other crates read through [`run_row`].
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS run (
        run_id TEXT NOT NULL PRIMARY KEY,
        kind TEXT NOT NULL,
        needs TEXT NOT NULL,
        caller TEXT NOT NULL,
        epoch INTEGER NOT NULL,
        inputs TEXT NOT NULL,
        lifecycle TEXT NOT NULL,
        stop_reason TEXT,
        verdict TEXT,
        completeness TEXT,
        started_at TEXT NOT NULL,
        ended_at TEXT,
        policy_version TEXT,
        weakened TEXT NOT NULL DEFAULT '[]',
        journal_position INTEGER NOT NULL DEFAULT 0,
        binding TEXT,
        clean_up_digest TEXT,
        uncapped INTEGER NOT NULL DEFAULT 0,
        progress_message TEXT,
        nothing_done TEXT,
        report TEXT,
        deferred_charges INTEGER NOT NULL DEFAULT 0
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT",
    // The "unclean" mark of a run file at `off` (ADR 0138): set in a synced
    // commit as its writer opens it, cleared by its clean stop, so a file
    // that still holds it after a crash is discarded rather than read.
    "CREATE TABLE IF NOT EXISTS unclean (id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1)) STRICT",
    // What the Run used of each Slice of the Usage ledger, by run epoch (ADR
    // 0041), which an engine reclaiming its Slices charges.
    "CREATE TABLE IF NOT EXISTS usage (
        slice_id INTEGER NOT NULL,
        epoch INTEGER NOT NULL,
        used INTEGER NOT NULL,
        PRIMARY KEY (slice_id, epoch)
    ) STRICT",
    // What the Run used of each Usage kind, by currency ('' for a count),
    // which its report and `run.usage` give (ADR 0101).
    "CREATE TABLE IF NOT EXISTS usage_totals (
        usage_kind TEXT NOT NULL,
        currency TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, currency)
    ) STRICT",
    // What the Run used of each part of a Usage kind, such as the proxy
    // kinds of `proxy_bytes` (ADR 0021), which its report and `run.usage`
    // give beside the kind's total.
    "CREATE TABLE IF NOT EXISTS usage_parts (
        usage_kind TEXT NOT NULL,
        part TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, part)
    ) STRICT",
    // The Run's data key, wrapped by its key-encryption key, once it has
    // one (ADR 0015).
    crate::run_key::TABLE,
    // The warnings the Run's owners handed over, in order (ADR 0102).
    "CREATE TABLE IF NOT EXISTS warnings (
        position INTEGER NOT NULL PRIMARY KEY,
        code TEXT NOT NULL,
        message TEXT NOT NULL,
        sessions TEXT NOT NULL,
        folder TEXT
    ) STRICT",
];

/// The statement that sets the "unclean" mark of a run file at `off` (ADR
/// 0138), in the commit its writer opens it with.
const MARK: &[&str] = &["INSERT INTO unclean (id) VALUES (1) ON CONFLICT (id) DO NOTHING"];

/// The tables one contribution adds to every run file: `CREATE TABLE IF NOT
/// EXISTS` and `CREATE INDEX IF NOT EXISTS` statements, run in order when a
/// file opens.
#[derive(Clone, Copy, Debug)]
pub struct RunStateTables {
    /// Who adds them.
    pub name: &'static str,
    /// Their statements.
    pub statements: &'static [&'static str],
}

/// The run file writer's page cache, in KiB: a bounded cache over what the
/// file keeps on disk, such as the Frontier's exact visited set, sized so
/// a Frontier of 100,000 URLs keeps under ADR 0134's 8 heap bytes per URL
/// (SQLite's default of 2,000 KiB alone is 20).
pub const PAGE_CACHE_KIB: u32 = 512;

/// The page cache, in KiB, of the writer's second connection at `full`,
/// which commits the groups that need no sync (ADR 0138's note of #1926):
/// small, as a claim reads few pages, so both caches together keep under
/// ADR 0134's 8 heap bytes per URL at 100,000 URLs.
pub const DEFERRING_PAGE_CACHE_KIB: u32 = 128;

/// The most of one Slice of the Usage ledger (ADR 0041) that the writes a
/// Run committed without a sync since its last synced commit may charge,
/// at `full` (ADR 0138's note of #1926): a power cut can take them, so a
/// reclaim after an unclean end charges this much more than the journal
/// holds. The first deferred write that charges declares it in a synced
/// commit, and a group that would pass it syncs.
pub const DEFERRED_CHARGES: u64 = 64;

/// How many bytes of WAL the writer lets grow before it asks its
/// checkpointer for a `PASSIVE` checkpoint (ADR 0138). Past twice as many
/// the writer tries to start the WAL again itself, and a WAL that started
/// again is truncated to this many (#2323): without a reader holding an
/// old snapshot, a run file's WAL stays under twice this and one commit.
pub const WAL_CHECKPOINT_AFTER_BYTES: u64 = 16 * 1024 * 1024;

/// How a run file opens.
#[derive(Clone, Debug)]
pub struct RunStateOptions {
    /// The name of the SQLite VFS it opens through: none in production, the
    /// crash VFS in tests (ADR 0060), and the install database's in the
    /// per-install layout ([`install`]).
    pub vfs: Option<String>,
    /// How its commits reach the disk (`run_state.durability`).
    pub durability: Durability,
    /// Where it counts its transactions and its writer's reads (ADR 0140).
    pub work: CountedWork,
    /// How many bytes of WAL its writer lets grow before its checkpointer
    /// runs a `PASSIVE` checkpoint: [`WAL_CHECKPOINT_AFTER_BYTES`], lower in
    /// tests.
    pub wal_checkpoint_after: u64,
    /// Where its writer reads the time at `periodic`, at each group and for
    /// the timer of an idle writer (ADR 0059).
    pub clock: Clock,
    /// Where it reports its WAL's size and the checkpoints a reader
    /// blocked: none by default.
    pub metrics: Option<Metrics>,
}

impl Default for RunStateOptions {
    fn default() -> Self {
        Self {
            vfs: None,
            durability: Durability::default(),
            work: CountedWork::default(),
            wal_checkpoint_after: WAL_CHECKPOINT_AFTER_BYTES,
            clock: Clock::system(),
            metrics: None,
        }
    }
}

/// How a run file's commits reach the disk, its Durability level
/// (`run_state.durability`, ADRs 0002 and 0138).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Durability {
    /// An acknowledged commit survives a power cut: `synchronous=FULL`,
    /// with `fullfsync` on macOS, for every group with a write that needs
    /// it.
    #[default]
    Full,
    /// An acknowledged commit survives a process crash only:
    /// `synchronous=NORMAL`, whose commits in WAL mode an OS crash or a
    /// power cut may roll back, without damaging the file.
    Fast,
    /// As `fast`, with one synced group once `interval` passed since the
    /// last one, and one for an idle writer's commits once it passed: a
    /// power cut takes at most `interval` of acknowledged commits.
    Periodic {
        /// `run_state.commit_interval`.
        interval: std::time::Duration,
    },
    /// Nothing synced (`synchronous=OFF`) but the file's "unclean" mark,
    /// set as its writer opens it and cleared by its clean stop: a crash
    /// leaves the mark, so [`unclean`] says the file is to be discarded.
    Off,
}

impl Durability {
    /// The Durability level `run_state.durability`'s value `level` names,
    /// `periodic` at `commit_interval`; `None` for a value that names none,
    /// which the settings resolver never lets through.
    pub fn named(level: &str, commit_interval: std::time::Duration) -> Option<Self> {
        match level {
            DURABILITY_FULL => Some(Self::Full),
            DURABILITY_FAST => Some(Self::Fast),
            DURABILITY_PERIODIC => Some(Self::Periodic {
                interval: commit_interval,
            }),
            DURABILITY_OFF => Some(Self::Off),
            _ => None,
        }
    }

    /// The `synchronous` level of the writer's connection, which commits
    /// the groups that sync at `full` and `periodic`.
    fn synchronous(self) -> Synchronous {
        match self {
            Self::Full | Self::Periodic { .. } => Synchronous::Full,
            Self::Fast => Synchronous::Normal,
            Self::Off => Synchronous::Off,
        }
    }

    /// The `synchronous` pragma of its checkpointer's connection: synced
    /// below `full` too, so a checkpoint's pages reach the disk before the
    /// WAL frames they came from are written over. A clean stop runs no
    /// checkpoint (#2117): the file reads whole with the `-wal` beside it.
    fn checkpoint_synchronous(self) -> Synchronous {
        match self {
            Self::Full => Synchronous::Full,
            Self::Fast | Self::Periodic { .. } | Self::Off => Synchronous::Normal,
        }
    }

    /// Whether the writer keeps a second connection at
    /// `synchronous=NORMAL` for the groups that do not sync.
    fn defers(self) -> bool {
        matches!(self, Self::Full | Self::Periodic { .. })
    }
}

/// How a connection of the writer's commits at the `synchronous` level
/// `synchronous`: the commit that opens its file at `opening`, through
/// `F_FULLFSYNC` on macOS, its WAL left beside the file as it closes
/// (#2117).
fn connection_durability(
    synchronous: Synchronous,
    opening: Synchronous,
) -> sqlite_store::Durability {
    sqlite_store::Durability {
        synchronous,
        opening,
        full_fsync: FullFsync::CommitsAndCheckpoints,
        wal_on_close: WalOnClose::Kept,
    }
}

/// Whether a write's commit must reach the disk before it is answered
/// (ADR 0138's note of #1926). The writer syncs a group at `full` when any
/// of its writes needs it, so a write that does not becomes durable with
/// the next group that syncs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Syncing {
    /// Synced before it is answered at `full`, as every write ADR 0002
    /// acknowledges: the default.
    #[default]
    Required,
    /// As `Required`, its group also declaring [`DEFERRED_CHARGES`] at
    /// `full` when the file does not yet: the start of a Run whose claims
    /// defer their charges, so that no claim's group syncs to declare it
    /// (#2117).
    Declaring,
    /// Committed without a sync of its own at `full`: a power cut before
    /// the next synced commit may take it, as it may an attempt's claim.
    Deferred {
        /// The most it charges of any one Slice of the Usage ledger, which
        /// counts against [`DEFERRED_CHARGES`].
        charged: u64,
    },
}

/// Why a run file did not open, or a write did not commit.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RunStateError {
    /// SQLite failed; the write committed nothing.
    #[error("run state at `{path}`: {message}")]
    Sqlite {
        /// The run file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The file holds another run epoch than the writer's: a newer writer
    /// took the Run, so this one commits nothing (ADR 0003).
    #[error("the run file holds run epoch {found}, and this writer's is {held}")]
    Fenced {
        /// The writer's run epoch.
        held: u64,
        /// The file's.
        found: u64,
    },
    /// The file is of a format version this engine cannot read (ADR 0066),
    /// or a shared store's lowest minor is newer than this engine (ADR
    /// 0098).
    #[error("the file is {format} {found}, which this engine cannot read")]
    Format {
        /// The format's name.
        format: &'static str,
        /// The file's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
        /// The Product version to install, when a shared store's lowest
        /// minor refused this engine.
        install: Option<String>,
    },
    /// The disk was full or failed an I/O at one of the commit's writes or at
    /// the commit itself (ADR 0122): it was rolled back, and nothing of it is
    /// acknowledged.
    #[error("run state at `{path}` could not be written to its storage: {message}")]
    Storage {
        /// The run file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The writer has stopped.
    #[error("the run file's writer has stopped")]
    Closed,
}

impl RunStateError {
    /// The error of a shared store's refusal (ADR 0098).
    pub(crate) fn refused(refusal: crate::lowest_minor::Refusal) -> Self {
        Self::Format {
            format: refusal.format,
            found: refusal.found.to_string(),
            compatibility: refusal.compatibility,
            install: refusal.install,
        }
    }
}

type Completion = Box<dyn FnOnce(Result<(), RunStateError>) + Send>;
/// A write: given the transaction, it runs its work and answers how to
/// report it once the commit ends, and the failure that ends its group when
/// its work met a full disk or an I/O error or left the transaction in a
/// state the writer cannot know; given none, it reports the failure only.
type QueuedWrite =
    Box<dyn FnOnce(Option<(&Connection, &Path)>) -> (Completion, Option<RunStateError>) + Send>;

/// What a job in the writer's queue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// A write.
    Write,
    /// The writer's last write.
    Last,
    /// The idle writer's timer at `periodic`: a group that syncs what the
    /// writer committed unsynced, if anything. It writes nothing itself.
    Tick,
}

/// A job in the writer's queue, and what it is.
struct Queued {
    write: QueuedWrite,
    job: Job,
    syncing: Syncing,
}

impl Queued {
    /// A tick of the idle writer's timer.
    fn tick() -> Self {
        Self {
            write: Box::new(|_| (Box::new(|_| {}), None)),
            job: Job::Tick,
            syncing: Syncing::Deferred { charged: 0 },
        }
    }
}

/// `work` as a write the writer's actor runs in its next commit, and the
/// answer it sends once that commit ends.
fn queued<T, F>(work: F) -> (QueuedWrite, oneshot::Receiver<Result<T, RunStateError>>)
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
{
    queued_holding(work, ())
}

/// As [`queued`], the write holding `held` until its commit ended and its
/// answer is sent, or until the writer drops it unrun.
fn queued_holding<T, F, H>(
    work: F,
    held: H,
) -> (QueuedWrite, oneshot::Receiver<Result<T, RunStateError>>)
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    H: Send + 'static,
{
    let (answer, answered) = oneshot::channel();
    let job: QueuedWrite = Box::new(move |transaction: Option<(&Connection, &Path)>| {
        let mut ends_group = None;
        let done = match transaction {
            Some((connection, path)) => {
                run_in_savepoint(connection, work).map_err(|Failed { error, ended }| {
                    if ended {
                        ends_group = Some(commit_error(path, &error));
                    }
                    sqlite_error(path, &error)
                })
            }
            None => Err(RunStateError::Closed),
        };
        let completion: Completion = Box::new(move |committed: Result<(), RunStateError>| {
            let _ = answer.send(committed.and(done));
            drop(held);
        });
        (completion, ends_group)
    });
    (job, answered)
}

/// A place [`RunState::reserve`] took in the writer's queue; dropped
/// unused, it frees the place.
pub struct ReservedWrite(mpsc::OwnedPermit<Queued>);

impl std::fmt::Debug for ReservedWrite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReservedWrite")
            .finish_non_exhaustive()
    }
}

impl ReservedWrite {
    /// Queues `work` in the place, at once, as [`RunState::write`] would
    /// have queued it, and answers the future of what it returns once its
    /// commit is acknowledged, with [`RunState::write`]'s errors: `Closed`
    /// too once the writer stopped without running it, after its last
    /// write. Dropping that future does not withdraw the write.
    pub fn write<T, F>(
        self,
        work: F,
    ) -> impl Future<Output = Result<T, RunStateError>> + Send + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.write_with(Syncing::Required, work)
    }

    /// As [`ReservedWrite::write`], its commit synced as `syncing` says.
    pub fn write_with<T, F>(
        self,
        syncing: Syncing,
        work: F,
    ) -> impl Future<Output = Result<T, RunStateError>> + Send + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let (write, answered) = queued(work);
        let mailbox = self.0.send(Queued {
            write,
            job: Job::Write,
            syncing,
        });
        async move {
            // A write the writer ran answers before its mailbox goes; one
            // sent once it went waits in no mailbox.
            tokio::select! {
                biased;
                answered = answered => answered.map_err(|_| RunStateError::Closed)?,
                () = mailbox.closed() => Err(RunStateError::Closed),
            }
        }
    }
}

/// The writer of one run file: a handle to its actor, which any task may
/// clone and send writes to.
#[derive(Clone, Debug)]
pub struct RunState {
    mailbox: mpsc::Sender<Queued>,
    path: PathBuf,
    epoch: u64,
    watched: std::sync::Arc<Watched>,
    readers: std::sync::Arc<RunStateReaders>,
    batches: std::sync::Arc<batch::Batches>,
}

/// What a writer's watchers follow: its acknowledged commits and its first
/// storage failure.
#[derive(Debug)]
struct Watched {
    commits: watch::Receiver<u64>,
    failures: watch::Receiver<Option<String>>,
}

impl RunState {
    /// Opens the run file at `path` as its writer under run epoch `epoch`,
    /// creating it and the tables of `tables` when it does not exist, its
    /// actor one of `owner`'s. The writer stops once every handle is
    /// dropped, or once [`RunState::write_last`] committed, and `owner`
    /// waits for it.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the file cannot be opened or its tables created,
    /// `Format` when it is of a format this engine cannot read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the file opened closes it again.
    pub async fn open(
        path: PathBuf,
        options: &RunStateOptions,
        tables: Vec<RunStateTables>,
        epoch: u64,
        owner: &TaskOwner,
    ) -> Result<Self, RunStateError> {
        let opened = options.clone();
        let opened_path = path.clone();
        let (connection, deferring, declared, checkpointing) = owner
            .blocking(move || open_connections(&opened_path, &opened, &tables))
            .await?;
        let mut checkpointer = WalCheckpointer::start(&path, options, checkpointing, owner)
            .map_err(|error| sqlite_error(&path, &error))?;
        let (mailbox, jobs) = mpsc::channel(MAILBOX);
        let actor_path = path.clone();
        let (committed, commits) = watch::channel(0_u64);
        let (failed, failures) = watch::channel(None);
        let counted = options.work.clone();
        let readers = std::sync::Arc::new(RunStateReaders::new(
            path.clone(),
            options.vfs.clone(),
            owner.blocking_threads(),
        ));
        let mut deferral = Deferral {
            deferring: options.durability == Durability::Full,
            declared,
            uncovered: 0,
        };
        let mut interval = Interval::of(options);
        let marked = options.durability == Durability::Off;
        let durability = options.durability;
        let connection = WriterConnection {
            connection,
            deferring,
            readers: std::sync::Arc::clone(&readers),
        };
        // The idle writer's timer at `periodic`, which ends with the writer.
        let writer_gone = idle_timer(owner, options, &mailbox);
        owner
            .actor_until("run-state-writer", jobs, move |mut group: Vec<Queued>| {
                let _held_until_the_writer_ends = &writer_gone;
                let signals = Signals {
                    commits: &committed,
                    failures: &failed,
                    work: &counted,
                };
                let ticked = without_ticks(&mut group);
                let last = group.iter().position(|queued| queued.job == Job::Last);
                let refused = last.map_or_else(Vec::new, |at| group.split_off(at + 1));
                let planned = match &interval {
                    Some(interval) => interval.plan(ticked, last.is_some()),
                    None => deferral.plan(&group),
                };
                // A tick with nothing committed unsynced commits nothing.
                let idle =
                    group.is_empty() && !interval.as_ref().is_some_and(|interval| interval.owed);
                // A clean stop commits synced at every level, `fast`'s and
                // `off`'s included, and `off`'s clears the mark: its sync
                // covers the WAL, which no checkpoint copies then (#2117).
                let synced_stop = last.is_some()
                    && matches!(durability, Durability::Fast | Durability::Off)
                    && connection
                        .connection
                        .pragma_update(None, "synchronous", "FULL")
                        .is_ok();
                let clean = marked && synced_stop;
                let group: Vec<QueuedWrite> =
                    group.into_iter().map(|queued| queued.write).collect();
                let committed = if idle {
                    None
                } else {
                    let on = connection.for_group(planned.synced);
                    let marks = Marks {
                        declare: planned.declare,
                        clean,
                    };
                    commit_group(on, &actor_path, (epoch, marks), group, &signals)
                };
                if let Some(declared) = committed {
                    deferral.committed(planned, declared);
                    if let Some(interval) = &mut interval {
                        interval.committed(planned.synced);
                    }
                    checkpointer.committed(&connection.connection, &actor_path, last.is_some());
                }
                for queued in refused {
                    (queued.write)(None).0(Err(RunStateError::Closed));
                }
                if last.is_some() {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .map_err(|error| sqlite_error(&path, &error))?;
        Ok(Self {
            mailbox,
            path,
            epoch,
            watched: std::sync::Arc::new(Watched { commits, failures }),
            readers,
            batches: std::sync::Arc::new(batch::Batches::new()),
        })
    }

    /// The first commit that met a full disk or an I/O error, as SQLite
    /// named it, once one did (ADR 0122): none until then.
    pub fn storage_failures(&self) -> watch::Receiver<Option<String>> {
        self.watched.failures.clone()
    }

    /// The run file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The run epoch it writes under, which each of its commits checks.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// How many commits it acknowledged, which moves on once each is in the
    /// file and before its writes are answered: a reader of the file waits
    /// on it for more committed state. Its sender is gone once the writer
    /// stops.
    pub fn commits(&self) -> watch::Receiver<u64> {
        self.watched.commits.clone()
    }

    /// Runs `work` in the writer's next commit, grouped with every write
    /// queued beside it, and answers what it returned once that commit is
    /// acknowledged. An error of `work` rolls back its own changes only.
    ///
    /// # Errors
    ///
    /// `Sqlite` when `work` or the commit failed, `Fenced` when the file
    /// holds another run epoch, `Closed` when the writer has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a write already queued: it may
    /// still commit.
    pub async fn write<T, F>(&self, work: F) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.queue(work, Job::Write, Syncing::Required).await
    }

    /// Runs `work` as [`RunState::write`] does, its commit synced as
    /// `syncing` says: a deferred write is answered once its commit is in
    /// the file, before a sync, at `full` too.
    ///
    /// # Errors
    ///
    /// As [`RunState::write`].
    ///
    /// # Cancel safety
    ///
    /// As [`RunState::write`]'s.
    pub async fn write_with<T, F>(&self, syncing: Syncing, work: F) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.queue(work, Job::Write, syncing).await
    }

    /// Runs `work` as [`RunState::write_with`] does, its commit deferred
    /// and charging nothing of the Usage ledger: for a write whose answer
    /// promises nothing outside, which the next synced commit covers.
    ///
    /// # Errors
    ///
    /// As [`RunState::write`].
    ///
    /// # Cancel safety
    ///
    /// As [`RunState::write`]'s.
    pub async fn write_deferred<T, F>(&self, work: F) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.write_with(Syncing::Deferred { charged: 0 }, work)
            .await
    }

    /// Runs `work` as [`RunState::write`] does, as the writer's last write:
    /// once its commit ends, the writer stops, though other handles still
    /// hold it, and every write queued after it, through any handle,
    /// answers `Closed`, as does every later one. A Run's end writes its
    /// record so, which no handle left outside its tasks can keep waiting
    /// or write after (#1763).
    ///
    /// # Errors
    ///
    /// As [`RunState::write`].
    ///
    /// # Cancel safety
    ///
    /// As [`RunState::write`]'s: once queued, the writer stops after it
    /// whether or not its answer is awaited.
    pub async fn write_last<T, F>(&self, work: F) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.queue(work, Job::Last, Syncing::Required).await
    }

    /// Queues `work` as `job`, and answers what it returned once its commit
    /// is acknowledged.
    async fn queue<T, F>(&self, work: F, job: Job, syncing: Syncing) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let (write, answered) = queued(work);
        self.mailbox
            .send(Queued {
                write,
                job,
                syncing,
            })
            .await
            .map_err(|_| RunStateError::Closed)?;
        answered.await.map_err(|_| RunStateError::Closed)?
    }

    /// A place in the writer's queue, for a write the caller queues once it
    /// knows it: without awaiting, so in order with a change it makes under
    /// a lock, such as a Run's charging and its journal.
    ///
    /// # Errors
    ///
    /// `Closed` when the writer has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future takes no place.
    pub async fn reserve(&self) -> Result<ReservedWrite, RunStateError> {
        self.mailbox
            .clone()
            .reserve_owned()
            .await
            .map(ReservedWrite)
            .map_err(|_| RunStateError::Closed)
    }

    /// Runs `work` in one read transaction of the file's committed state,
    /// on one of its readers' connections, beside the writer: it never
    /// waits for a commit, and sees every write acknowledged before it
    /// began, as a write is answered once its commit is in the file.
    ///
    /// # Errors
    ///
    /// `Sqlite` when `work` failed or the file could not be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn read<T, F>(&self, work: F) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.readers
            .read(move |connection| readers::in_one_transaction(connection, work))
            .await
            .map_err(|error| sqlite_error(&self.path, &error))
    }

    /// Its readers, which read the file as [`RunState::read`] does, for a
    /// reader of the Run that must not hold its writer open.
    pub(crate) fn readers(&self) -> std::sync::Arc<RunStateReaders> {
        std::sync::Arc::clone(&self.readers)
    }
}

/// The writer's connections, which close their readers' first, and leave
/// the WAL beside the file as they close (#2117). At `full` it has two (ADR 0138's note of #1926): `connection`
/// commits with `synchronous=FULL`, and `deferring` with `NORMAL` commits
/// the groups that need no sync, as SQLite sets a connection's sync level
/// only as it prepares `PRAGMA synchronous`, which a group's switch would
/// then prepare again each time.
struct WriterConnection {
    connection: Connection,
    deferring: Option<Connection>,
    readers: std::sync::Arc<RunStateReaders>,
}

impl WriterConnection {
    /// The connection a group commits on: the synced one when it syncs.
    fn for_group(&self, synced: bool) -> &Connection {
        match &self.deferring {
            Some(deferring) if !synced => deferring,
            _ => &self.connection,
        }
    }
}

/// What the writer knows of the charges its unsynced commits hold, against
/// [`DEFERRED_CHARGES`].
struct Deferral {
    /// Whether it defers at all: at `full` only, as `fast` syncs no group.
    deferring: bool,
    /// Whether the file declares [`DEFERRED_CHARGES`] in a synced commit.
    declared: bool,
    /// What its commits since the last synced one charged.
    uncovered: u64,
}

/// How the writer commits one group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Planned {
    /// On the synced connection.
    synced: bool,
    /// Declaring [`DEFERRED_CHARGES`] in the file as it commits.
    declare: bool,
    /// What its deferred writes charge.
    charged: u64,
}

impl Deferral {
    /// How `group` commits: synced when any of its writes needs it, and
    /// declaring the bound when one of them asks to; synced too when it
    /// charges before the file declares the bound, which it then declares,
    /// or when it would pass the bound; else on the deferring connection.
    fn plan(&self, group: &[Queued]) -> Planned {
        let mut required = false;
        let mut declaring = false;
        let mut charged = 0_u64;
        for queued in group {
            match queued.syncing {
                Syncing::Required => required = true,
                Syncing::Declaring => {
                    required = true;
                    declaring = true;
                }
                Syncing::Deferred { charged: more } => charged = charged.saturating_add(more),
            }
        }
        let synced = |declare| Planned {
            synced: true,
            declare,
            charged,
        };
        if !self.deferring {
            synced(false)
        } else if required {
            synced(declaring && !self.declared)
        } else if charged == 0 {
            Planned {
                synced: false,
                declare: false,
                charged,
            }
        } else if !self.declared {
            synced(true)
        } else if self.uncovered.saturating_add(charged) > DEFERRED_CHARGES {
            synced(false)
        } else {
            Planned {
                synced: false,
                declare: false,
                charged,
            }
        }
    }

    /// Notes that a group planned as `planned` committed, the bound
    /// `declared` in its commit or not.
    fn committed(&mut self, planned: Planned, declared: bool) {
        self.declared |= declared;
        self.uncovered = if planned.synced {
            0
        } else {
            self.uncovered.saturating_add(planned.charged)
        };
    }
}

/// When the writer syncs a group at `periodic` (ADR 0138): once `every`
/// passed since its last synced commit, read on its Clock at each group,
/// and at a tick of its idle timer while a commit is unsynced.
struct Interval {
    every: std::time::Duration,
    clock: Clock,
    /// When it last committed synced.
    synced_at: tokio::time::Instant,
    /// Whether a commit since then is unsynced.
    owed: bool,
}

impl Interval {
    /// The writer's interval at `options`' Durability level: `periodic`'s
    /// only.
    fn of(options: &RunStateOptions) -> Option<Self> {
        match options.durability {
            Durability::Periodic { interval } => Some(Self {
                every: interval,
                clock: options.clock,
                synced_at: options.clock.now(),
                owed: false,
            }),
            Durability::Full | Durability::Fast | Durability::Off => None,
        }
    }

    /// How a group commits: synced once the interval passed, at a tick
    /// that finds a commit unsynced, and for the writer's `last` write,
    /// else on the deferring connection.
    fn plan(&self, ticked: bool, last: bool) -> Planned {
        let due = self.clock.now() >= self.synced_at + self.every;
        Planned {
            synced: due || last || (ticked && self.owed),
            declare: false,
            charged: 0,
        }
    }

    /// Notes that a group committed, `synced` or not.
    fn committed(&mut self, synced: bool) {
        if synced {
            self.synced_at = self.clock.now();
            self.owed = false;
        } else {
            self.owed = true;
        }
    }
}

/// Starts the idle writer's timer at `options`' Durability level when it
/// is `periodic`, one of `owner`'s tasks: every interval on its Clock, it
/// queues a tick into `mailbox`, so a commit no later group syncs is synced
/// within the interval (ADR 0138). It holds no handle on the writer, and
/// ends once the answered sender is dropped, the writer having ended, or
/// once `owner` is cancelled.
fn idle_timer(
    owner: &TaskOwner,
    options: &RunStateOptions,
    mailbox: &mpsc::Sender<Queued>,
) -> oneshot::Sender<()> {
    let (writer_gone, mut timer_stops) = oneshot::channel::<()>();
    let Durability::Periodic { interval } = options.durability else {
        return writer_gone;
    };
    if interval.is_zero() {
        return writer_gone;
    }
    let (clock, mailbox) = (options.clock, mailbox.downgrade());
    let cancelled = owner.token().clone();
    owner.spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut timer_stops => break,
                () = cancelled.cancelled() => break,
                () = clock.sleep(interval) => {}
            }
            let Some(mailbox) = mailbox.upgrade() else {
                break;
            };
            // A full mailbox holds a group already, which plans its sync.
            let _ = mailbox.try_send(Queued::tick());
        }
    });
    writer_gone
}

/// Opens the writer's connections on the run file at `path`: its own, the
/// deferring one at the levels that defer, whether the file declares
/// [`DEFERRED_CHARGES`], and its checkpointer's. It blocks.
fn open_connections(
    path: &Path,
    options: &RunStateOptions,
    tables: &[RunStateTables],
) -> Result<(Connection, Option<Connection>, bool, Connection), RunStateError> {
    let connection = open_file(path, options, tables)?;
    // It replaces SQLite's automatic checkpoint, which this connection then
    // never runs (ADR 0138).
    connection.wal_hook(Some(note_wal_pages));
    let deferring = options
        .durability
        .defers()
        .then(|| open_deferring(path, options))
        .transpose()?;
    let declared = declared_deferral(&connection).map_err(|error| sqlite_error(path, &error))?;
    // Opened beside the writer's, before any write: once the writer closed,
    // an open would meet its last close.
    let checkpointing = open_wal_checkpointer(path, options.vfs.as_deref(), options.durability)
        .map_err(|error| sqlite_error(path, &error))?;
    Ok((connection, deferring, declared, checkpointing))
}

/// Takes the idle timer's ticks out of `group`: whether it held one.
fn without_ticks(group: &mut Vec<Queued>) -> bool {
    let before = group.len();
    group.retain(|queued| queued.job != Job::Tick);
    group.len() != before
}

/// Whether the file at `connection` declares [`DEFERRED_CHARGES`]: a Run's
/// earlier run epoch may have.
fn declared_deferral(connection: &Connection) -> rusqlite::Result<bool> {
    match connection.query_row("SELECT deferred_charges FROM run", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(declared) => Ok(declared > 0),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(error),
    }
}

impl Drop for WriterConnection {
    fn drop(&mut self) {
        self.readers.close();
    }
}

/// Takes the next run epoch of the run file at `path` (ADR 0003): one
/// higher than both the one it holds and `after`, the run epoch a run file
/// this one replaced held, so a copy's run epoch never goes down; written
/// and synced before the new writer opens it with [`RunState::open`], so a
/// writer of an earlier epoch commits nothing more. Only the holder of the
/// Run's writer lock calls it. It blocks: run it under a blocking policy.
///
/// # Errors
///
/// As [`RunState::open`], and `Sqlite` when the file holds no Run.
pub(crate) fn next_epoch(
    path: &Path,
    options: &RunStateOptions,
    tables: &[RunStateTables],
    after: u64,
) -> Result<u64, RunStateError> {
    let connection = open_file(path, options, tables)?;
    let epoch: i64 = connection
        .query_row(
            "UPDATE run SET epoch = max(epoch, ?1) + 1 RETURNING epoch",
            [i64::try_from(after).unwrap_or(i64::MAX)],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_error(path, &error))?;
    Ok(u64::try_from(epoch).unwrap_or_default())
}

/// The run epoch the run file at `path` holds, read without its writer;
/// `None` when it cannot be read. It blocks: run it under a blocking policy.
pub(crate) fn epoch_of(path: &Path) -> Option<u64> {
    let connection = File::new(path, None).connect(Access::ReadOnly).ok()?;
    run_row::epoch(&connection).ok()
}

pub(crate) fn sqlite_error(path: &Path, error: &impl std::fmt::Display) -> RunStateError {
    RunStateError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

/// The failure of a commit of the run file at `path` on `error`: a full
/// disk or an I/O error as such.
fn commit_error(path: &Path, error: &rusqlite::Error) -> RunStateError {
    if sqlite_store::is_storage(error) {
        RunStateError::Storage {
            path: path.to_owned(),
            message: error.to_string(),
        }
    } else {
        sqlite_error(path, error)
    }
}

/// Opens the run file at `path` to read it beside its writer, through the
/// VFS `vfs`; WAL gives it a consistent view as of one commit per
/// transaction.
///
/// # Errors
///
/// `Sqlite` when the file cannot be opened.
pub(crate) fn open_reader(path: &Path, vfs: Option<&str>) -> Result<Connection, RunStateError> {
    File::new(path, vfs)
        .connect(Access::ReadOnly)
        .map_err(|error| sqlite_error(path, &error))
}

/// Opens the file, sets its durability and creates its tables.
fn open_file(
    path: &Path,
    options: &RunStateOptions,
    tables: &[RunStateTables],
) -> Result<Connection, RunStateError> {
    let marked = options.durability == Durability::Off;
    // Below `off` the opening's commit, the file's tables and its format,
    // syncs nothing: no one is told of it, and the next synced commit, a
    // Run's start or a new run epoch, covers it, with the WAL's header
    // (#2117). At `off` the commit that sets the "unclean" mark syncs,
    // before any that does not.
    let opening = if marked {
        Synchronous::Full
    } else {
        Synchronous::Off
    };
    let statements: Vec<&[&str]> = std::iter::once(SCHEMA)
        .chain(tables.iter().map(|table| table.statements))
        .chain(marked.then_some(MARK))
        .collect();
    let connection = File::new(path, options.vfs.as_deref())
        .open(&Opening {
            folder: Folder::Exists,
            access: Access::Create,
            wait: Wait::Switch(SWITCH_TIMEOUT),
            durability: connection_durability(options.durability.synchronous(), opening),
            // Beneath every owner, so a commit beside the writer's jobs
            // counts too (ADR 0140).
            counting: Some(Counting {
                work: &options.work,
                opens: None,
                transactions: Row::RunTransactions,
                counts_switch: false,
            }),
            schema: Some(Schema {
                format: &RUN_STATE,
                statements: &statements,
            }),
        })
        .map_err(|error| match error {
            OpenError::Format(refusal) => refusal.into(),
            other => sqlite_error(path, &other),
        })?;
    tune(&connection, PAGE_CACHE_KIB, options.wal_checkpoint_after)
        .map_err(|error| sqlite_error(path, &error))?;
    Ok(connection)
}

/// Whether the run file at `path`, opened through the VFS `vfs`, is one a
/// crash left at `off` (ADR 0138): it holds the "unclean" mark, or it is
/// damaged and the Run's Durability level is `durability`, `off`, at which
/// a power cut may damage it. A file that is not there, or never held the
/// mark, is not. Its Run is then discarded, never read. It blocks: run it
/// under a blocking policy.
///
/// # Errors
///
/// `Sqlite` when the file cannot be read, damaged below `off` included.
pub(crate) fn unclean(
    path: &Path,
    vfs: Option<&str>,
    durability: Durability,
) -> Result<bool, RunStateError> {
    let opened = File::new(path, vfs).connect(Access::ReadWrite);
    // Reading the mark copies no WAL into the file as it closes (#2117):
    // the file stays as its writer left it.
    let marked = opened.and_then(|connection| {
        sqlite_store::no_checkpoint_on_close(&connection)?;
        connection
            .query_row("SELECT count(*) FROM unclean", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|marks| marks > 0)
    });
    match marked {
        Ok(marked) => Ok(marked),
        Err(error) => match error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::CannotOpen) => Ok(false),
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
                if durability == Durability::Off =>
            {
                Ok(true)
            }
            // A file of no table yet, or of a Run that never ran at `off`.
            _ if error.to_string().starts_with("no such table") => Ok(false),
            _ => Err(sqlite_error(path, &error)),
        },
    }
}

/// Opens the writer's deferring connection on the file at `path`, already
/// in WAL mode, which commits with `synchronous=NORMAL`.
fn open_deferring(path: &Path, options: &RunStateOptions) -> Result<Connection, RunStateError> {
    let failed = |error: rusqlite::Error| sqlite_error(path, &error);
    let connection = File::new(path, options.vfs.as_deref())
        .connect(Access::ReadWrite)
        .map_err(failed)?;
    // Beneath every owner, so a commit beside the writer's jobs counts too
    // (ADR 0140).
    Counting {
        work: &options.work,
        opens: None,
        transactions: Row::RunTransactions,
        counts_switch: false,
    }
    .count_commits(&connection)
    .map_err(failed)?;
    tune(
        &connection,
        DEFERRING_PAGE_CACHE_KIB,
        options.wal_checkpoint_after,
    )
    .map_err(failed)?;
    connection_durability(Synchronous::Normal, Synchronous::Normal)
        .apply(&connection)
        .map_err(failed)?;
    connection.wal_hook(Some(note_wal_pages));
    Ok(connection)
}

/// Sets up a writer's connection beyond its durability: its statement
/// cache, a page cache of `cache_kib`, and a WAL it truncates to
/// `wal_limit` bytes at its first commit after the WAL started again.
fn tune(connection: &Connection, cache_kib: u32, wal_limit: u64) -> rusqlite::Result<()> {
    connection.set_prepared_statement_cache_capacity(STATEMENTS);
    connection.pragma_update(
        None,
        "journal_size_limit",
        i64::try_from(wal_limit).unwrap_or(i64::MAX),
    )?;
    // Savepoints' statement journals stay in memory, never in `$TMPDIR`
    // (ADR 0138).
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    // A negative size counts KiB rather than pages.
    connection.pragma_update(None, "cache_size", -i64::from(cache_kib))
}

impl From<FormatRefusal> for RunStateError {
    fn from(refusal: FormatRefusal) -> Self {
        Self::Format {
            format: refusal.format,
            found: refusal.found,
            compatibility: refusal.compatibility,
            install: None,
        }
    }
}

/// Why a write's savepoint failed.
struct Failed {
    /// SQLite's own error, or the write's.
    error: rusqlite::Error,
    /// Whether the group's transaction can go on: not after a full disk or
    /// an I/O error, nor once SQLite ended the transaction, nor when the
    /// savepoint's own statements failed, which leave its state unknown.
    ended: bool,
}

/// Runs `work` in a savepoint of its own, which an error rolls back. A full
/// disk or an I/O error can end the whole transaction in SQLite, closing
/// its savepoints (ADR 0122); the savepoint is then never rolled back to,
/// since a statement run after it would commit on its own.
fn run_in_savepoint<T, F>(connection: &Connection, work: F) -> Result<T, Failed>
where
    F: FnOnce(&Connection) -> rusqlite::Result<T>,
{
    let ended = |error| Failed { error, ended: true };
    run_cached(connection, "SAVEPOINT write").map_err(ended)?;
    match work(connection) {
        Ok(answer) => {
            run_cached(connection, "RELEASE write").map_err(ended)?;
            Ok(answer)
        }
        Err(error) if sqlite_store::is_storage(&error) || connection.is_autocommit() => {
            Err(ended(error))
        }
        Err(error) => match run_cached(connection, "ROLLBACK TO write")
            .and_then(|()| run_cached(connection, "RELEASE write"))
        {
            Ok(()) => Err(Failed {
                error,
                ended: false,
            }),
            Err(_) => Err(ended(error)),
        },
    }
}

/// Where the writer signals each acknowledged commit and the first storage
/// failure.
struct Signals<'a> {
    commits: &'a watch::Sender<u64>,
    failures: &'a watch::Sender<Option<String>>,
    work: &'a CountedWork,
}

/// What a group's commit also writes in the run file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Marks {
    /// [`DEFERRED_CHARGES`], declared.
    declare: bool,
    /// The "unclean" mark, cleared: `off`'s clean stop.
    clean: bool,
}

/// Commits `group` in one transaction under `epoch`, declaring
/// [`DEFERRED_CHARGES`] in the file when `marks` say so and clearing its
/// "unclean" mark when they say so, counts the commit in
/// `commits`, then answers each write: all of them with the failure when
/// the transaction fails or the file holds another epoch, and a full disk or
/// an I/O error also goes to `failures`. The commit hook counts the commit,
/// and each other job of its group counts here: a transaction is one job,
/// whatever group it commits in (ADR 0138's note). Answers, once it
/// committed, whether it declared the bound.
fn commit_group(
    connection: &Connection,
    path: &Path,
    (epoch, marks): (u64, Marks),
    group: Vec<QueuedWrite>,
    signals: &Signals<'_>,
) -> Option<bool> {
    let grouped = u64::try_from(group.len().saturating_sub(1)).unwrap_or(u64::MAX);
    let mut declared = None;
    let (completions, committed) = match commit_writes(connection, path, (epoch, marks), group) {
        Ok((completions, declaring)) => {
            declared = Some(declaring);
            signals.work.count_many(Row::RunTransactions, grouped);
            signals.commits.send_modify(|count| *count += 1);
            (completions, Ok(()))
        }
        Err((completions, error)) => {
            if let RunStateError::Storage { message, .. } = &error {
                tracing::warn!(path = %path.display(), %message, "a commit of run state met a full disk or an I/O error and was rolled back");
                signals.storage_failed(message);
            }
            (completions, Err(error))
        }
    };
    for completion in completions {
        completion(committed.clone());
    }
    declared
}

impl Signals<'_> {
    /// Signals a storage failure, `message`, unless one came first.
    fn storage_failed(&self, message: &str) {
        self.failures.send_if_modified(|failure| {
            let first = failure.is_none();
            if first {
                *failure = Some(message.to_owned());
            }
            first
        });
    }
}

fn commit_writes(
    connection: &Connection,
    path: &Path,
    (epoch, marks): (u64, Marks),
    group: Vec<QueuedWrite>,
) -> Result<(Vec<Completion>, bool), (Vec<Completion>, RunStateError)> {
    let refuse = |group: Vec<QueuedWrite>, error: RunStateError| {
        let completions = group.into_iter().map(|job| job(None).0).collect();
        (completions, error)
    };
    if let Err(error) = run_cached(connection, "BEGIN IMMEDIATE") {
        return Err(refuse(group, commit_error(path, &error)));
    }
    let transaction = connection;
    let found = transaction
        .prepare_cached("SELECT epoch FROM run")
        .and_then(|mut epoch| epoch.query_row([], |row| row.get::<_, i64>(0)))
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            error => Err(error),
        });
    match found {
        Err(error) => {
            let _ = run_cached(transaction, "ROLLBACK");
            return Err(refuse(group, commit_error(path, &error)));
        }
        Ok(Some(found)) if i64::try_from(epoch).ok() != Some(found) => {
            let _ = run_cached(transaction, "ROLLBACK");
            let found = u64::try_from(found).unwrap_or_default();
            return Err(refuse(group, RunStateError::Fenced { held: epoch, found }));
        }
        Ok(_) => {}
    }
    let mut completions = Vec::with_capacity(group.len());
    let mut jobs = group.into_iter();
    let mut ended = None;
    for job in jobs.by_ref() {
        let (completion, ends_group) = job(Some((transaction, path)));
        completions.push(completion);
        if ends_group.is_some() {
            ended = ends_group;
            break;
        }
    }
    let mut declared = false;
    let ended = ended.or_else(|| {
        marks
            .clean
            .then(|| run_cached(transaction, "DELETE FROM unclean").err())
            .flatten()
            .map(|error| commit_error(path, &error))
    });
    let failed = match ended {
        // The rest of the group never runs: in a transaction SQLite ended,
        // each of its statements would commit on its own.
        Some(error) => {
            completions.extend(jobs.map(|job| job(None).0));
            Some(error)
        }
        None if marks.declare => transaction
            .prepare_cached(
                "UPDATE run SET journal_position = journal_position + 1, deferred_charges = ?1",
            )
            .and_then(|mut statement| {
                statement.execute([i64::try_from(DEFERRED_CHARGES).unwrap_or(i64::MAX)])
            })
            .map(|changed| declared = changed == 1)
            .err()
            .map(|error| commit_error(path, &error)),
        None => run_cached(
            transaction,
            "UPDATE run SET journal_position = journal_position + 1",
        )
        .err()
        .map(|error| commit_error(path, &error)),
    };
    if let Some(error) = failed {
        // SQLite may have rolled the transaction back already after a full
        // disk or an I/O error; `ROLLBACK` makes the state the same either
        // way (ADR 0122).
        let _ = run_cached(transaction, "ROLLBACK");
        return Err((completions, error));
    }
    match run_cached(transaction, "COMMIT") {
        Ok(()) => Ok((completions, declared)),
        Err(error) => {
            // A commit that fails leaves the transaction open: it is rolled
            // back, as SQLite advises (ADR 0122).
            let _ = run_cached(transaction, "ROLLBACK");
            Err((completions, commit_error(path, &error)))
        }
    }
}

/// Runs `statement`, which binds nothing, through `connection`'s statement
/// cache: the writer's transactions and savepoints prepare once per
/// connection, not once per group or job, as rusqlite's own transactions
/// do (ADR 0138).
pub(crate) fn run_cached(connection: &Connection, statement: &str) -> rusqlite::Result<()> {
    connection.prepare_cached(statement)?.execute([]).map(drop)
}

#[cfg(test)]
mod tests;
