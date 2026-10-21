//! A Run's folder (ADRs 0002 and 0031): the Run's own row as it begins and
//! as it ends, and the readers that open its state without a lock to
//! compare a start's inputs, read its report or read what it recorded of
//! itself. Its calls block: the registry runs them under its owner's
//! blocking policy.

use std::collections::BTreeMap;
use std::io::{self, Read as _, Write as _};
use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{
    Lifecycle, PointInTime, RunId, RunKind, RunReport, StartInput, Tags,
};
use goodfetch_protocol::settings::SettingLimit;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _};
use serde_json::{Map, Value};

use super::CALLER;
use crate::clock::Clock;

/// The Run's own row, written once when it begins.
pub(super) struct RunRow {
    pub(super) run_id: RunId,
    pub(super) kind: RunKind,
    pub(super) needs: Product,
    pub(super) epoch: u64,
    pub(super) inputs: String,
    pub(super) started_at: PointInTime,
    pub(super) policy_version: Option<String>,
    pub(super) weakened_by: Vec<SettingLimit>,
    /// What binds it, as JSON.
    pub(super) binding: Option<String>,
    pub(super) clean_up_digest: Option<String>,
}

impl RunRow {
    pub(super) fn insert(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute(
            "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at, policy_version, weakened_by, binding, clean_up_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            (
                self.run_id.as_str(),
                self.kind.as_str(),
                text(&self.needs),
                CALLER,
                i64::try_from(self.epoch).unwrap_or(i64::MAX),
                &self.inputs,
                Lifecycle::Running.as_str(),
                self.started_at.as_str(),
                self.policy_version.as_deref(),
                serde_json::to_string(&self.weakened_by)
                    .expect("invariant: setting limits encode"),
                self.binding.as_deref(),
                self.clean_up_digest.as_deref(),
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
    report(&connection)
}

/// The Run's report, once it ended.
fn report(connection: &Connection) -> rusqlite::Result<Option<RunReport>> {
    connection
        .query_row(
            "SELECT run_id, lifecycle, stop_reason, verdict, completeness, started_at, ended_at, policy_version, weakened_by FROM run
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
                    policy_version: row.get(7)?,
                    weakened_by: serde_json::from_str(&row.get::<_, String>(8)?).map_err(
                        |error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)),
                    )?,
                    checkpoint: None,
                })
            },
        )
        .optional()
}

/// What a Run's state records of the Run, as a reader finds it.
pub(super) struct Recorded {
    pub(super) run_id: RunId,
    pub(super) kind: RunKind,
    pub(super) needs: Product,
    pub(super) tags: Tags,
    pub(super) lifecycle: Lifecycle,
    pub(super) started_at: PointInTime,
    pub(super) policy_version: Option<String>,
    pub(super) weakened_by: Vec<SettingLimit>,
    /// Its start's inputs, as canonical JSON.
    pub(super) inputs: String,
    /// Its report, once it ended.
    pub(super) report: Option<RunReport>,
}

/// What the Run's state at `file` records of the Run, or `None` when there
/// is no state there.
pub(super) fn read_recorded(file: &Path, vfs: Option<&str>) -> rusqlite::Result<Option<Recorded>> {
    let Some(connection) = open_reader(file, vfs)? else {
        return Ok(None);
    };
    let found = connection
        .query_row(
            "SELECT run_id, kind, needs, lifecycle, started_at, policy_version, weakened_by, inputs FROM run",
            [],
            |row| {
                Ok((
                    parsed::<RunId>(&row.get::<_, String>(0)?)?,
                    (
                        parsed::<RunKind>(&row.get::<_, String>(1)?)?,
                        parsed::<Product>(&row.get::<_, String>(2)?)?,
                        parsed::<Lifecycle>(&row.get::<_, String>(3)?)?,
                    ),
                    PointInTime::from_rfc3339(row.get(4)?),
                    row.get::<_, Option<String>>(5)?,
                    (row.get::<_, String>(6)?, row.get::<_, String>(7)?),
                ))
            },
        )
        .optional()?;
    let Some((run_id, (kind, needs, lifecycle), started_at, policy_version, (weakened_by, inputs))) =
        found
    else {
        return Ok(None);
    };
    let mut statement = connection.prepare("SELECT tag FROM tags ORDER BY position")?;
    let tags: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let decode =
        |error: serde_json::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(error));
    Ok(Some(Recorded {
        run_id,
        kind,
        needs,
        tags: serde_json::from_value(Value::from(tags)).map_err(decode)?,
        lifecycle,
        started_at,
        policy_version,
        weakened_by: serde_json::from_str(&weakened_by).map_err(decode)?,
        inputs,
        report: report(&connection)?,
    }))
}

/// The machine's identity kept at `path`, or `drawn`, kept there first
/// when the file does not exist yet.
pub(super) fn machine_identity(path: &Path, drawn: &str) -> io::Result<String> {
    let read = |path: &Path| -> io::Result<String> {
        let mut text = String::new();
        std::fs::OpenOptions::new()
            .read(true)
            .open(path)?
            .read_to_string(&mut text)?;
        Ok(text.trim().to_owned())
    };
    match read(path) {
        Ok(machine) if !machine.is_empty() => return Ok(machine),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(folder) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).create(folder)?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(format!("{drawn}\n").as_bytes())?;
            file.sync_all()?;
            Ok(drawn.to_owned())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read(path),
        Err(error) => Err(error),
    }
}

/// Writes the Run's end, as `report` states it, into its own row.
pub(super) fn write_end(connection: &Connection, report: &RunReport) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE run SET lifecycle = ?1, stop_reason = ?2, verdict = ?3, completeness = ?4, ended_at = ?5",
        (
            report.lifecycle.as_str(),
            report.stop_reason.map(|reason| {
                serde_json::to_string(&reason).expect("invariant: a stop reason encodes")
            }),
            report.verdict.as_str(),
            report.completeness.as_str(),
            report.ended_at.as_str(),
        ),
    )?;
    Ok(())
}
