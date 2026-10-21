//! The Run index (ADRs 0031 and 0098): the per-user, per-major SQLite
//! catalog of every Run of either Product, which every engine writes when a
//! Run starts and when one of its axes changes, each row pointing at the
//! Run's own state, from which it can be rebuilt, naming the engine that
//! owns the Run while it is live, and saying whether its state is there.
//! Several engines write it, each through its own short-lived connection;
//! WAL and a busy timeout let them take turns. Its calls block: callers run
//! them under their owner's blocking policy.

use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::formats::RUN_INDEX;
use goodfetch_protocol::run::{Completeness, PointInTime, Presence, RunId, RunIndexRow, Verdict};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, Row};

use crate::run_state::{RunStateError, check_format};

/// How long a write waits for another engine's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT",
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
        presence TEXT NOT NULL
    ) STRICT",
];

const COLUMNS: &str = "run_id, location, kind, needs, tags, lifecycle, verdict, completeness, started_at, ended_at, owner, presence";

fn failed(path: &Path, error: &impl std::fmt::Display) -> RunStateError {
    RunStateError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

fn open(path: &Path) -> Result<Connection, RunStateError> {
    if let Some(folder) = path.parent() {
        #[expect(
            clippy::disallowed_methods,
            reason = "the Run index's calls run under their caller's blocking policy"
        )]
        std::fs::create_dir_all(folder).map_err(|error| failed(path, &error))?;
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| failed(path, &error))?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| failed(path, &error))?;
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .map_err(|error| failed(path, &error))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| failed(path, &error))?;
    for statement in SCHEMA {
        connection
            .execute(statement, [])
            .map_err(|error| failed(path, &error))?;
    }
    connection
        .execute(
            "INSERT INTO format (name, version) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING",
            (RUN_INDEX.name, RUN_INDEX.version.to_string()),
        )
        .map_err(|error| failed(path, &error))?;
    let found: String = connection
        .query_row(
            "SELECT version FROM format WHERE name = ?1",
            [RUN_INDEX.name],
            |row| row.get(0),
        )
        .map_err(|error| failed(path, &error))?;
    check_format(&RUN_INDEX, &found)?;
    Ok(connection)
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
    connection.execute(
        &format!(
            "INSERT INTO runs ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (run_id) DO UPDATE SET
                location = excluded.location, kind = excluded.kind, needs = excluded.needs,
                tags = excluded.tags, lifecycle = excluded.lifecycle, verdict = excluded.verdict,
                completeness = excluded.completeness, started_at = excluded.started_at,
                ended_at = excluded.ended_at, owner = excluded.owner, presence = excluded.presence"
        ),
        (
            row.run_id.as_str(),
            &row.location,
            row.kind.as_str(),
            text(&row.needs),
            serde_json::to_string(&row.tags).expect("invariant: tags encode"),
            row.lifecycle.as_str(),
            row.verdict.map(Verdict::as_str),
            row.completeness.map(Completeness::as_str),
            row.started_at.as_str(),
            row.ended_at.as_ref().map(PointInTime::as_str),
            row.owner
                .as_ref()
                .map(|owner| serde_json::to_string(owner).expect("invariant: an endpoint encodes")),
            row.presence.as_str(),
        ),
    )?;
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
        started_at: PointInTime::from_rfc3339(row.get(8)?),
        ended_at: row
            .get::<_, Option<String>>(9)?
            .map(PointInTime::from_rfc3339),
        owner: owner
            .map(|owner| {
                serde_json::from_str(&owner)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .transpose()?,
        presence: parsed(&row.get::<_, String>(11)?)?,
    })
}

/// Writes `row` into the Run index at `path`, replacing the Run's earlier
/// row.
///
/// # Errors
///
/// `Sqlite` when the index cannot be opened or written.
pub(crate) fn write(path: &Path, row: &RunIndexRow) -> Result<(), RunStateError> {
    let connection = open(path)?;
    insert(&connection, row).map_err(|error| failed(path, &error))
}

/// The row of `run_id` in the Run index at `path`, if it lists the Run.
///
/// # Errors
///
/// `Sqlite` when the index cannot be opened or read.
pub(crate) fn read(path: &Path, run_id: &RunId) -> Result<Option<RunIndexRow>, RunStateError> {
    let connection = open(path)?;
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM runs WHERE run_id = ?1"),
            [run_id.as_str()],
            row_of,
        )
        .optional()
        .map_err(|error| failed(path, &error))
}

/// Every row of the Run index at `path`, in Run ID order.
///
/// # Errors
///
/// `Sqlite` when the index cannot be opened or read.
pub(crate) fn rows(path: &Path) -> Result<Vec<RunIndexRow>, RunStateError> {
    let connection = open(path)?;
    let mut statement = connection
        .prepare(&format!("SELECT {COLUMNS} FROM runs ORDER BY run_id"))
        .map_err(|error| failed(path, &error))?;
    statement
        .query_map([], row_of)
        .and_then(Iterator::collect)
        .map_err(|error| failed(path, &error))
}

/// Replaces every row of the Run index at `path` with `rows`, in one
/// transaction.
///
/// # Errors
///
/// `Sqlite` when the index cannot be opened or written; it is then left as
/// it was.
pub(crate) fn replace(path: &Path, rows: &[RunIndexRow]) -> Result<(), RunStateError> {
    let mut connection = open(path)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| failed(path, &error))?;
    transaction
        .execute("DELETE FROM runs", [])
        .map_err(|error| failed(path, &error))?;
    for row in rows {
        insert(&transaction, row).map_err(|error| failed(path, &error))?;
    }
    transaction.commit().map_err(|error| failed(path, &error))
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
