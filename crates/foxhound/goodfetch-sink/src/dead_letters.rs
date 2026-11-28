//! Per-Sink dead letters (ADR 0026): Records a destination refused (or a
//! keyed Sink received without its key fields, ADR 0103), isolated by
//! splitting rejected batches and kept in the run file's
//! `sink_dead_letters` table with the destination's masked reason so one
//! bad Record never blocks a Sink and a later Run can retry them.

use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::run_state::RunStateTables;
use rusqlite::Connection;
use serde_json::Value;

use crate::contract::{DeliveryId, RunCopy};

/// The run-state table holding each Sink's dead letters (ADR 0026),
/// registered in the run-state compatibility corpus (ADR 0066).
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-sink",
    statements: &["CREATE TABLE IF NOT EXISTS sink_dead_letters (
            sink TEXT NOT NULL,
            delivery_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            run_epoch INTEGER NOT NULL,
            dataset TEXT NOT NULL,
            record_number INTEGER NOT NULL,
            committed_at TEXT NOT NULL,
            record TEXT NOT NULL,
            reason TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            PRIMARY KEY (sink, delivery_id)
        ) STRICT, WITHOUT ROWID"],
};

/// One Record on a Sink's dead-letter list.
#[derive(Clone, Debug, PartialEq)]
pub struct DeadLetter {
    /// The Sink's name, its key under `sinks`.
    pub sink: String,
    /// Its delivery ID, kept across retries so the destination can dedup.
    pub delivery_id: DeliveryId,
    /// The Run copy that last attempted it.
    pub run: RunCopy,
    /// Its Dataset.
    pub dataset: String,
    /// Its number in its Dataset.
    pub number: u64,
    /// When it was committed, RFC 3339 in UTC.
    pub committed_at: String,
    /// The Record's JSON value, masked at `Scope::Kept`.
    pub value: Value,
    /// Why it was dead-lettered (the destination's or keying reason),
    /// masked at `Scope::Kept`.
    pub reason: String,
    /// How many times delivery was attempted.
    pub attempts: u64,
}

/// Creates the `sink_dead_letters` table on `connection` when absent.
///
/// # Errors
///
/// SQLite's, when the statement fails.
pub fn ensure_table(connection: &Connection) -> rusqlite::Result<()> {
    for statement in TABLES.statements {
        connection.execute_batch(statement)?;
    }
    Ok(())
}

/// Whether `connection` holds the `sink_dead_letters` table; an older run
/// file written before Sinks may lack it.
///
/// # Errors
///
/// SQLite's, when the catalog cannot be queried.
pub(crate) fn has_table(connection: &Connection) -> rusqlite::Result<bool> {
    connection
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sink_dead_letters')",
        )?
        .query_row([], |row| row.get(0))
}

/// Records `entry` in `sink_dead_letters`, masking both its Record JSON
/// and its reason with `masks` at [`Scope::Kept`] (ADR 0067). An existing
/// entry for the same Sink and delivery ID updates its epoch and reason
/// and increments its attempt count.
///
/// # Errors
///
/// SQLite's, when the write fails.
pub fn record(connection: &Connection, entry: &DeadLetter, masks: &Masks) -> rusqlite::Result<()> {
    let record_json = masks.json(&entry.value, Scope::Kept);
    let reason = masks.apply(&entry.reason, Scope::Kept);
    connection
        .prepare_cached(
            "INSERT INTO sink_dead_letters (
                 sink, delivery_id, run_id, run_epoch, dataset, record_number,
                 committed_at, record, reason, attempts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (sink, delivery_id) DO UPDATE SET
                 run_epoch = excluded.run_epoch,
                 record = excluded.record,
                 reason = excluded.reason,
                 attempts = sink_dead_letters.attempts + 1",
        )?
        .execute((
            entry.sink.as_str(),
            entry.delivery_id.as_str(),
            entry.run.run_id.as_str(),
            integer(entry.run.run_epoch),
            entry.dataset.as_str(),
            integer(entry.number),
            entry.committed_at.as_str(),
            record_json.as_str(),
            reason.as_ref(),
            integer(entry.attempts.max(1)),
        ))?;
    Ok(())
}

/// Every dead letter stored for `sink` in `connection`, in Dataset and
/// Record number order. Answers empty when the table does not exist.
///
/// # Errors
///
/// SQLite's, when the query or JSON decoding fails.
pub fn list(connection: &Connection, sink: &str) -> rusqlite::Result<Vec<DeadLetter>> {
    if !has_table(connection)? {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare_cached(
        "SELECT sink, delivery_id, run_id, run_epoch, dataset, record_number,
                committed_at, record, reason, attempts
         FROM sink_dead_letters
         WHERE sink = ?1
         ORDER BY dataset, record_number, delivery_id",
    )?;
    statement
        .query_map([sink], |row| {
            let record_text: String = row.get(7)?;
            let value: Value = serde_json::from_str(&record_text)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let run_epoch: i64 = row.get(3)?;
            let number: i64 = row.get(5)?;
            let attempts: i64 = row.get(9)?;
            Ok(DeadLetter {
                sink: row.get(0)?,
                delivery_id: DeliveryId::from_stored(row.get(1)?),
                run: RunCopy {
                    run_id: row.get(2)?,
                    run_epoch: u64::try_from(run_epoch).unwrap_or_default(),
                },
                dataset: row.get(4)?,
                number: u64::try_from(number).unwrap_or_default(),
                committed_at: row.get(6)?,
                value,
                reason: row.get(8)?,
                attempts: u64::try_from(attempts).unwrap_or(1),
            })
        })?
        .collect()
}

/// Removes the dead letter `delivery_id` of `sink` once a retry delivered
/// it.
///
/// # Errors
///
/// SQLite's, when the delete fails.
pub fn remove(
    connection: &Connection,
    sink: &str,
    delivery_id: &DeliveryId,
) -> rusqlite::Result<()> {
    if !has_table(connection)? {
        return Ok(());
    }
    connection
        .prepare_cached("DELETE FROM sink_dead_letters WHERE sink = ?1 AND delivery_id = ?2")?
        .execute((sink, delivery_id.as_str()))?;
    Ok(())
}

/// How many dead letters `connection` holds for `sink`.
///
/// # Errors
///
/// SQLite's, when the query fails.
pub fn count(connection: &Connection, sink: &str) -> rusqlite::Result<u64> {
    if !has_table(connection)? {
        return Ok(0);
    }
    let count: i64 = connection
        .prepare_cached("SELECT count(*) FROM sink_dead_letters WHERE sink = ?1")?
        .query_row([sink], |row| row.get(0))?;
    Ok(u64::try_from(count).unwrap_or_default())
}

/// A count or number as SQLite's integer.
fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
