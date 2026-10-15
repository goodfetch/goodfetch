//! Crawl output in code (ADR 0103, decided in #479): Datasets, Records, Exports and Sinks, which are
//! Foxhound's (ADR 0088). Every Run's Key-value store, Collie's included, is the Collie client crate's.

use crate::Call;
use crate::CrawlContext;
use crate::CrawlRun;
use crate::Dataset;
use serde::de::DeserializeOwned;
use crate::Error;
use crate::EventStream;
use crate::KeyValueStore;
use std::path::Path;
use crate::SchemaIssue;
use serde::Serialize;
use std::time::SystemTime;

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
    /// Its number in the Dataset's rejection table: 1 for the first, in commit order, with no gaps.
    pub number: u64,
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

    /// Copies it to a path on the program's machine through a hidden temporary file renamed over
    /// the path once whole, so the path holds the whole Export or what it held before (ADR 0108).
    /// It creates missing parent folders and replaces an existing file. A path it cannot write fails
    /// with `path_not_writable`; full or failing storage with `file_write_failed` and
    /// [`FileWriteSide::Program`](goodfetch_collie_client::FileWriteSide::Program), after which it
    /// can be called again, since the engine still holds the Export.
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

impl CrawlRun {
    /// The Dataset's committed Records from its first, following the Run live until it ends and
    /// every Record is read (ADR 0103). A cursor over run state: reading slowly, or not at all,
    /// never slows the crawl. A Record `T`'s `Deserialize` rejects arrives as `record_invalid`
    /// carrying its number. Through a handler's `ctx.run()`, it reads what has committed and ends,
    /// since the Run cannot end while the handler holds its request.
    pub fn records<T: DeserializeOwned>(&self, dataset: &Dataset<T>) -> EventStream<DatasetRecord<T>> {
        let _ = dataset;
        unimplemented!()
    }

    /// The same, after the Record numbered `after`, to resume a reader.
    pub fn records_after<T: DeserializeOwned>(&self, dataset: &Dataset<T>, after: u64) -> EventStream<DatasetRecord<T>> {
        let _ = (dataset, after);
        unimplemented!()
    }

    /// The Dataset's rejection table, each entry numbered in commit order, following the Run as
    /// `records` does.
    pub fn rejected_records<T>(&self, dataset: &Dataset<T>) -> EventStream<RejectedRecord> {
        let _ = dataset;
        unimplemented!()
    }

    /// The same, after the entry numbered `after`.
    pub fn rejected_records_after<T>(&self, dataset: &Dataset<T>, after: u64) -> EventStream<RejectedRecord> {
        let _ = (dataset, after);
        unimplemented!()
    }

    /// Writes an Export of the Records committed so far (ADR 0026), to save or stream wherever the
    /// engine runs. Resolves only once its file is whole; a write the engine cannot make fails with
    /// `file_write_failed` on the engine's side and leaves no unfinished file (ADR 0108).
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
    /// Dataset, or is discarded if the attempt fails or ends early (`skip`, `retry`, an escalation,
    /// the handler's timeout, a lost lease, the Run's close), after which a push fails with
    /// `attempt_ended`, as `enqueue` does. Resolves once staged. A Record failing the Dataset's schema, not JSON, or lacking its key's
    /// fields fails with `record_invalid` and is not staged; a Dataset of another crawl fails with
    /// `dataset_unknown`.
    pub fn push<T: Serialize>(&self, dataset: &Dataset<T>, record: &T) -> Call<()> {
        let _ = (dataset, record);
        unimplemented!()
    }
}

/// A Sink's write mode (ADR 0026). The keyed ones use the Sink's own key, or else each Dataset's
/// key. The Run's start refuses with `setting_invalid` naming `sinks.<name>.key` a Sink in a keyed
/// mode with no key of its own while one of its Datasets has none, and a Sink key naming a field
/// that a schema of one of its Datasets does not require.
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
    /// A Sink of this type receiving these Datasets, by name. Set on the Run, one the crawl does
    /// not declare is refused at start with `setting_invalid` naming the closest; inherited from the
    /// connection or configuration, the Sink delivers only the crawl's declared Datasets, and is
    /// inactive when there are none.
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

/// `dataset_duplicate`: a second Dataset of one name in one crawl.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatasetDuplicate {
    /// The Dataset.
    pub dataset: String,
}

/// `dataset_unknown`: a push to a Dataset another crawl declared, or a resume whose crawl no longer
/// declares a Dataset that holds Records; nothing ran.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatasetUnknown {
    /// The Dataset.
    pub dataset: String,
    /// The Records it holds, when a resume found it no longer declared; 0 on a push.
    pub records: u64,
}

/// `dataset_key_invalid`: a key naming a field the schema does not require, or a resume whose
/// Dataset's key changed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatasetKeyInvalid {
    /// The Dataset.
    pub dataset: String,
    /// The fields at fault.
    pub fields: Vec<String>,
}

