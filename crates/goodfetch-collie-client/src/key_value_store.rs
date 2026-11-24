//! A Run's Key-value store (ADR 0103): JSON values by key, in the program's
//! own types through serde (ADR 0082), and bytes with their content type.
//! `get` answers `None` only for an absent key, since the store refuses
//! null. Through the program's own Run value a write commits at once;
//! through a Product's handle bound to a crawl attempt it names that
//! attempt, and is staged with it (ADR 0093). TypeScript's
//! `key-value-store.ts` and Python's `key_value_store.py` are its twins.

use goodfetch_protocol::key_value_store::{
    KEY_VALUE_STORE_DELETE, KEY_VALUE_STORE_GET, KEY_VALUE_STORE_GET_BYTES, KEY_VALUE_STORE_SET,
    KEY_VALUE_STORE_SET_BYTES, KeyValueStoreAttempt, KeyValueStoreBytesValue, KeyValueStoreContent,
    KeyValueStoreDeleted, KeyValueStoreKeyParams, KeyValueStoreSet, KeyValueStoreSetBytesParams,
    KeyValueStoreSetParams, KeyValueStoreValue, Refused, VALUE_INVALID, ValueInvalidDetails,
    ValueInvalidReason,
};
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::call::Call;
use crate::error::Error;
use crate::generated::handles::Run;
use crate::host::Host;
use crate::markers::warn_operation;
use crate::schema::{self, SchemaError};

/// A Run's Key-value store, which [`Run::key_value_store`] answers. Cloning
/// it is cheap and every clone is the same store.
///
/// A key is 1 to 512 Unicode scalar values with no control character;
/// another fails with `value_invalid`, naming it. A write through the
/// program's own Run value commits at once. A crawl handler that captures
/// it commits on every attempt that runs it, whatever the attempt becomes:
/// a write staged with the attempt goes through the handler's context,
/// Foxhound's `ctx.run()`.
#[derive(Clone, Debug)]
pub struct KeyValueStore {
    host: Host<RunId>,
    /// The crawl attempt its calls name, when it is bound to one.
    attempt: Option<KeyValueStoreAttempt>,
}

/// Bytes a Key-value store holds for a key, with their content type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredBytes {
    /// The bytes.
    pub content: Vec<u8>,
    /// Their content type, as `set_bytes` gave it, such as `image/png`.
    pub content_type: String,
}

impl Run {
    /// The Run's Key-value store, whose writes commit at once. It sends
    /// nothing, and once the Run is closed each of the store's calls fails
    /// with `closed`.
    #[must_use]
    pub fn key_value_store(&self) -> KeyValueStore {
        KeyValueStore {
            host: self.host.clone(),
            attempt: None,
        }
    }
}

impl KeyValueStore {
    /// The store bound to the crawl attempt `attempt`, which each of its
    /// calls then names: its writes are staged with the attempt and its
    /// reads see them first. A Product's handler context answers it, as
    /// Foxhound's `ctx.run()` does; a program never calls it.
    #[must_use]
    pub fn bound_to(self, attempt: KeyValueStoreAttempt) -> Self {
        Self {
            host: self.host,
            attempt: Some(attempt),
        }
    }

    /// Answers the JSON value a Run's Key-value store holds for a key, as
    /// committed, or null when the key is absent. A key holding bytes fails
    /// with `value_invalid`, naming `getBytes`. Takes no writer lock, so it
    /// reads a Run no program holds without resuming it. Through a crawl
    /// attempt (`attempt`), it reads what the attempt staged first.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The value stored for `key`, read as the program's `T`
    /// (`key_value_store.get`), or `None` when the key is absent, which
    /// awaiting asks; `get::<serde_json::Value>` reads any JSON.
    ///
    /// # Errors
    ///
    /// `value_invalid` when `T`'s own `Deserialize` does not read the
    /// stored value, or the key holds bytes, or is no key.
    pub fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        key: impl Into<String>,
    ) -> Call<Option<T>, KeyValueStoreKeyParams> {
        warn_operation(KEY_VALUE_STORE_GET.name);
        let key = key.into();
        self.host
            .call(KEY_VALUE_STORE_GET.name, self.key_params(key.clone()))
            .try_map(move |held: KeyValueStoreValue| read_back(&key, held.value))
    }

    /// Commits a JSON value for a key in a live Run's Key-value store,
    /// replacing the one it held: the last commit wins. A null value fails
    /// with `value_invalid`; remove a key with `delete`. Through a crawl
    /// attempt (`attempt`), the write is staged with the attempt and commits
    /// or is discarded with it; once the attempt ended, it fails with
    /// `attempt_ended`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Stores `value` for `key` in the form `T`'s own `Deserialize` reads
    /// back (`key_value_store.set`), which awaiting sends.
    ///
    /// # Errors
    ///
    /// `value_invalid` when something in `value` is not JSON, such as NaN,
    /// or `T` does not read it back as given, both before anything is sent;
    /// when `value` is null, or `key` is no key.
    pub fn set<T: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<String>,
        value: &T,
    ) -> Call<(), KeyValueStoreSetParams> {
        warn_operation(KEY_VALUE_STORE_SET.name);
        let key = key.into();
        let encoded = schema::encode_read_back(value).map_err(|error| refused(&key, &error));
        let params = KeyValueStoreSetParams {
            run_id: self.host.id().clone(),
            key,
            value: Value::Null,
            attempt: self.attempt.clone(),
        };
        self.host
            .call(KEY_VALUE_STORE_SET.name, params)
            .try_edit(|params: &mut KeyValueStoreSetParams| {
                params.value = encoded?;
                Ok(())
            })
            .map(|KeyValueStoreSet {}| ())
    }

    /// Answers the bytes and content type a Run's Key-value store holds for
    /// a key, as committed, or null when the key is absent. A key holding
    /// JSON fails with `value_invalid`, naming `get`. Takes no writer lock.
    /// Through a crawl attempt (`attempt`), it reads what the attempt staged
    /// first.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The bytes stored for `key` (`key_value_store.get_bytes`), or `None`
    /// when the key is absent, which awaiting asks.
    ///
    /// # Errors
    ///
    /// `value_invalid` when the key holds JSON, or is no key.
    pub fn get_bytes(
        &self,
        key: impl Into<String>,
    ) -> Call<Option<StoredBytes>, KeyValueStoreKeyParams> {
        warn_operation(KEY_VALUE_STORE_GET_BYTES.name);
        self.host
            .call(KEY_VALUE_STORE_GET_BYTES.name, self.key_params(key.into()))
            .map(|held: KeyValueStoreBytesValue| {
                held.bytes.map(|bytes| StoredBytes {
                    content: bytes.content.into_bytes(),
                    content_type: bytes.content_type,
                })
            })
    }

    /// Commits bytes with their content type for a key in a live Run's
    /// Key-value store, replacing what it held: the last commit wins. Bytes
    /// over 1 MiB are kept as a file in the Run's folder, which its
    /// Checkpoints carry.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Stores `content`, of `content_type`, for `key`
    /// (`key_value_store.set_bytes`), which awaiting sends. Through a store
    /// bound to a crawl attempt, the write is staged with the attempt.
    ///
    /// # Errors
    ///
    /// `value_invalid` when `key` is no key.
    pub fn set_bytes(
        &self,
        key: impl Into<String>,
        content: impl Into<Vec<u8>>,
        content_type: impl Into<String>,
    ) -> Call<(), KeyValueStoreSetBytesParams> {
        warn_operation(KEY_VALUE_STORE_SET_BYTES.name);
        let params = KeyValueStoreSetBytesParams {
            run_id: self.host.id().clone(),
            key: key.into(),
            content: KeyValueStoreContent::new(content.into()),
            content_type: content_type.into(),
            attempt: self.attempt.clone(),
        };
        self.host
            .call(KEY_VALUE_STORE_SET_BYTES.name, params)
            .map(|KeyValueStoreSet {}| ())
    }

    /// Removes a key from a live Run's Key-value store and answers whether
    /// it existed. Through a crawl attempt (`attempt`), the removal is
    /// staged with the attempt, and whether the key existed counts what the
    /// attempt staged.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Removes `key` (`key_value_store.delete`), which awaiting sends.
    ///
    /// # Errors
    ///
    /// `value_invalid` when `key` is no key.
    pub fn delete(&self, key: impl Into<String>) -> Call<bool, KeyValueStoreKeyParams> {
        warn_operation(KEY_VALUE_STORE_DELETE.name);
        self.host
            .call(KEY_VALUE_STORE_DELETE.name, self.key_params(key.into()))
            .map(|deleted: KeyValueStoreDeleted| deleted.existed)
    }

    /// The params of a call that names `key` alone.
    fn key_params(&self, key: String) -> KeyValueStoreKeyParams {
        KeyValueStoreKeyParams {
            run_id: self.host.id().clone(),
            key,
            attempt: self.attempt.clone(),
        }
    }

    /// The Connection its calls go through.
    pub(crate) fn connection(&self) -> &crate::ClientConnection {
        self.host.connection()
    }
}

/// What `get` answers of `held`, the value the engine answered for `key`:
/// null is an absent key, which the store never holds.
///
/// # Errors
///
/// `value_invalid` when `T` does not read it.
fn read_back<T: DeserializeOwned>(key: &str, held: Value) -> Result<Option<T>, Error> {
    if held.is_null() {
        return Ok(None);
    }
    schema::decode(held).map(Some).map_err(|error| {
        invalid(
            key,
            &format!("The type read does not accept the value stored for {key:?}: {error}."),
            "Read it as the type it was stored from, or as `serde_json::Value`, and store it \
             again in the new shape.",
            ValueInvalidReason::ValueSchema,
            String::new(),
        )
    })
}

/// The `value_invalid` of the value given for `key`, which `error` refused
/// before anything was sent.
fn refused(key: &str, error: &SchemaError) -> Error {
    let (reason, path) = match error {
        SchemaError::NotJson { path, .. } => (ValueInvalidReason::ValueNotJson, path.clone()),
        SchemaError::ChangesOnReadBack { path, .. } => {
            (ValueInvalidReason::ValueSchema, path.clone())
        }
        _ => (ValueInvalidReason::ValueSchema, String::new()),
    };
    invalid(
        key,
        &format!("The Key-value store refused the value of {key:?}: {error}."),
        "Give a value that is JSON and that its type's `Deserialize` reads back as it was given.",
        reason,
        path,
    )
}

/// The `value_invalid` of `key`'s value, refused for `reason` at `path`.
fn invalid(
    key: &str,
    what_failed: &str,
    fix: &str,
    reason: ValueInvalidReason,
    path: String,
) -> Error {
    Error::of_sdk(
        &VALUE_INVALID,
        what_failed,
        fix,
        &ValueInvalidDetails {
            key: key.to_owned(),
            refused: Refused::Value,
            reason,
            path: Some(path),
            fix: fix.to_owned(),
        },
    )
}

/// C-SEND-SYNC: a store moves between threads, is shared between them and
/// outlives the Run value that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<KeyValueStore>();
    send_sync_static::<StoredBytes>();
};
