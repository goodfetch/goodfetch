//! The Run index (ADRs 0031 and 0098): the per-user, per-major SQLite
//! catalog of every Run of either Product, which every engine writes when a
//! Run starts and when one of its axes changes, each row pointing at the
//! Run's own state, from which it can be rebuilt, naming the engine that
//! owns the Run while it is live, and saying whether its state is there.
//! Several engines write it, each through its own short-lived connection;
//! WAL and a busy timeout let them take turns. Its calls block: callers run
//! them under their owner's blocking policy.
//!
//! Engines of several minors of the major open it at once, so it records
//! the lowest minor that can use it (ADR 0098), which each call reads and
//! checks in its own transaction: an engine that finds it raised past
//! itself stops using the index with the version refusal.

use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::RUN_INDEX;
use goodfetch_protocol::run::{Completeness, PointInTime, Presence, RunId, RunIndexRow, Verdict};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension as _, Row, Transaction, TransactionBehavior,
};

use crate::lowest_minor::{self, Opener};
use crate::run_state::RunStateError;

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
        progress_message TEXT
    ) STRICT",
];

const COLUMNS: &str = "run_id, location, kind, needs, tags, lifecycle, verdict, completeness, started_at, ended_at, owner, presence, progress_message";

fn failed(path: &Path, error: &impl std::fmt::Display) -> RunStateError {
    RunStateError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

/// The Run index as the engine of Product version `engine` opens it.
fn opener(engine: &ProductVersion) -> Opener {
    Opener::new(&RUN_INDEX, engine.major_minor())
}

fn open(path: &Path, opener: &Opener) -> Result<Connection, RunStateError> {
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
    let created = opener.created();
    connection
        .execute(
            "INSERT INTO format (name, version, lowest_minor) VALUES (?1, ?2, ?3) ON CONFLICT (name) DO NOTHING",
            (
                RUN_INDEX.name,
                created.version.to_string(),
                i64::from(created.lowest_minor),
            ),
        )
        .map_err(|error| failed(path, &error))?;
    Ok(connection)
}

/// Runs `work` in one transaction of the index at `path`, after checking
/// its marker in that transaction (ADR 0098); a write records this engine's
/// format version in the marker.
fn in_transaction<T>(
    path: &Path,
    opener: &Opener,
    writes: bool,
    work: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
) -> Result<T, RunStateError> {
    let mut connection = open(path, opener)?;
    let behavior = if writes {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|error| failed(path, &error))?;
    let found = lowest_minor::read_sqlite(&transaction, RUN_INDEX.name)
        .map_err(|error| failed(path, &error))?
        .unwrap_or_else(|| opener.created());
    opener.check(&found).map_err(RunStateError::refused)?;
    let answer = work(&transaction).map_err(|error| failed(path, &error))?;
    if writes {
        let marked = opener.written(found, None);
        if marked != found {
            lowest_minor::write_sqlite(&transaction, RUN_INDEX.name, marked)
                .map_err(|error| failed(path, &error))?;
        }
    }
    transaction.commit().map_err(|error| failed(path, &error))?;
    Ok(answer)
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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (run_id) DO UPDATE SET
                location = excluded.location, kind = excluded.kind, needs = excluded.needs,
                tags = excluded.tags, lifecycle = excluded.lifecycle, verdict = excluded.verdict,
                completeness = excluded.completeness, started_at = excluded.started_at,
                ended_at = excluded.ended_at, owner = excluded.owner, presence = excluded.presence,
                progress_message = excluded.progress_message"
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
            row.progress_message.as_deref(),
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
        progress_message: row.get(12)?,
    })
}

/// Writes `row` into the Run index at `path`, replacing the Run's earlier
/// row, on an engine of Product version `engine`.
///
/// # Errors
///
/// `Sqlite` when the index cannot be opened or written, `Format` when its
/// marker refuses this engine.
pub(crate) fn write(
    path: &Path,
    engine: &ProductVersion,
    row: &RunIndexRow,
) -> Result<(), RunStateError> {
    write_as(path, &opener(engine), row)
}

fn write_as(path: &Path, opener: &Opener, row: &RunIndexRow) -> Result<(), RunStateError> {
    in_transaction(path, opener, true, |transaction| insert(transaction, row))
}

/// The row of `run_id` in the Run index at `path`, if it lists the Run.
///
/// # Errors
///
/// As [`write`], when the index cannot be opened or read.
pub(crate) fn read(
    path: &Path,
    engine: &ProductVersion,
    run_id: &RunId,
) -> Result<Option<RunIndexRow>, RunStateError> {
    in_transaction(path, &opener(engine), false, |transaction| {
        transaction
            .query_row(
                &format!("SELECT {COLUMNS} FROM runs WHERE run_id = ?1"),
                [run_id.as_str()],
                row_of,
            )
            .optional()
    })
}

/// Every row of the Run index at `path`, in Run ID order.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn rows(
    path: &Path,
    engine: &ProductVersion,
) -> Result<Vec<RunIndexRow>, RunStateError> {
    rows_as(path, &opener(engine))
}

fn rows_as(path: &Path, opener: &Opener) -> Result<Vec<RunIndexRow>, RunStateError> {
    in_transaction(path, opener, false, |transaction| {
        let mut statement =
            transaction.prepare(&format!("SELECT {COLUMNS} FROM runs ORDER BY run_id"))?;
        statement.query_map([], row_of).and_then(Iterator::collect)
    })
}

/// Whether the engine of Product version `engine` may use the Run index at
/// `path`, as a Run's start asks before it writes its row.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn check(path: &Path, engine: &ProductVersion) -> Result<(), RunStateError> {
    in_transaction(path, &opener(engine), false, |_| Ok(()))
}

/// Replaces every row of the Run index at `path` with `rows`, in one
/// transaction.
///
/// # Errors
///
/// As [`write`]; the index is then left as it was.
pub(crate) fn replace(
    path: &Path,
    engine: &ProductVersion,
    rows: &[RunIndexRow],
) -> Result<(), RunStateError> {
    in_transaction(path, &opener(engine), true, |transaction| {
        transaction.execute("DELETE FROM runs", [])?;
        for row in rows {
            insert(transaction, row)?;
        }
        Ok(())
    })
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
