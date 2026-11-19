//! The Run index (ADRs 0031 and 0098): the per-user, per-major SQLite
//! catalog of every Run of either Product, which every engine writes when a
//! Run starts and when one of its axes changes, each row pointing at the
//! Run's own state, from which it can be rebuilt, naming the engine that
//! owns the Run while it is live, and saying whether its state is there.
//! Beside the Runs it keeps the items of "Needs you" a person dismissed,
//! so every engine of the major hides them (spec #592), and the domains
//! whose consent wall each Run met, which "Needs you" offers for
//! `consent.domains` (ADR 0048), and those on which a Run's refetch without
//! the site's stored data stopped, which "Needs you" lists (ADR 0052).
//! Several engines write it; WAL and a busy timeout let them take turns.
//! An engine opens it once, as a [`RunIndex`] (#1886): its writes on a
//! writer, an actor of the engine's owner that keeps its connection, and
//! its reads on that owner's blocking threads, on one connection they keep
//! beside the writer's, so a read never waits behind a write, and every
//! statement is prepared once on each. `runs reindex`, which runs without
//! an engine, opens it through an [`IndexFile`] whose calls block.
//!
//! Engines of several minors of the major open it at once, so it records
//! the lowest minor that can use it (ADR 0098), which each call reads and
//! checks in its own transaction: an engine that finds it raised past
//! itself stops using the index with the version refusal.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::RUN_INDEX;
use goodfetch_protocol::run::{Completeness, Lifecycle, Presence, RunId, RunIndexRow, Verdict};
use rusqlite::{Connection, OptionalExtension as _, Row};

use crate::lowest_minor::{self, Opener};
use crate::run_state::RunStateError;
use crate::sqlite_store::{
    self, Access, Durability, File, Folder, KeptStore, Opening, StoreFile, Wait,
};
use crate::stored_time::StoredTime;
use crate::tasks::{ActorStopped, TaskOwner};

/// How long a write waits for another engine's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &[&str] = &[
    lowest_minor::FORMAT_TABLE,
    "CREATE TABLE IF NOT EXISTS runs (
        run_id TEXT NOT NULL PRIMARY KEY,
        location TEXT NOT NULL,
        kind TEXT NOT NULL,
        needs TEXT NOT NULL,
        tags TEXT NOT NULL,
        lifecycle TEXT NOT NULL,
        verdict TEXT,
        completeness TEXT,
        started_at TEXT NOT NULL,
        ended_at TEXT,
        owner TEXT,
        presence TEXT NOT NULL,
        progress_message TEXT,
        stop_reason TEXT,
        replayed INTEGER NOT NULL DEFAULT 0
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS needs_you_dismissed (
        item TEXT NOT NULL PRIMARY KEY,
        dismissed_at TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS consent_walls (
        domain TEXT NOT NULL,
        run_id TEXT NOT NULL,
        seen_at TEXT NOT NULL,
        PRIMARY KEY (domain, run_id)
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS refetch_stops (
        domain TEXT NOT NULL,
        run_id TEXT NOT NULL,
        cause TEXT NOT NULL,
        project_file TEXT,
        stopped_at TEXT NOT NULL,
        PRIMARY KEY (domain, run_id)
    ) STRICT",
];

/// A row's columns, in the order [`row_of`] reads them.
macro_rules! columns {
    () => {
        "run_id, location, kind, needs, tags, lifecycle, verdict, completeness, started_at, ended_at, owner, presence, progress_message, stop_reason, replayed"
    };
}

/// A Run's row, written in place of its earlier one.
const INSERT: &str = concat!(
    "INSERT INTO runs (",
    columns!(),
    ")
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
     ON CONFLICT (run_id) DO UPDATE SET
        location = excluded.location, kind = excluded.kind, needs = excluded.needs,
        tags = excluded.tags, lifecycle = excluded.lifecycle, verdict = excluded.verdict,
        completeness = excluded.completeness, started_at = excluded.started_at,
        ended_at = excluded.ended_at, owner = excluded.owner, presence = excluded.presence,
        progress_message = excluded.progress_message, stop_reason = excluded.stop_reason,
        replayed = excluded.replayed"
);

/// One Run's row.
const SELECT_ONE: &str = concat!("SELECT ", columns!(), " FROM runs WHERE run_id = ?1");

/// Every row, in Run ID order.
const SELECT_ALL: &str = concat!("SELECT ", columns!(), " FROM runs ORDER BY run_id");

/// The rows of the Runs a storage stop left stopped (ADR 0122) and present.
const STOPPED_BY_STORAGE: &str = concat!(
    "SELECT ",
    columns!(),
    " FROM runs
     WHERE lifecycle = ?1 AND presence = ?2
       AND json_extract(stop_reason, '$.kind') = 'resources'
       AND json_extract(stop_reason, '$.resource') = 'storage'
     ORDER BY run_id"
);

/// The rows of the Runs a Limit stopped (ADR 0041) and present.
const STOPPED_BY_LIMIT: &str = concat!(
    "SELECT ",
    columns!(),
    " FROM runs
     WHERE lifecycle = ?1 AND presence = ?2 AND json_extract(stop_reason, '$.kind') = 'limit'
     ORDER BY run_id"
);

fn failed(path: &Path, error: &impl std::fmt::Display) -> RunStateError {
    RunStateError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

/// The Run index of one user and major, as one engine opens it: its writer
/// and the connection its reads keep, which its clones share.
#[derive(Clone, Debug)]
pub(crate) struct RunIndex {
    store: Arc<KeptStore<IndexFile>>,
}

impl RunIndex {
    /// The index at `path`, as the engine of Product version `engine` opens
    /// it through the SQLite VFS named `vfs` (none in production, ADR
    /// 0060), its writer an actor of `owner` and its reads on `owner`'s
    /// blocking threads.
    pub(crate) fn of(
        path: PathBuf,
        engine: &ProductVersion,
        vfs: Option<String>,
        owner: Arc<TaskOwner>,
    ) -> Self {
        let file = IndexFile::new(path, opener(engine), vfs);
        Self {
            store: Arc::new(KeptStore::of(file, "run-index-writer", owner)),
        }
    }

    /// Writes `row`, replacing the Run's earlier row.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the index cannot be opened or written, or its writer
    /// stopped, `Format` when its marker refuses this engine.
    pub(crate) async fn write(&self, row: RunIndexRow) -> Result<(), RunStateError> {
        self.store
            .write(None, move |file, connection| file.write(connection, &row))
            .await
    }

    /// The row of `run_id`, if the index lists the Run.
    ///
    /// # Errors
    ///
    /// As [`Self::write`], when the index cannot be opened or read.
    pub(crate) async fn read(&self, run_id: RunId) -> Result<Option<RunIndexRow>, RunStateError> {
        self.store
            .read(None, move |file, connection| file.read(connection, &run_id))
            .await
    }

    /// Every row, in Run ID order.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn rows(&self) -> Result<Vec<RunIndexRow>, RunStateError> {
        self.store.read(None, IndexFile::rows).await
    }

    /// Whether this engine may use the index, as a Run's start asks before
    /// it writes its row.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn check(&self) -> Result<(), RunStateError> {
        self.store.read(None, IndexFile::check).await
    }

    /// Marks the row of the discarded Run `run_id` *erased*, as
    /// [`IndexFile::erase`] does, and answers whether the index listed it.
    ///
    /// # Errors
    ///
    /// As [`IndexFile::erase`], and as [`Self::write`] when its writer
    /// stopped.
    pub(crate) async fn erase(&self, run_id: RunId) -> Result<bool, RunStateError> {
        self.store
            .write(None, move |file, connection| {
                file.erase(connection, &run_id)
            })
            .await
    }

    /// The rows of the Runs a storage stop left stopped (ADR 0122), present,
    /// in Run ID order: "Needs you" lists them. An index not made yet lists
    /// none, and is not made.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn stopped_by_storage(&self) -> Result<Vec<RunIndexRow>, RunStateError> {
        self.store
            .read(Some(Vec::new()), |file, connection| {
                file.stopped_where(connection, STOPPED_BY_STORAGE)
            })
            .await
    }

    /// The rows of the Runs a Limit stopped (ADR 0041), present, in Run ID
    /// order: "Needs you" lists those whose Limit still has nothing left.
    /// An index not made yet lists none, and is not made.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn stopped_by_limit(&self) -> Result<Vec<RunIndexRow>, RunStateError> {
        self.store
            .read(Some(Vec::new()), |file, connection| {
                file.stopped_where(connection, STOPPED_BY_LIMIT)
            })
            .await
    }

    /// The items of "Needs you" a person dismissed, as the index keeps them,
    /// so that every engine of the major hides them. An index not made yet
    /// keeps none, and is not made.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn dismissed(&self) -> Result<BTreeSet<String>, RunStateError> {
        self.store
            .read(Some(BTreeSet::new()), |file, connection| {
                file.in_transaction(connection, false, |connection| {
                    connection
                        .prepare_cached("SELECT item FROM needs_you_dismissed")?
                        .query_map([], |row| row.get(0))?
                        .collect()
                })
            })
            .await
    }

    /// Records that a person dismissed the item `item` of "Needs you" at
    /// `at`.
    ///
    /// # Errors
    ///
    /// As [`Self::write`].
    pub(crate) async fn dismiss(&self, item: String, at: SystemTime) -> Result<(), RunStateError> {
        self.store
            .write(None, move |file, connection| {
                file.in_transaction(connection, true, |connection| {
                    connection
                        .prepare_cached(
                            "INSERT INTO needs_you_dismissed (item, dismissed_at) VALUES (?1, ?2)
                             ON CONFLICT (item) DO NOTHING",
                        )?
                        .execute((item.as_str(), StoredTime(at)))
                        .map(drop)
                })
            })
            .await
    }

    /// Forgets the dismissals of `items`, which no source lists any longer.
    ///
    /// # Errors
    ///
    /// As [`Self::write`].
    pub(crate) async fn forget_dismissed(&self, items: Vec<String>) -> Result<(), RunStateError> {
        self.store
            .write(None, move |file, connection| {
                file.in_transaction(connection, true, |connection| {
                    let mut forget = connection
                        .prepare_cached("DELETE FROM needs_you_dismissed WHERE item = ?1")?;
                    for item in &items {
                        forget.execute([item])?;
                    }
                    Ok(())
                })
            })
            .await
    }

    /// Records that the Run `run_id` met a consent wall on `domain` at `at`,
    /// keeping the first time it did.
    ///
    /// # Errors
    ///
    /// As [`Self::write`].
    pub(crate) async fn record_consent_wall(
        &self,
        domain: String,
        run_id: RunId,
        at: SystemTime,
    ) -> Result<(), RunStateError> {
        self.store
            .write(None, move |file, connection| {
                file.in_transaction(connection, true, |connection| {
                    connection
                        .prepare_cached(
                            "INSERT INTO consent_walls (domain, run_id, seen_at) VALUES (?1, ?2, ?3)
                             ON CONFLICT (domain, run_id) DO NOTHING",
                        )?
                        .execute((domain.as_str(), run_id.as_str(), StoredTime(at)))
                        .map(drop)
                })
            })
            .await
    }

    /// Each domain whose consent wall a Run met, in domain order, with when
    /// one first met it and the last that did. An index not made yet keeps
    /// none, and is not made.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn consent_walls(&self) -> Result<Vec<ConsentWallSeen>, RunStateError> {
        self.store
            .read(Some(Vec::new()), |file, connection| {
                file.in_transaction(connection, false, consent_walls_in)
            })
            .await
    }

    /// Records that the refetch of the Run `stop.last_run` stopped on
    /// `stop.domain` at `at`, for its cause, keeping the Run's first stop.
    ///
    /// # Errors
    ///
    /// As [`Self::write`].
    pub(crate) async fn record_refetch_stop(
        &self,
        stop: RefetchStopped,
        at: SystemTime,
    ) -> Result<(), RunStateError> {
        self.store
            .write(None, move |file, connection| {
                file.in_transaction(connection, true, |connection| {
                    connection
                        .prepare_cached(
                            "INSERT INTO refetch_stops (domain, run_id, cause, project_file, stopped_at)
                             VALUES (?1, ?2, ?3, ?4, ?5)
                             ON CONFLICT (domain, run_id) DO NOTHING",
                        )?
                        .execute((
                            stop.domain.as_str(),
                            stop.last_run.as_str(),
                            stop.cause.as_str(),
                            stop.project_file.as_deref(),
                            StoredTime(at),
                        ))
                        .map(drop)
                })
            })
            .await
    }

    /// Each domain on which a Run's refetch stopped, in domain order, with
    /// when one first stopped there, and the last Run that did with its
    /// cause and project configuration file. An index not made yet keeps
    /// none, and is not made.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub(crate) async fn refetch_stops(&self) -> Result<Vec<RefetchStopped>, RunStateError> {
        self.store
            .read(Some(Vec::new()), |file, connection| {
                file.in_transaction(connection, false, refetch_stops_in)
            })
            .await
    }
}

/// The Run index's file as one engine opens it: its path, the engine as an
/// opener of its format, and the VFS it opens through.
#[derive(Debug)]
pub(crate) struct IndexFile {
    path: PathBuf,
    opener: Opener,
    vfs: Option<String>,
}

/// What a transaction of the index fails with: SQLite's error, or its
/// marker's refusal of this engine.
enum Failure {
    Sqlite(rusqlite::Error),
    Refused(RunStateError),
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl StoreFile for IndexFile {
    type Error = RunStateError;

    fn path(&self) -> &Path {
        &self.path
    }

    /// Opens the index, its tables made outside a transaction, as its
    /// lowest-minor marker is checked in each transaction instead (ADR
    /// 0098). Only a first open writes: a marker already there takes no
    /// write lock, so a read never waits behind another engine's write.
    fn open(&self) -> Result<Connection, RunStateError> {
        let failed = |error: &dyn std::fmt::Display| failed(&self.path, &error.to_string());
        let connection = File::new(&self.path, self.vfs.as_deref())
            .open(&Opening {
                folder: Folder::Make,
                access: Access::Create,
                wait: Wait::Busy(BUSY_TIMEOUT),
                durability: Durability::SYNCED,
                counting: None,
                schema: None,
            })
            .map_err(|error| failed(&error))?;
        for statement in SCHEMA {
            sqlite_store::run_cached(&connection, statement).map_err(|error| failed(&error))?;
        }
        let marked = lowest_minor::read_sqlite(&connection, RUN_INDEX.name)
            .map_err(|error| failed(&error))?;
        if marked.is_some() {
            return Ok(connection);
        }
        let created = self.opener.created();
        connection
            .prepare_cached(
                "INSERT INTO format (name, version, lowest_minor) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (name) DO NOTHING",
            )
            .and_then(|mut statement| {
                statement.execute((
                    RUN_INDEX.name,
                    created.version.to_string(),
                    i64::from(created.lowest_minor),
                ))
            })
            .map_err(|error| failed(&error))?;
        Ok(connection)
    }

    /// As [`StoreFile::open`]: a read that opens a new index records its
    /// marker, and its deferred transactions take no write lock.
    fn open_read(&self) -> Result<Connection, RunStateError> {
        self.open()
    }

    fn stopped(&self, stopped: &ActorStopped) -> RunStateError {
        failed(&self.path, stopped)
    }
}

impl IndexFile {
    /// The index at `path`, as the engine of Product version `engine` opens
    /// it without an engine of its own, as `runs reindex` does.
    pub(crate) fn of(path: PathBuf, engine: &ProductVersion) -> Self {
        Self::new(path, opener(engine), None)
    }

    fn new(path: PathBuf, opener: Opener, vfs: Option<String>) -> Self {
        Self { path, opener, vfs }
    }

    /// Runs `work` in one transaction of `connection`, after checking the
    /// marker in that transaction (ADR 0098); a write records this engine's
    /// format version in the marker.
    fn in_transaction<T>(
        &self,
        connection: &Connection,
        writes: bool,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, RunStateError> {
        let checked = |connection: &Connection| -> Result<T, Failure> {
            let found = lowest_minor::read_sqlite(connection, RUN_INDEX.name)?
                .unwrap_or_else(|| self.opener.created());
            self.opener
                .check(&found)
                .map_err(|refusal| Failure::Refused(RunStateError::refused(refusal)))?;
            let answer = work(connection)?;
            if writes {
                let marked = self.opener.written(found, None);
                if marked != found {
                    lowest_minor::write_sqlite(connection, RUN_INDEX.name, marked)?;
                }
            }
            Ok(answer)
        };
        let done = if writes {
            sqlite_store::immediate(connection, checked)
        } else {
            sqlite_store::deferred(connection, checked)
        };
        done.map_err(|failure| match failure {
            Failure::Sqlite(error) => failed(&self.path, &error),
            Failure::Refused(refused) => refused,
        })
    }

    /// Writes `row` through `connection`, replacing the Run's earlier row.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the index cannot be written, `Format` when its marker
    /// refuses this engine.
    pub(crate) fn write(
        &self,
        connection: &Connection,
        row: &RunIndexRow,
    ) -> Result<(), RunStateError> {
        self.in_transaction(connection, true, |connection| insert(connection, row))
    }

    /// The row of `run_id`, read through `connection`.
    fn read(
        &self,
        connection: &Connection,
        run_id: &RunId,
    ) -> Result<Option<RunIndexRow>, RunStateError> {
        self.in_transaction(connection, false, |connection| {
            connection
                .prepare_cached(SELECT_ONE)?
                .query_row([run_id.as_str()], row_of)
                .optional()
        })
    }

    /// Every row, in Run ID order, read through `connection`.
    ///
    /// # Errors
    ///
    /// As [`Self::write`], when the index cannot be read.
    pub(crate) fn rows(&self, connection: &Connection) -> Result<Vec<RunIndexRow>, RunStateError> {
        self.in_transaction(connection, false, |connection| {
            connection
                .prepare_cached(SELECT_ALL)?
                .query_map([], row_of)?
                .collect()
        })
    }

    fn check(&self, connection: &Connection) -> Result<(), RunStateError> {
        self.in_transaction(connection, false, |_| Ok(()))
    }

    /// Replaces every row with `rows`, in one transaction.
    ///
    /// # Errors
    ///
    /// As [`Self::write`]; the index is then left as it was.
    pub(crate) fn replace(
        &self,
        connection: &Connection,
        rows: &[RunIndexRow],
    ) -> Result<(), RunStateError> {
        self.in_transaction(connection, true, |connection| {
            connection.prepare_cached("DELETE FROM runs")?.execute([])?;
            for row in rows {
                insert(connection, row)?;
            }
            Ok(())
        })
    }

    /// Marks the row of the discarded Run `run_id` *erased*, without its
    /// owner, tags or status line, nor the domains whose consent walls it
    /// met or whose refetch stopped, then truncates the index's WAL with
    /// `wal_checkpoint(TRUNCATE)`, so that neither the index nor its `-wal`
    /// file keeps them (ADRs 0001 and 0036). Answers whether the index
    /// listed the Run.
    ///
    /// # Errors
    ///
    /// As [`Self::write`], and `Sqlite` when another engine's reader held
    /// the WAL through the busy timeout.
    fn erase(&self, connection: &Connection, run_id: &RunId) -> Result<bool, RunStateError> {
        let listed = self.in_transaction(connection, true, |connection| {
            connection
                .prepare_cached("DELETE FROM consent_walls WHERE run_id = ?1")?
                .execute([run_id.as_str()])?;
            connection
                .prepare_cached("DELETE FROM refetch_stops WHERE run_id = ?1")?
                .execute([run_id.as_str()])?;
            connection
                .prepare_cached(
                    "UPDATE runs SET presence = ?2, owner = NULL, tags = '[]', progress_message = NULL
                     WHERE run_id = ?1",
                )?
                .execute((run_id.as_str(), Presence::Erased.as_str()))
        })? > 0;
        let busy: i64 = connection
            .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")
            .and_then(|mut checkpoint| checkpoint.query_row([], |row| row.get(0)))
            .map_err(|error| failed(&self.path, &error))?;
        if busy != 0 {
            return Err(failed(
                &self.path,
                &"another engine's reader held the Run index's WAL, so it was not truncated",
            ));
        }
        Ok(listed)
    }

    /// The rows of the Runs left stopped and present that `select`, one of
    /// [`STOPPED_BY_STORAGE`] and [`STOPPED_BY_LIMIT`], reads, in Run ID
    /// order.
    fn stopped_where(
        &self,
        connection: &Connection,
        select: &str,
    ) -> Result<Vec<RunIndexRow>, RunStateError> {
        self.in_transaction(connection, false, |connection| {
            connection
                .prepare_cached(select)?
                .query_map(
                    (Lifecycle::Stopped.as_str(), Presence::Present.as_str()),
                    row_of,
                )?
                .collect()
        })
    }
}

/// The Run index as the engine of Product version `engine` opens it.
fn opener(engine: &ProductVersion) -> Opener {
    Opener::new(&RUN_INDEX, engine.major_minor())
}

fn text<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("invariant: a row's value encodes") {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    }
}

fn parsed<T: serde::de::DeserializeOwned>(text: &str) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .or_else(|_| serde_json::from_str(text))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn insert(connection: &Connection, row: &RunIndexRow) -> rusqlite::Result<()> {
    connection.prepare_cached(INSERT)?.execute((
        row.run_id.as_str(),
        &row.location,
        row.kind.as_str(),
        text(&row.needs),
        serde_json::to_string(&row.tags).expect("invariant: tags encode"),
        row.lifecycle.as_str(),
        row.verdict.as_ref().map(Verdict::as_str),
        row.completeness.map(Completeness::as_str),
        StoredTime(row.started_at),
        row.ended_at.map(StoredTime),
        row.owner
            .as_ref()
            .map(|owner| serde_json::to_string(owner).expect("invariant: an endpoint encodes")),
        row.presence.as_str(),
        row.progress_message.as_deref(),
        row.stop_reason
            .as_ref()
            .map(|reason| serde_json::to_string(reason).expect("invariant: a stop reason encodes")),
        row.replayed,
    ))?;
    Ok(())
}

fn row_of(row: &Row<'_>) -> rusqlite::Result<RunIndexRow> {
    let owner: Option<String> = row.get(10)?;
    Ok(RunIndexRow {
        run_id: parsed(&row.get::<_, String>(0)?)?,
        location: row.get(1)?,
        kind: parsed(&row.get::<_, String>(2)?)?,
        needs: parsed(&row.get::<_, String>(3)?)?,
        tags: serde_json::from_str(&row.get::<_, String>(4)?)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
        lifecycle: parsed(&row.get::<_, String>(5)?)?,
        verdict: row
            .get::<_, Option<String>>(6)?
            .as_deref()
            .map(parsed)
            .transpose()?,
        completeness: row
            .get::<_, Option<String>>(7)?
            .as_deref()
            .map(parsed)
            .transpose()?,
        started_at: row.get::<_, StoredTime>(8)?.0,
        ended_at: row.get::<_, Option<StoredTime>>(9)?.map(|at| at.0),
        owner: owner
            .map(|owner| {
                serde_json::from_str(&owner)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .transpose()?,
        presence: parsed(&row.get::<_, String>(11)?)?,
        progress_message: row.get(12)?,
        stop_reason: row
            .get::<_, Option<String>>(13)?
            .map(|reason| {
                serde_json::from_str(&reason)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .transpose()?,
        replayed: row.get(14)?,
    })
}

/// The lowest minor the Run index at `path` records (ADR 0098), read
/// without opening it for writes; none when it is not there or cannot be
/// read. It blocks: run it under a blocking policy.
///
/// It opens the file for this one read rather than through a [`RunIndex`]
/// (#1886): the Daemon's pick of an executable asks it, before any engine
/// runs or while none is in the process, and it must see a marker another
/// engine raised since, which no connection kept for later could promise.
pub(crate) fn lowest_minor(path: &Path) -> Option<u16> {
    if !path.is_file() {
        return None;
    }
    let connection = File::new(path, None).connect(Access::ReadOnly).ok()?;
    lowest_minor::read_sqlite(&connection, RUN_INDEX.name)
        .ok()
        .flatten()
        .map(|marker| marker.lowest_minor)
}

/// A domain whose consent wall the Runs met, as the Run index keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConsentWallSeen {
    /// The registrable domain.
    pub(crate) domain: String,
    /// When a Run first met it.
    pub(crate) first_seen: SystemTime,
    /// The last Run that met it.
    pub(crate) last_run: RunId,
}

/// Each domain whose consent wall a Run met, read through `connection`.
fn consent_walls_in(connection: &Connection) -> rusqlite::Result<Vec<ConsentWallSeen>> {
    connection
        .prepare_cached(
            "SELECT domain, MIN(seen_at),
                    (SELECT last.run_id FROM consent_walls AS last
                     WHERE last.domain = walls.domain
                     ORDER BY last.seen_at DESC, last.run_id DESC LIMIT 1)
             FROM consent_walls AS walls
             GROUP BY domain
             ORDER BY domain",
        )?
        .query_map([], |row| {
            let run: String = row.get(2)?;
            Ok(ConsentWallSeen {
                domain: row.get(0)?,
                first_seen: row.get::<_, StoredTime>(1)?.0,
                last_run: RunId::parse(&run).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
            })
        })?
        .collect()
}

/// A domain on which the Runs' refetch without the site's stored data
/// stopped, as the Run index keeps it (ADR 0052).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefetchStopped {
    /// The registrable domain.
    pub(crate) domain: String,
    /// When a Run's refetch first stopped there.
    pub(crate) first_stopped: SystemTime,
    /// The last Run whose refetch stopped there.
    pub(crate) last_run: RunId,
    /// What stopped that Run's refetch: the Block cause, the `401` or `403`
    /// status, or the login page.
    pub(crate) cause: String,
    /// The project configuration file that Run's `wall.refetch_domains`
    /// came from, if it did.
    pub(crate) project_file: Option<String>,
}

/// Each domain on which a Run's refetch stopped, read through
/// `connection`.
fn refetch_stops_in(connection: &Connection) -> rusqlite::Result<Vec<RefetchStopped>> {
    connection
        .prepare_cached(
            "SELECT stops.domain,
                    (SELECT MIN(first.stopped_at) FROM refetch_stops AS first
                     WHERE first.domain = stops.domain),
                    stops.run_id, stops.cause, stops.project_file
             FROM refetch_stops AS stops
             WHERE stops.run_id = (SELECT last.run_id FROM refetch_stops AS last
                                   WHERE last.domain = stops.domain
                                   ORDER BY last.stopped_at DESC, last.run_id DESC LIMIT 1)
             ORDER BY stops.domain",
        )?
        .query_map([], |row| {
            let run: String = row.get(2)?;
            Ok(RefetchStopped {
                domain: row.get(0)?,
                first_stopped: row.get::<_, StoredTime>(1)?.0,
                last_run: RunId::parse(&run).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                cause: row.get(3)?,
                project_file: row.get(4)?,
            })
        })?
        .collect()
}

/// A row whose presence is `presence`, and that names no owner unless its
/// state is there.
pub(crate) fn kept_as(row: RunIndexRow, presence: Presence) -> RunIndexRow {
    RunIndexRow {
        owner: row.owner.filter(|_| presence == Presence::Present),
        presence,
        ..row
    }
}

#[cfg(test)]
mod tests;
