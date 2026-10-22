//! A Run's Key-value store (ADRs 0088, 0102 and 0103): JSON values by key
//! in a table of the Run's run state, so they travel in its Checkpoints and
//! a resume, here or on another machine, finds them (ADR 0002). The store's
//! operations run on a connection of the run file: a live Run's writer for
//! `set` and `delete`, whose commit decides the value, the last commit
//! winning per key with no compare-and-set, and the Run reader's read-only
//! connection for `get` and `keys`, so a read sees committed values and
//! never takes the writer lock: it serves a view of a Run no program holds
//! without resuming it. A crawl attempt's staging runs the same operations
//! in its own commit (ADR 0093), so this module never commits on its own
//! inside an attempt. A key is 1 to 512 Unicode scalar values with no
//! control character, and a top-level null is no value, so a null from
//! `get` always means an absent key; both are refused with `value_invalid`.

use goodfetch_protocol::key_value_store::{
    KEY, KEY_MAX_SCALARS, KEY_VALUE_STORE_DELETE, KEY_VALUE_STORE_GET, KEY_VALUE_STORE_KEYS,
    KEY_VALUE_STORE_SET, KeyValueStoreDeleted, KeyValueStoreKey, KeyValueStoreKeyParams,
    KeyValueStoreKeysEnded, KeyValueStoreKeysParams, KeyValueStoreSet, KeyValueStoreSetParams,
    KeyValueStoreValue, Refused, VALUE_INVALID, ValueInvalidDetails, ValueInvalidReason,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::Value;

use crate::composition::{CallContext, CallError, Handler};
use crate::rpc::notification_line;
use crate::run::run_error;
use crate::run_state::{RunStateError, RunStateTables};
use crate::runs::{Run, RunError};

/// The store's table, which every run file holds.
pub const TABLES: RunStateTables = RunStateTables {
    name: "key-value-store",
    statements: &[
        "CREATE TABLE IF NOT EXISTS key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT NOT NULL) STRICT",
    ],
};

/// The most keys one read of `keys` takes from the run file.
const KEYS_PAGE: usize = 256;

/// Why a key is refused, or `None` for a key of the store's shape.
pub fn key_refusal(key: &str) -> Option<ValueInvalidReason> {
    if key.is_empty() {
        Some(ValueInvalidReason::KeyEmpty)
    } else if key.chars().any(char::is_control) {
        Some(ValueInvalidReason::KeyControlCharacter)
    } else if key.chars().nth(KEY_MAX_SCALARS).is_some() {
        Some(ValueInvalidReason::KeyTooLong)
    } else {
        None
    }
}

/// The value the store holds for `key`, or `None` when it is absent.
///
/// # Errors
///
/// SQLite's, and a conversion failure for a stored value that is not JSON.
pub fn get(connection: &Connection, key: &str) -> rusqlite::Result<Option<Value>> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM key_values WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    stored
        .map(|text| {
            serde_json::from_str(&text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

/// Stores `value` for `key`, replacing the value it held.
///
/// # Errors
///
/// SQLite's.
pub fn set(connection: &Connection, key: &str, value: &Value) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT INTO key_values (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (key, value.to_string()),
        )
        .map(drop)
}

/// Removes `key`, and answers whether it existed.
///
/// # Errors
///
/// SQLite's.
pub fn delete(connection: &Connection, key: &str) -> rusqlite::Result<bool> {
    Ok(connection.execute("DELETE FROM key_values WHERE key = ?1", [key])? > 0)
}

/// At most `most` keys that start with `prefix`, after `after` when given,
/// in the order of their Unicode scalar values: SQLite compares text by its
/// UTF-8 bytes, which order as their scalar values do.
///
/// # Errors
///
/// SQLite's.
pub fn keys(
    connection: &Connection,
    prefix: &str,
    after: Option<&str>,
    most: usize,
) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT key FROM key_values
         WHERE substr(key, 1, length(?1)) = ?1 AND (?2 IS NULL OR key > ?2)
         ORDER BY key LIMIT ?3",
    )?;
    let most = i64::try_from(most).unwrap_or(i64::MAX);
    statement
        .query_map((prefix, after, most), |row| row.get(0))?
        .collect()
}

/// The store's operations' handlers.
pub(crate) fn handlers() -> [Handler; 4] {
    [
        Handler::new(&KEY_VALUE_STORE_GET, handle_get),
        Handler::new(&KEY_VALUE_STORE_SET, handle_set),
        Handler::new(&KEY_VALUE_STORE_DELETE, handle_delete),
        Handler::new(&KEY_VALUE_STORE_KEYS, handle_keys).streaming(),
    ]
}

/// `value_invalid` for `key`, of which `refused` was refused for `reason`.
fn value_invalid(key: String, reason: ValueInvalidReason) -> CallError {
    let (refused, message, fix) = match reason {
        ValueInvalidReason::ValueNull => (
            Refused::Value,
            format!(
                "The Key-value store keeps no null, which `get` answers for an absent key: the value of {key:?} is null."
            ),
            "Remove the key with `delete`, or store a value other than null.",
        ),
        ValueInvalidReason::KeyEmpty => (
            Refused::Key,
            "The Key-value store refused an empty key.".to_owned(),
            "Give a key of 1 to 512 Unicode scalar values with no control character.",
        ),
        ValueInvalidReason::KeyTooLong => (
            Refused::Key,
            format!(
                "The Key-value store refused the key {key:?}: it holds more than 512 Unicode scalar values."
            ),
            "Give a key of 1 to 512 Unicode scalar values with no control character.",
        ),
        ValueInvalidReason::KeyControlCharacter => (
            Refused::Key,
            format!("The Key-value store refused the key {key:?}: it holds a control character."),
            "Give a key of 1 to 512 Unicode scalar values with no control character.",
        ),
    };
    CallError::new(
        &VALUE_INVALID,
        message,
        &ValueInvalidDetails {
            key,
            refused,
            reason,
            fix: fix.to_owned(),
        },
    )
}

/// Refuses a key out of the store's shape.
fn checked(key: String) -> Result<String, CallError> {
    match key_refusal(&key) {
        Some(reason) => Err(value_invalid(key, reason)),
        None => Ok(key),
    }
}

/// The live Run `run_id`, whose writer a write goes through.
async fn live(run_id: &RunId, context: &CallContext) -> Result<Run, CallError> {
    context
        .runs()
        .live(run_id.clone())
        .await
        .map_err(|error| match error {
            RunError::NotFound(_) => CallError::new(
                &RUN_NOT_FOUND,
                format!("No live Run has the Run ID `{run_id}` here: start or resume it to write its Key-value store."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ),
            other => run_error(other, context.identity()),
        })
}

/// The failure of a write to `run`'s state.
fn unwritten(run: &Run, error: &RunStateError, context: &CallContext) -> CallError {
    run_error(
        RunError::Unavailable {
            run_id: run.id().clone(),
            folder: run.folder().to_owned(),
            error: error.to_string(),
        },
        context.identity(),
    )
}

async fn handle_get(
    params: KeyValueStoreKeyParams,
    context: CallContext,
) -> Result<KeyValueStoreValue, CallError> {
    let key = checked(params.key)?;
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context.runs().reader(params.run_id).await.map_err(failed)?;
    let value = reader
        .read(move |connection| get(connection, &key))
        .await
        .map_err(failed)?;
    Ok(KeyValueStoreValue {
        value: value.unwrap_or(Value::Null),
    })
}

async fn handle_set(
    params: KeyValueStoreSetParams,
    context: CallContext,
) -> Result<KeyValueStoreSet, CallError> {
    let key = checked(params.key)?;
    if params.value.is_null() {
        return Err(value_invalid(key, ValueInvalidReason::ValueNull));
    }
    let run = live(&params.run_id, &context).await?;
    let value = params.value;
    run.state()
        .write(move |connection| set(connection, &key, &value))
        .await
        .map_err(|error| unwritten(&run, &error, &context))?;
    Ok(KeyValueStoreSet {})
}

async fn handle_delete(
    params: KeyValueStoreKeyParams,
    context: CallContext,
) -> Result<KeyValueStoreDeleted, CallError> {
    let key = checked(params.key)?;
    let run = live(&params.run_id, &context).await?;
    let existed = run
        .state()
        .write(move |connection| delete(connection, &key))
        .await
        .map_err(|error| unwritten(&run, &error, &context))?;
    Ok(KeyValueStoreDeleted { existed })
}

/// Reads the keys a page at a time from the committed state, and sends
/// each in a `$/key` notification once a credit allows it.
async fn handle_keys(
    params: KeyValueStoreKeysParams,
    context: CallContext,
) -> Result<KeyValueStoreKeysEnded, CallError> {
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context.runs().reader(params.run_id).await.map_err(failed)?;
    let prefix = params.prefix.unwrap_or_default();
    let stream = context.stream();
    let mut after: Option<String> = None;
    let mut count: u64 = 0;
    loop {
        let (page_prefix, page_after) = (prefix.clone(), after.clone());
        let page = reader
            .read(move |connection| {
                keys(connection, &page_prefix, page_after.as_deref(), KEYS_PAGE)
            })
            .await
            .map_err(failed)?;
        let last = page.len() < KEYS_PAGE;
        for key in page {
            let _ = stream.credits.spend(1).await;
            let line = notification_line(
                KEY,
                &KeyValueStoreKey {
                    id: stream.id.clone(),
                    key: key.clone(),
                },
            );
            if stream.outgoing.send(line).await.is_err() {
                return Ok(KeyValueStoreKeysEnded { count });
            }
            count += 1;
            after = Some(key);
        }
        if last {
            return Ok(KeyValueStoreKeysEnded { count });
        }
    }
}

#[cfg(test)]
mod tests;
