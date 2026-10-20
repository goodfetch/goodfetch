//! A Run's folder (ADRs 0002 and 0031): the writer's lock on the lock file
//! beside the Run's state, the Run's own row as it begins, and the readers
//! that open its state without a lock to compare a start's inputs or read
//! its report. Its calls block: the registry runs them under its owner's
//! blocking policy.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{Lifecycle, PointInTime, RunId, RunKind, RunReport, StartInput};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _};
use serde_json::{Map, Value};

use super::{CALLER, LOCK_FILE};
use crate::clock::Clock;

/// The Run's own row, written once when it begins.
pub(super) struct RunRow {
    pub(super) run_id: RunId,
    pub(super) kind: RunKind,
    pub(super) needs: Product,
    pub(super) epoch: u64,
    pub(super) inputs: String,
    pub(super) started_at: PointInTime,
}

impl RunRow {
    pub(super) fn insert(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute(
            "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                self.run_id.as_str(),
                self.kind.as_str(),
                text(&self.needs),
                CALLER,
                i64::try_from(self.epoch).unwrap_or(i64::MAX),
                &self.inputs,
                Lifecycle::Running.as_str(),
                self.started_at.as_str(),
            ),
        )?;
        Ok(())
    }
}

/// What a start compares of a Run found under its Run ID.
pub(super) struct Found {
    pub(super) kind: RunKind,
    pub(super) needs: Product,
    pub(super) caller: String,
    pub(super) inputs: String,
    pub(super) lifecycle: Lifecycle,
}

pub(super) fn text<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("invariant: a value encodes") {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

pub(super) fn parsed<T: serde::de::DeserializeOwned>(text: &str) -> rusqlite::Result<T> {
    serde_json::from_value(Value::String(text.to_owned()))
        .or_else(|_| serde_json::from_str(text))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

pub(super) fn point_in_time(clock: &Clock) -> PointInTime {
    let wall = jiff::Timestamp::try_from(clock.wall())
        .expect("invariant: the clock reads a time jiff can hold");
    PointInTime::from_rfc3339(
        wall.round(jiff::Unit::Millisecond)
            .unwrap_or(wall)
            .to_string(),
    )
}

/// The inputs as canonical JSON: an object of each input by its name, in
/// the order of the names.
pub(super) fn canonical_inputs(inputs: &BTreeMap<StartInput, Value>) -> String {
    let object: Map<String, Value> = inputs
        .iter()
        .map(|(input, value)| (text(input), value.clone()))
        .collect();
    Value::Object(object).to_string()
}

/// Each input whose value differs between two canonical inputs.
pub(super) fn input_differences(found: &str, given: &str) -> Vec<StartInput> {
    let read = |text: &str| -> Map<String, Value> {
        match serde_json::from_str(text) {
            Ok(Value::Object(object)) => object,
            _ => Map::new(),
        }
    };
    let (found, given) = (read(found), read(given));
    let mut names: Vec<&String> = found.keys().chain(given.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| found.get(*name) != given.get(*name))
        .filter_map(|name| parsed::<StartInput>(name).ok())
        .collect()
}

/// Opens a Run's state read-only, as a reader that takes no lock.
pub(super) fn open_reader(file: &Path, vfs: Option<&str>) -> rusqlite::Result<Option<Connection>> {
    if vfs.is_none() && !file.exists() {
        return Ok(None);
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = match vfs {
        Some(vfs) => Connection::open_with_flags_and_vfs(file, flags, vfs),
        None => Connection::open_with_flags(file, flags),
    };
    match connection {
        Ok(connection) => Ok(Some(connection)),
        Err(rusqlite::Error::SqliteFailure(failure, _))
            if failure.code == rusqlite::ErrorCode::CannotOpen =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn read_found(file: &Path, vfs: Option<&str>) -> Result<Option<Found>, String> {
    let read = || -> rusqlite::Result<Option<Found>> {
        let Some(connection) = open_reader(file, vfs)? else {
            return Ok(None);
        };
        connection
            .query_row(
                "SELECT kind, needs, caller, inputs, lifecycle FROM run",
                [],
                |row| {
                    Ok(Found {
                        kind: parsed(&row.get::<_, String>(0)?)?,
                        needs: parsed(&row.get::<_, String>(1)?)?,
                        caller: row.get(2)?,
                        inputs: row.get(3)?,
                        lifecycle: parsed(&row.get::<_, String>(4)?)?,
                    })
                },
            )
            .optional()
    };
    read().map_err(|error| error.to_string())
}

pub(super) fn read_report(file: &Path, vfs: Option<&str>) -> rusqlite::Result<Option<RunReport>> {
    let Some(connection) = open_reader(file, vfs)? else {
        return Ok(None);
    };
    connection
        .query_row(
            "SELECT run_id, lifecycle, stop_reason, verdict, completeness, started_at, ended_at FROM run
             WHERE ended_at IS NOT NULL",
            [],
            |row| {
                let stop_reason: Option<String> = row.get(2)?;
                Ok(RunReport {
                    run_id: parsed(&row.get::<_, String>(0)?)?,
                    lifecycle: parsed(&row.get::<_, String>(1)?)?,
                    stop_reason: stop_reason
                        .map(|reason| {
                            serde_json::from_str(&reason).map_err(|error| {
                                rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                            })
                        })
                        .transpose()?,
                    verdict: parsed(&row.get::<_, String>(3)?)?,
                    completeness: parsed(&row.get::<_, String>(4)?)?,
                    started_at: PointInTime::from_rfc3339(row.get(5)?),
                    ended_at: PointInTime::from_rfc3339(row.get(6)?),
                })
            },
        )
        .optional()
}

pub(super) enum LockError {
    /// Another engine holds it.
    Held,
    /// The OS refused the folder or the file.
    Io(String),
}

/// Creates the Run's folder and takes its writer's lock (ADR 0031).
pub(super) fn take_lock(folder: &Path) -> Result<File, LockError> {
    #[expect(
        clippy::disallowed_methods,
        reason = "runs under the registry owner's blocking policy"
    )]
    std::fs::create_dir_all(folder).map_err(|error| LockError::Io(error.to_string()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(LOCK_FILE))
        .map_err(|error| LockError::Io(error.to_string()))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockError::Held),
        Err(std::fs::TryLockError::Error(error)) => Err(LockError::Io(error.to_string())),
    }
}
