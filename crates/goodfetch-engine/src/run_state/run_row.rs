//! The Run's own row of its run file (ADR 0002), the `run` table this
//! module's parent creates, read typed: the readers every other crate of an
//! engine reads its kind, its lifecycle, its Clean-up rules' digest and its
//! run epoch through, on a connection to the file, so a change of the
//! table's columns or their encoding changes these functions alone. Each
//! keeps its statement in the connection's cache, so a reader called per
//! page prepares nothing more.

use goodfetch_protocol::run::{Lifecycle, RunKind};
use rusqlite::{Connection, OptionalExtension as _};
use serde::de::DeserializeOwned;
use serde_json::Value;

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
