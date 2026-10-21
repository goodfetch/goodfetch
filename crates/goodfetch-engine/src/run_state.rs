//! A Run's own SQLite file (ADRs 0002, 0003 and 0060): bundled SQLite in WAL
//! mode, `synchronous=FULL` with `fullfsync` on macOS, so an acknowledged
//! commit survives a power cut, and one writer, an actor of the run's owner
//! that commits every write its mailbox holds in one transaction. Each
//! commit is accepted only under the writer's run epoch: a writer whose
//! epoch the file no longer holds commits nothing. The file opens through a
//! VFS named by a value, none in production, the crash VFS in tests. Each
//! acknowledged commit moves a counter on, which a reader of the file's
//! committed state waits on for more, never holding the writer back.
//!
//! The file holds its format's `major.minor` (ADR 0066), the Run's own row
//! and tags, and the tables each contribution of the composition adds, such
//! as a crawl's work items.

use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Format, RUN_STATE};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use tokio::sync::{mpsc, oneshot, watch};

use crate::tasks::TaskOwner;

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
        weakened_by TEXT NOT NULL DEFAULT '[]',
        journal_position INTEGER NOT NULL DEFAULT 0,
        binding TEXT,
        clean_up_digest TEXT,
        progress_message TEXT
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT",
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

/// How a run file opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunStateOptions {
    /// The name of the SQLite VFS it opens through: none in production, the
    /// crash VFS in tests (ADR 0060).
    pub vfs: Option<String>,
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
    /// The file is of a format version this engine cannot read (ADR 0066).
    #[error("the file is {format} {found}, which this engine cannot read")]
    Format {
        /// The format's name.
        format: &'static str,
        /// The file's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
    /// The writer has stopped.
    #[error("the run file's writer has stopped")]
    Closed,
}

type Completion = Box<dyn FnOnce(Result<(), RunStateError>) + Send>;
/// A write: given the transaction, it runs its work and answers how to
/// report it once the commit ends; given none, it reports the failure only.
type QueuedWrite = Box<dyn FnOnce(Option<(&Connection, &Path)>) -> Completion + Send>;

/// The writer of one run file: a handle to its actor, which any task may
/// clone and send writes to.
#[derive(Clone, Debug)]
pub struct RunState {
    mailbox: mpsc::Sender<QueuedWrite>,
    path: PathBuf,
    epoch: u64,
    commits: watch::Receiver<u64>,
}

impl RunState {
    /// Opens the run file at `path` as its writer under run epoch `epoch`,
    /// creating it and the tables of `tables` when it does not exist, its
    /// actor one of `owner`'s. The writer stops once every handle is
    /// dropped, and `owner` waits for it.
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
        let vfs = options.vfs.clone();
        let opened_path = path.clone();
        let connection = owner
            .blocking(move || open_file(&opened_path, vfs.as_deref(), &tables))
            .await?;
        let (mailbox, jobs) = mpsc::channel(MAILBOX);
        let actor_path = path.clone();
        let mut connection = connection;
        let (committed, commits) = watch::channel(0_u64);
        owner
            .actor("run-state-writer", jobs, move |group: Vec<QueuedWrite>| {
                commit_group(&mut connection, &actor_path, epoch, group, &committed);
            })
            .map_err(|error| sqlite_error(&path, &error))?;
        Ok(Self {
            mailbox,
            path,
            epoch,
            commits,
        })
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
        self.commits.clone()
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
        let (answer, answered) = oneshot::channel();
        let job: QueuedWrite = Box::new(move |transaction: Option<(&Connection, &Path)>| {
            let done = match transaction {
                Some((connection, path)) => {
                    run_in_savepoint(connection, work).map_err(|error| sqlite_error(path, &error))
                }
                None => Err(RunStateError::Closed),
            };
            Box::new(move |committed: Result<(), RunStateError>| {
                let _ = answer.send(committed.and(done));
            })
        });
        self.mailbox
            .send(job)
            .await
            .map_err(|_| RunStateError::Closed)?;
        answered.await.map_err(|_| RunStateError::Closed)?
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
    let connection = open_file(path, options.vfs.as_deref(), tables)?;
    let epoch: i64 = connection
        .query_row(
            "UPDATE run SET epoch = max(epoch, ?1) + 1 RETURNING epoch",
            [i64::try_from(after).unwrap_or(i64::MAX)],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_error(path, &error))?;
    Ok(u64::try_from(epoch).unwrap_or_default())
}

fn sqlite_error(path: &Path, error: &impl std::fmt::Display) -> RunStateError {
    RunStateError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
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
    vfs: Option<&str>,
    tables: &[RunStateTables],
) -> Result<Connection, RunStateError> {
    let failed = |error: rusqlite::Error| sqlite_error(path, &error);
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut connection = match vfs {
        Some(vfs) => Connection::open_with_flags_and_vfs(path, flags, vfs),
        None => Connection::open_with_flags(path, flags),
    }
    .map_err(failed)?;
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .map_err(failed)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
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
    };
    let version: ProtocolVersion = found
        .parse()
        .map_err(|_| refused(Compatibility::NewerMajor))?;
    match format.rule.compatibility(format.version, version) {
        Compatibility::Compatible => Ok(()),
        other => Err(refused(other)),
    }
}

/// Runs `work` in a savepoint of its own, which an error rolls back.
fn run_in_savepoint<T>(
    connection: &Connection,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    connection.execute_batch("SAVEPOINT write")?;
    match work(connection) {
        Ok(answer) => {
            connection.execute_batch("RELEASE write")?;
            Ok(answer)
        }
        Err(error) => {
            connection.execute_batch("ROLLBACK TO write; RELEASE write")?;
            Err(error)
        }
    }
}

/// Commits `group` in one transaction under `epoch`, counts the commit in
/// `commits`, then answers each write: all of them with the failure when
/// the transaction fails or the file holds another epoch.
fn commit_group(
    connection: &mut Connection,
    path: &Path,
    epoch: u64,
    group: Vec<QueuedWrite>,
    commits: &watch::Sender<u64>,
) {
    let (completions, committed) = match commit_writes(connection, path, epoch, group) {
        Ok(completions) => {
            commits.send_modify(|count| *count += 1);
            (completions, Ok(()))
        }
        Err((completions, error)) => (completions, Err(error)),
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
        let completions = group.into_iter().map(|job| job(None)).collect();
        (completions, error)
    };
    let transaction = match connection.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(transaction) => transaction,
        Err(error) => return Err(refuse(group, sqlite_error(path, &error))),
    };
    let found = transaction
        .query_row("SELECT epoch FROM run", [], |row| row.get::<_, i64>(0))
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            error => Err(error),
        });
    match found {
        Err(error) => return Err(refuse(group, sqlite_error(path, &error))),
        Ok(Some(found)) if i64::try_from(epoch).ok() != Some(found) => {
            let found = u64::try_from(found).unwrap_or_default();
            return Err(refuse(group, RunStateError::Fenced { held: epoch, found }));
        }
        Ok(_) => {}
    }
    let completions: Vec<Completion> = group
        .into_iter()
        .map(|job| job(Some((&transaction, path))))
        .collect();
    if let Err(error) =
        transaction.execute("UPDATE run SET journal_position = journal_position + 1", [])
    {
        return Err((completions, sqlite_error(path, &error)));
    }
    match transaction.commit() {
        Ok(()) => Ok(completions),
        Err(error) => Err((completions, sqlite_error(path, &error))),
    }
}

#[cfg(test)]
mod tests;
