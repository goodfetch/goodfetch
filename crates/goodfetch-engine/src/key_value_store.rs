//! A Run's Key-value store (ADRs 0088, 0102 and 0103): JSON values, and
//! bytes with their content type, by key in a table of the Run's run
//! state, so they travel in its Checkpoints and a resume, here or on
//! another machine, finds them (ADR 0002). Bytes of 1 MiB or less stay in
//! the key's row; larger ones are kept as a content-addressed file of the
//! Run's folder (`run_files`), which the row references and Checkpoints
//! carry, and which goes once no key or other record names it. The store's
//! operations run on a connection of the run file: a live Run's writer for
//! `set` and `delete`, whose commit decides the value, the last commit
//! winning per key with no compare-and-set, and the Run reader's read-only
//! connection for `get` and `keys`, so a read sees committed values and
//! never takes the writer lock: it serves a view of a Run no program holds
//! without resuming it. A crawl attempt's staging runs the same operations
//! in its own commit (ADR 0093), so this module never commits on its own
//! inside an attempt. A key is 1 to 512 Unicode scalar values with no
//! control character, and a top-level null is no value, so a null from
//! `get` always means an absent key; both are refused with `value_invalid`,
//! as is reading bytes with `get` or JSON with `get_bytes` (S2).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::key_value_store::{
    KEY, KEY_MAX_SCALARS, KEY_VALUE_STORE_DELETE, KEY_VALUE_STORE_GET, KEY_VALUE_STORE_GET_BYTES,
    KEY_VALUE_STORE_KEYS, KEY_VALUE_STORE_SET, KEY_VALUE_STORE_SET_BYTES, KeyValueStoreAttempt,
    KeyValueStoreBytes, KeyValueStoreBytesValue, KeyValueStoreContent, KeyValueStoreDeleted,
    KeyValueStoreKey, KeyValueStoreKeyParams, KeyValueStoreKeysEnded, KeyValueStoreKeysParams,
    KeyValueStoreSet, KeyValueStoreSetBytesParams, KeyValueStoreSetParams, KeyValueStoreValue,
    Refused, VALUE_INVALID, ValueInvalidDetails, ValueInvalidReason,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::Value;

use crate::composition::{CallContext, CallError, Handler};
use crate::run::{run_error, through_owner};
use crate::run_files::{self, FileKind};
use crate::run_state::{RunStateError, RunStateTables};
use crate::runs::{Run, RunError, RunReader};
use crate::staging::{AttemptWrites, StagedKey, StagedWrite, Unstaged};

/// The store's table, which every run file holds. A row holds JSON in
/// `value`, or bytes with their `content_type`, either in `content` or in
/// the kept file at the position `file` of the Run's `run_files`.
pub const TABLES: RunStateTables = RunStateTables {
    name: "key-value-store",
    statements: &[
        "CREATE TABLE IF NOT EXISTS key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT, content BLOB, content_type TEXT, file INTEGER, \
         CHECK ((value IS NULL) <> (content_type IS NULL)), \
         CHECK (content_type IS NULL OR ((content IS NULL) <> (file IS NULL)))) STRICT",
    ],
};

/// The most bytes a key's row holds itself; larger bytes are kept as a
/// file of the Run's folder.
pub const INLINE_MAX_BYTES: usize = 1024 * 1024;

/// The most keys one read of `keys` takes from the run file.
const KEYS_PAGE: usize = 256;

/// The most times `get_bytes` reads a key whose kept file went while it
/// read it.
const READS: usize = 3;

/// What the store holds for a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    /// A JSON value.
    Json(Value),
    /// Bytes, with their content type.
    Bytes {
        /// Their content type, as `set_bytes` gave it.
        content_type: String,
        /// Where they are.
        content: Content,
    },
}

/// Where a key's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// In its row.
    Inline(Vec<u8>),
    /// In the kept file of the Run's folder with this SHA-256.
    File {
        /// The SHA-256 of the bytes, in lowercase hexadecimal.
        sha256: String,
        /// Their size in bytes.
        size: u64,
    },
}

/// What a write took out of the store for its key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removed {
    /// Whether the key held something.
    pub existed: bool,
    /// The SHA-256 of the kept file the key referenced, which goes once no
    /// record names it.
    pub file: Option<String>,
}

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

/// A row of the store: `value`, `content`, `content_type` and `file`.
type Row = (Option<String>, Option<Vec<u8>>, Option<String>, Option<i64>);

/// What the store holds for `key`, or `None` when it is absent.
///
/// # Errors
///
/// SQLite's, and a conversion failure for a stored value that is not JSON
/// or for bytes whose kept file the Run has no record of.
pub fn get(connection: &Connection, key: &str) -> rusqlite::Result<Option<Stored>> {
    let row: Option<Row> = connection
        .query_row(
            "SELECT value, content, content_type, file FROM key_values WHERE key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let damaged = |what: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Null,
            format!("the Key-value store's row of {key:?} {what}").into(),
        )
    };
    match row {
        None => Ok(None),
        Some((Some(text), ..)) => serde_json::from_str(&text)
            .map(|value| Some(Stored::Json(value)))
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            }),
        Some((None, Some(bytes), Some(content_type), _)) => Ok(Some(Stored::Bytes {
            content_type,
            content: Content::Inline(bytes),
        })),
        Some((None, None, Some(content_type), Some(position))) => {
            let record = match u64::try_from(position) {
                Ok(position) => run_files::find(connection, position)?,
                Err(_) => None,
            }
            .ok_or_else(|| damaged("names a kept file the Run has no record of"))?;
            Ok(Some(Stored::Bytes {
                content_type,
                content: Content::File {
                    sha256: record.sha256,
                    size: record.size,
                },
            }))
        }
        Some(_) => Err(damaged("holds neither JSON nor bytes")),
    }
}

/// Takes out what the store holds for `key`, with the record of its kept
/// file.
fn take(connection: &Connection, key: &str) -> rusqlite::Result<Removed> {
    let removed: Option<Option<i64>> = connection
        .query_row(
            "DELETE FROM key_values WHERE key = ?1 RETURNING file",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    let file = match removed
        .flatten()
        .and_then(|position| u64::try_from(position).ok())
    {
        Some(position) => run_files::unrecord(connection, position)?,
        None => None,
    };
    Ok(Removed {
        existed: removed.is_some(),
        file,
    })
}

/// Stores `value` for `key`, replacing what it held, and answers what it
/// took out.
///
/// # Errors
///
/// SQLite's.
pub fn set(connection: &Connection, key: &str, value: &Value) -> rusqlite::Result<Removed> {
    let removed = take(connection, key)?;
    connection.execute(
        "INSERT INTO key_values (key, value) VALUES (?1, ?2)",
        (key, value.to_string()),
    )?;
    Ok(removed)
}

/// Stores bytes of `content_type` for `key`, replacing what it held, and
/// answers what it took out: in its row, or, for [`Content::File`], as a
/// reference to the kept file the Run's folder already holds, which this
/// records.
///
/// # Errors
///
/// SQLite's.
pub fn set_bytes(
    connection: &Connection,
    key: &str,
    content_type: &str,
    content: &Content,
) -> rusqlite::Result<Removed> {
    let removed = take(connection, key)?;
    match content {
        Content::Inline(bytes) => connection.execute(
            "INSERT INTO key_values (key, content, content_type) VALUES (?1, ?2, ?3)",
            (key, bytes, content_type),
        )?,
        Content::File { sha256, size } => {
            let position = run_files::record(connection, FileKind::KeyValue, key, sha256, *size)?;
            connection.execute(
                "INSERT INTO key_values (key, content_type, file) VALUES (?1, ?2, ?3)",
                (
                    key,
                    content_type,
                    i64::try_from(position).unwrap_or(i64::MAX),
                ),
            )?
        }
    };
    Ok(removed)
}

/// Removes `key`, and answers what it took out.
///
/// # Errors
///
/// SQLite's.
pub fn delete(connection: &Connection, key: &str) -> rusqlite::Result<Removed> {
    take(connection, key)
}

/// At most `most` keys that start with `prefix`, after `after` when given,
/// in the order of their Unicode scalar values: SQLite compares text by its
/// UTF-8 bytes, which order as their scalar values do. A prefix's keys are
/// therefore contiguous in the key index: a page searches it from its
/// first key and stops at the first key past the prefix, so it reads only
/// the keys it answers.
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
    let mut statement = match after {
        Some(after) if after >= prefix => {
            let mut statement =
                connection.prepare("SELECT key FROM key_values WHERE key > ?1 ORDER BY key")?;
            statement.raw_bind_parameter(1, after)?;
            statement
        }
        _ => {
            let mut statement =
                connection.prepare("SELECT key FROM key_values WHERE key >= ?1 ORDER BY key")?;
            statement.raw_bind_parameter(1, prefix)?;
            statement
        }
    };
    let mut rows = statement.raw_query();
    let mut found = Vec::new();
    while found.len() < most {
        let Some(row) = rows.next()? else { break };
        let key: String = row.get(0)?;
        if !key.starts_with(prefix) {
            break;
        }
        found.push(key);
    }
    Ok(found)
}

/// The store's operations' handlers.
pub(crate) fn handlers() -> [Handler; 6] {
    [
        Handler::new(&KEY_VALUE_STORE_GET, handle_get),
        Handler::new(&KEY_VALUE_STORE_SET, handle_set),
        Handler::new(&KEY_VALUE_STORE_GET_BYTES, handle_get_bytes),
        Handler::new(&KEY_VALUE_STORE_SET_BYTES, handle_set_bytes),
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
        ValueInvalidReason::ValueBytes => (
            Refused::Value,
            format!(
                "The key {key:?} holds bytes, which `get` does not read: it answers JSON only."
            ),
            "Read it with `getBytes` (`get_bytes` in Python and Rust), which answers the bytes and their content type.",
        ),
        ValueInvalidReason::ValueJson => (
            Refused::Value,
            format!(
                "The key {key:?} holds JSON, which `getBytes` does not read: it answers bytes only."
            ),
            "Read it with `get`, which answers the JSON value.",
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

/// Where `run`'s crawl attempts stage their writes; `invalid_params` on a
/// Run with no crawl, which has no attempt.
fn attempt_writes_of(run: &Run) -> Result<Arc<dyn AttemptWrites>, CallError> {
    run.attempt_writes().ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "The Run `{}` is no crawl, so its Key-value store takes no `attempt`: call it without one.",
                run.id()
            ),
        )
    })
}

/// The call's failure for a write its attempt did not stage.
fn unstaged(unstaged: Unstaged) -> CallError {
    CallError::new(unstaged.code, unstaged.message, &unstaged.details)
}

/// What `attempt`, when a read names one, staged for `key` in the live Run
/// `run_id`; untouched for a read through no attempt.
async fn staged_for(
    run_id: &RunId,
    attempt: Option<&KeyValueStoreAttempt>,
    key: &str,
    context: &CallContext,
) -> Result<StagedKey, CallError> {
    let Some(attempt) = attempt else {
        return Ok(StagedKey::Untouched);
    };
    let run = live(run_id, context).await?;
    attempt_writes_of(&run)?
        .staged(&attempt.crawl_request_id, attempt.lease_epoch, key)
        .map_err(unstaged)
}

/// Applies an attempt's staged `writes`, in order, inside the transaction
/// of `connection` that commits the attempt (ADR 0093), and answers the
/// kept files they took out, which [`forget_files`] forgets once that
/// transaction committed. Bytes past [`INLINE_MAX_BYTES`] are kept as a
/// file of the Run's folder `folder`, written now, so a discarded attempt
/// leaves no file; the writer that runs this also runs every forget, so
/// none deletes the file before its record commits.
///
/// # Errors
///
/// SQLite's, and a kept file that cannot be written, which fails the
/// commit.
pub fn apply(
    connection: &Connection,
    folder: &Path,
    writes: &[StagedWrite],
) -> rusqlite::Result<Vec<String>> {
    let mut files = Vec::new();
    for write in writes {
        let removed = match write {
            StagedWrite::Set { key, value } => set(connection, key, value)?,
            StagedWrite::SetBytes {
                key,
                content_type,
                bytes,
            } if bytes.len() > INLINE_MAX_BYTES => {
                let (sha256, size) = run_files::store(folder, &mut bytes.as_slice())
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                set_bytes(
                    connection,
                    key,
                    content_type,
                    &Content::File { sha256, size },
                )?
            }
            StagedWrite::SetBytes {
                key,
                content_type,
                bytes,
            } => set_bytes(
                connection,
                key,
                content_type,
                &Content::Inline(bytes.clone()),
            )?,
            StagedWrite::Delete { key } => delete(connection, key)?,
        };
        files.extend(removed.file);
    }
    Ok(files)
}

/// Forgets each of `files`, kept files of `run`'s folder that committed
/// writes took out, once no record names it.
///
/// # Cancel safety
///
/// Dropping the future leaves the files not yet forgotten in the folder,
/// where no record names them.
pub async fn forget_files(run: &Run, files: Vec<String>) {
    for file in files {
        forget(run, Some(file)).await;
    }
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

/// The failure of keeping bytes in `run`'s folder.
fn unkept(run: &Run, error: &std::io::Error, context: &CallContext) -> CallError {
    run_error(
        RunError::Unavailable {
            run_id: run.id().clone(),
            folder: run.folder().to_owned(),
            error: format!("could not keep the bytes in the Run's folder: {error}"),
        },
        context.identity(),
    )
}

/// The failure of reading the kept file of `key` of the Run `run_id` from
/// its folder `folder`.
fn unread(
    run_id: RunId,
    folder: PathBuf,
    key: &str,
    error: &std::io::Error,
    context: &CallContext,
) -> CallError {
    run_error(
        RunError::Unavailable {
            run_id,
            folder,
            error: format!("could not read the kept file of the key {key:?}: {error}"),
        },
        context.identity(),
    )
}

/// Has the kept file `file` deleted from `run`'s folder once no record
/// names it, in the writer's order after the commit that took it out.
async fn forget(run: &Run, file: Option<String>) {
    let Some(sha256) = file else {
        return;
    };
    let folder = run.folder().to_owned();
    if let Err(error) = run
        .state()
        .write(move |connection| run_files::forget(connection, &folder, &sha256))
        .await
    {
        tracing::warn!(run_id = %run.id(), %error, "a kept file the Key-value store no longer names stays in the Run's folder");
    }
}

async fn handle_get(
    params: KeyValueStoreKeyParams,
    context: CallContext,
) -> Result<KeyValueStoreValue, CallError> {
    if let Some(answer) =
        through_owner(&context, &KEY_VALUE_STORE_GET, &params.run_id, &params).await?
    {
        return Ok(answer);
    }
    let key = checked(params.key)?;
    match staged_for(&params.run_id, params.attempt.as_ref(), &key, &context).await? {
        StagedKey::Set(value) => return Ok(KeyValueStoreValue { value }),
        StagedKey::SetBytes { .. } => {
            return Err(value_invalid(key, ValueInvalidReason::ValueBytes));
        }
        StagedKey::Deleted => return Ok(KeyValueStoreValue { value: Value::Null }),
        StagedKey::Untouched => {}
    }
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context.runs().reader(params.run_id).await.map_err(failed)?;
    let read = key.clone();
    let stored = reader
        .read(move |connection| get(connection, &read))
        .await
        .map_err(failed)?;
    match stored {
        None => Ok(KeyValueStoreValue { value: Value::Null }),
        Some(Stored::Json(value)) => Ok(KeyValueStoreValue { value }),
        Some(Stored::Bytes { .. }) => Err(value_invalid(key, ValueInvalidReason::ValueBytes)),
    }
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
    if let Some(attempt) = &params.attempt {
        attempt_writes_of(&run)?
            .stage(
                &attempt.crawl_request_id,
                attempt.lease_epoch,
                StagedWrite::Set { key, value },
            )
            .map_err(unstaged)?;
        return Ok(KeyValueStoreSet {});
    }
    let removed = run
        .state()
        .write(move |connection| set(connection, &key, &value))
        .await
        .map_err(|error| unwritten(&run, &error, &context))?;
    forget(&run, removed.file).await;
    Ok(KeyValueStoreSet {})
}

/// What `get_bytes` read of a key.
enum Read {
    /// The key is absent.
    Absent,
    /// The key holds JSON.
    Json,
    /// The key holds these bytes, of this content type.
    Bytes(String, Vec<u8>),
}

/// What the store holds for `key` of the Run `run_id`, a kept file's bytes
/// read from the Run's folder: a file forgotten while it was read, its key
/// replaced or deleted, makes it read the key again.
async fn read_bytes(run_id: RunId, key: &str, context: &CallContext) -> Result<Read, CallError> {
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader: RunReader = context
        .runs()
        .reader(run_id.clone())
        .await
        .map_err(failed)?;
    let folder = context
        .runs()
        .folder(run_id.clone())
        .await
        .map_err(failed)?;
    let owner = Arc::clone(&context.runs().limits().owner);
    let mut gone = std::io::Error::from(std::io::ErrorKind::NotFound);
    for _ in 0..READS {
        let read = key.to_owned();
        let stored = reader
            .read(move |connection| get(connection, &read))
            .await
            .map_err(failed)?;
        let (content_type, sha256) = match stored {
            None => return Ok(Read::Absent),
            Some(Stored::Json(_)) => return Ok(Read::Json),
            Some(Stored::Bytes {
                content_type,
                content: Content::Inline(bytes),
            }) => return Ok(Read::Bytes(content_type, bytes)),
            Some(Stored::Bytes {
                content_type,
                content: Content::File { sha256, .. },
            }) => (content_type, sha256),
        };
        let kept = folder.clone();
        match owner
            .blocking(move || run_files::read(&kept, &sha256))
            .await
        {
            Ok(bytes) => return Ok(Read::Bytes(content_type, bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => gone = error,
            Err(error) => return Err(unread(run_id, folder, key, &error, context)),
        }
    }
    Err(unread(run_id, folder, key, &gone, context))
}

async fn handle_get_bytes(
    params: KeyValueStoreKeyParams,
    context: CallContext,
) -> Result<KeyValueStoreBytesValue, CallError> {
    if let Some(answer) = through_owner(
        &context,
        &KEY_VALUE_STORE_GET_BYTES,
        &params.run_id,
        &params,
    )
    .await?
    {
        return Ok(answer);
    }
    let key = checked(params.key)?;
    match staged_for(&params.run_id, params.attempt.as_ref(), &key, &context).await? {
        StagedKey::Set(_) => return Err(value_invalid(key, ValueInvalidReason::ValueJson)),
        StagedKey::SetBytes {
            content_type,
            bytes,
        } => {
            return Ok(KeyValueStoreBytesValue {
                bytes: Some(KeyValueStoreBytes {
                    content: KeyValueStoreContent::new(bytes),
                    content_type,
                }),
            });
        }
        StagedKey::Deleted => return Ok(KeyValueStoreBytesValue { bytes: None }),
        StagedKey::Untouched => {}
    }
    match read_bytes(params.run_id, &key, &context).await? {
        Read::Absent => Ok(KeyValueStoreBytesValue { bytes: None }),
        Read::Json => Err(value_invalid(key, ValueInvalidReason::ValueJson)),
        Read::Bytes(content_type, bytes) => Ok(KeyValueStoreBytesValue {
            bytes: Some(KeyValueStoreBytes {
                content: KeyValueStoreContent::new(bytes),
                content_type,
            }),
        }),
    }
}

/// Keeps `bytes`, over [`INLINE_MAX_BYTES`], as a file of `run`'s folder
/// and references it from `key`'s row, and answers what the row held.
async fn set_kept(
    run: &Run,
    key: String,
    content_type: String,
    mut bytes: Vec<u8>,
    context: &CallContext,
) -> Result<Removed, CallError> {
    loop {
        let folder = run.folder().to_owned();
        let (given, kept) = run
            .owner()
            .blocking(move || {
                let kept = run_files::store(&folder, &mut bytes.as_slice());
                (bytes, kept)
            })
            .await;
        bytes = given;
        let (sha256, size) = kept.map_err(|error| unkept(run, &error, context))?;
        let file = run_files::file(run.folder(), &sha256)
            .expect("invariant: a SHA-256 in lowercase hex names a file");
        let (key, content_type) = (key.clone(), content_type.clone());
        let held = Content::File { sha256, size };
        // A `forget` of the same bytes ordered before this write may have
        // deleted the file just stored: then it is stored again.
        let written = run
            .state()
            .write(move |connection| {
                if file.exists() {
                    set_bytes(connection, &key, &content_type, &held).map(Some)
                } else {
                    Ok(None)
                }
            })
            .await
            .map_err(|error| unwritten(run, &error, context))?;
        if let Some(removed) = written {
            return Ok(removed);
        }
    }
}

async fn handle_set_bytes(
    params: KeyValueStoreSetBytesParams,
    context: CallContext,
) -> Result<KeyValueStoreSet, CallError> {
    let key = checked(params.key)?;
    let run = live(&params.run_id, &context).await?;
    let content_type = params.content_type;
    let bytes = params.content.into_bytes();
    if let Some(attempt) = &params.attempt {
        attempt_writes_of(&run)?
            .stage(
                &attempt.crawl_request_id,
                attempt.lease_epoch,
                StagedWrite::SetBytes {
                    key,
                    content_type,
                    bytes,
                },
            )
            .map_err(unstaged)?;
        return Ok(KeyValueStoreSet {});
    }
    let removed = if bytes.len() > INLINE_MAX_BYTES {
        set_kept(&run, key, content_type, bytes, &context).await?
    } else {
        let held = Content::Inline(bytes);
        run.state()
            .write(move |connection| set_bytes(connection, &key, &content_type, &held))
            .await
            .map_err(|error| unwritten(&run, &error, &context))?
    };
    forget(&run, removed.file).await;
    Ok(KeyValueStoreSet {})
}

async fn handle_delete(
    params: KeyValueStoreKeyParams,
    context: CallContext,
) -> Result<KeyValueStoreDeleted, CallError> {
    let key = checked(params.key)?;
    let run = live(&params.run_id, &context).await?;
    if let Some(attempt) = &params.attempt {
        let before = attempt_writes_of(&run)?
            .stage(
                &attempt.crawl_request_id,
                attempt.lease_epoch,
                StagedWrite::Delete { key: key.clone() },
            )
            .map_err(unstaged)?;
        let existed = match before {
            StagedKey::Set(_) | StagedKey::SetBytes { .. } => true,
            StagedKey::Deleted => false,
            StagedKey::Untouched => {
                let failed = |error: RunError| run_error(error, context.identity());
                let mut reader = context.runs().reader(params.run_id).await.map_err(failed)?;
                reader
                    .read(move |connection| get(connection, &key))
                    .await
                    .map_err(failed)?
                    .is_some()
            }
        };
        return Ok(KeyValueStoreDeleted { existed });
    }
    let removed = run
        .state()
        .write(move |connection| delete(connection, &key))
        .await
        .map_err(|error| unwritten(&run, &error, &context))?;
    forget(&run, removed.file).await;
    Ok(KeyValueStoreDeleted {
        existed: removed.existed,
    })
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
            let _ = stream.spend(1).await;
            let sent = stream
                .notify(
                    KEY,
                    &KeyValueStoreKey {
                        id: stream.id().clone(),
                        key: key.clone(),
                    },
                )
                .await;
            if !sent {
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
