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
//! synced commit covers them. The file also declares how the Run's
//! charges of the Usage ledger reach it ([`Charges`]): each synced before
//! it is answered, deferred within [`DEFERRED_CHARGES`], or unsynced, in a
//! Run-state batch or at a `run_state.batch.sync` other than `each`; the
//! declaration only rises, in the commit of the first write it covers.
//!
//! Size: one concern, the run file's writer on its one connection: how it
//! opens, queues jobs and commits each group, which read side by side.

use std::num::NonZeroU64;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::{Compatibility, RUN_STATE};
use goodfetch_protocol::run::durability::{
    BATCH_BYTES, BATCH_INTERVAL_MS, BATCH_PAGES, BatchThreshold, MEMORY_BUDGET, SYNC_EACH,
    SYNC_INTERVAL, SYNC_NEVER, SYNC_OFF,
};
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;
use crate::metrics::Metrics;
use crate::sqlite_store::{
    self, Access, Counting, File, Folder, FormatRefusal, FullFsync, OpenError, Opening, Schema,
    Synchronous, Wait, WalOnClose, run_cached,
};
use crate::tasks::TaskOwner;
use wal_checkpointer::{WalCheckpointer, note_wal_pages, open_wal_checkpointer};

mod batch;
mod batch_end;
mod filesystem;
pub mod install;
mod open_batch;
mod readers;
pub mod run_row;
mod wal_checkpointer;

pub use batch::{BatchRows, BatchStatement, BatchWriter, SealedBatch};
pub use batch_end::BatchEnd;
pub use filesystem::UnsupportedFolder;
pub(crate) use readers::RunStateReaders;

/// How many writes wait for the writer before their callers wait.
pub(crate) const MAILBOX: usize = 1024;

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
        fixed_settings TEXT NOT NULL DEFAULT '{}',
        setting_digests TEXT NOT NULL DEFAULT '{}',
        uncapped INTEGER NOT NULL DEFAULT 0,
        progress_message TEXT,
        nothing_done TEXT,
        report TEXT,
        charges TEXT NOT NULL DEFAULT 'synced' CHECK (charges IN ('synced', 'deferred', 'unsynced'))
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

/// A column of a run file that holds what a Run creates itself, such as a
/// Session's cookies, a token or login state (ADR 0015). Its owner declares
/// it as a constant beside its tables, where the column is a `BLOB`; every
/// value written to it is one `Run::seal` sealed for it and its row under
/// the Run's data key, and every value read from it goes through
/// `Run::open`, so the run file, its WAL and its Checkpoints hold it sealed
/// alone. A Run that writes none makes no data key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecretColumn {
    /// Its table.
    pub table: &'static str,
    /// Its name in that table.
    pub column: &'static str,
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

/// How a run file says the Run's charges of the Usage ledger (ADR 0041)
/// reach it, which a reclaim after an unclean end reads (ADR 0142's note of
/// G-11 Q3): `run.charges`. A file declares more only: from `synced` to
/// `deferred` to `unsynced`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Charges {
    /// Each synced before it is answered: nothing was declared.
    Synced,
    /// Deferred at `full`, at most [`DEFERRED_CHARGES`] of a Slice since
    /// the last synced commit.
    Deferred,
    /// Answered before they were synced, in a Run-state batch or at a
    /// `run_state.batch.sync` other than `each`: a power cut may take any
    /// of them.
    Unsynced,
}

impl Charges {
    /// Its value in `run.charges`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Deferred => "deferred",
            Self::Unsynced => "unsynced",
        }
    }

    /// The declaration `run.charges` holds as `value`: `unsynced`, the
    /// most, for a value it never holds.
    pub fn stored(value: &str) -> Self {
        match value {
            "synced" => Self::Synced,
            "deferred" => Self::Deferred,
            _ => Self::Unsynced,
        }
    }
}

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
    /// The most bytes of what its writes wrote that the Run keeps in
    /// memory beside the file (`run_state.memory.budget`, #2169).
    pub memory_budget: u64,
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
            memory_budget: MEMORY_BUDGET,
            work: CountedWork::default(),
            wal_checkpoint_after: WAL_CHECKPOINT_AFTER_BYTES,
            clock: Clock::system(),
            metrics: None,
        }
    }
}

/// How a run file's changes reach the disk: `run_state.durability`'s level
/// unfolded into its preset's `run_state.batch.*` keys, each of which a
/// layer may also set (ADRs 0002, 0138 and 0142).
///
/// With `per_attempt`, the writer commits each group of writes as it
/// comes. Without it, the writer holds one transaction open across groups,
/// the open batch ([`open_batch`]): an attempt's write is answered once it
/// is in it, and the batch commits at a barrier, a write that must be in
/// the file before it is answered, at a threshold, or as the writer ends.
/// `sync` says which commits reach the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Durability {
    /// `run_state.batch.per_attempt`: each attempt's claim and outcome a
    /// batch of its own.
    pub per_attempt: bool,
    /// `run_state.batch.pages`: the attempts that seal a batch.
    pub pages: BatchThreshold,
    /// `run_state.batch.interval`: the milliseconds that seal a batch.
    pub interval: BatchThreshold,
    /// `run_state.batch.bytes`: the bytes of rows that seal a batch.
    pub bytes: BatchThreshold,
    /// `run_state.batch.sync`: which batches reach the disk.
    pub sync: BatchSync,
}

/// Which of a run file's batches reach the disk before the next one starts
/// (`run_state.batch.sync`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchSync {
    /// Every batch survives a power cut: `synchronous=FULL`, with
    /// `fullfsync` on macOS, for every group with a write that needs it.
    Each,
    /// As `never`, with one synced batch once `run_state.commit_interval`
    /// passed since the last one, and one for an idle writer's batches once
    /// it passed: a power cut takes at most that interval of work.
    Interval(std::time::Duration),
    /// A batch survives a process crash only: `synchronous=NORMAL`, whose
    /// commits in WAL mode an OS crash or a power cut may roll back,
    /// without damaging the file.
    Never,
    /// Nothing synced (`synchronous=OFF`) but the file's "unclean" mark,
    /// set as its writer opens it and cleared by its clean stop: a crash
    /// leaves the mark, so [`unclean`] says the file is to be discarded.
    Off,
}

impl BatchSync {
    /// The sync `run_state.batch.sync`'s value `sync` names, `interval` at
    /// `commit_interval`; `None` for a value that names none, which the
    /// settings resolver never lets through.
    pub fn named(sync: &str, commit_interval: std::time::Duration) -> Option<Self> {
        match sync {
            SYNC_EACH => Some(Self::Each),
            SYNC_INTERVAL => Some(Self::Interval(commit_interval)),
            SYNC_NEVER => Some(Self::Never),
            SYNC_OFF => Some(Self::Off),
            _ => None,
        }
    }
}

const PER_ATTEMPT_THRESHOLD: BatchThreshold = BatchThreshold::Never;

impl Durability {
    /// `full`'s preset: each attempt's writes synced as they come.
    pub const FULL: Self = Self::per_attempt(BatchSync::Each);
    /// `fast`'s preset: as `full`, never synced.
    pub const FAST: Self = Self::per_attempt(BatchSync::Never);
    /// `batch`'s preset, the default: batches of 1,000 attempts, 1 s or
    /// 8 MiB, each synced.
    pub const BATCH: Self = Self::batches(BatchSync::Each);
    /// `off`'s preset: as `batch`, nothing synced.
    pub const OFF: Self = Self::batches(BatchSync::Off);

    /// `periodic`'s preset, one synced batch every `commit_interval`.
    pub const fn periodic(commit_interval: std::time::Duration) -> Self {
        Self::per_attempt(BatchSync::Interval(commit_interval))
    }

    const fn per_attempt(sync: BatchSync) -> Self {
        Self {
            per_attempt: true,
            pages: PER_ATTEMPT_THRESHOLD,
            interval: PER_ATTEMPT_THRESHOLD,
            bytes: PER_ATTEMPT_THRESHOLD,
            sync,
        }
    }

    const fn batches(sync: BatchSync) -> Self {
        Self {
            per_attempt: false,
            pages: BatchThreshold::At(NonZeroU64::new(BATCH_PAGES).expect("invariant: 1,000")),
            interval: BatchThreshold::At(
                NonZeroU64::new(BATCH_INTERVAL_MS).expect("invariant: 1 s"),
            ),
            bytes: BatchThreshold::At(NonZeroU64::new(BATCH_BYTES).expect("invariant: 8 MiB")),
            sync,
        }
    }

    /// The `synchronous` level of the writer's connection, which commits
    /// the groups that sync at `each` and `interval`.
    fn synchronous(self) -> Synchronous {
        match self.sync {
            BatchSync::Each | BatchSync::Interval(_) => Synchronous::Full,
            BatchSync::Never => Synchronous::Normal,
            BatchSync::Off => Synchronous::Off,
        }
    }

    /// The `synchronous` pragma of its checkpointer's connection: synced
    /// below `each` too, so a checkpoint's pages reach the disk before the
    /// WAL frames they came from are written over. A clean stop runs no
    /// checkpoint (#2117): the file reads whole with the `-wal` beside it.
    fn checkpoint_synchronous(self) -> Synchronous {
        match self.sync {
            BatchSync::Each => Synchronous::Full,
            BatchSync::Interval(_) | BatchSync::Never | BatchSync::Off => Synchronous::Normal,
        }
    }

    /// Whether the writer keeps a second connection at
    /// `synchronous=NORMAL` for the commits that do not sync: the deferred
    /// writes' at `each` when each attempt is a batch, and the batches
    /// between two synced ones at `interval`.
    fn defers(self) -> bool {
        match self.sync {
            BatchSync::Each => self.per_attempt,
            BatchSync::Interval(_) => true,
            BatchSync::Never | BatchSync::Off => false,
        }
    }

    /// How the writer's charges reach the file when they are not each
    /// synced as they are answered: deferred within [`DEFERRED_CHARGES`]
    /// when each attempt is a batch synced as it comes, else unsynced.
    fn charges(self) -> Charges {
        if self.per_attempt && self.sync == BatchSync::Each {
            Charges::Deferred
        } else {
            Charges::Unsynced
        }
    }

    /// Whether nothing is synced and a crash discards the file: `sync: off`.
    pub(crate) fn unsynced(self) -> bool {
        self.sync == BatchSync::Off
    }

    /// The writer's sync interval: `sync: interval`'s only.
    fn sync_interval(self) -> Option<std::time::Duration> {
        match self.sync {
            BatchSync::Interval(interval) => Some(interval),
            BatchSync::Each | BatchSync::Never | BatchSync::Off => None,
        }
    }
}

impl Default for Durability {
    fn default() -> Self {
        Self::BATCH
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
    /// As `Required`, its group also declaring how the Run's charges reach
    /// the file ([`Charges`]) when the file does not yet: the start of a
    /// Run whose claims defer their charges, so that no claim's group syncs
    /// to declare it (#2117).
    Declaring,
    /// Committed without a sync of its own at `full`: a power cut before
    /// the next synced commit may take it, as it may an attempt's claim.
    /// Without `per_attempt`, answered once it is in the open batch, unless
    /// it charges before the file declares [`Charges::Unsynced`]: then it
    /// declares it, answered once its batch commits.
    Deferred {
        /// The most it charges of any one Slice of the Usage ledger, which
        /// counts against [`DEFERRED_CHARGES`].
        charged: u64,
    },
    /// An attempt's write that tells something outside, such as its
    /// outcome (ADR 0142): as `Required` when each attempt is a batch of
    /// its own (`run_state.batch.per_attempt`), else answered once it is in
    /// the open batch, whose thresholds it counts towards.
    Attempt {
        /// The attempts it ends, towards `run_state.batch.pages`.
        ended: u64,
        /// The bytes of the rows it writes, towards `run_state.batch.bytes`.
        bytes: u64,
    },
    /// An Effect's sent mark (ADR 0106): synced before it is answered at
    /// every level but `off`, `fast`'s and `periodic`'s included, so no
    /// outside action follows a mark a power cut may take (ADR 0138's note
    /// of #1550). At `fast`, its group commits on the writer's connection
    /// switched to `synchronous=FULL`, then switched back.
    Sent,
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
    /// The file's folder is on a filesystem whose locking SQLite cannot
    /// rely on (ADR 0049), so it was not opened.
    #[error(
        "run state in `{}` is on {}, which the engine keeps no run state on",
        .0.folder.display(),
        .0.found
    )]
    FilesystemUnsupported(UnsupportedFolder),
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
    Box<dyn FnOnce(Option<Running<'_>>) -> (Completion, Option<RunStateError>) + Send>;

/// What a write runs in: the transaction open on the writer's connection,
/// the run file's path, and the watch of the writer's writes, once one is
/// installed.
#[derive(Clone, Copy)]
struct Running<'a> {
    connection: &'a Connection,
    path: &'a Path,
    watch: Option<&'a dyn WriteWatch>,
}

/// What keeps in memory, beside the run file, what the writer's writes
/// change (#2169): told, on the writer's thread, as each write's work
/// begins, and when it failed, its changes then rolled back to the
/// savepoint it began at, so none of them reach the file. The writer runs
/// one write at a time, so what it is told between a `began` and the next
/// is one write's.
pub trait WriteWatch: Send + Sync + std::fmt::Debug {
    /// A write's work begins.
    fn began(&self);
    /// The write's work that began last failed: none of its changes reach
    /// the file.
    fn failed(&self);
}

/// Where the writer finds the watch of its writes, which a handle installs
/// once.
type Watching = std::sync::Arc<std::sync::OnceLock<std::sync::Arc<dyn WriteWatch>>>;

/// What a job in the writer's queue is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    /// A write.
    Write,
    /// The writer's last write.
    Last,
    /// The idle writer's timer at `periodic`: a group that syncs what the
    /// writer committed unsynced, if anything. It writes nothing itself.
    Tick,
    /// These statements prepared on each of the writer's connections. It
    /// writes nothing and commits in no group.
    Prepare(Vec<&'static str>),
    /// A seal of the open batch ([`RunState::seal`]): it writes nothing
    /// itself, and is answered once the batch holding every write queued
    /// before it is in the file.
    Seal,
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

/// A job that runs nothing, a preparation's or a seal's, and its answer,
/// which the writer's actor sends.
fn preparation() -> (QueuedWrite, oneshot::Receiver<Result<(), RunStateError>>) {
    let (answer, answered) = oneshot::channel();
    let job: QueuedWrite = Box::new(move |_| {
        let completion: Completion = Box::new(move |prepared: Result<(), RunStateError>| {
            let _ = answer.send(prepared);
        });
        (completion, None)
    });
    (job, answered)
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
    let job: QueuedWrite = Box::new(move |transaction: Option<Running<'_>>| {
        let mut ends_group = None;
        let done = match transaction {
            Some(Running {
                connection,
                path,
                watch,
            }) => {
                if let Some(watch) = watch {
                    watch.began();
                }
                run_in_savepoint(connection, work).map_err(|Failed { error, ended }| {
                    if let Some(watch) = watch {
                        watch.failed();
                    }
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
    per_attempt: bool,
    memory_budget: u64,
    watching: Watching,
    ending: batch_end::Ending,
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
    /// `FilesystemUnsupported` when its folder is on a network filesystem
    /// or a FUSE mount of object storage (ADR 0049), `Sqlite` when the
    /// file cannot be opened or its tables created, `Format` when it is of
    /// a format this engine cannot read.
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
            .blocking(move || {
                filesystem::check(&opened_path)?;
                open_connections(&opened_path, &opened, &tables)
            })
            .await?;
        let checkpointer = WalCheckpointer::start(&path, options, checkpointing, owner)
            .map_err(|error| sqlite_error(&path, &error))?;
        let (mailbox, jobs) = mpsc::channel(MAILBOX);
        let (committed, commits) = watch::channel(0_u64);
        let (failed, failures) = watch::channel(None);
        let watching = Watching::default();
        let ending = batch_end::Ending::default();
        let readers = std::sync::Arc::new(RunStateReaders::new(
            path.clone(),
            options.vfs.clone(),
            owner.blocking_threads(),
        ));
        let mut writer = Writer {
            connection: WriterConnection {
                connection,
                deferring,
                readers: std::sync::Arc::clone(&readers),
            },
            path: path.clone(),
            epoch,
            durability: options.durability,
            marked: options.durability.unsynced(),
            deferral: Deferral {
                deferring: options.durability.sync == BatchSync::Each,
                declared,
                declares: options.durability.charges(),
                from_start: options.durability.sync != BatchSync::Each,
                uncovered: 0,
            },
            interval: Interval::of(options),
            checkpointer,
            signals: Signals {
                commits: committed,
                failures: failed,
                work: options.work.clone(),
                watching: std::sync::Arc::clone(&watching),
                ending: std::sync::Arc::clone(&ending),
            },
            batching: open_batch::Batching::of(options, owner, &mailbox),
            // The idle writer's timer at `periodic`, which ends with the
            // writer.
            _idle_timer: idle_timer(owner, options, &mailbox),
        };
        owner
            .actor_until("run-state-writer", jobs, move |group: Vec<Queued>| {
                writer.handle(group)
            })
            .map_err(|error| sqlite_error(&path, &error))?;
        Ok(Self {
            mailbox,
            path,
            epoch,
            watched: std::sync::Arc::new(Watched { commits, failures }),
            readers,
            batches: std::sync::Arc::new(batch::Batches::new()),
            per_attempt: options.durability.per_attempt,
            memory_budget: options.memory_budget,
            watching,
            ending,
        })
    }

    /// Whether each attempt's claim and outcome is a batch of its own
    /// (`run_state.batch.per_attempt`): a write is then in the file before
    /// it is answered, and only a caller that waits for a claim knows it
    /// is in the file before its fetch.
    pub fn per_attempt(&self) -> bool {
        self.per_attempt
    }

    /// The most bytes of what its writes wrote that the Run keeps in
    /// memory beside the file (`run_state.memory.budget`).
    pub fn memory_budget(&self) -> u64 {
        self.memory_budget
    }

    /// Tells `watch`, from now on, as each write's work begins and when it
    /// fails ([`WriteWatch`]). Once is enough; a later call changes
    /// nothing.
    pub fn watch_writes(&self, watch: std::sync::Arc<dyn WriteWatch>) {
        let _installed = self.watching.get_or_init(|| watch);
    }

    /// Has the writer run `ending` at the end of each batch it commits,
    /// from now on ([`BatchEnd`]). Once is enough; a later call changes
    /// nothing.
    pub fn end_batches_with(&self, ending: std::sync::Arc<dyn BatchEnd>) {
        let _installed = self.ending.get_or_init(|| ending);
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

    /// Writes the open batch, if one is open, and answers once it is in the
    /// file: a read that follows sees every write answered before the seal
    /// was asked, the attempts' writes that rode the batch included (ADR
    /// 0142). It writes nothing of its own, so with no batch open, or when
    /// each attempt is a batch of its own, it commits nothing.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the batch did not commit, `Closed` when the writer has
    /// stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a seal already queued.
    pub async fn seal(&self) -> Result<(), RunStateError> {
        let (write, answered) = preparation();
        self.mailbox
            .send(Queued {
                write,
                job: Job::Seal,
                syncing: Syncing::Required,
            })
            .await
            .map_err(|_| RunStateError::Closed)?;
        answered.await.map_err(|_| RunStateError::Closed)?
    }

    /// Prepares each of `statements` on every connection the writer commits
    /// on, in no transaction, so that no later write prepares one of them,
    /// whichever connection its group commits on (ADR 0138's note of
    /// #1926), and on each of its readers' connections before its next read,
    /// so that no read does either: what a Run does once, as it starts or
    /// resumes, for the statements its units run later (ADR 0140).
    ///
    /// # Errors
    ///
    /// `Sqlite` when a statement does not prepare, `Closed` when the writer
    /// has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a preparation already queued.
    pub async fn prepare(&self, statements: Vec<&'static str>) -> Result<(), RunStateError> {
        self.readers.prepare(statements.clone()).await;
        let (write, answered) = preparation();
        self.mailbox
            .send(Queued {
                write,
                job: Job::Prepare(statements),
                syncing: Syncing::Deferred { charged: 0 },
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

    /// Takes the preparations out of `group` and answers each once its
    /// statements are prepared on every connection, for the file at `path`.
    fn prepare(&self, group: &mut Vec<Queued>, path: &Path) {
        let (preparations, writes): (Vec<Queued>, Vec<Queued>) = std::mem::take(group)
            .into_iter()
            .partition(|queued| matches!(queued.job, Job::Prepare(_)));
        *group = writes;
        for Queued { write, job, .. } in preparations {
            let Job::Prepare(statements) = job else {
                continue;
            };
            let prepared = std::iter::once(&self.connection)
                .chain(&self.deferring)
                .try_for_each(|connection| {
                    statements
                        .iter()
                        .try_for_each(|statement| connection.prepare_cached(statement).map(drop))
                })
                .map_err(|error| sqlite_error(path, &error));
            write(None).0(prepared);
        }
    }
}

/// The writer's actor: its connections, what it knows of its syncs and
/// charges, and, at a level that batches attempts, its open batch.
struct Writer {
    connection: WriterConnection,
    path: PathBuf,
    /// The run epoch it commits under.
    epoch: u64,
    durability: Durability,
    /// Whether the file carries the "unclean" mark, which its clean stop
    /// clears: at `off`.
    marked: bool,
    deferral: Deferral,
    /// At `run_state.batch.sync: interval` only.
    interval: Option<Interval>,
    checkpointer: WalCheckpointer,
    signals: Signals,
    /// Without `run_state.batch.per_attempt` only.
    batching: Option<open_batch::Batching>,
    /// Held until the writer ends, which ends the idle timer.
    _idle_timer: oneshot::Sender<()>,
}

impl Writer {
    /// Handles one group of the writer's jobs: up to its last write, whose
    /// commit stops the writer and refuses the jobs after it.
    fn handle(&mut self, mut group: Vec<Queued>) -> ControlFlow<()> {
        let ticked = without_ticks(&mut group);
        self.connection.prepare(&mut group, &self.path);
        let last = group.iter().position(|queued| queued.job == Job::Last);
        let refused = last.map_or_else(Vec::new, |at| group.split_off(at + 1));
        let last = last.is_some();
        let seals = without_seals(&mut group);
        let sent = self.syncs_sent(&group, last);
        let switched = sent && self.switch_to_full();
        if self.batching.is_some() {
            self.batch(group, ticked, last, sent);
            self.seal(seals);
        } else {
            self.commit_each(group, ticked, last, sent);
            // Each write committed in its group, answered once it did.
            for seal in seals {
                seal(None).0(Ok(()));
            }
        }
        if switched {
            self.switch_back();
        }
        for queued in refused {
            (queued.write)(None).0(Err(RunStateError::Closed));
        }
        if last {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    /// Commits `group` in a transaction of its own, each attempt a batch of
    /// its own (`run_state.batch.per_attempt`).
    fn commit_each(&mut self, group: Vec<Queued>, ticked: bool, last: bool, sent: bool) {
        let mut planned = match &self.interval {
            Some(interval) => interval.plan(ticked, last),
            None => self.deferral.plan(&group),
        };
        planned.synced |= sent;
        // A tick with nothing committed unsynced commits nothing.
        let idle =
            group.is_empty() && !self.interval.as_ref().is_some_and(|interval| interval.owed);
        if idle {
            return;
        }
        let synced_stop = self.synced_stop(last);
        let clean = self.marked && synced_stop;
        let group: Vec<QueuedWrite> = group.into_iter().map(|queued| queued.write).collect();
        let on = self.connection.for_group(planned.synced);
        let marks = Marks {
            declare: self.deferral.declaring(planned.declare),
            clean,
        };
        if let Some(declared) =
            commit_group(on, &self.path, (self.epoch, marks), group, &self.signals)
        {
            self.deferral.committed(planned, declared);
            self.committed(planned.synced, last);
        }
    }

    /// Whether `group` holds an Effect's sent mark that syncs: at every
    /// level but `off`. The writer's `last` write syncs at every level
    /// already.
    fn syncs_sent(&self, group: &[Queued], last: bool) -> bool {
        !last
            && self.durability.sync != BatchSync::Off
            && group.iter().any(|queued| queued.syncing == Syncing::Sent)
    }

    /// Switches the writer's connection to `synchronous=FULL` for a group
    /// that holds a sent mark at `fast`, whose connection syncs nothing:
    /// whether it switched. A connection switches outside a transaction
    /// only, so a batch open on it commits first.
    fn switch_to_full(&mut self) -> bool {
        if self.durability.sync != BatchSync::Never {
            return false;
        }
        if self.batching.is_some() {
            self.commit_open_batch();
        }
        self.connection
            .connection
            .pragma_update(None, "synchronous", "FULL")
            .is_ok()
    }

    /// Switches the writer's connection back to `fast`'s
    /// `synchronous=NORMAL` after a group that holds a sent mark.
    fn switch_back(&self) {
        if let Err(error) = self
            .connection
            .connection
            .pragma_update(None, "synchronous", "NORMAL")
        {
            tracing::warn!(path = %self.path.display(), %error, "the run state writer did not switch back to synchronous=NORMAL after an Effect's sent mark");
        }
    }

    /// Whether the writer's `last` write syncs at a level that syncs
    /// nothing else, having switched its connection to `synchronous=FULL`:
    /// a clean stop commits synced at every level, `fast`'s and `off`'s
    /// included, and `off`'s clears the mark: its sync covers the WAL,
    /// which no checkpoint copies then (#2117). No transaction may be open
    /// on the connection.
    fn synced_stop(&self, last: bool) -> bool {
        last && matches!(self.durability.sync, BatchSync::Never | BatchSync::Off)
            && self
                .connection
                .connection
                .pragma_update(None, "synchronous", "FULL")
                .is_ok()
    }

    /// Notes a commit, `synced` or not, the writer's `last` or not.
    fn committed(&mut self, synced: bool, last: bool) {
        if let Some(interval) = &mut self.interval {
            interval.committed(synced);
        }
        self.checkpointer
            .committed(&self.connection.connection, &self.path, last);
    }
}

/// What the writer knows of the charges its unsynced commits hold, against
/// [`DEFERRED_CHARGES`], and of what the file declares of them.
struct Deferral {
    /// Whether it defers at all: at `full` only, as `fast` syncs no group.
    deferring: bool,
    /// What the file declares.
    declared: Charges,
    /// What the writer declares once a charge may reach the file unsynced.
    declares: Charges,
    /// Whether every charge may: at a `run_state.batch.sync` other than
    /// `each`, where the writer declares with its first commit of the Run's
    /// row.
    from_start: bool,
    /// What its commits since the last synced one charged.
    uncovered: u64,
}

/// How the writer commits one group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Planned {
    /// On the synced connection.
    synced: bool,
    /// Declaring how the Run's charges reach the file as it commits.
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
                Syncing::Required | Syncing::Attempt { .. } | Syncing::Sent => required = true,
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
            synced(declaring && self.owes())
        } else if charged == 0 {
            Planned {
                synced: false,
                declare: false,
                charged,
            }
        } else if self.owes() {
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

    /// Whether the file declares less than the writer would.
    fn owes(&self) -> bool {
        self.declared < self.declares
    }

    /// What a commit declares, its writes asking to (`asked`) or not: what
    /// the writer declares while the file owes it, at every commit when
    /// every charge may reach the file unsynced.
    fn declaring(&self, asked: bool) -> Option<Charges> {
        ((asked || self.from_start) && self.owes()).then_some(self.declares)
    }

    /// Notes that a commit declared what [`Self::declaring`] gave, or not.
    fn declared(&mut self, declared: bool) {
        if declared {
            self.declared = self.declared.max(self.declares);
        }
    }

    /// Notes that a group planned as `planned` committed, the writer's
    /// declaration in its commit or not.
    fn committed(&mut self, planned: Planned, declared: bool) {
        self.declared(declared);
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
        options.durability.sync_interval().map(|interval| Self {
            every: interval,
            clock: options.clock,
            synced_at: options.clock.now(),
            owed: false,
        })
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
    let Some(interval) = options.durability.sync_interval() else {
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
/// deferring one at the levels that defer, what the file declares of the
/// Run's charges, and its checkpointer's. It blocks.
fn open_connections(
    path: &Path,
    options: &RunStateOptions,
    tables: &[RunStateTables],
) -> Result<(Connection, Option<Connection>, Charges, Connection), RunStateError> {
    let connection = open_file(path, options, tables)?;
    // It replaces SQLite's automatic checkpoint, which this connection then
    // never runs (ADR 0138).
    connection.wal_hook(Some(note_wal_pages));
    let deferring = options
        .durability
        .defers()
        .then(|| open_deferring(path, options))
        .transpose()?;
    let declared = declared_charges(&connection).map_err(|error| sqlite_error(path, &error))?;
    // Opened beside the writer's, before any write: once the writer closed,
    // an open would meet its last close.
    let checkpointing = open_wal_checkpointer(path, options.vfs.as_deref(), options.durability)
        .map_err(|error| sqlite_error(path, &error))?;
    Ok((connection, deferring, declared, checkpointing))
}

/// Takes the seals out of `group`, which the writer answers once the
/// group's writes are in the file.
fn without_seals(group: &mut Vec<Queued>) -> Vec<QueuedWrite> {
    let (seals, writes): (Vec<Queued>, Vec<Queued>) = std::mem::take(group)
        .into_iter()
        .partition(|queued| queued.job == Job::Seal);
    *group = writes;
    seals.into_iter().map(|seal| seal.write).collect()
}

/// Takes the idle timer's ticks out of `group`: whether it held one.
fn without_ticks(group: &mut Vec<Queued>) -> bool {
    let before = group.len();
    group.retain(|queued| queued.job != Job::Tick);
    group.len() != before
}

/// What the file at `connection` declares of the Run's charges: a Run's
/// earlier run epoch may have declared them.
fn declared_charges(connection: &Connection) -> rusqlite::Result<Charges> {
    match connection
        .prepare_cached("SELECT charges FROM run")?
        .query_row([], |row| row.get::<_, String>(0))
    {
        Ok(declared) => Ok(Charges::stored(&declared)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Charges::Synced),
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
        .prepare_cached("UPDATE run SET epoch = max(epoch, ?1) + 1 RETURNING epoch")
        .and_then(|mut statement| {
            statement.query_row([i64::try_from(after).unwrap_or(i64::MAX)], |row| row.get(0))
        })
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
    let marked = options.durability.unsynced();
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
            .prepare_cached("SELECT count(*) FROM unclean")?
            .query_row([], |row| row.get::<_, i64>(0))
            .map(|marks| marks > 0)
    });
    match marked {
        Ok(marked) => Ok(marked),
        Err(error) => match error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::CannotOpen) => Ok(false),
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
                if durability.unsynced() =>
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
struct Signals {
    commits: watch::Sender<u64>,
    failures: watch::Sender<Option<String>>,
    work: CountedWork,
    /// The watch of its writes, once a handle installed one.
    watching: Watching,
    /// What it writes at the end of each batch, once a handle installed it.
    ending: batch_end::Ending,
}

/// What a group's commit also writes in the run file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Marks {
    /// How the Run's charges reach the file, declared.
    declare: Option<Charges>,
    /// The "unclean" mark, cleared: `off`'s clean stop.
    clean: bool,
}

/// Commits `group` in one transaction under `epoch`, declaring how the
/// Run's charges reach the file when `marks` say so and clearing its
/// "unclean" mark when they say so, counts the commit in
/// `commits`, then answers each write: all of them with the failure when
/// the transaction fails or the file holds another epoch, and a full disk or
/// an I/O error also goes to `failures`. The commit hook counts the commit,
/// and each other job of its group counts here: a transaction is one job,
/// whatever group it commits in (ADR 0138's note). Answers, once it
/// committed, whether it declared what `marks` say.
fn commit_group(
    connection: &Connection,
    path: &Path,
    (epoch, marks): (u64, Marks),
    group: Vec<QueuedWrite>,
    signals: &Signals,
) -> Option<bool> {
    let grouped = u64::try_from(group.len().saturating_sub(1)).unwrap_or(u64::MAX);
    let mut declared = None;
    let watch = signals.watch();
    let running = Running {
        connection,
        path,
        watch,
    };
    let ending = signals.ending();
    let (completions, committed) = match commit_writes(running, ending, (epoch, marks), group) {
        Ok((completions, declaring)) => {
            declared = Some(declaring);
            signals.work.count_many(Row::RunTransactions, grouped);
            signals.commits.send_modify(|count| *count += 1);
            (completions, Ok(()))
        }
        Err((completions, error)) => {
            signals.failed(path, &error);
            (completions, Err(error))
        }
    };
    for completion in completions {
        completion(committed.clone());
    }
    declared
}

impl Signals {
    /// The watch of the writer's writes, once a handle installed one.
    fn watch(&self) -> Option<&dyn WriteWatch> {
        self.watching.get().map(|watch| &**watch)
    }

    /// What the writer writes at the end of each batch, once a handle
    /// installed it.
    fn ending(&self) -> Option<&dyn BatchEnd> {
        self.ending.get().map(|ending| &**ending)
    }

    /// Signals a commit of the file at `path` that failed on `error`: a
    /// full disk or an I/O error goes to the storage failures.
    fn failed(&self, path: &Path, error: &RunStateError) {
        if let RunStateError::Storage { message, .. } = error {
            tracing::warn!(path = %path.display(), %message, "a commit of run state met a full disk or an I/O error and was rolled back");
            self.storage_failed(message);
        }
    }

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
    running: Running<'_>,
    ending: Option<&dyn BatchEnd>,
    (epoch, marks): (u64, Marks),
    group: Vec<QueuedWrite>,
) -> Result<(Vec<Completion>, bool), (Vec<Completion>, RunStateError)> {
    let Running {
        connection, path, ..
    } = running;
    if let Err(error) = begin(connection, path, epoch) {
        let completions = group.into_iter().map(|job| job(None).0).collect();
        return Err((completions, error));
    }
    let mut completions = Vec::with_capacity(group.len());
    let mut jobs = group.into_iter();
    for job in jobs.by_ref() {
        let (completion, ends_group) = job(Some(running));
        completions.push(completion);
        if let Some(error) = ends_group {
            // The rest of the group never runs: in a transaction SQLite
            // ended, each of its statements would commit on its own.
            completions.extend(jobs.map(|job| job(None).0));
            roll_back(connection);
            return Err((completions, error));
        }
    }
    if let Some(error) = batch_end::end(running, ending) {
        roll_back(connection);
        return Err((completions, error));
    }
    match finish(connection, path, marks) {
        Ok(declared) => Ok((completions, declared)),
        Err(error) => Err((completions, error)),
    }
}

/// Begins a transaction on `connection` that commits only under `epoch`:
/// the file at `path` holds another epoch, and the transaction is rolled
/// back, `Fenced`.
fn begin(connection: &Connection, path: &Path, epoch: u64) -> Result<(), RunStateError> {
    run_cached(connection, "BEGIN IMMEDIATE").map_err(|error| commit_error(path, &error))?;
    let found = connection
        .prepare_cached("SELECT epoch FROM run")
        .and_then(|mut epoch| epoch.query_row([], |row| row.get::<_, i64>(0)))
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            error => Err(error),
        });
    match found {
        Err(error) => {
            roll_back(connection);
            Err(commit_error(path, &error))
        }
        Ok(Some(found)) if i64::try_from(epoch).ok() != Some(found) => {
            roll_back(connection);
            let found = u64::try_from(found).unwrap_or_default();
            Err(RunStateError::Fenced { held: epoch, found })
        }
        Ok(_) => Ok(()),
    }
}

/// Writes `marks` and the journal's position in the transaction open on
/// `connection`, then commits it: answers whether it declared how the Run's
/// charges reach the file, which needs the Run's row. A failure rolls the
/// transaction back.
fn finish(connection: &Connection, path: &Path, marks: Marks) -> Result<bool, RunStateError> {
    let mut declared = false;
    let cleared = if marks.clean {
        run_cached(connection, "DELETE FROM unclean")
    } else {
        Ok(())
    };
    let written = cleared.and_then(|()| {
        if let Some(charges) = marks.declare {
            connection
                .prepare_cached(
                    "UPDATE run SET journal_position = journal_position + 1, charges = ?1",
                )
                .and_then(|mut statement| statement.execute([charges.as_str()]))
                .map(|changed| declared = changed == 1)
        } else {
            run_cached(
                connection,
                "UPDATE run SET journal_position = journal_position + 1",
            )
        }
    });
    // A commit that fails leaves the transaction open: it is rolled back,
    // as SQLite advises (ADR 0122).
    match written.and_then(|()| run_cached(connection, "COMMIT")) {
        Ok(()) => Ok(declared),
        Err(error) => {
            roll_back(connection);
            Err(commit_error(path, &error))
        }
    }
}

/// Rolls back the transaction open on `connection`. SQLite may have rolled
/// it back already after a full disk or an I/O error; `ROLLBACK` makes the
/// state the same either way (ADR 0122).
fn roll_back(connection: &Connection) {
    let _ = run_cached(connection, "ROLLBACK");
}

#[cfg(test)]
mod tests;
