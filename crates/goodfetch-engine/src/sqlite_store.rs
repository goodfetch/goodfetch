//! How the engine opens a store's SQLite file (ADRs 0002, 0030, 0060, 0122
//! and 0140): the one place that connects to one, through a VFS named by a
//! value (none in production, the crash VFS in tests), read-only or
//! read-write. A store that owns its file opens it whole through
//! [`File::open`]: its folder made, another engine's hold waited out, the
//! file switched to WAL through [`sqlite_wal`], every pragma of the
//! [`Durability`] its caller chose for this opening set, its commits
//! counted (ADR 0140), and in one transaction its `format` table and its
//! tables created and a format version this engine cannot read refused
//! (ADR 0030). It also says which SQLite failure is a full disk or an I/O
//! error (ADR 0122), which each store maps into its own typed error.
//!
//! Its calls block: callers run them under their owner's blocking policy,
//! or keep a file's connections in a [`KeptStore`], which runs them there.

mod kept;

use std::path::Path;
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Format};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::sqlite_wal;

pub(crate) use kept::{KeptStore, StoreFile};

/// The table that names a store's format and its version (ADR 0066), the
/// first of every store's tables.
const FORMAT_TABLE: &str = "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT";

/// A store's SQLite file: its path, and the VFS it opens through.
#[derive(Clone, Copy, Debug)]
pub(crate) struct File<'a> {
    path: &'a Path,
    vfs: Option<&'a str>,
}

/// What a connection may do to its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// Read it, taking no write lock.
    ReadOnly,
    /// Read and write it, as it already is.
    ReadWrite,
    /// Read and write it, creating it when it is not there.
    Create,
}

/// Whether [`File::open`] makes the file's folder first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Folder {
    /// Made with its parents when it is not there.
    Make,
    /// Left as it is: its caller made it, or the VFS keeps no folders.
    Exists,
}

/// How long an opening waits while another connection holds the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    /// Every statement of the connection waits up to this, its switch to
    /// WAL among them.
    Busy(Duration),
    /// Only its switch to WAL waits, up to this: a later statement meets
    /// another connection's hold as busy at once.
    Switch(Duration),
}

/// A connection's `synchronous` level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Synchronous {
    /// Nothing synced.
    Off,
    /// In WAL mode, a commit survives a process crash, and an OS crash or a
    /// power cut may roll it back without damaging the file.
    Normal,
    /// A commit survives a power cut.
    Full,
}

impl Synchronous {
    /// The pragma's value.
    pub(crate) fn pragma(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
        }
    }
}

/// Which syncs macOS's `F_FULLFSYNC` backs, which reach the disk's own
/// cache's contents too: elsewhere a sync is one already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FullFsync {
    /// None.
    Off,
    /// Its commits' (`fullfsync`).
    Commits,
    /// Its commits' and its checkpoints' (`fullfsync` and
    /// `checkpoint_fullfsync`).
    CommitsAndCheckpoints,
}

/// What a connection does with the write-ahead log as it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalOnClose {
    /// SQLite's default: the file's last connection copies it into the file,
    /// which syncs both.
    Checkpointed,
    /// Left as it is ([`no_checkpoint_on_close`], #2110).
    Kept,
}

/// How a connection's commits reach the disk, which each caller chooses as
/// it opens one: a store's own, or a Run's Durability level for its run
/// file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Durability {
    /// The level of its commits.
    pub(crate) synchronous: Synchronous,
    /// The level of the commit [`File::open`] creates the store's tables in:
    /// unsynced where the first commit that matters syncs them, with the
    /// WAL's header (#2117).
    pub(crate) opening: Synchronous,
    /// Which of its syncs `F_FULLFSYNC` backs on macOS.
    pub(crate) full_fsync: FullFsync,
    /// What it does with the WAL as it closes.
    pub(crate) wal_on_close: WalOnClose,
}

impl Durability {
    /// A synced per-user store's: each commit synced, through `F_FULLFSYNC`
    /// on macOS, and its WAL left as it is when a connection closes (#2110).
    pub(crate) const SYNCED: Self = Self {
        synchronous: Synchronous::Full,
        opening: Synchronous::Full,
        full_fsync: FullFsync::Commits,
        wal_on_close: WalOnClose::Kept,
    };

    /// Sets every pragma of this durability on `connection` but its opening
    /// commit's.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub(crate) fn apply(self, connection: &Connection) -> rusqlite::Result<()> {
        connection.pragma_update(None, "synchronous", self.synchronous.pragma())?;
        if cfg!(target_os = "macos") {
            if self.full_fsync != FullFsync::Off {
                connection.pragma_update(None, "fullfsync", "ON")?;
            }
            if self.full_fsync == FullFsync::CommitsAndCheckpoints {
                connection.pragma_update(None, "checkpoint_fullfsync", "ON")?;
            }
        }
        if self.wal_on_close == WalOnClose::Kept {
            no_checkpoint_on_close(connection)?;
        }
        Ok(())
    }
}

/// Where a store counts its connections' work (ADR 0140): beneath every
/// owner, so a commit beside its own calls' counts too.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Counting<'a> {
    /// The counted work.
    pub(crate) work: &'a CountedWork,
    /// The row each connection opened counts on, if any.
    pub(crate) opens: Option<Row>,
    /// The row each commit counts on.
    pub(crate) transactions: Row,
    /// Whether a new file's switch to WAL, which commits its first page,
    /// counts among its commits, as each store's counter was first measured
    /// (#1881): the Audit log's, the Usage ledger's and the Politeness
    /// file's do, the run file's does not.
    pub(crate) counts_switch: bool,
}

impl Counting<'_> {
    /// Has each commit of `connection` count on [`Counting::transactions`].
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub(crate) fn count_commits(&self, connection: &Connection) -> rusqlite::Result<()> {
        let work = self.work.clone();
        let row = self.transactions;
        connection.commit_hook(Some(move || {
            work.count(row);
            false
        }))
    }
}

/// A store's format and the statements that create its tables.
#[derive(Clone, Copy)]
pub(crate) struct Schema<'a> {
    /// Its format, whose name and version the `format` table holds.
    pub(crate) format: &'static Format,
    /// The statements run after the `format` table is created, in order:
    /// the store's own tables, then each contribution's.
    pub(crate) statements: &'a [&'a [&'a str]],
}

impl Schema<'_> {
    /// Creates the `format` table and the store's tables on `connection` in
    /// one `IMMEDIATE` transaction, records the format's version in a file
    /// that holds none, and refuses one this engine cannot read, which rolls
    /// the transaction back.
    ///
    /// # Errors
    ///
    /// `Sqlite` when SQLite fails, `Format` for a version this engine
    /// cannot read.
    pub(crate) fn create(&self, connection: &mut Connection) -> Result<(), OpenError> {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(OpenError::Sqlite)?;
        let statements = std::iter::once(FORMAT_TABLE).chain(
            self.statements
                .iter()
                .flat_map(|statements| statements.iter().copied()),
        );
        for statement in statements {
            transaction
                .execute(statement, [])
                .map_err(OpenError::Sqlite)?;
        }
        transaction
            .execute(
                "INSERT INTO format (name, version) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING",
                (self.format.name, self.format.version.to_string()),
            )
            .map_err(OpenError::Sqlite)?;
        let found: String = transaction
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [self.format.name],
                |row| row.get(0),
            )
            .map_err(OpenError::Sqlite)?;
        check_format(self.format, &found).map_err(OpenError::Format)?;
        transaction.commit().map_err(OpenError::Sqlite)
    }
}

/// How [`File::open`] opens a store's file.
#[derive(Clone, Copy)]
pub(crate) struct Opening<'a> {
    /// Whether it makes the file's folder first.
    pub(crate) folder: Folder,
    /// [`Access::Create`], or [`Access::ReadWrite`] for a file that must be
    /// there already.
    pub(crate) access: Access,
    /// How long it waits while another connection holds the file.
    pub(crate) wait: Wait,
    /// How its commits reach the disk.
    pub(crate) durability: Durability,
    /// Where it counts its work, if anywhere.
    pub(crate) counting: Option<Counting<'a>>,
    /// The tables it creates and the format it checks, for a store that
    /// keeps them in the `format` table; none for one that creates its own.
    pub(crate) schema: Option<Schema<'a>>,
}

/// Why [`File::open`] did not open a store.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OpenError {
    /// Its folder could not be made.
    #[error(transparent)]
    Folder(std::io::Error),
    /// SQLite could not open the file.
    #[error(transparent)]
    Unopened(rusqlite::Error),
    /// SQLite failed once it opened the file.
    #[error(transparent)]
    Sqlite(rusqlite::Error),
    /// The file is of a format version this engine cannot read.
    #[error("the file is {} {}, which this engine cannot read", .0.format, .0.found)]
    Format(FormatRefusal),
}

/// A store whose format version this engine cannot read (ADR 0030).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FormatRefusal {
    /// The format's name.
    pub(crate) format: &'static str,
    /// The file's version.
    pub(crate) found: String,
    /// What this engine makes of it.
    pub(crate) compatibility: Compatibility,
}

impl<'a> File<'a> {
    /// The file at `path`, opened through the VFS `vfs`.
    pub(crate) fn new(path: &'a Path, vfs: Option<&'a str>) -> Self {
        Self { path, vfs }
    }

    /// A connection to the file with `access`, and nothing more set.
    ///
    /// # Errors
    ///
    /// SQLite's error, `CannotOpen` for a file not there without
    /// [`Access::Create`].
    pub(crate) fn connect(&self, access: Access) -> rusqlite::Result<Connection> {
        let flags = OpenFlags::SQLITE_OPEN_NO_MUTEX
            | match access {
                Access::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY,
                Access::ReadWrite => OpenFlags::SQLITE_OPEN_READ_WRITE,
                Access::Create => OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            };
        match self.vfs {
            Some(vfs) => Connection::open_with_flags_and_vfs(self.path, flags, vfs),
            None => Connection::open_with_flags(self.path, flags),
        }
    }

    /// As [`File::connect`], none when SQLite cannot open the file, as
    /// when it is not there.
    ///
    /// # Errors
    ///
    /// SQLite's other errors.
    pub(crate) fn connect_existing(&self, access: Access) -> rusqlite::Result<Option<Connection>> {
        match self.connect(access) {
            Ok(connection) => Ok(Some(connection)),
            Err(error) if error.sqlite_error_code() == Some(rusqlite::ErrorCode::CannotOpen) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Opens the store's file as `opening` says: makes its folder, connects,
    /// counts the opening, waits out another connection's switch to WAL or
    /// its own, sets every pragma of its durability, and creates its
    /// tables in a commit at its opening level, checking its format.
    ///
    /// # Errors
    ///
    /// `Folder` when its folder cannot be made, `Unopened` when SQLite
    /// cannot open it, `Sqlite` when it fails after, `Format` for a format
    /// version this engine cannot read.
    pub(crate) fn open(&self, opening: &Opening<'_>) -> Result<Connection, OpenError> {
        if opening.folder == Folder::Make
            && let Some(folder) = self.path.parent()
        {
            #[expect(
                clippy::disallowed_methods,
                reason = "a store's calls run under their caller's blocking policy"
            )]
            std::fs::create_dir_all(folder).map_err(OpenError::Folder)?;
        }
        let mut connection = self.connect(opening.access).map_err(OpenError::Unopened)?;
        if let Some(counting) = &opening.counting {
            if let Some(row) = counting.opens {
                counting.work.count(row);
            }
            if counting.counts_switch {
                counting
                    .count_commits(&connection)
                    .map_err(OpenError::Sqlite)?;
            }
        }
        let timeout = match opening.wait {
            Wait::Busy(timeout) => {
                connection
                    .busy_timeout(timeout)
                    .map_err(OpenError::Sqlite)?;
                timeout
            }
            Wait::Switch(timeout) => timeout,
        };
        sqlite_wal::enter(&connection, timeout).map_err(OpenError::Sqlite)?;
        if let Some(counting) = &opening.counting
            && !counting.counts_switch
        {
            counting
                .count_commits(&connection)
                .map_err(OpenError::Sqlite)?;
        }
        let durability = opening.durability;
        durability.apply(&connection).map_err(OpenError::Sqlite)?;
        if let Some(schema) = &opening.schema {
            let apart = durability.opening != durability.synchronous;
            if apart {
                set_synchronous(&connection, durability.opening)?;
            }
            schema.create(&mut connection)?;
            if apart {
                set_synchronous(&connection, durability.synchronous)?;
            }
        }
        Ok(connection)
    }
}

/// Sets `connection`'s `synchronous` level.
fn set_synchronous(connection: &Connection, level: Synchronous) -> Result<(), OpenError> {
    connection
        .pragma_update(None, "synchronous", level.pragma())
        .map_err(OpenError::Sqlite)
}

/// Refuses a file of `format` at the version `found` that this engine
/// cannot read, under the format's rule (ADR 0030): an unparsable version
/// counts as a newer major.
///
/// # Errors
///
/// The refusal.
pub(crate) fn check_format(format: &'static Format, found: &str) -> Result<(), FormatRefusal> {
    let refused = |compatibility| FormatRefusal {
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

/// Whether `error` is a full disk or an I/O error, after which SQLite may
/// have rolled the whole transaction back (ADR 0122). A read that met an
/// I/O error (EIO) counts: SQLite's Unix VFS reports it as
/// `SQLITE_IOERR_CORRUPTFS`, which reaches its caller as `SQLITE_CORRUPT`.
pub(crate) fn is_storage(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(
            rusqlite::ErrorCode::DiskFull
                | rusqlite::ErrorCode::SystemIoFailure
                | rusqlite::ErrorCode::DatabaseCorrupt
        )
    )
}

/// Has `connection` leave the write-ahead log as it is when it closes,
/// rather than copy it into the file, which syncs both, when it is the
/// file's last connection (#2110). A commit at `synchronous=FULL` already
/// synced the log, so what it holds survives a power cut, and the next
/// connection reads it; SQLite's automatic checkpoint still copies it into
/// the file once it passes its thousand pages, so it does not grow without
/// bound.
///
/// # Errors
///
/// SQLite's error.
pub(crate) fn no_checkpoint_on_close(connection: &Connection) -> rusqlite::Result<()> {
    connection
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .map(drop)
}

/// Runs `statement`, which binds nothing, through `connection`'s statement
/// cache: a writer's transactions and savepoints prepare once per
/// connection, not once per group, job or entry, as rusqlite's own
/// transactions do (ADR 0138).
pub(crate) fn run_cached(connection: &Connection, statement: &str) -> rusqlite::Result<()> {
    connection.prepare_cached(statement)?.execute([]).map(drop)
}

/// Runs `work` in one `BEGIN IMMEDIATE` transaction of `connection`, its
/// `BEGIN`, `COMMIT` and `ROLLBACK` through the statement cache, as
/// [`run_cached`] runs them: commits once `work` succeeds, and rolls back
/// when it or the commit fails. A rollback that fails too leaves the
/// transaction open, so its caller lets `connection` go, which ends it.
///
/// # Errors
///
/// `work`'s error, or SQLite's at the `BEGIN` or the `COMMIT`.
pub(crate) fn immediate<T, E: From<rusqlite::Error>>(
    connection: &Connection,
    work: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E> {
    transaction(connection, "BEGIN IMMEDIATE", work)
}

/// Runs `work` in one deferred transaction of `connection`, which takes no
/// write lock until `work` writes, as [`immediate`] runs its own.
///
/// # Errors
///
/// As [`immediate`].
pub(crate) fn deferred<T, E: From<rusqlite::Error>>(
    connection: &Connection,
    work: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E> {
    transaction(connection, "BEGIN", work)
}

fn transaction<T, E: From<rusqlite::Error>>(
    connection: &Connection,
    begin: &str,
    work: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E> {
    run_cached(connection, begin)?;
    let done =
        work(connection).and_then(|answer| Ok(run_cached(connection, "COMMIT").map(|()| answer)?));
    if done.is_err() && !connection.is_autocommit() {
        let _ = run_cached(connection, "ROLLBACK");
    }
    done
}

#[cfg(test)]
mod tests;
