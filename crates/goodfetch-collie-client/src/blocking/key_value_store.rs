//! A Run's Key-value store for a program that does not use async (ADR
//! 0081): the twin of [`crate::KeyValueStore`], whose calls end in
//! `send()`.

use goodfetch_protocol::key_value_store::{
    KeyValueStoreKeyParams, KeyValueStoreSetBytesParams, KeyValueStoreSetParams,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Call, Run};
use crate::StoredBytes;

/// A Run's Key-value store, as [`crate::KeyValueStore`], whose calls block.
/// Cloning it is cheap and every clone is the same store.
#[derive(Clone, Debug)]
pub struct KeyValueStore(crate::KeyValueStore);

/// The blocking face of `store`, whose calls block: the twin a Product's
/// blocking API hands out of a store it holds, such as a blocking crawl
/// handler's, bound to its attempt.
impl From<crate::KeyValueStore> for KeyValueStore {
    fn from(store: crate::KeyValueStore) -> Self {
        Self(store)
    }
}

impl Run {
    /// The Run's Key-value store, whose writes commit at once, as
    /// [`crate::Run::key_value_store`].
    #[must_use]
    pub fn key_value_store(&self) -> KeyValueStore {
        KeyValueStore(self.inner.key_value_store())
    }
}

impl KeyValueStore {
    /// Answers the JSON value a Run's Key-value store holds for a key, as
    /// committed, or null when the key is absent. A key holding bytes fails
    /// with `value_invalid`, naming `getBytes`. Takes no writer lock, so it
    /// reads a Run no program holds without resuming it. Through a crawl
    /// attempt (`attempt`), it reads what the attempt staged first.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The value stored for `key`, as [`crate::KeyValueStore::get`], which
    /// `send()` asks.
    pub fn get<T: DeserializeOwned + Send + 'static>(
        &self,
        key: impl Into<String>,
    ) -> Call<Option<T>, KeyValueStoreKeyParams> {
        Call::new(self.0.get(key), self.0.connection())
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
    /// Stores `value` for `key`, as [`crate::KeyValueStore::set`], which
    /// `send()` sends.
    pub fn set<T: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<String>,
        value: &T,
    ) -> Call<(), KeyValueStoreSetParams> {
        Call::new(self.0.set(key, value), self.0.connection())
    }

    /// Answers the bytes and content type a Run's Key-value store holds for
    /// a key, as committed, or null when the key is absent. A key holding
    /// JSON fails with `value_invalid`, naming `get`. Takes no writer lock.
    /// Through a crawl attempt (`attempt`), it reads what the attempt staged
    /// first.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The bytes stored for `key`, as [`crate::KeyValueStore::get_bytes`],
    /// which `send()` asks.
    pub fn get_bytes(
        &self,
        key: impl Into<String>,
    ) -> Call<Option<StoredBytes>, KeyValueStoreKeyParams> {
        Call::new(self.0.get_bytes(key), self.0.connection())
    }

    /// Commits bytes with their content type for a key in a live Run's
    /// Key-value store, replacing what it held: the last commit wins. Bytes
    /// over 1 MiB are kept as a file in the Run's folder, which its
    /// Checkpoints carry.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Stores `content`, of `content_type`, for `key`, as
    /// [`crate::KeyValueStore::set_bytes`], which `send()` sends.
    pub fn set_bytes(
        &self,
        key: impl Into<String>,
        content: impl Into<Vec<u8>>,
        content_type: impl Into<String>,
    ) -> Call<(), KeyValueStoreSetBytesParams> {
        Call::new(
            self.0.set_bytes(key, content, content_type),
            self.0.connection(),
        )
    }

    /// Removes a key from a live Run's Key-value store and answers whether
    /// it existed. Through a crawl attempt (`attempt`), the removal is
    /// staged with the attempt, and whether the key existed counts what the
    /// attempt staged.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Removes `key`, as [`crate::KeyValueStore::delete`], which `send()`
    /// sends.
    pub fn delete(&self, key: impl Into<String>) -> Call<bool, KeyValueStoreKeyParams> {
        Call::new(self.0.delete(key), self.0.connection())
    }
}
