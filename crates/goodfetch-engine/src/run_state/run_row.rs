//! The Run's own row of its run file (ADR 0002), the `run` table this
//! module's parent creates, read typed: the readers every other crate of an
//! engine reads its kind, its lifecycle, its Clean-up rules' digest and its
//! run epoch through, on a connection to the file, so a change of the
//! table's columns or their encoding changes these functions alone. Each
//! keeps its statement in the connection's cache, so a reader called per
//! page prepares nothing more.

use std::time::SystemTime;

use goodfetch_protocol::clean_up_rules::{CleanUpRulesReport, CleanUpRulesSwitch};
use goodfetch_protocol::run::{Lifecycle, RunKind};
use rusqlite::{Connection, OptionalExtension as _};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::StoredTime;

/// Whether the Run is of the kind `kind`: not, for a kind this engine does
/// not know, which a newer minor's run file may hold, so a part of its
/// report or progress that is of one kind goes without a word.
///
/// # Errors
///
/// SQLite's, `QueryReturnedNoRows` for a file that holds no Run.
pub fn is_of_kind(connection: &Connection, kind: RunKind) -> rusqlite::Result<bool> {
    Ok(text(connection, "SELECT kind FROM run")? == kind.as_str())
}

/// Where the Run is in its lifecycle, as its last commit left it.
///
/// # Errors
///
/// SQLite's, `QueryReturnedNoRows` for a file that holds no Run, and a
/// conversion failure for a lifecycle this engine does not know.
pub fn lifecycle(connection: &Connection) -> rusqlite::Result<Lifecycle> {
    parsed(&text(connection, "SELECT lifecycle FROM run")?)
}

/// The digest of the Clean-up rules the Run holds to; none for a Run that
/// holds to none, or a file that holds no Run.
///
/// # Errors
///
/// SQLite's.
pub fn clean_up_digest(connection: &Connection) -> rusqlite::Result<Option<String>> {
    Ok(connection
        .prepare_cached("SELECT clean_up_digest FROM run")
        .and_then(|mut cached| cached.query_row([], |row| row.get::<_, Option<String>>(0)))
        .optional()?
        .flatten())
}

/// Records the Clean-up rules version digest and publication date that the
/// Run currently holds to (ADRs 0047 and 0139).
///
/// # Errors
///
/// SQLite's.
pub fn set_clean_up_version(
    connection: &Connection,
    digest: &str,
    published_at: SystemTime,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached("UPDATE run SET clean_up_digest = ?1, clean_up_published_at = ?2")
        .and_then(|mut cached| cached.execute((digest, StoredTime(published_at))))?;
    Ok(())
}

/// Appends one Clean-up rules version switch adopted at a Safe point under
/// `clean_up.follow_latest` (ADR 0139).
///
/// # Errors
///
/// SQLite's.
pub fn record_clean_up_switch(
    connection: &Connection,
    switch: &CleanUpRulesSwitch,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO clean_up_switches (from_version, to_version, published_at, switched_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .and_then(|mut cached| {
            cached.execute((
                &switch.from_version,
                &switch.to_version,
                StoredTime(switch.published_at),
                StoredTime(switch.switched_at),
            ))
        })?;
    Ok(())
}

/// The Clean-up rules report of the Run, when its state records a version
/// and publication date; none otherwise.
///
/// # Errors
///
/// SQLite's.
pub fn clean_up_rules_report(
    connection: &Connection,
) -> rusqlite::Result<Option<CleanUpRulesReport>> {
    let row = connection
        .prepare_cached("SELECT clean_up_digest, clean_up_published_at FROM run")
        .and_then(|mut cached| {
            cached.query_row([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<StoredTime>>(1)?,
                ))
            })
        })
        .optional()?;
    let Some((Some(version), Some(StoredTime(published_at)))) = row else {
        return Ok(None);
    };
    let mut statement = connection.prepare_cached(
        "SELECT from_version, to_version, published_at, switched_at \
         FROM clean_up_switches ORDER BY position ASC",
    )?;
    let switches = statement
        .query_map([], |row| {
            let StoredTime(published_at) = row.get(2)?;
            let StoredTime(switched_at) = row.get(3)?;
            Ok(CleanUpRulesSwitch {
                from_version: row.get(0)?,
                to_version: row.get(1)?,
                published_at,
                switched_at,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(CleanUpRulesReport {
        version,
        published_at,
        switches,
    }))
}

/// The Run's run epoch (ADR 0003).
///
/// # Errors
///
/// SQLite's, `QueryReturnedNoRows` for a file that holds no Run, and a
/// conversion failure for a negative one.
pub fn epoch(connection: &Connection) -> rusqlite::Result<u64> {
    let epoch: i64 = connection
        .prepare_cached("SELECT epoch FROM run")
        .and_then(|mut cached| cached.query_row([], |row| row.get(0)))?;
    u64::try_from(epoch).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

/// Reads one key from the Run's `fixed_settings` JSON object, if present.
///
/// # Errors
///
/// SQLite's, `QueryReturnedNoRows` for a file that holds no Run, or a JSON
/// decode failure.
pub fn fixed_setting(connection: &Connection, key: &str) -> rusqlite::Result<Option<Value>> {
    let raw = text(connection, "SELECT fixed_settings FROM run")?;
    let map: serde_json::Map<String, Value> = serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(map.get(key).cloned())
}

/// Reads a boolean key from the Run's `fixed_settings` JSON object, returning
/// `false` when absent or not `true`.
///
/// # Errors
///
/// SQLite's, `QueryReturnedNoRows` for a file that holds no Run, or a JSON
/// decode failure.
pub fn fixed_setting_bool(connection: &Connection, key: &str) -> rusqlite::Result<bool> {
    Ok(fixed_setting(connection, key)?.and_then(|v| v.as_bool()) == Some(true))
}

/// The one text column `statement` reads of the Run's row.
fn text(connection: &Connection, statement: &str) -> rusqlite::Result<String> {
    connection
        .prepare_cached(statement)
        .and_then(|mut cached| cached.query_row([], |row| row.get(0)))
}

/// `text` as the value whose name it is, such as `finished`.
fn parsed<T: DeserializeOwned>(text: &str) -> rusqlite::Result<T> {
    serde_json::from_value(Value::String(text.to_owned())).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

#[cfg(test)]
mod tests;
