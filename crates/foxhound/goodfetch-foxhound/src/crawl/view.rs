//! A read-only view of a crawl's Run another program started (ADRs 0102 and
//! 0103; issue 1489): opened by [`crate::Foxhound::open_crawl_run`] with no
//! round trip and no writer lock. Reads its progress, report, trace,
//! Key-value store, outcomes, failed requests, Records, rejected Records and
//! Exports, typed by the program's own [`Dataset`] handles. Closing it frees
//! only the view: the Run goes on.

use std::path::Path;

use goodfetch_collie_client::{
    Answer, Call, ClientConnection, Error, EventStream, KeyValueStoreView, RunTrace, RunView,
    SaveFile,
};
use goodfetch_protocol::foxhound::export::ExportFormat;
use goodfetch_protocol::foxhound::progress::CrawlRunProgress;
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::{RecordingSaved, RunId, RunParams};

use super::Dataset;
use super::export::ExportDataset;
use super::outcomes::{ReadFailedRequests, ReadOutcomes};
use super::records::{Reach, ReadRecords, ReadRejectedRecords};

/// A read-only view of a crawl's Run named by its Run ID, opened by
/// [`crate::Foxhound::open_crawl_run`] with no round trip (ADRs 0102 and
/// 0103). It takes no writer lock and never resumes the Run; closing it frees
/// only the view.
///
/// On a Run with no live writer, its sequences (`progress`, `outcomes`,
/// `records`, `rejected_records`) end after what is committed, and `report()`
/// fails with `run_interrupted` whose `fix` names a start with the Run's ID.
#[derive(Clone, Debug)]
#[must_use = "a view is the Run's ID; closing it frees only the view"]
pub struct CrawlRunView {
    view: RunView,
    connection: ClientConnection,
}

impl CrawlRunView {
    /// Opens a crawl view over `view` and `connection`.
    pub(crate) fn new(view: RunView, connection: ClientConnection) -> Self {
        Self { view, connection }
    }

    /// The Run's ID, which names it for good. Read without a round trip.
    pub fn id(&self) -> &RunId {
        self.view.id()
    }

    /// Reads a Run's progress: what it does and why it waits. Without `after`,
    /// it answers at once; given `after`, the position of the value the reader
    /// last read, it waits for a change and answers the latest value, so values
    /// in between are coalesced. The value whose `last` is true ends the
    /// sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
    /// or no engine works it (`writer: none`) until a start with its Run ID
    /// resumes it. A Run another engine works is read through that engine, the
    /// owner the Run index names, and one no engine works from its committed
    /// state; a read never takes the Run's writer lock. A Run ID the Run index
    /// does not list fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn progress(&self) -> EventStream<Answer<CrawlRunProgress>> {
        self.view.progress().typed()
    }

    /// Reads a Run's report: at once for a Run that ended, and at its end for a
    /// live one, which the read never ends. A Run another engine works is read
    /// through that engine, and one no engine works from its committed state,
    /// never taking its writer lock. A Run that has no live writer and has not
    /// ended fails with `run_interrupted`, whose `fix` names a start with its
    /// Run ID, and a Run ID the Run index does not list with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn report(&self) -> Call<Answer<CrawlRunReport>, RunParams> {
        self.view.report().typed()
    }

    /// Reads a Run's trace copy as recorded so far, or null when the Run keeps
    /// no trace (ADRs 0102 and 0116). Returns only once the copy is whole,
    /// with its size and SHA-256 digest; a write the engine cannot make fails
    /// with `file_write_failed`, side `engine`, and leaves no unfinished file.
    /// Takes no writer lock, so a view reads a Run no program holds without
    /// resuming it. A Run ID the Run index does not list fails with
    /// `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn trace(&self) -> Call<Option<RunTrace>, RunParams> {
        self.view.trace()
    }

    /// Read-only view of the Run's Key-value store (ADRs 0102 and 0103):
    /// `get`, `get_with`, `get_bytes` and `keys`, taking no writer lock.
    #[must_use]
    pub fn key_value_store(&self) -> KeyValueStoreView {
        self.view.key_value_store()
    }

    /// Saves a Run's recorded Response recording as a WARC 1.1 file, whole or
    /// not at all (ADRs 0105 and 0116). An Attached engine given the program's
    /// path writes it through a hidden temporary file beside it, synced and
    /// renamed over it, creating missing folders and replacing a file there;
    /// any other engine writes nothing on its own machine and sends the file
    /// in `$/chunk` notifications of at most 64 KiB, one per credit granted
    /// with `$/credit`, for the SDK to write. A Run that recorded no responses
    /// raises `recording_invalid`. A path that cannot be written raises
    /// `path_not_writable`, and full or failing storage `file_write_failed`,
    /// leaving the path as it was.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn save_recording(&self, path: impl AsRef<Path>) -> SaveFile<RecordingSaved> {
        self.view.save_recording(path)
    }

    /// Reads the committed Records of one Dataset of a crawl's Run in number
    /// order, from the first, or after the number `after`, which resumes a
    /// reader from the last Record it handled. Each Record goes in a
    /// `$/record` notification with its number, its value, its request's ID
    /// and URL and when it was committed, one per credit granted with
    /// `$/credit`. While the Run is live it follows the Run, sending each
    /// Record once it commits; once the Run has ended, or has no live writer,
    /// and every Record is sent, it answers how many it sent and the last
    /// one's number. With `through`, such as the number `dataset.page` answers
    /// as the last committed, it reads no Record numbered past it and answers
    /// once it sent every Record through it, a live Run's included. Records
    /// are read from the Run's committed state on a connection of their own,
    /// so a reader that grants no credit never slows the crawl. With `wall`
    /// set to `true` it reads only the Records whose page was read past a
    /// Wall, which their Provenance marks, and with `false` only the others
    /// (ADR 0052). A Dataset the crawl does not declare fails with
    /// `dataset_unknown`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn records<I>(&self, dataset: &Dataset<I>) -> ReadRecords<I> {
        ReadRecords::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            Reach::Follows,
        )
    }

    /// Reads the committed Records of `dataset` numbered past `after`
    /// (`run.records`, ADR 0103), shorthand for `self.records(dataset).after(after)`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn records_after<I>(&self, dataset: &Dataset<I>, after: u64) -> ReadRecords<I> {
        self.records(dataset).after(after)
    }

    /// Reads one Dataset's rejection table from a crawl's committed state:
    /// each Record rejected rather than committed, numbered from 1 in commit
    /// order, with the Record as JSON, the reason, each issue's path, the Hook
    /// when one rejected it, and the request's ID and URL. It starts at the
    /// first entry, or after `after`, the number a reader last handled. While
    /// the Run is live and nothing follows, it waits for the next commit; a
    /// page says `ended` once the Run has ended, or has no live writer, and no
    /// entry follows. With `through`, such as the `rejected` count
    /// `dataset.list` answers, it reads no entry numbered past it and never
    /// waits, a live Run's included: a page says `ended` once it reaches that
    /// number or has read every committed entry. A reader never slows the Run.
    /// A Dataset the crawl does not declare fails with `dataset_unknown`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn rejected_records<I>(&self, dataset: &Dataset<I>) -> ReadRejectedRecords {
        ReadRejectedRecords::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            Reach::Follows,
        )
    }

    /// Reads `dataset`'s rejection table past entry `after`
    /// (`run.rejected_records`, ADRs 0103 and 0104), shorthand for
    /// `self.rejected_records(dataset).after(after)`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn rejected_records_after<I>(
        &self,
        dataset: &Dataset<I>,
        after: u64,
    ) -> ReadRejectedRecords {
        self.rejected_records(dataset).after(after)
    }

    /// Writes an Export of one Dataset of a crawl's Run: the Records it
    /// committed when the call is made, in number order, in the format named,
    /// as its `format` value says: a table (`csv`, `xlsx`, `html_table`,
    /// `markdown`, and typed, `parquet`, `avro` and `arrow_ipc`) has one
    /// column per field seen in any of its Records, in first-seen order, and
    /// one row per Record. Every text Export is UTF-8 with no BOM unless
    /// `export.csv.bom` puts one on CSV, characters written as themselves (a
    /// character XML 1.0 cannot hold as U+FFFD in XML and feeds) and each
    /// value as it was committed: a table cell holds a string as its text, a
    /// number, `true` or `false` as JSON writes it, nothing for `null` or a
    /// field its Record lacks, and an object or array as its JSON text, or, in
    /// CSV under `export.csv.nested: flatten`, an object's fields in dotted
    /// columns. A Record whose page was read past a Wall carries its Wall
    /// marker, `{kind, vendor?}` as its Provenance holds it, as a `#wall`
    /// field in every format but the feeds, where it is a category of the
    /// scheme `urn:goodfetch:wall` naming its kind; `wall` set to `true`
    /// writes only those Records, `false` only the others. `compression`
    /// `gzip` or `zstd` compresses the whole file. Its `settings` and
    /// `project` are the run layer and the project configuration its
    /// `export.*` settings resolve under; a value the format cannot take fails
    /// with `setting_invalid`. Answers the Export, with its Record count, its
    /// last Record's number, its size, its SHA-256 and the run epoch of the
    /// Run's state it was written from, only once its file is whole in the
    /// Run's folder, where `export.save_as` and `export.stream` read it.
    /// Records committed after the call starts are not in it. A write the
    /// engine's storage refuses fails with `file_write_failed`, side `engine`,
    /// leaving no unfinished file, as does a call while that storage is at the
    /// `critical` level, before any byte is written. A Dataset the crawl does
    /// not declare fails with `dataset_unknown`; an `xlsx` sheet past Excel's
    /// limits, with `sheet_capacity_exceeded`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn export<I>(&self, dataset: &Dataset<I>, format: ExportFormat) -> ExportDataset {
        ExportDataset::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            format,
        )
    }

    /// A log of endings, not current state: a re-admitted request's new
    /// outcome supersedes the old by `request.id` (ADRs 0102 and 0117).
    ///
    /// Reads a crawl's outcomes from its committed state, in commit order:
    /// from the first, after a position, or with `after: "now"` from the next
    /// commit. Its filters, `kinds`, `labels` and `codes` (an Error code any
    /// of the request's failed attempts failed with), run in the engine.
    /// While the Run is live and nothing new matches, it waits for the next
    /// commit; a page says `ended` once the Run has ended, or has no live
    /// writer, and nothing more matches. A reader never slows the Run. A
    /// request `run.retry_failed` re-admits ends again, and its later outcome
    /// supersedes the earlier one by `request.id`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn outcomes(&self) -> ReadOutcomes {
        ReadOutcomes::new(self.connection.clone(), self.id().clone())
    }

    /// Reads from a crawl's committed state the requests whose latest outcome
    /// is `failed`, and with `blocked` also `blocked`, each with that outcome:
    /// exactly those `run.retry_failed` re-admits with the same option. A skip
    /// is a decision and is never listed.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn failed_requests(&self) -> ReadFailedRequests {
        ReadFailedRequests::new(self.connection.clone(), self.id().clone())
    }

    /// Closes the view. Frees only the view: the Run goes on.
    pub fn close(self) {
        self.view.close();
    }
}

impl crate::Foxhound {
    /// Opens a read-only view of the crawl Run `run_id` names, one another
    /// program started, with no round trip (ADRs 0102 and 0103). The view
    /// takes no writer lock and never resumes the Run; closing it frees only
    /// the view.
    ///
    /// # Errors
    ///
    /// `closed` once the connection is closed, and `connection_forked` in
    /// another process than the one that opened it.
    pub fn open_crawl_run(&self, run_id: &RunId) -> Result<CrawlRunView, Error> {
        let connection: &ClientConnection = self.as_ref();
        let view = connection.open_run(run_id)?;
        Ok(CrawlRunView::new(view, connection.clone()))
    }
}

/// C-SEND-SYNC: a crawl view moves between threads, is shared between them
/// and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<CrawlRunView>();
};
