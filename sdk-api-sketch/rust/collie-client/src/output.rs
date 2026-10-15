//! Crawl output in code (ADR 0103, decided in #479).
//!
//! A crawl declares its Datasets as it declares its Labels, each a typed handle (ADR 0082), with no
//! default Dataset. A handler pushes Records through its context, which stages them with its
//! attempt: they commit with it, numbered in commit order, or are discarded when it ends early
//! (ADR 0003). A program reads committed Records back from the crawl's Run as an [`EventStream`]
//! that starts at the Dataset's first Record, follows the Run live, and never slows the crawl, since
//! Records are already in run state. Sinks are settings ([`Settings::sink`]); an [`Export`] is a
//! file the engine writes, which the program saves or streams wherever the engine runs. Datasets
//! and Sinks are Foxhound's (ADR 0084), and so are the Records and Exports made from them; every
//! Run, Collie's included, has a [`KeyValueStore`] (ADR 0088).


use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::concurrency::EventStream;
use crate::data::{Schema, SchemaIssue};
use crate::lifecycle::Run;
use crate::time::Call;

/// Bytes kept under a key, with their content type.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StoredBytes {
    /// The bytes.
    pub content: Vec<u8>,
    /// Their content type.
    pub content_type: String,
}

/// A Run's Key-value store: files, blobs and the program's own state, in the run file,
/// checkpointed with it (ADR 0002). Reached from the program's Run, it writes at once. Reached
/// through a handler's `ctx.run()`, which is bound to its attempt (ADR 0093), it stages its writes
/// with the attempt, which commit or are discarded with its Records and enqueues, and its reads see
/// the attempt's own writes first. For each key the last commit wins; another attempt's staged
/// writes stay invisible until it commits, and there is no compare-and-set: a value concurrent
/// handlers share live stays in the program's own memory, within one process; across worker
/// processes a program counts from committed Records or outcomes. A cheap clone (ADR 0081).
#[derive(Debug, Clone)]
pub struct KeyValueStore {
    _private: (),
}

impl KeyValueStore {
    /// The value under the key, built by `T`'s `Deserialize`, or `None` when there is none: an
    /// answer, never an error. `serde_json::Value` reads any JSON; a rejection fails with
    /// `value_invalid`.
    pub fn get<T: DeserializeOwned + JsonSchema>(&self, key: &str) -> Call<Option<T>> {
        let _ = key;
        unimplemented!()
    }

    /// The value under the key, checked against a hand-written JSON Schema: the way down.
    pub fn get_with(&self, key: &str, schema: Schema<serde_json::Value>) -> Call<Option<serde_json::Value>> {
        let _ = (key, schema);
        unimplemented!()
    }

    /// Writes a JSON value. Null fails with `value_invalid`, whose `fix` names `delete`, so `None`
    /// from `get` always means absent.
    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> Call<()> {
        let _ = (key, value);
        unimplemented!()
    }

    /// The bytes under the key, or `None`.
    pub fn get_bytes(&self, key: &str) -> Call<Option<StoredBytes>> {
        let _ = key;
        unimplemented!()
    }

    /// Writes bytes with their content type.
    pub fn set_bytes(&self, key: &str, content: Vec<u8>, content_type: &str) -> Call<()> {
        let _ = (key, content, content_type);
        unimplemented!()
    }

    /// Whether the key existed.
    pub fn delete(&self, key: &str) -> Call<bool> {
        let _ = key;
        unimplemented!()
    }

    /// The keys starting with `prefix`; `""` for all.
    pub fn keys(&self, prefix: &str) -> EventStream<String> {
        let _ = prefix;
        unimplemented!()
    }
}

impl Run {
    /// The Run's Key-value store, Collie's included: writes commit at once.
    pub fn key_value_store(&self) -> KeyValueStore {
        unimplemented!()
    }
}

// The Error codes of crawl output, all in the `program` group and none retryable (ADR 0103). Each
// is matched by code on `Error`; these are their typed details.

/// `value_invalid`: a Key-value store value that is not JSON or is null (`fix` names `delete`),
/// fails its type, or holds bytes (`fix` names `get_bytes`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ValueInvalid {
    /// The key.
    pub key: String,
    /// `not_json`, `null`, `schema` or `bytes`, as received.
    pub problem: String,
    /// What was wrong, by path.
    pub issues: Vec<SchemaIssue>,
}
