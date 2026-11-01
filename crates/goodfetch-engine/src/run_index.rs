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
//! Several engines write it, each through its own short-lived connection;
//! WAL and a busy timeout let them take turns. Its calls block: callers run
//! them under their owner's blocking policy.
//!
//! Engines of several minors of the major open it at once, so it records
//! the lowest minor that can use it (ADR 0098), which each call reads and
//! checks in its own transaction: an engine that finds it raised past
//! itself stops using the index with the version refusal.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::RUN_INDEX;
use goodfetch_protocol::run::{
    Completeness, Lifecycle, PointInTime, Presence, RunId, RunIndexRow, Verdict,
};
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

const COLUMNS: &str = "run_id, location, kind, needs, tags, lifecycle, verdict, completeness, started_at, ended_at, owner, presence, progress_message, stop_reason, replayed";

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
    crate::sqlite_wal::enter(&connection, BUSY_TIMEOUT).map_err(|error| failed(path, &error))?;
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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT (run_id) DO UPDATE SET
                location = excluded.location, kind = excluded.kind, needs = excluded.needs,
                tags = excluded.tags, lifecycle = excluded.lifecycle, verdict = excluded.verdict,
                completeness = excluded.completeness, started_at = excluded.started_at,
                ended_at = excluded.ended_at, owner = excluded.owner, presence = excluded.presence,
                progress_message = excluded.progress_message, stop_reason = excluded.stop_reason,
                replayed = excluded.replayed"
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
            row.stop_reason.as_ref().map(|reason| {
                serde_json::to_string(reason).expect("invariant: a stop reason encodes")
            }),
            row.replayed,
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

/// Marks the row of the discarded Run `run_id` in the Run index at `path`
/// *erased*, without its owner, tags or status line, nor the domains whose
/// consent walls it met or whose refetch stopped, then truncates the
/// index's WAL with `wal_checkpoint(TRUNCATE)`, so that neither the index
/// nor its `-wal` file keeps them (ADRs 0001 and 0036). Answers whether the
/// index listed the Run.
///
/// # Errors
///
/// As [`write`], and `Sqlite` when another engine's reader held the WAL
/// through the busy timeout.
pub(crate) fn erase(
    path: &Path,
    engine: &ProductVersion,
    run_id: &RunId,
) -> Result<bool, RunStateError> {
    let opener = opener(engine);
    let listed = in_transaction(path, &opener, true, |transaction| {
        transaction.execute(
            "DELETE FROM consent_walls WHERE run_id = ?1",
            [run_id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM refetch_stops WHERE run_id = ?1",
            [run_id.as_str()],
        )?;
        transaction.execute(
            "UPDATE runs SET presence = ?2, owner = NULL, tags = '[]', progress_message = NULL
             WHERE run_id = ?1",
            (run_id.as_str(), Presence::Erased.as_str()),
        )
    })? > 0;
    let connection = open(path, &opener)?;
    let busy: i64 = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(|error| failed(path, &error))?;
    if busy != 0 {
        return Err(failed(
            path,
            &"another engine's reader held the Run index's WAL, so it was not truncated",
        ));
    }
    Ok(listed)
}

/// The rows of the Runs a storage stop left stopped in the Run index at
/// `path` (ADR 0122), present, in Run ID order: "Needs you" lists them.
/// An index not made yet lists none, and is not made.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn stopped_by_storage(
    path: &Path,
    engine: &ProductVersion,
) -> Result<Vec<RunIndexRow>, RunStateError> {
    stopped_where(
        path,
        engine,
        "json_extract(stop_reason, '$.kind') = 'resources'
         AND json_extract(stop_reason, '$.resource') = 'storage'",
    )
}

/// The rows of the Runs a Limit stopped in the Run index at `path` (ADR
/// 0041), present, in Run ID order: "Needs you" lists those whose Limit
/// still has nothing left. An index not made yet lists none, and is not
/// made.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn stopped_by_limit(
    path: &Path,
    engine: &ProductVersion,
) -> Result<Vec<RunIndexRow>, RunStateError> {
    stopped_where(
        path,
        engine,
        "json_extract(stop_reason, '$.kind') = 'limit'",
    )
}

/// The rows of the Runs left stopped and present in the Run index at
/// `path` whose stop reason meets `condition`, in Run ID order.
fn stopped_where(
    path: &Path,
    engine: &ProductVersion,
    condition: &str,
) -> Result<Vec<RunIndexRow>, RunStateError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    in_transaction(path, &opener(engine), false, |transaction| {
        let mut statement = transaction.prepare(&format!(
            "SELECT {COLUMNS} FROM runs
             WHERE lifecycle = ?1 AND presence = ?2 AND {condition}
             ORDER BY run_id"
        ))?;
        statement
            .query_map(
                (Lifecycle::Stopped.as_str(), Presence::Present.as_str()),
                row_of,
            )
            .and_then(Iterator::collect)
    })
}

/// The items of "Needs you" a person dismissed, as the Run index at `path`
/// keeps them, so that every engine of the major hides them. An index not
/// made yet keeps none, and is not made.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn dismissed(
    path: &Path,
    engine: &ProductVersion,
) -> Result<BTreeSet<String>, RunStateError> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    in_transaction(path, &opener(engine), false, |transaction| {
        let mut statement = transaction.prepare("SELECT item FROM needs_you_dismissed")?;
        statement
            .query_map([], |row| row.get(0))
            .and_then(Iterator::collect)
    })
}

/// Records that a person dismissed the item `item` of "Needs you" at `at`.
///
/// # Errors
///
/// As [`write`].
pub(crate) fn dismiss(
    path: &Path,
    engine: &ProductVersion,
    item: &str,
    at: &PointInTime,
) -> Result<(), RunStateError> {
    in_transaction(path, &opener(engine), true, |transaction| {
        transaction.execute(
            "INSERT INTO needs_you_dismissed (item, dismissed_at) VALUES (?1, ?2)
             ON CONFLICT (item) DO NOTHING",
            (item, at.as_str()),
        )?;
        Ok(())
    })
}

/// Forgets the dismissals of `items`, which no source lists any longer.
///
/// # Errors
///
/// As [`write`].
pub(crate) fn forget_dismissed(
    path: &Path,
    engine: &ProductVersion,
    items: &[String],
) -> Result<(), RunStateError> {
    in_transaction(path, &opener(engine), true, |transaction| {
        for item in items {
            transaction.execute("DELETE FROM needs_you_dismissed WHERE item = ?1", [item])?;
        }
        Ok(())
    })
}

/// A domain whose consent wall the Runs met, as the Run index keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConsentWallSeen {
    /// The registrable domain.
    pub(crate) domain: String,
    /// When a Run first met it.
    pub(crate) first_seen: PointInTime,
    /// The last Run that met it.
    pub(crate) last_run: RunId,
}

/// Records that the Run `run_id` met a consent wall on `domain` at `at`,
/// keeping the first time it did.
///
/// # Errors
///
/// As [`write`].
pub(crate) fn record_consent_wall(
    path: &Path,
    engine: &ProductVersion,
    domain: &str,
    run_id: &RunId,
    at: &PointInTime,
) -> Result<(), RunStateError> {
    in_transaction(path, &opener(engine), true, |transaction| {
        transaction.execute(
            "INSERT INTO consent_walls (domain, run_id, seen_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (domain, run_id) DO NOTHING",
            (domain, run_id.as_str(), at.as_str()),
        )?;
        Ok(())
    })
}

/// Each domain whose consent wall a Run in the Run index at `path` met, in
/// domain order, with when one first met it and the last that did. An
/// index not made yet keeps none, and is not made.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn consent_walls(
    path: &Path,
    engine: &ProductVersion,
) -> Result<Vec<ConsentWallSeen>, RunStateError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    in_transaction(path, &opener(engine), false, |transaction| {
        let mut statement = transaction.prepare(
            "SELECT domain, MIN(seen_at),
                    (SELECT last.run_id FROM consent_walls AS last
                     WHERE last.domain = walls.domain
                     ORDER BY last.seen_at DESC, last.run_id DESC LIMIT 1)
             FROM consent_walls AS walls
             GROUP BY domain
             ORDER BY domain",
        )?;
        statement
            .query_map([], |row| {
                let run: String = row.get(2)?;
                Ok(ConsentWallSeen {
                    domain: row.get(0)?,
                    first_seen: PointInTime::from_rfc3339(row.get::<_, String>(1)?),
                    last_run: RunId::parse(&run).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                })
            })
            .and_then(Iterator::collect)
    })
}

/// A domain on which the Runs' refetch without the site's stored data
/// stopped, as the Run index keeps it (ADR 0052).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefetchStopped {
    /// The registrable domain.
    pub(crate) domain: String,
    /// When a Run's refetch first stopped there.
    pub(crate) first_stopped: PointInTime,
    /// The last Run whose refetch stopped there.
    pub(crate) last_run: RunId,
    /// What stopped that Run's refetch: the Block cause, the `401` or `403`
    /// status, or the login page.
    pub(crate) cause: String,
    /// The project configuration file that Run's `wall.refetch_domains`
    /// came from, if it did.
    pub(crate) project_file: Option<String>,
}

/// Records that the refetch of the Run `stop.last_run` stopped on
/// `stop.domain` at `at`, for its cause, keeping the Run's first stop.
///
/// # Errors
///
/// As [`write`].
pub(crate) fn record_refetch_stop(
    path: &Path,
    engine: &ProductVersion,
    stop: &RefetchStopped,
    at: &PointInTime,
) -> Result<(), RunStateError> {
    in_transaction(path, &opener(engine), true, |transaction| {
        transaction.execute(
            "INSERT INTO refetch_stops (domain, run_id, cause, project_file, stopped_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (domain, run_id) DO NOTHING",
            (
                stop.domain.as_str(),
                stop.last_run.as_str(),
                stop.cause.as_str(),
                stop.project_file.as_deref(),
                at.as_str(),
            ),
        )?;
        Ok(())
    })
}

/// Each domain on which a Run's refetch stopped in the Run index at
/// `path`, in domain order, with when one first stopped there, and the
/// last Run that did with its cause and project configuration file. An
/// index not made yet keeps none, and is not made.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn refetch_stops(
    path: &Path,
    engine: &ProductVersion,
) -> Result<Vec<RefetchStopped>, RunStateError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    in_transaction(path, &opener(engine), false, |transaction| {
        let mut statement = transaction.prepare(
            "SELECT stops.domain,
                    (SELECT MIN(first.stopped_at) FROM refetch_stops AS first
                     WHERE first.domain = stops.domain),
                    stops.run_id, stops.cause, stops.project_file
             FROM refetch_stops AS stops
             WHERE stops.run_id = (SELECT last.run_id FROM refetch_stops AS last
                                   WHERE last.domain = stops.domain
                                   ORDER BY last.stopped_at DESC, last.run_id DESC LIMIT 1)
             ORDER BY stops.domain",
        )?;
        statement
            .query_map([], |row| {
                let run: String = row.get(2)?;
                Ok(RefetchStopped {
                    domain: row.get(0)?,
                    first_stopped: PointInTime::from_rfc3339(row.get::<_, String>(1)?),
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
            })
            .and_then(Iterator::collect)
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
