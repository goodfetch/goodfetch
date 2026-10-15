//! Crawl output in code (ADR 0103, decided in #479).
//!
//! A crawl declares its Datasets as it declares its Labels, each a typed handle (ADR 0082), with no
//! default Dataset. A handler pushes Records through its context, which stages them with its
//! attempt: they commit with it, numbered in commit order, or are discarded when it ends early
//! (ADR 0003). A program reads committed Records back from the crawl's Run as an [`EventStream`]
//! that starts at the Dataset's first Record, follows the Run live, and never slows the crawl, since
//! Records are already in run state. Sinks are settings ([`Settings::sink`]); an [`Export`] is a
//! file the engine writes, which the program saves or streams wherever the engine runs. Every Run,
//! Collie's included, has a [`KeyValueStore`]; Datasets, Records, Exports and Sinks are Foxhound's
//! (ADR 0084).

use std::path::Path;
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::concurrency::EventStream;
use crate::crawl::{CrawlContext, CrawlRun};
use crate::data::{Dataset, Schema, SchemaIssue};
use crate::lifecycle::Run;
use crate::results::Error;
use crate::settings::Settings;
use crate::time::Call;

/// One committed Record, as a program reads it back.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatasetRecord<T> {
    /// Its number in its Dataset: 1 for the first, in commit order, with no gaps.
    pub number: u64,
    /// Built by the type's own `Deserialize` (ADR 0082).
    pub value: T,
    /// The crawl request whose attempt committed it.
    pub request_id: String,
    /// The request's URL.
    pub url: String,
    /// When its attempt committed.
    pub committed_at: SystemTime,
}

/// A Record a "Record before commit" Hook rejected (ADR 0033), or that a Hook's transform left
/// outside the schema.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RejectedRecord {
    /// As JSON, never parsed by the schema it failed (G3).
    pub record: serde_json::Value,
    /// Why.
    pub reason: String,
    /// The Hook that rejected it, or `None` when the schema did, after a Hook's transform.
    pub hook: Option<String>,
    /// The crawl request whose attempt pushed it.
    pub request_id: String,
    /// The request's URL.
    pub url: String,
}

/// What a request's commit did to its Records in one Dataset, counted on its `Succeeded` outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecordCounts {
    /// Committed and numbered.
    pub committed: u32,
    /// Dropped with a reason by a "Record before commit" Hook: counted, never stored.
    pub dropped: u32,
    /// In the Dataset's rejection table, read with [`CrawlRun::rejected_records`].
    pub rejected: u32,
}

/// The Export formats of ADR 0026, named, never inferred from a file name. Each format's own
/// options are `export.*` settings (area 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExportFormat {
    /// `json`.
    Json,
    /// `jsonl`.
    Jsonl,
    /// `csv`.
    Csv,
    /// `xlsx`.
    Xlsx,
    /// `xml`.
    Xml,
    /// `parquet`.
    Parquet,
    /// `html_table`.
    HtmlTable,
    /// `rss`.
    Rss,
    /// `atom`.
    Atom,
    /// `avro`.
    Avro,
    /// `arrow_ipc`.
    ArrowIpc,
    /// `markdown`.
    Markdown,
}

/// An Export's compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// gzip.
    Gzip,
    /// zstd.
    Zstd,
}

/// A file the engine wrote from a Dataset's Records committed when the call was made, wherever the
/// engine runs (ADR 0001). Playwright's `Download` names.
#[derive(Debug)]
pub struct Export {
    _private: (),
}

impl Export {
    /// Its format.
    pub fn format(&self) -> ExportFormat {
        unimplemented!()
    }

    /// How many Records it holds.
    pub fn records(&self) -> u64 {
        unimplemented!()
    }

    /// The number of the last Record it holds.
    pub fn last_number(&self) -> u64 {
        unimplemented!()
    }

    /// Its size in bytes.
    pub fn size(&self) -> u64 {
        unimplemented!()
    }

    /// Its SHA-256, in hex.
    pub fn sha256(&self) -> &str {
        unimplemented!()
    }

    /// Copies it to a path on the program's machine.
    pub fn save_as(&self, path: impl AsRef<Path>) -> Call<()> {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// Its bytes, read once.
    pub fn stream(&self) -> EventStream<Vec<u8>> {
        unimplemented!()
    }
}

/// Writing an Export, not sent until awaited.
#[derive(Debug)]
#[must_use = "nothing is written until this is awaited"]
pub struct ExportCall {
    _private: (),
}

impl ExportCall {
    /// Compresses it. Built-in: none.
    pub fn compression(self, compression: Compression) -> Self {
        let _ = compression;
        unimplemented!()
    }
}

impl IntoFuture for ExportCall {
    type Output = Result<Export, Error>;
    type IntoFuture = crate::time::CallFuture<Export>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

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
/// the attempt's own writes first. A cheap clone (ADR 0081).
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

    /// Writes a JSON value.
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

impl CrawlRun {
    /// The Dataset's committed Records from its first, following the Run live until it ends and
    /// every Record is read (ADR 0103). A cursor over run state: reading slowly, or not at all,
    /// never slows the crawl. A Record `T`'s `Deserialize` rejects arrives as `record_invalid`
    /// carrying its number.
    pub fn records<T: DeserializeOwned>(&self, dataset: &Dataset<T>) -> EventStream<DatasetRecord<T>> {
        let _ = dataset;
        unimplemented!()
    }

    /// The same, after the Record numbered `after`, to resume a reader.
    pub fn records_after<T: DeserializeOwned>(&self, dataset: &Dataset<T>, after: u64) -> EventStream<DatasetRecord<T>> {
        let _ = (dataset, after);
        unimplemented!()
    }

    /// The Dataset's rejection table, in the same order.
    pub fn rejected_records<T>(&self, dataset: &Dataset<T>) -> EventStream<RejectedRecord> {
        let _ = dataset;
        unimplemented!()
    }

    /// Writes an Export of the Records committed so far (ADR 0026), to save or stream wherever the
    /// engine runs.
    pub fn export<T>(&self, dataset: &Dataset<T>, format: ExportFormat) -> ExportCall {
        let _ = (dataset, format);
        unimplemented!()
    }

    /// The Run's Key-value store: writes commit at once, or, through a handler's `ctx.run()`, are
    /// staged with its attempt.
    pub fn key_value_store(&self) -> KeyValueStore {
        unimplemented!()
    }
}

impl<P> CrawlContext<P> {
    /// Stages a Record with this attempt (ADR 0103): it commits with the attempt, numbered in its
    /// Dataset, or is discarded if the attempt fails, retries, escalates, skips or loses its lease.
    /// Resolves once staged. A Record failing the Dataset's schema, not JSON, or lacking its key's
    /// fields fails with `record_invalid` and is not staged; a Dataset of another crawl fails with
    /// `dataset_unknown`.
    pub fn push<T: Serialize>(&self, dataset: &Dataset<T>, record: &T) -> Call<()> {
        let _ = (dataset, record);
        unimplemented!()
    }
}

/// A Sink's write mode (ADR 0026). The keyed ones need a key: the Sink's own or its Datasets'.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SinkWriteMode {
    /// `append`, the built-in.
    #[default]
    Append,
    /// `upsert`.
    Upsert,
    /// `insert_only`.
    InsertOnly,
    /// `replace`.
    Replace,
    /// `history`.
    History,
}

/// `sinks.<name>.*`: one Sink (ADRs 0026, 0103), set with [`Settings::sink`] on a connection or a
/// crawl's Run, never on a Label. A sample: each Sink type's own keys (its destination as a Secret
/// reference, its table or bucket) are area 6's spec.
#[derive(Debug, Clone)]
pub struct SinkConfig {
    _private: (),
}

impl SinkConfig {
    /// A Sink of this type receiving these Datasets, by name. One the crawl does not declare is
    /// refused when the Run starts, with `setting_invalid` naming the closest.
    pub fn new(sink_type: &str, datasets: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let _ = (sink_type, datasets.into_iter().map(Into::into).collect::<Vec<String>>());
        unimplemented!()
    }

    /// Its write mode. Built-in: append.
    pub fn mode(self, mode: SinkWriteMode) -> Self {
        let _ = mode;
        unimplemented!()
    }

    /// Its key. Built-in: each Dataset's own.
    pub fn key(self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let _ = fields.into_iter().map(Into::into).collect::<Vec<String>>();
        unimplemented!()
    }
}

impl Settings {
    /// Configures the Sink `name` (`sinks.<name>.*`) for this scope. Foxhound's: a Collie Run has
    /// no Datasets and refuses it with `setting_not_applicable`.
    pub fn sink(self, name: &str, sink: SinkConfig) -> Self {
        let _ = (name, sink);
        unimplemented!()
    }
}

// The Error codes of crawl output, all in the `program` group and none retryable (ADR 0103). Each
// is matched by code on `Error`; these are their typed details.

/// `record_invalid`: a pushed Record failed its Dataset's schema, natively or in the engine, could
/// not be serialized as JSON, or lacked its key's fields, and was not staged; or a reader's
/// `Deserialize` rejected a committed one.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RecordInvalid {
    /// The Dataset.
    pub dataset: String,
    /// What was wrong, by path.
    pub issues: Vec<SchemaIssue>,
    /// The Record's number when reading one back failed; `None` on a push.
    pub number: Option<u64>,
}

/// `dataset_key_invalid`: a key naming a field the schema does not require.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatasetKeyInvalid {
    /// The Dataset.
    pub dataset: String,
    /// The fields at fault.
    pub fields: Vec<String>,
}

/// `value_invalid`: a Key-value store value that is not JSON, fails its type, or holds bytes
/// (`fix` names `get_bytes`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ValueInvalid {
    /// The key.
    pub key: String,
    /// `not_json`, `schema` or `bytes`, as received.
    pub problem: String,
    /// What was wrong, by path.
    pub issues: Vec<SchemaIssue>,
}
