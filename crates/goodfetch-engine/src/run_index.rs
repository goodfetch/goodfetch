//! The Run index (ADRs 0031 and 0098): the per-user, per-major SQLite
//! catalog of every Run of either Product, which every engine writes when a
//! Run starts and when one of its axes changes, each row pointing at the
//! Run's own state, from which it can be rebuilt. Several engines write it,
//! each through its own short-lived connection; WAL and a busy timeout let
//! them take turns. Its calls block: callers run them under their owner's
//! blocking policy.

use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::formats::RUN_INDEX;
use goodfetch_protocol::run::{Completeness, PointInTime, RunIndexRow, Verdict};
use rusqlite::{Connection, OpenFlags};

use crate::run_state::RunStateError;

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
        ended_at TEXT
    ) STRICT",
];

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
    Ok(connection)
}

fn text<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("invariant: a row's value encodes") {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    }
}

/// Writes `row` into the Run index at `path`, replacing the Run's earlier
/// row.
///
/// # Errors
///
/// `Sqlite` when the index cannot be opened or written.
pub(crate) fn write(path: &Path, row: &RunIndexRow) -> Result<(), RunStateError> {
    let connection = open(path)?;
    connection
        .execute(
            "INSERT INTO runs (run_id, location, kind, needs, tags, lifecycle, verdict, completeness, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (run_id) DO UPDATE SET
                location = excluded.location, kind = excluded.kind, needs = excluded.needs,
                tags = excluded.tags, lifecycle = excluded.lifecycle, verdict = excluded.verdict,
                completeness = excluded.completeness, started_at = excluded.started_at,
                ended_at = excluded.ended_at",
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
            ),
        )
        .map_err(|error| failed(path, &error))?;
    Ok(())
}
