//! A crawl's Run once started (ADRs 0093, 0109 and 0112): the [`CrawlRun`]
//! a crawl serves, the stream of its Crawl requests for a program that
//! answers them itself, each a [`CrawlCall`], and its Datasets' Records.

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll, ready};

use futures_util::Stream;
use goodfetch_collie_client::{
    Answer, BoxError, Call, ClientConnection, Error, EventStream, HandlerCall, Reply, Run,
};
use goodfetch_protocol::foxhound::export::ExportFormat;
use goodfetch_protocol::foxhound::frontier::Enqueued;
use goodfetch_protocol::foxhound::progress::CrawlRunProgress;
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::{
    RUN_REPORT, RunFinishParams, RunId, RunParams, RunStopParams, StartedAs,
};
use serde_json::Value;

use super::export::ExportDataset;
use super::outcomes::{ReadFailedRequests, ReadOutcomes, RetryFailed};
use super::records::{Reach, ReadRecords, ReadRejectedRecords};
use super::seeds::{AddSeed, AddSeeds};
use super::{
    CrawlContext, CrawlId, Dataset, Delivered, Forget, Seed, answer, delivered, params_unread, sent,
};
use crate::crawl_requests::CrawlRequests;
use crate::sync::Mutex;

/// A crawl's Run, which [`Crawl::serve`](super::Crawl::serve) serves: the [`Run`] it is, and
/// the connection's Crawl requests its serve reads. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct CrawlRun {
    pub(super) run: Run,
    pub(super) connection: ClientConnection,
    pub(super) requests: Arc<CrawlRequests>,
    /// The crawl that started the Run, whose Labels its requests name.
    pub(super) crawl: CrawlId,
    pub(super) declared: Arc<[String]>,
    /// Each seed's original answer from `crawl.start`, read without a round trip.
    pub(super) seeds: Arc<[Enqueued]>,
}

impl CrawlRun {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        self.run.id()
    }

    /// Lists each seed's answer, in the order the crawl's start gave them, as
    /// the Frontier answered it when the Run began: added, with its crawl
    /// request's ID and its URL as parsed, or not added, with its reason
    /// (`duplicate`, `domain_policy`, `unsupported_scheme`, `invalid_url` or a
    /// cap's name). A relative seed is `invalid_url`, since nothing gives it a
    /// base. So a crawl that does nothing says why. Read from the Run's
    /// committed state, so it works on a Run no program holds.
    ///
    /// Read from `crawl.start`'s answer with no round trip, including after a
    /// resume (ADR 0102).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn seeds(&self) -> &[Enqueued] {
        &self.seeds
    }

    /// Adds one seed to a running crawl, `run.addSeed`, admitted and answered
    /// as soon as the Run state accepts it (written with the next Run-state
    /// batch at the default durability, or committed before answering at
    /// `full`), and answers as an enqueue does: added, with its crawl request's
    /// ID and its URL as parsed, or not added, with its reason (`duplicate`,
    /// `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is
    /// `invalid_url`. An idle `crawl.keep_alive` crawl takes it and goes on.
    /// Through a handler's context, naming its `attempt`, it is staged with
    /// that attempt: once the attempt ended it fails with `attempt_ended`. A
    /// Run this engine does not run as a live crawl, or a crawl already ending
    /// its Run, fails with `run_not_found`.
    ///
    /// The seed carries the Label's typed Payload (`Seed::new` refuses a wrong
    /// Payload type or a Payload on a `Label<()>` at compile time, ADR 0082):
    ///
    /// ```compile_fail
    /// use goodfetch_foxhound::{Crawl, CrawlRun, Seed};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct Shelf { number: u32 }
    ///
    /// fn wrong_payload(run: &CrawlRun, crawl: &mut Crawl<'_>) {
    ///     let shelf = crawl.label::<Shelf>("shelf").unwrap();
    ///     let _ = run.add_seed(Seed::new("https://shop.test/shelf/1", &shelf, ()));
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use goodfetch_foxhound::{Crawl, CrawlRun, Seed};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct Shelf { number: u32 }
    ///
    /// fn payload_on_unit_label(run: &CrawlRun, crawl: &mut Crawl<'_>) {
    ///     let page = crawl.label::<()>("page").unwrap();
    ///     let _ = run.add_seed(Seed::new("https://shop.test/", &page, Shelf { number: 1 }));
    /// }
    /// ```
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn add_seed(&self, seed: Seed) -> AddSeed {
        AddSeed::new(
            self.connection.clone(),
            self.id().clone(),
            self.crawl,
            &self.declared,
            seed,
            None,
        )
    }

    /// Takes a stream of seeds for a running crawl, `run.addSeeds`, and answers
    /// each as `run.add_seed` does, in order, so a million seeds never sit in
    /// memory. The engine grants the call 256 credits with `$/credit`, one seed
    /// each; the Client sends seeds in `$/seeds {id, seeds, end?}` batches,
    /// never more than its credits, and the engine admits each batch (staged
    /// with `attempt` when named, or written with the next Run-state batch at
    /// default durability), sends its answers in one `$/enqueued {id, answers}`
    /// and grants as many credits again. Once it answered the batch marked
    /// `end`, the call answers how many seeds were added and how many were not.
    /// Cancelling it keeps the seeds already answered. A batch beyond the
    /// credits granted, or a seed that is no seed, fails the call with
    /// `invalid_params`; a Run this engine does not run as a live crawl, or a
    /// crawl already ending its Run, fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn add_seeds<S>(&self, seeds: S) -> AddSeeds
    where
        S: Stream<Item = Seed> + Send + 'static,
    {
        AddSeeds::new(
            self.connection.clone(),
            self.id().clone(),
            self.crawl,
            Arc::clone(&self.declared),
            seeds,
            None,
        )
    }

    /// Answers whether the start that opened the Run's handle began the Run,
    /// `new`, resumed it, `resumed`, or found it ended, `ended` (ADR 0102).
    /// The handle answers it from that start's answer, without a message to
    /// the engine.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn started_as(&self) -> StartedAs {
        self.run.started_as()
    }

    /// The Run, with every member a Run has, such as `close`.
    pub fn run(&self) -> &Run {
        &self.run
    }

    /// Lets a Run finish and answers its Run report: a Run its program drives
    /// ends at once, `finished` and `complete`; a crawl ends once its work
    /// items are done, and a keep-alive crawl (`crawl.keep_alive`), which waits
    /// for new work while nothing is left, ends once this call lets it and its
    /// work items are done. A Run something stopped first answers its report
    /// saying `stopped`. A Run that ended `failed` fails the call with the
    /// error that failed it, and a write at its end that failed, such as its
    /// Plan file's, with that write's error. Cancelling the call, or its
    /// `timeout` passing, which fails it with `deadline_exceeded`, ends only
    /// the wait.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// As [`Run::finish`], with the report typed as a crawl's (ADRs 0102 and
    /// 0120): its request totals, its Records per Dataset and its
    /// Completeness, each reason naming the setting that capped it and the
    /// count it refused.
    ///
    /// # Errors
    ///
    /// Once sent, as [`Run::finish`]: a Run that ended `failed` fails with
    /// the error that failed it, and the call's timeout passing fails it
    /// with `deadline_exceeded`, ending only the wait.
    pub fn finish(&self) -> Call<Answer<CrawlRunReport>, RunFinishParams> {
        self.run.finish().typed()
    }

    /// Stops a Run at its next safe point with a Checkpoint, `stopped` and
    /// `incomplete` as an abort by its program, resumable by its Run ID, and
    /// answers its Run report; an ended Run's report is answered as it is. The
    /// wait is bounded by `timeout.close`, or the call's `timeout`: past it,
    /// the call fails with `deadline_exceeded` and the Run still stops at its
    /// next safe point. A Run that ended `failed` fails the call as
    /// `run.finish` does.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// As [`Run::stop`], with the report typed as a crawl's.
    ///
    /// # Errors
    ///
    /// Once sent, as [`Run::stop`].
    pub fn stop(&self) -> Call<Answer<CrawlRunReport>, RunStopParams> {
        self.run.stop().typed()
    }

    /// Reads a Run's report: at once for a Run that ended, and at its end for a
    /// live one, which the read never ends. A Run another engine works is read
    /// through that engine, and one no engine works from its committed state,
    /// never taking its writer lock. A Run that has no live writer and has not
    /// ended fails with `run_interrupted`, whose `fix` names a start with its
    /// Run ID, and a Run ID the Run index does not list with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// As [`Run::report`], with the report typed as a crawl's.
    ///
    /// # Errors
    ///
    /// Once sent, as [`Run::report`].
    pub fn report(&self) -> Call<Answer<CrawlRunReport>, RunParams> {
        self.run.report().typed()
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
    ///
    /// Follows it as [`Run::progress`] does, each value typed as a crawl's:
    /// its request counts, and whether a keep-alive crawl waits idle.
    ///
    /// # Errors
    ///
    /// An item fails as [`Run::progress`]'s do, and with `internal` when a
    /// value does not read as a crawl's progress.
    pub fn progress(&self) -> EventStream<Answer<CrawlRunProgress>> {
        self.run.progress().typed()
    }

    /// The Run's Crawl requests, for a program that answers them itself
    /// (ADRs 0093 and 0112): the stream beneath [`Crawl::serve`](super::Crawl::serve), for a
    /// crawl that attaches no handler, such as one whose state cannot be
    /// `Send`. Each item is a [`CrawlCall`], answered with
    /// [`CrawlCall::finish`]; the stream ends once the Run ends, as the
    /// serve returns then, or once the connection has ended, and the Run's
    /// `report` then reads how it ended. A request whose params do not
    /// read is answered with that error, which is the stream's item in its
    /// place.
    ///
    /// ```no_run
    /// use goodfetch_foxhound::{ConnectOptions, Crawl, Foxhound, Seed};
    ///
    /// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
    /// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    /// let mut crawl = Crawl::new();
    /// let page = crawl.label::<()>("page")?;
    /// let run = foxhound
    ///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &page, ())])
    ///     .await?;
    /// let mut calls = run.crawl_requests();
    /// while let Some(call) = calls.next().await {
    ///     let call = call?;
    ///     let answered = if call.context().url().ends_with(".pdf") {
    ///         Err(call.context().skip("no PDF"))
    ///     } else {
    ///         Ok(())
    ///     };
    ///     call.finish(answered)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream gives back each request of the Run the
    /// connection kept aside for it, as dropping a serve does; a
    /// [`CrawlCall`] it handed out is given back when it is dropped
    /// unanswered.
    pub fn crawl_requests(&self) -> EventStream<CrawlCall> {
        EventStream::from_items(Reading {
            next: None,
            ended: Some(Mutex::new(self.ended())),
            forget: Forget {
                requests: Arc::clone(&self.requests),
                run_id: self.id().clone(),
            },
            connection: self.connection.clone(),
            run: self.run.clone(),
            crawl: self.crawl,
            declared: Arc::clone(&self.declared),
        })
    }

    /// Reads the committed Records of one Dataset of a crawl's Run in
    /// number order, from the first, or after the number `after`, which
    /// resumes a reader from the last Record it handled. Each Record goes
    /// in a `$/record` notification with its number, its value, its
    /// request's ID and URL and when it was committed, one per credit
    /// granted with `$/credit`. While the Run is live it follows the Run,
    /// sending each Record once it commits; once the Run has ended, or has
    /// no live writer, and every Record is sent, it answers how many it
    /// sent and the last one's number. With `through`, such as the number
    /// `dataset.page` answers as the last committed, it reads no Record
    /// numbered past it and answers once it sent every Record through it, a
    /// live Run's included. Records are read from the Run's committed state
    /// on a connection of their own, so a reader that grants no credit
    /// never slows the crawl. With `wall` set to `true` it reads only the
    /// Records whose page was read past a Wall, which their Provenance
    /// marks, and with `false` only the others (ADR 0052). A Dataset the
    /// crawl does not declare fails with `dataset_unknown`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Each Record is a [`DatasetRecord`] whose value is the Dataset's own
    /// type `I`, which `I`'s `Deserialize` parses from the Record's JSON
    /// (ADR 0103). A field `I` does not declare is kept as `I` keeps it: a
    /// `#[serde(flatten)]` map holds each one, and a Dataset of
    /// [`serde_json::Value`] of the same name, from any [`Crawl`](super::Crawl),
    /// reads each Record's JSON whole. A Record `I` refuses ends the stream
    /// with `record_invalid`, whose details name the Dataset and the
    /// Record's number, with one issue at the root that carries serde's
    /// message; [`ReadRecords::after`] resumes past it.
    ///
    /// ```no_run
    /// use goodfetch_foxhound::{ConnectOptions, Crawl, Foxhound, Seed};
    ///
    /// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
    /// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    /// let mut crawl = Crawl::new();
    /// let page = crawl.label::<()>("page")?;
    /// let titles = crawl.dataset::<String>("titles")?;
    /// crawl.handle(&page, |_ctx| async { Ok(()) })?;
    /// let run = foxhound
    ///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &page, ())])
    ///     .await?;
    /// crawl.serve(&run).await?;
    /// let mut records = run.records(&titles);
    /// while let Some(record) = records.next().await {
    ///     let record = record?;
    ///     println!("{}: {}", record.number, record.value);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// An item is `dataset_unknown` for a Dataset the crawl does not
    /// declare, `record_invalid` for a Record `I` refuses, `closed` once
    /// the connection has ended, and the engine's error when the read
    /// fails, after which the stream ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the read, and leaves the Run as it is.
    pub fn records<I>(&self, dataset: &Dataset<I>) -> ReadRecords<I> {
        ReadRecords::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            Reach::Follows,
        )
    }

    /// Reads one Dataset's rejection table from a crawl's committed state:
    /// each Record rejected rather than committed, numbered from 1 in
    /// commit order, with the Record as JSON, the reason, each issue's
    /// path, the Hook when one rejected it, and the request's ID and URL.
    /// It starts at the first entry, or after `after`, the number a reader
    /// last handled. While the Run is live and nothing follows, it waits
    /// for the next commit; a page says `ended` once the Run has ended, or
    /// has no live writer, and no entry follows. With `through`, such as
    /// the `rejected` count `dataset.list` answers, it reads no entry
    /// numbered past it and never waits, a live Run's included: a page says
    /// `ended` once it reaches that number or has read every committed
    /// entry. A reader never slows the Run. A
    /// Dataset the crawl does not declare fails with `dataset_unknown`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Each entry is a [`RejectedRecord`]; the stream reads the table page
    /// after page and ends with the page that says it ended (ADR 0104).
    /// [`ReadRejectedRecords::after`] sets `after`; a handler reads through
    /// [`AttemptRun::rejected_records`](crate::AttemptRun::rejected_records),
    /// which sets `through`.
    ///
    /// # Errors
    ///
    /// An item is `dataset_unknown` for a Dataset the crawl does not
    /// declare, `closed` once the connection has ended, and the engine's
    /// error when a page's read fails, after which the stream ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the page being read, and leaves the Run
    /// as it is.
    pub fn rejected_records<I>(&self, dataset: &Dataset<I>) -> ReadRejectedRecords {
        ReadRejectedRecords::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            Reach::Follows,
        )
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
    ///
    /// Returns an [`ExportDataset`] builder that resolves to an
    /// [`Export`](crate::Export) only once the engine's file is whole
    /// (ADRs 0103 and 0116).
    ///
    /// # Errors
    ///
    /// Once awaited: `dataset_unknown`, `setting_invalid`,
    /// `sheet_capacity_exceeded`, `file_write_failed`, `closed`, or the
    /// engine's error.
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
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the page being read and leaves the Run as
    /// it is.
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

    /// Re-admits, in the same Run and its Datasets, every request of a crawl
    /// whose latest outcome is `failed`, and with `blocked` also `blocked`:
    /// exactly those `run.failed_requests` lists with the same option. Each
    /// keeps its ID, Label, Payload and depth; its attempts count on, with a
    /// fresh `crawl.retry.max`. A skip is a decision and is never re-admitted.
    /// It answers how many it re-admitted, 0 when there was nothing to retry.
    /// On a crawl this engine runs, the requests join its work at once, unless
    /// the crawl is already ending its Run: the call then waits for that end
    /// and re-admits them as on an ended Run. On an ended Run with requests to
    /// retry, the Run returns to `running` under `settings`, as a resume does,
    /// its Verdict and Completeness given anew when it ends, and the calling
    /// Connection serves its handlers. A Run with no writer that has not ended
    /// fails with `run_interrupted`: resume it first. A request to retry whose
    /// Label the crawl no longer declares, which a resume may drop, fails the
    /// call with `label_unknown`, naming that Label and those declared, and
    /// nothing is re-admitted: on an ended Run, against the Labels its latest
    /// start declared, before the Run returns to `running`. It is not sent
    /// again after a lost Connection, which fails it with `connection_lost`:
    /// `run.failed_requests` then lists the requests still to retry.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn retry_failed(&self) -> RetryFailed {
        RetryFailed::new(self.connection.clone(), self.id().clone())
    }

    /// The Run's report, which the engine answers once the Run ends.
    pub(super) fn ended(&self) -> Reply<CrawlRunReport> {
        self.connection
            .call::<CrawlRunReport>(
                RUN_REPORT.name,
                &RunParams {
                    run_id: self.id().clone(),
                },
            )
            .no_timeout()
            .into_future()
    }
}

/// The next Crawl request of a Run, or `None` once the connection ended.
type NextRequest = Pin<Box<dyn Future<Output = Option<HandlerCall>> + Send + Sync>>;

/// The stream of a Run's Crawl requests, which ends with the Run.
struct Reading {
    /// The read of the Run's next request, once one was asked for: dropped
    /// before `forget`, so no request it read stays kept for the Run.
    next: Option<NextRequest>,
    /// The Run's report, whose answer ends the stream, or `None` once the
    /// stream ended. The lock only makes the stream `Sync`: the report is
    /// polled through `get_mut`, never locked.
    ended: Option<Mutex<Reply<CrawlRunReport>>>,
    forget: Forget,
    connection: ClientConnection,
    run: Run,
    crawl: CrawlId,
    declared: Arc<[String]>,
}

impl Stream for Reading {
    type Item = Result<CrawlCall, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let reading = self.get_mut();
        let Some(ended) = reading.ended.as_mut() else {
            return Poll::Ready(None);
        };
        let report = ended.get_mut().unwrap_or_else(PoisonError::into_inner);
        if report.as_mut().poll(context).is_ready() {
            reading.next = None;
            reading.ended = None;
            return Poll::Ready(None);
        }
        let forget = &reading.forget;
        let next = reading.next.get_or_insert_with(|| {
            let requests = Arc::clone(&forget.requests);
            let run_id = forget.run_id.clone();
            Box::pin(async move { requests.next(&run_id).await })
        });
        let call = ready!(next.as_mut().poll(context));
        reading.next = None;
        let Some(call) = call else {
            reading.ended = None;
            return Poll::Ready(None);
        };
        Poll::Ready(Some(reading.call_of(call)))
    }
}

impl Reading {
    /// The [`CrawlCall`] of `call`, or the refusal of params that do not
    /// read, which answers it.
    fn call_of(&self, call: HandlerCall) -> Result<CrawlCall, Error> {
        match delivered(
            &call,
            &self.connection,
            &self.run,
            self.crawl,
            &self.declared,
        ) {
            Ok(Delivered { attempt, payload }) => Ok(CrawlCall {
                call,
                context: CrawlContext::new(attempt, payload),
            }),
            Err(error) => {
                sent(answer(call, None, Err(Box::new(params_unread(&error)))));
                Err(params_unread(&error))
            }
        }
    }
}

/// One Crawl request of a Run that the program answers itself, read from
/// [`CrawlRun::crawl_requests`] (ADRs 0093 and 0109): its attempt's
/// context, whose Payload is the request's JSON, and the answer
/// [`CrawlCall::finish`] gives. Dropped unanswered, it is given back to
/// the engine: the request returns to pending without counting an
/// attempt, and the Run delivers it again.
#[derive(Debug)]
pub struct CrawlCall {
    call: HandlerCall,
    context: CrawlContext<Value>,
}

impl CrawlCall {
    /// The attempt's context, as a handler is given it, with the request's
    /// Payload as JSON.
    pub fn context(&self) -> &CrawlContext<Value> {
        &self.context
    }

    /// Answers the attempt with `result`, as a handler's return does: `Ok`
    /// commits what the attempt pushed; an `Err` fails it with
    /// `handler_failed`, a Goodfetch [`Error`] keeping its own code; a
    /// [`CrawlContext::skip`] or [`CrawlContext::retry`] called on the
    /// context ends it as it asked, whatever `result` is (ADR 0109).
    ///
    /// # Errors
    ///
    /// `closed` once the connection has ended.
    pub fn finish(self, result: Result<(), BoxError>) -> Result<(), Error> {
        answer(self.call, self.context.attempt.ending(), result)
    }
}
