//! The Schedule store (ADRs 0036, 0060 and 0098): one per-user, per-major
//! SQLite file beside the Run index, in the Goodfetch home, which the
//! engine opens as a user store ([`goodfetch_engine::user_store`]) through
//! a VFS named by a value, none in production, and writes on one writer
//! actor. It is the source of truth of a user's Schedules, never rebuilt.
//!
//! It records its format's `major.minor` when it is made, and holds a store
//! ID drawn from the engine's `Entropy` by its first write, not a secret,
//! which later tells one store's run IDs from another's. Its statements
//! name only the columns they carry, so a row or a table of a later minor
//! stays as it is (ADR 0098).

use goodfetch_engine::StoredTime;
use goodfetch_engine::user_store::{UserStore, UserStoreError, UserStoreKind};
use goodfetch_protocol::catalog::{ProtocolVersion, schema_of};
use goodfetch_protocol::formats::{Format, Rule, Storage};
use goodfetch_protocol::home;
use goodfetch_protocol::schedule::{ScheduleId, ScheduleInfo};
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The Schedule store's format: one row a Schedule. An engine reads its
/// own major up to its own minor and leaves a later minor's rows as they
/// are (`Rule::SharedStore`); its registration in the format list and its
/// compatibility corpus are the formats ticket's.
pub(crate) const SCHEDULE_STORE: Format = Format {
    name: "schedule-store",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::SharedStore,
    hello_from: &[],
    schema: schema_of::<ScheduleInfo>,
};

/// The store's tables of this version.
const TABLES: &[&str] = &["store", "schedules"];

/// The statements that make them.
const STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS store (
        only INTEGER PRIMARY KEY CHECK (only = 1),
        store_id TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS schedules (
        id TEXT PRIMARY KEY,
        target TEXT NOT NULL,
        trigger TEXT NOT NULL,
        time_zone TEXT NOT NULL,
        version INTEGER NOT NULL,
        state TEXT NOT NULL,
        creator TEXT NOT NULL,
        created_at TEXT NOT NULL
    ) STRICT",
];

/// The Schedule store, as the schedule crate's contribution declares it.
pub(crate) static STORE: UserStoreKind = UserStoreKind::new(
    home::SCHEDULE_STORE,
    &SCHEDULE_STORE,
    TABLES,
    STATEMENTS,
    "schedule store",
);

/// The columns a Schedule's row is read from, in [`schedule`]'s order.
const COLUMNS: &str = "id, target, trigger, time_zone, version, state, creator, created_at";

/// Records `schedule` unless the store holds a Schedule of its ID, the
/// store's ID drawn by `store_id` first when it holds none yet, in one
/// transaction: whether it recorded it.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn create(
    store: &UserStore,
    schedule: ScheduleInfo,
    store_id: impl FnOnce() -> String + Send + 'static,
) -> Result<bool, UserStoreError> {
    store
        .write(move |connection| {
            let held: Option<String> = connection
                .prepare_cached("SELECT store_id FROM store")?
                .query_row([], |row| row.get(0))
                .optional()?;
            if held.is_none() {
                connection
                    .prepare_cached("INSERT INTO store (only, store_id) VALUES (1, ?1)")?
                    .execute([store_id()])?;
            }
            let version = i64::try_from(schedule.version)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let inserted = connection
                .prepare_cached(
                    "INSERT INTO schedules
                        (id, target, trigger, time_zone, version, state, creator, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT (id) DO NOTHING",
                )?
                .execute(params![
                    schedule.schedule_id.as_str(),
                    json(&schedule.target)?,
                    json(&schedule.trigger)?,
                    schedule.time_zone,
                    version,
                    schedule.state.as_str(),
                    schedule.creator,
                    StoredTime(schedule.created_at),
                ])?;
            Ok(inserted == 1)
        })
        .await
}

/// Every Schedule the store holds, by ID.
///
/// # Errors
///
/// The store's, or a row this version cannot read.
pub(crate) async fn list(store: &UserStore) -> Result<Vec<ScheduleInfo>, UserStoreError> {
    store
        .read(Vec::new(), |connection| {
            connection
                .prepare_cached(&format!("SELECT {COLUMNS} FROM schedules ORDER BY id"))?
                .query_map([], schedule)?
                .collect()
        })
        .await
}

/// The Schedule of `id`, if the store holds one.
///
/// # Errors
///
/// As [`list`].
pub(crate) async fn get(
    store: &UserStore,
    id: &ScheduleId,
) -> Result<Option<ScheduleInfo>, UserStoreError> {
    let id = id.as_str().to_owned();
    store
        .read(None, move |connection| {
            connection
                .prepare_cached(&format!("SELECT {COLUMNS} FROM schedules WHERE id = ?1"))?
                .query_row([id], schedule)
                .optional()
        })
        .await
}

/// Deletes the Schedule of `id`: whether the store held one.
///
/// # Errors
///
/// The store's, after which nothing was deleted.
pub(crate) async fn delete(store: &UserStore, id: &ScheduleId) -> Result<bool, UserStoreError> {
    let id = id.as_str().to_owned();
    store
        .write(move |connection| {
            let deleted = connection
                .prepare_cached("DELETE FROM schedules WHERE id = ?1")?
                .execute([id])?;
            Ok(deleted == 1)
        })
        .await
}

/// A store ID made of the draws `high` and `low`: their 128 bits in
/// lowercase Crockford base32, 26 characters.
pub(crate) fn made_store_id(high: u64, low: u64) -> String {
    ulid::Ulid(u128::from(high) << 64 | u128::from(low))
        .to_string()
        .to_lowercase()
}

fn json(value: &impl Serialize) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn from_json<T: DeserializeOwned>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

/// The Schedule of a row of [`COLUMNS`].
fn schedule(row: &Row<'_>) -> rusqlite::Result<ScheduleInfo> {
    let id: String = row.get(0)?;
    let version: i64 = row.get(4)?;
    let state: String = row.get(5)?;
    let StoredTime(created_at) = row.get(7)?;
    Ok(ScheduleInfo {
        schedule_id: ScheduleId::parse(&id).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
        })?,
        target: from_json(row, 1)?,
        trigger: from_json(row, 2)?,
        time_zone: row.get(3)?,
        version: u64::try_from(version).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Integer, Box::new(error))
        })?,
        state: serde_json::from_value(serde_json::Value::String(state)).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(error))
        })?,
        creator: row.get(6)?,
        created_at,
    })
}

#[cfg(test)]
mod tests;
