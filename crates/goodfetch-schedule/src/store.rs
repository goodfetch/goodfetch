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
        created_at TEXT NOT NULL,
        jitter_ms INTEGER NOT NULL DEFAULT 0,
        settings TEXT,
        limits TEXT
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
const COLUMNS: &str =
    "id, target, trigger, time_zone, version, state, creator, created_at, settings, limits";

/// Outcome of an atomic [`create`] attempt inside the store's write transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreateOutcome {
    /// The Schedule was inserted.
    Created,
    /// A Schedule with that ID already existed.
    Exists,
    /// The store already held `existing_count` Schedules, reaching `max_count`.
    CapExceeded { existing_count: u64 },
}

/// Records `schedule` with its fixed `jitter_ms` unless the store already
/// holds a Schedule of its ID or already holds at least `max_count`
/// Schedules, drawing the store's ID via `store_id` on first write, all in
/// one transaction.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn create(
    store: &UserStore,
    schedule: ScheduleInfo,
    jitter_ms: u64,
    max_count: Option<u64>,
    store_id: impl FnOnce() -> String + Send + 'static,
) -> Result<CreateOutcome, UserStoreError> {
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
            let exists: bool = connection
                .prepare_cached("SELECT 1 FROM schedules WHERE id = ?1")?
                .query_row([schedule.schedule_id.as_str()], |_| Ok(()))
                .optional()?
                .is_some();
            if exists {
                return Ok(CreateOutcome::Exists);
            }
            if let Some(cap) = max_count {
                let count_i64: i64 = connection
                    .prepare_cached("SELECT COUNT(*) FROM schedules")?
                    .query_row([], |row| row.get(0))?;
                let existing_count = u64::try_from(count_i64).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
                })?;
                if existing_count >= cap {
                    return Ok(CreateOutcome::CapExceeded { existing_count });
                }
            }
            let version = i64::try_from(schedule.version)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let jitter = i64::try_from(jitter_ms)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let settings_json = schedule.settings.as_ref().map(json).transpose()?;
            let limits_json = schedule.limits.as_ref().map(json).transpose()?;
            let inserted = connection
                .prepare_cached(
                    "INSERT INTO schedules
                        (id, target, trigger, time_zone, version, state, creator, created_at, jitter_ms, settings, limits)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
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
                    jitter,
                    settings_json,
                    limits_json,
                ])?;
            Ok(if inserted == 1 {
                CreateOutcome::Created
            } else {
                CreateOutcome::Exists
            })
        })
        .await
}

/// How many Schedules the store currently holds (0 when the store does not
/// exist yet).
///
/// # Errors
///
/// The store's, or a negative count from SQLite.
pub(crate) async fn count(store: &UserStore) -> Result<u64, UserStoreError> {
    store
        .read(0, |connection| {
            let total: i64 = connection
                .prepare_cached("SELECT COUNT(*) FROM schedules")?
                .query_row([], |row| row.get(0))?;
            u64::try_from(total).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
            })
        })
        .await
}

/// The stored fixed jitter offset of `id`, in milliseconds, if the store
/// holds it.
///
/// # Errors
///
/// The store's, or a negative value in the row.
#[cfg(test)]
pub(crate) async fn jitter_ms(
    store: &UserStore,
    id: &ScheduleId,
) -> Result<Option<u64>, UserStoreError> {
    let id = id.as_str().to_owned();
    store
        .read(None, move |connection| {
            let stored: Option<i64> = connection
                .prepare_cached("SELECT jitter_ms FROM schedules WHERE id = ?1")?
                .query_row([id], |row| row.get(0))
                .optional()?;
            stored
                .map(|value| {
                    u64::try_from(value).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
                    })
                })
                .transpose()
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

fn from_optional_json<T: DeserializeOwned>(
    row: &Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = row.get(column)?;
    text.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
        })
    })
    .transpose()
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
        settings: from_optional_json(row, 8)?,
        limits: from_optional_json(row, 9)?,
    })
}

#[cfg(test)]
mod tests;
