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
//! committed state waits on for more, never holding the writer back. The
//! writer stops once every handle on it is gone, or once its last write
//! commits, as a Run's end writes its record: whatever any handle queues
//! after that is refused (#1763).
//!
//! The file holds its format's `major.minor` (ADR 0066), the Run's own row
//! and tags, and the tables each contribution of the composition adds, such
//! as a crawl's work items.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Format, RUN_STATE};
use goodfetch_protocol::run::DURABILITY_FAST;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use tokio::sync::{mpsc, oneshot, watch};

use crate::tasks::TaskOwner;

pub mod install;

/// How many writes wait for the writer before their callers wait.
const MAILBOX: usize = 1024;

/// The engine's own tables, which every run file holds.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT",
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
        report TEXT
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT",
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

/// How a run file opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunStateOptions {
    /// The name of the SQLite VFS it opens through: none in production, the
    /// crash VFS in tests (ADR 0060), and the install database's in the
    /// per-install layout ([`install`]).
    pub vfs: Option<String>,
    /// How its commits reach the disk (`run_state.durability`).
    pub durability: Durability,
}

/// How a run file's commits reach the disk (`run_state.durability`, ADR
/// 0002).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Durability {
    /// An acknowledged commit survives a power cut: `synchronous=FULL`,
    /// with `fullfsync` on macOS.
    #[default]
    Full,
    /// An acknowledged commit survives a process crash only:
    /// `synchronous=NORMAL`, whose commits in WAL mode an OS crash or a
    /// power cut may roll back, without damaging the file.
    Fast,
}

impl Durability {
    /// The durability `run_state.durability`'s value names; `full` for any
    /// other, which the settings resolver never lets through.
    pub fn named(value: &serde_json::Value) -> Self {
        if value.as_str() == Some(DURABILITY_FAST) {
            Self::Fast
        } else {
            Self::Full
        }
    }

    /// Its `synchronous` pragma.
    fn synchronous(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Fast => "NORMAL",
        }
    }
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

/// A write in the writer's queue, and whether it is the writer's last.
struct Queued {
    write: QueuedWrite,
    last: bool,
}

/// `work` as a write the writer's actor runs in its next commit, and the
/// answer it sends once that commit ends.
fn queued<T, F>(work: F) -> (QueuedWrite, oneshot::Receiver<Result<T, RunStateError>>)
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
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
        let (write, answered) = queued(work);
        let mailbox = self.0.send(Queued { write, last: false });
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
        let connection = owner
            .blocking(move || open_file(&opened_path, &opened, &tables))
            .await?;
        let (mailbox, jobs) = mpsc::channel(MAILBOX);
        let actor_path = path.clone();
        let mut connection = connection;
        let (committed, commits) = watch::channel(0_u64);
        let (failed, failures) = watch::channel(None);
        owner
            .actor_until("run-state-writer", jobs, move |mut group: Vec<Queued>| {
                let signals = Signals {
                    commits: &committed,
                    failures: &failed,
                };
                let last = group.iter().position(|queued| queued.last);
                let refused = last.map_or_else(Vec::new, |at| group.split_off(at + 1));
                let group = group.into_iter().map(|queued| queued.write).collect();
                commit_group(&mut connection, &actor_path, epoch, group, &signals);
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
        self.queue(work, false).await
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
        self.queue(work, true).await
    }

    /// Queues `work`, the writer's last write when `last`, and answers what
    /// it returned once its commit is acknowledged.
    async fn queue<T, F>(&self, work: F, last: bool) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let (write, answered) = queued(work);
        self.mailbox
            .send(Queued { write, last })
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

    /// Runs `work` on the writer's connection, in order with its writes, so
    /// it sees every acknowledged one.
    ///
    /// # Errors
    ///
    /// As [`RunState::write`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub async fn read<T, F>(&self, work: F) -> Result<T, RunStateError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.write(work).await
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
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let epoch: i64 = connection
        .query_row("SELECT epoch FROM run", [], |row| row.get(0))
        .ok()?;
    u64::try_from(epoch).ok()
}

pub(crate) fn sqlite_error(path: &Path, error: &impl std::fmt::Display) -> RunStateError {
    RunStateError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

/// Whether `error` is a full disk or an I/O error, after which SQLite may
/// have rolled the whole transaction back (ADR 0122). A read that met an
/// I/O error (EIO) counts: SQLite's Unix VFS reports it as
/// `SQLITE_IOERR_CORRUPTFS`, which reaches its caller as `SQLITE_CORRUPT`.
fn is_storage(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(
            rusqlite::ErrorCode::DiskFull
                | rusqlite::ErrorCode::SystemIoFailure
                | rusqlite::ErrorCode::DatabaseCorrupt
        )
    )
}

/// The failure of a commit of the run file at `path` on `error`: a full
/// disk or an I/O error as such.
fn commit_error(path: &Path, error: &rusqlite::Error) -> RunStateError {
    if is_storage(error) {
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
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    match vfs {
        Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs),
        None => Connection::open_with_flags(path, flags),
    }
    .map_err(|error| sqlite_error(path, &error))
}

/// Opens the file, sets its durability and creates its tables.
fn open_file(
    path: &Path,
    options: &RunStateOptions,
    tables: &[RunStateTables],
) -> Result<Connection, RunStateError> {
    let failed = |error: rusqlite::Error| sqlite_error(path, &error);
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut connection = match options.vfs.as_deref() {
        Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs),
        None => Connection::open_with_flags(path, flags),
    }
    .map_err(failed)?;
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .map_err(failed)?;
    connection
        .pragma_update(None, "synchronous", options.durability.synchronous())
        .map_err(failed)?;
    // A negative size counts KiB rather than pages.
    connection
        .pragma_update(None, "cache_size", -i64::from(PAGE_CACHE_KIB))
        .map_err(failed)?;
    if cfg!(target_os = "macos") {
        connection
            .pragma_update(None, "fullfsync", "ON")
            .map_err(failed)?;
        connection
            .pragma_update(None, "checkpoint_fullfsync", "ON")
            .map_err(failed)?;
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failed)?;
    let statements = SCHEMA
        .iter()
        .chain(tables.iter().flat_map(|table| table.statements));
    for statement in statements {
        transaction.execute(statement, []).map_err(failed)?;
    }
    let version = RUN_STATE.version.to_string();
    transaction
        .execute(
            "INSERT INTO format (name, version) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING",
            (RUN_STATE.name, &version),
        )
        .map_err(failed)?;
    let found: String = transaction
        .query_row(
            "SELECT version FROM format WHERE name = ?1",
            [RUN_STATE.name],
            |row| row.get(0),
        )
        .map_err(failed)?;
    check_format(&RUN_STATE, &found)?;
    transaction.commit().map_err(failed)?;
    Ok(connection)
}

/// Refuses a file of `format` at the version `found` that this engine cannot
/// read, under the format's rule (ADR 0030).
pub(crate) fn check_format(format: &Format, found: &str) -> Result<(), RunStateError> {
    let refused = |compatibility| RunStateError::Format {
        format: format.name,
        found: found.to_owned(),
        compatibility,
        install: None,
    };
    let version: ProtocolVersion = found
        .parse()
        .map_err(|_| refused(Compatibility::NewerMajor))?;
    match format.rule.compatibility(format.version, version) {
        Compatibility::Compatible => Ok(()),
        other => Err(refused(other)),
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
fn run_in_savepoint<T>(
    connection: &Connection,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> Result<T, Failed> {
    let ended = |error| Failed { error, ended: true };
    connection.execute_batch("SAVEPOINT write").map_err(ended)?;
    match work(connection) {
        Ok(answer) => {
            connection.execute_batch("RELEASE write").map_err(ended)?;
            Ok(answer)
        }
        Err(error) if is_storage(&error) || connection.is_autocommit() => Err(ended(error)),
        Err(error) => match connection.execute_batch("ROLLBACK TO write; RELEASE write") {
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
}

/// Commits `group` in one transaction under `epoch`, counts the commit in
/// `commits`, then answers each write: all of them with the failure when
/// the transaction fails or the file holds another epoch, and a full disk or
/// an I/O error also goes to `failures`.
fn commit_group(
    connection: &mut Connection,
    path: &Path,
    epoch: u64,
    group: Vec<QueuedWrite>,
    signals: &Signals<'_>,
) {
    let (completions, committed) = match commit_writes(connection, path, epoch, group) {
        Ok(completions) => {
            signals.commits.send_modify(|count| *count += 1);
            (completions, Ok(()))
        }
        Err((completions, error)) => {
            if let RunStateError::Storage { message, .. } = &error {
                tracing::warn!(path = %path.display(), %message, "a commit of run state met a full disk or an I/O error and was rolled back");
                signals.failures.send_if_modified(|failure| {
                    let first = failure.is_none();
                    if first {
                        *failure = Some(message.clone());
                    }
                    first
                });
            }
            (completions, Err(error))
        }
    };
    for completion in completions {
        completion(committed.clone());
    }
}

fn commit_writes(
    connection: &mut Connection,
    path: &Path,
    epoch: u64,
    group: Vec<QueuedWrite>,
) -> Result<Vec<Completion>, (Vec<Completion>, RunStateError)> {
    let refuse = |group: Vec<QueuedWrite>, error: RunStateError| {
        let completions = group.into_iter().map(|job| job(None).0).collect();
        (completions, error)
    };
    let transaction = match connection.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(transaction) => transaction,
        Err(error) => return Err(refuse(group, commit_error(path, &error))),
    };
    let found = transaction
        .query_row("SELECT epoch FROM run", [], |row| row.get::<_, i64>(0))
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            error => Err(error),
        });
    match found {
        Err(error) => return Err(refuse(group, commit_error(path, &error))),
        Ok(Some(found)) if i64::try_from(epoch).ok() != Some(found) => {
            let found = u64::try_from(found).unwrap_or_default();
            return Err(refuse(group, RunStateError::Fenced { held: epoch, found }));
        }
        Ok(_) => {}
    }
    let mut completions = Vec::with_capacity(group.len());
    let mut jobs = group.into_iter();
    let mut ended = None;
    for job in jobs.by_ref() {
        let (completion, ends_group) = job(Some((&transaction, path)));
        completions.push(completion);
        if ends_group.is_some() {
            ended = ends_group;
            break;
        }
    }
    let failed = match ended {
        // The rest of the group never runs: in a transaction SQLite ended,
        // each of its statements would commit on its own.
        Some(error) => {
            completions.extend(jobs.map(|job| job(None).0));
            Some(error)
        }
        None => transaction
            .execute("UPDATE run SET journal_position = journal_position + 1", [])
            .err()
            .map(|error| commit_error(path, &error)),
    };
    if let Some(error) = failed {
        // SQLite may have rolled the transaction back already after a full
        // disk or an I/O error; `ROLLBACK` makes the state the same either
        // way (ADR 0122).
        let _ = transaction.rollback();
        return Err((completions, error));
    }
    // A commit that fails is rolled back as its transaction drops.
    match transaction.commit() {
        Ok(()) => Ok(completions),
        Err(error) => Err((completions, commit_error(path, &error))),
    }
}

#[cfg(test)]
mod tests;
