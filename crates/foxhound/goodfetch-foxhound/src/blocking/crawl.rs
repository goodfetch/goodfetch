//! A crawl for a program that does not use async (ADRs 0081 and 0093):
//! [`Crawl`] declares as [`crate::Crawl`] does, its handlers plain
//! functions, `Fn + Send + Sync + 'static`, each attempt's run on one of
//! the connection's tracked blocking threads, never on its runtime's
//! workers, as many at once as the engine sends; [`Crawl::serve`] blocks
//! the calling thread until the Run ends and returns its report.
//!
//! Size: one concern, the blocking crawl facade (`Crawl`, `CrawlRun`,
//! `StartCrawl`, `CrawlContext`, `AttemptRun`) mirroring the async crawl
//! surface on the connection's blocking runtime.

use std::fmt;
use std::future::IntoFuture as _;
use std::sync::Arc;

use goodfetch_collie_client::{Answer, BoxError, Error, Limits};
use goodfetch_protocol::foxhound::export::ExportFormat;
use goodfetch_protocol::foxhound::frontier::{Enqueued, EnqueuedLinks, HttpMethod};
use goodfetch_protocol::foxhound::progress::CrawlRunProgress;
use goodfetch_protocol::foxhound::rejections::RejectedRecord;
use goodfetch_protocol::foxhound::report::{CrawlRunReport, RUN_REPORT_SO_FAR, RunReportSoFar};
use goodfetch_protocol::run::{RunFinishParams, RunId, RunParams, RunStopParams, Tags};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Call, EventIter, Foxhound, Run, blocking};
use crate::settings::Settings;
use crate::{
    Dataset, DatasetDeclaration, DatasetRecord, Label, LabelDeclaration, LinkPayload, Seed,
};

/// A crawl the program declares, as [`crate::Crawl`], whose handlers are
/// plain functions and whose [`Crawl::serve`] blocks. It holds no
/// connection and does no I/O, so it may sit at module load and serve
/// every Run started from it.
///
/// ```no_run
/// use goodfetch_foxhound::blocking::{Crawl, CrawlContext, Foxhound};
/// use goodfetch_foxhound::{ConnectOptions, Seed};
///
/// # fn crawl() -> Result<(), goodfetch_foxhound::Error> {
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct Book {
///     shelf: u32,
/// }
///
/// let foxhound = Foxhound::connect(ConnectOptions::default())?;
/// let mut crawl = Crawl::new();
/// let book = crawl.label::<Book>("book")?;
/// let titles = crawl.dataset::<String>("titles")?;
/// crawl.handle(&book, move |ctx: CrawlContext<Book>| {
///     ctx.push(&titles, &ctx.url().to_owned())?;
///     Ok(())
/// })?;
/// let run = foxhound
///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &book, Book { shelf: 1 })])
///     .send()?;
/// let report = crawl.serve(&run)?;
/// foxhound.close()?;
/// # Ok(())
/// # }
/// ```
pub struct Crawl {
    inner: crate::Crawl<'static>,
}

impl Default for Crawl {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Crawl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(formatter)
    }
}

impl Crawl {
    /// A crawl that declares nothing yet.
    pub fn new() -> Self {
        Self {
            inner: crate::Crawl::new(),
        }
    }

    /// Declares a Label, by a bare name or a [`LabelDeclaration`], as
    /// [`crate::Crawl::label`] does.
    ///
    /// # Errors
    ///
    /// As [`crate::Crawl::label`].
    pub fn label<P: Serialize + DeserializeOwned>(
        &mut self,
        declaration: impl Into<LabelDeclaration<'static, P>>,
    ) -> Result<Label<P>, Error> {
        self.inner.label(declaration)
    }

    /// Declares a Dataset, by a bare name or a [`DatasetDeclaration`], as
    /// [`crate::Crawl::dataset`] does.
    ///
    /// # Errors
    ///
    /// As [`crate::Crawl::dataset`].
    pub fn dataset<I>(
        &mut self,
        declaration: impl Into<DatasetDeclaration<I>>,
    ) -> Result<Dataset<I>, Error> {
        self.inner.dataset(declaration)
    }

    /// Attaches `handler` to `label`, as [`crate::Crawl::handle`] does:
    /// the serve calls it once for each attempt of a request of the Label,
    /// on one of the connection's tracked blocking threads, and its
    /// return, or its panic, ends the attempt as an async handler's does.
    ///
    /// # Errors
    ///
    /// As [`crate::Crawl::handle`].
    pub fn handle<P, F>(&mut self, label: &Label<P>, handler: F) -> Result<(), Error>
    where
        P: DeserializeOwned + Send + 'static,
        F: Fn(CrawlContext<P>) -> Result<(), BoxError> + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        self.inner
            .handle(label, move |inner: crate::CrawlContext<P>| async move {
                let connection = inner.connection().clone();
                match connection
                    .run_blocking(move || Ok(handler(CrawlContext { inner })))
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => Err(Box::new(error) as BoxError),
                }
            })
    }

    /// Serves `run`, a Run this crawl started, as [`crate::Crawl::serve`]
    /// does, and blocks the calling thread, and only it, until the Run
    /// ends; returns the Run's report.
    ///
    /// # Errors
    ///
    /// As [`crate::Crawl::serve`], and `closed` once the connection's
    /// runtime has shut down.
    pub fn serve(&self, run: &CrawlRun) -> Result<CrawlRunReport, Error> {
        let crawl = self.inner.shared();
        let served = run.inner.clone();
        run.connection
            .wait(async move { crawl.serve(&served).await })
    }
}

/// A crawl's Run bound to one attempt of its handler, as
/// [`crate::AttemptRun`].
#[derive(Clone, Debug)]
pub struct AttemptRun {
    inner: crate::AttemptRun,
    connection: blocking::ClientConnection,
}

impl AttemptRun {
    /// The Run's ID, read without a round trip.
    pub fn id(&self) -> &RunId {
        self.inner.id()
    }

    /// The Run's Key-value store, its writes staged with the attempt, as
    /// [`crate::AttemptRun::key_value_store`].
    #[must_use]
    pub fn key_value_store(&self) -> blocking::KeyValueStore {
        blocking::KeyValueStore::from(self.inner.key_value_store())
    }

    /// The Records of `dataset` committed when the read is first iterated,
    /// as [`crate::AttemptRun::records`], as an iterator that blocks the
    /// calling thread until the next one, and ends once it read them.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// An item fails as [`crate::CrawlRun::records`]'s does, after which
    /// the iterator ends.
    pub fn records<I>(&self, dataset: &Dataset<I>) -> ReadRecords<I> {
        ReadRecords::new(self.inner.records(dataset), self.connection.clone())
    }

    /// The entries of `dataset`'s rejection table committed when the read
    /// is first iterated, as [`crate::AttemptRun::rejected_records`], as an
    /// iterator that blocks the calling thread until the next one, and
    /// ends once it read them.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// An item fails as [`crate::CrawlRun::rejected_records`]'s does,
    /// after which the iterator ends.
    pub fn rejected_records<I>(&self, dataset: &Dataset<I>) -> ReadRejectedRecords {
        ReadRejectedRecords::new(
            self.inner.rejected_records(dataset),
            self.connection.clone(),
        )
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
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn add_seed(&self, seed: Seed) -> super::AddSeed {
        super::AddSeed::new(self.inner.add_seed(seed), self.connection.clone())
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
    pub fn add_seeds<I>(&self, seeds: I) -> super::AddSeeds
    where
        I: IntoIterator<Item = Seed>,
        I::IntoIter: Send + 'static,
    {
        super::AddSeeds::new(
            self.inner.add_seeds(futures_util::stream::iter(seeds)),
            self.connection.clone(),
        )
    }
}

/// What a blocking handler is given for one attempt of a request of its
/// Label, as [`crate::CrawlContext`], whose calls block the handler's
/// thread until they are answered.
#[derive(Debug)]
pub struct CrawlContext<P> {
    inner: crate::CrawlContext<P>,
}

impl<P> CrawlContext<P> {
    /// The request's Payload.
    pub fn payload(&self) -> &P {
        self.inner.payload()
    }

    /// The request's URL.
    pub fn url(&self) -> &str {
        self.inner.url()
    }

    /// The crawl request's ID within its Run.
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    /// The request's Label.
    pub fn label(&self) -> &str {
        self.inner.label()
    }

    /// The extra request headers carried from the request's seed or enqueue.
    pub fn headers(&self) -> &std::collections::BTreeMap<String, String> {
        self.inner.headers()
    }

    /// The attempt's number, 1 for the first.
    pub fn attempt(&self) -> u32 {
        self.inner.attempt()
    }

    /// The Run's ID.
    pub fn run_id(&self) -> &RunId {
        self.inner.run_id()
    }

    /// The crawl's Run, bound to this attempt, as
    /// [`crate::CrawlContext::run`].
    #[must_use]
    pub fn run(&self) -> AttemptRun {
        AttemptRun {
            inner: self.inner.run().clone(),
            connection: self.blocking_connection(),
        }
    }

    /// Already navigated. The handler runs once the request's navigation
    /// has happened, so a route set on this page applies from its next
    /// navigation.
    ///
    /// The attempt's page, as [`crate::CrawlContext::page`], whose calls
    /// block.
    ///
    /// # Errors
    ///
    /// As [`crate::CrawlContext::page`].
    pub fn page(&self) -> Result<blocking::Page, Error> {
        self.inner.page().cloned().map(blocking::Page::from)
    }

    /// Skips the request, as [`crate::CrawlContext::skip`] does: the
    /// handler returns what this returns, `return Err(ctx.skip(…))`.
    #[must_use = "a skip is returned from the handler: `return Err(ctx.skip(…))`"]
    pub fn skip(&self, message: &str) -> BoxError {
        self.inner.skip(message)
    }

    /// Ends the attempt with `retry_requested`, as
    /// [`crate::CrawlContext::retry`] does: the handler returns what this
    /// returns, `return Err(ctx.retry(…))`.
    #[must_use = "a retry is returned from the handler: `return Err(ctx.retry(…))`"]
    pub fn retry(&self, message: &str) -> BoxError {
        self.inner.retry(message)
    }

    /// Pushes one Record or several, in one message, into a Dataset the
    /// crawl declares from inside a handler's attempt, a handler context's
    /// `push`, and answers once they are staged: they commit once with the
    /// attempt's other outputs when the handler returns, and are discarded
    /// with them when the attempt ends any other way. Each Record is
    /// checked first. A push of no Record fails with `invalid_params`. A
    /// Dataset the crawl does not declare fails with `dataset_unknown`, and
    /// none of the Records is staged. A Record that is no JSON object,
    /// fails its Dataset's JSON Schema or lacks one of its key's fields is
    /// not staged: its rejection is staged instead, entering the Dataset's
    /// rejection table (`run.rejected_records`) if the attempt commits,
    /// while the Records that hold are staged and the attempt keeps what it
    /// staged before; the call then fails with `record_invalid`, naming
    /// each such Record by its position in the list and each of its issues
    /// by its path. An attempt that already ended fails with
    /// `attempt_ended`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// This pushes `record` into `dataset`, as
    /// [`crate::CrawlContext::push`] does, and blocks until the engine has
    /// staged it.
    ///
    /// # Errors
    ///
    /// As [`crate::CrawlContext::push`].
    pub fn push<I: Serialize>(&self, dataset: &Dataset<I>, record: &I) -> Result<(), Error> {
        self.blocking_connection()
            .wait(self.inner.push(dataset, record))
    }

    /// Adds one request to a crawl from inside a handler's attempt, a
    /// handler context's `enqueue`, and answers what the Frontier did with
    /// it: added, with its crawl request's ID and its URL as parsed, or not
    /// added, with its reason (`duplicate`, `out_of_scope`,
    /// `unsupported_scheme`, `invalid_url`, `max_depth`, `max_requests`,
    /// `path_budget` or a URL cap's name). A relative URL resolves against
    /// the document's base URL: its `<base href>`, resolved against the URL
    /// the document was reached at after its redirects, or that URL itself.
    /// A request is `duplicate` when one of its identity was already
    /// admitted: its method, its URL's key (the fragment, `utm_*`, `gclid`,
    /// `fbclid`, empty pairs and `;jsessionid=` dropped, the parameters
    /// sorted) and its body's hash, or the `key` the program declares in
    /// their place; it is still fetched at its own URL. The request sits
    /// one level deeper than the one whose attempt added it. An attempt
    /// whose lease no longer holds fails with `attempt_ended`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// This is the enqueue of the request of `url`, as
    /// [`crate::CrawlContext::enqueue`], which `send()` sends.
    pub fn enqueue<Q: Serialize + DeserializeOwned>(
        &self,
        url: impl Into<String>,
        label: &Label<Q>,
        payload: Q,
    ) -> Enqueue {
        Enqueue {
            enqueue: self.inner.enqueue(url, label, payload),
            connection: self.blocking_connection(),
        }
    }

    /// Adds every link a locator matches on the page a handler's attempt
    /// was given, a handler context's `enqueueLinks`, each a `GET` of its
    /// `href` resolved against the document's base URL (its `<base href>`,
    /// resolved against the URL the document was reached at after its
    /// redirects, or that URL itself), its query percent-encoded in the
    /// document's encoding as a browser encodes it, and answers each
    /// matched element in document order: added, with its crawl request's
    /// ID and URL; not added, with its reason, as `crawl_request.enqueue`
    /// answers; `no_link` for an element without an `href`; or
    /// `skipped_by_payload` for one whose entry in `payloads` is `null`.
    /// Every link takes `payload`, or its own entry of `payloads`, the
    /// program's Payload function's answers for the elements
    /// `crawl_request.links` read; a Payload that does not fit its Label
    /// fails the call with `payload_invalid` and adds none. `limit` adds at
    /// most that many links: once they are added, each later element whose
    /// link the Frontier would judge is not added, with `link_limit`.
    /// `include` and `exclude` only narrow the crawl's scope; a pattern the
    /// engine cannot match fails with `pattern_unsupported`. A page no
    /// fetch tier serves fails with `closed`, and an attempt whose lease no
    /// longer holds with `attempt_ended`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// This is the enqueue of the link of each element `links` matches, as
    /// [`crate::CrawlContext::enqueue_links`], which `send()` sends; a
    /// [`LinkPayload::with`] function runs on the connection's runtime
    /// while the handler's thread waits.
    pub fn enqueue_links<Q: Serialize + DeserializeOwned + 'static>(
        &self,
        links: &blocking::Locator,
        label: &Label<Q>,
        payload: impl Into<LinkPayload<Q>>,
    ) -> EnqueueLinks {
        EnqueueLinks {
            enqueue: self
                .inner
                .enqueue_links_of(links.id(), label, payload.into()),
            connection: self.blocking_connection(),
        }
    }

    /// The attempt's connection, blocking, whose runtime the calls run on
    /// while the handler's thread waits.
    fn blocking_connection(&self) -> blocking::ClientConnection {
        blocking::ClientConnection::from(self.inner.connection().clone())
    }
}

impl Foxhound {
    /// Starts a crawl from its seeds, and answers once its Run is started.
    /// The engine then leases each work item to the program's handler as a
    /// `crawl_request.handle` request, and the crawl's Run finishes once
    /// every work item is done, or, with `crawl.keep_alive`, once
    /// `run.finish` lets it and every work item is done. A Run ID works as
    /// `run.start`'s.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of `crawl`'s Run from `seeds`, as
    /// [`crate::Foxhound::start_crawl`], which `send()` sends.
    ///
    /// A code-free crawl, a [`crate::CrawlDeclaration`] or a
    /// [`crate::CrawlExtraction`], starts the same way with no handler, as
    /// it does there.
    ///
    /// With nothing set, a crawl obeys `robots.txt`, keeps one request in
    /// flight per domain (`politeness.domain.concurrency: 1`) and waits at
    /// least one second between starts (`politeness.domain.delay: 1s`);
    /// pressing harder is set explicitly and recorded in the Run report's
    /// `weakened` and a warning (ADR 0086).
    ///
    /// Close the Run on `SIGINT` and `SIGTERM` so the engine checkpoints in-flight
    /// work cleanly instead of losing the process mid-attempt. If a process dies
    /// before closing, leased attempts are marked `handler_lost` and retried on
    /// resume against `crawl.retry.max`; raise `crawl.retry.max` or call
    /// [`CrawlRun::retry_failed`] to retry items that hit the cap.
    pub fn start_crawl(
        &self,
        crawl: impl crate::CrawlTarget,
        seeds: impl IntoIterator<Item = Seed>,
    ) -> StartCrawl {
        StartCrawl {
            start: crate::StartCrawl::new(
                self.linked.connection(),
                &self.crawl_requests,
                crawl,
                seeds,
            ),
            connection: self.connection.clone(),
        }
    }
}

impl crate::crawl::target::Parts for &Crawl {
    fn parts(self, seeds: Vec<Seed>) -> crate::crawl::target::Started {
        crate::crawl::target::Parts::parts(&self.inner, seeds)
    }
}

impl crate::CrawlTarget for &Crawl {}

/// The start of a crawl's Run, as [`crate::StartCrawl`]. Nothing is sent
/// until [`StartCrawl::send`].
#[must_use = "a crawl's start is sent only by send()"]
#[derive(Debug)]
pub struct StartCrawl {
    start: crate::StartCrawl,
    connection: blocking::ClientConnection,
}

impl StartCrawl {
    /// Names the Run for good, as [`crate::StartCrawl::run_id`] does.
    pub fn run_id(self, run_id: &RunId) -> Self {
        Self {
            start: self.start.run_id(run_id),
            ..self
        }
    }

    /// Tags the Run, as [`crate::StartCrawl::tags`] does.
    pub fn tags(self, tags: Tags) -> Self {
        Self {
            start: self.start.tags(tags),
            ..self
        }
    }

    /// Sets the Run's own `limits`, as [`crate::StartCrawl::limits`] does.
    pub fn limits(self, limits: Limits) -> Self {
        Self {
            start: self.start.limits(limits),
            ..self
        }
    }

    /// Sets the Run's own settings, as [`crate::StartCrawl::settings`]
    /// does.
    pub fn settings(self, settings: Settings) -> Self {
        Self {
            start: self.start.settings(settings),
            ..self
        }
    }

    /// Lets the Run outlive the Connection, as
    /// [`crate::StartCrawl::detached`] does.
    pub fn detached(self) -> Self {
        Self {
            start: self.start.detached(),
            ..self
        }
    }

    /// Offers this Connection's handler to a live crawl another Connection
    /// started, as each of `workers=N` handler processes does (ADR 0024): the
    /// engine leases the crawl's work items to it as `crawl_request.handle`
    /// requests too, as many at once as `crawl.handler.concurrency` allows on
    /// each Connection. Answers the crawl's Run report once the Run ends;
    /// cancelling the call stops new work items, while those already sent still
    /// expect their answers. A Run this engine does not run as a crawl fails
    /// with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Runs `count` serving loops on the Connection's runtime, as
    /// [`crate::StartCrawl::workers`] does (ADR 0024).
    pub fn workers(self, count: usize) -> Self {
        Self {
            start: self.start.workers(count),
            ..self
        }
    }

    /// Starts the crawl's Run and blocks the calling thread until it is
    /// started.
    ///
    /// # Errors
    ///
    /// As awaiting [`crate::StartCrawl`].
    pub fn send(self) -> Result<CrawlRun, Error> {
        let Self { start, connection } = self;
        let inner = connection.wait(start.into_future())?;
        Ok(CrawlRun {
            run: Run::from(inner.run().clone()),
            inner,
            connection,
        })
    }
}

/// The enqueue of one request from inside a blocking handler's attempt, as
/// [`crate::Enqueue`]. Nothing is sent until [`Enqueue::send`].
#[must_use = "an enqueue is sent only by send()"]
#[derive(Debug)]
pub struct Enqueue {
    enqueue: crate::Enqueue,
    connection: blocking::ClientConnection,
}

impl Enqueue {
    /// The request, sent with `method`, as [`crate::Enqueue::method`].
    pub fn method(self, method: HttpMethod) -> Self {
        self.with(|enqueue| enqueue.method(method))
    }

    /// The request, sent with the extra header `name: value`, as
    /// [`crate::Enqueue::header`].
    pub fn header(self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.with(|enqueue| enqueue.header(name, value))
    }

    /// The request, sent with `body`, as [`crate::Enqueue::body`].
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        self.with(|enqueue| enqueue.body(body))
    }

    /// The request, of `priority` among the requests of its domain, as
    /// [`crate::Enqueue::priority`].
    pub fn priority(self, priority: i64) -> Self {
        self.with(|enqueue| enqueue.priority(priority))
    }

    /// The request, ahead of every request its domain holds pending, as
    /// [`crate::Enqueue::forefront`].
    pub fn forefront(self) -> Self {
        self.with(crate::Enqueue::forefront)
    }

    /// The request, identified in the Frontier by `key`, as
    /// [`crate::Enqueue::key`].
    pub fn key(self, key: impl Into<String>) -> Self {
        self.with(|enqueue| enqueue.key(key))
    }

    /// Enqueues the request and blocks the calling thread until the
    /// Frontier answers what it did with it.
    ///
    /// # Errors
    ///
    /// As awaiting [`crate::Enqueue`].
    pub fn send(self) -> Result<Enqueued, Error> {
        self.connection.wait(self.enqueue.into_future())
    }

    /// The enqueue with `set` applied to its async twin.
    fn with(self, set: impl FnOnce(crate::Enqueue) -> crate::Enqueue) -> Self {
        Self {
            enqueue: set(self.enqueue),
            ..self
        }
    }
}

/// The enqueue of every link a locator matches from inside a blocking
/// handler's attempt, as [`crate::EnqueueLinks`]. Nothing is sent until
/// [`EnqueueLinks::send`].
#[must_use = "an enqueue of links is sent only by send()"]
#[derive(Debug)]
pub struct EnqueueLinks {
    enqueue: crate::EnqueueLinks,
    connection: blocking::ClientConnection,
}

impl EnqueueLinks {
    /// The links, each matching one of `patterns`, as
    /// [`crate::EnqueueLinks::include`].
    pub fn include<I>(self, patterns: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.with(|enqueue| enqueue.include(patterns))
    }

    /// The links, each matching none of `patterns`, as
    /// [`crate::EnqueueLinks::exclude`].
    pub fn exclude<I>(self, patterns: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.with(|enqueue| enqueue.exclude(patterns))
    }

    /// At most `limit` links added, as [`crate::EnqueueLinks::limit`].
    pub fn limit(self, limit: u64) -> Self {
        self.with(|enqueue| enqueue.limit(limit))
    }

    /// The links, of `priority` among the requests of their domain, as
    /// [`crate::EnqueueLinks::priority`].
    pub fn priority(self, priority: i64) -> Self {
        self.with(|enqueue| enqueue.priority(priority))
    }

    /// Enqueues the links and blocks the calling thread until the Frontier
    /// answers what it did with each.
    ///
    /// # Errors
    ///
    /// As awaiting [`crate::EnqueueLinks`].
    pub fn send(self) -> Result<EnqueuedLinks, Error> {
        self.connection.wait(self.enqueue.into_future())
    }

    /// The enqueue with `set` applied to its async twin.
    fn with(self, set: impl FnOnce(crate::EnqueueLinks) -> crate::EnqueueLinks) -> Self {
        Self {
            enqueue: set(self.enqueue),
            ..self
        }
    }
}

/// A crawl's Run, which [`Crawl::serve`] serves, as [`crate::CrawlRun`].
/// Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct CrawlRun {
    pub(super) inner: crate::CrawlRun,
    run: Run,
    pub(super) connection: blocking::ClientConnection,
}

impl CrawlRun {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        self.inner.id()
    }

    /// Lists each seed's answer, in the order the crawl's start gave them, as
    /// the Frontier answered it when the Run began: added, with its crawl
    /// request's ID and its URL as parsed, or not added, with its reason
    /// (`duplicate`, `domain_policy`, `unsupported_scheme`, `invalid_url` or a
    /// cap's name). A relative seed is `invalid_url`, since nothing gives it a
    /// base. So a crawl that does nothing says why. Read from the Run's
    /// committed state, so it works on a Run no program holds.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn seeds(&self) -> &[Enqueued] {
        self.inner.seeds()
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
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn add_seed(&self, seed: Seed) -> super::AddSeed {
        super::AddSeed::new(self.inner.add_seed(seed), self.connection.clone())
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
    pub fn add_seeds<I>(&self, seeds: I) -> super::AddSeeds
    where
        I: IntoIterator<Item = Seed>,
        I::IntoIter: Send + 'static,
    {
        super::AddSeeds::new(
            self.inner.add_seeds(futures_util::stream::iter(seeds)),
            self.connection.clone(),
        )
    }

    /// Answers whether the start that opened the Run's handle began the Run,
    /// `new`, resumed it, `resumed`, or found it ended, `ended` (ADR 0102).
    /// The handle answers it from that start's answer, without a message to
    /// the engine.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn started_as(&self) -> goodfetch_protocol::run::StartedAs {
        self.inner.started_as()
    }

    /// The Run, with every member a blocking Run has, such as `close`.
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
    /// The report typed as a crawl's, as [`crate::CrawlRun::finish`] answers
    /// it.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::CrawlRun::finish`].
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
    /// The report typed as a crawl's, as [`crate::CrawlRun::stop`] answers
    /// it.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::CrawlRun::stop`].
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
    /// The report typed as a crawl's, as [`crate::CrawlRun::report`] answers
    /// it.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::CrawlRun::report`].
    pub fn report(&self) -> Call<Answer<CrawlRunReport>, RunParams> {
        self.run.report().typed()
    }

    /// Reads a Continuous crawl's report while it runs: its Lifecycle, Usage,
    /// Learned rules, Sinks, warnings and Freshness, built from the same parts
    /// as the Run report, with no Verdict and no Completeness. A Run another
    /// engine works is read through that engine, and one no engine works from
    /// its committed state, never taking its writer lock. On a Run that is not
    /// a Continuous crawl, fails with `run_not_continuous`; on a Run ID the Run
    /// index does not list, fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// Once sent, fails with `run_not_continuous` on any Run that is not a
    /// Continuous crawl, or `run_not_found` if the Run does not exist.
    pub fn report_so_far(&self) -> Call<Answer<RunReportSoFar>> {
        self.connection.call(
            RUN_REPORT_SO_FAR.name,
            &RunParams {
                run_id: self.id().clone(),
            },
        )
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
    /// The crawl's progress, as [`crate::CrawlRun::progress`] follows it, as
    /// an iterator that blocks the calling thread until the next value.
    ///
    /// # Errors
    ///
    /// An item fails as [`crate::CrawlRun::progress`]'s does.
    pub fn progress(&self) -> EventIter<Answer<CrawlRunProgress>> {
        self.run.progress().typed()
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
    /// The Records of `dataset` the Run has kept, as
    /// [`crate::CrawlRun::records`], as an iterator that blocks the calling
    /// thread until the next one. Dropping it cancels the read.
    ///
    /// # Errors
    ///
    /// An item fails as [`crate::CrawlRun::records`]'s does, after which
    /// the iterator ends.
    pub fn records<I>(&self, dataset: &Dataset<I>) -> ReadRecords<I> {
        ReadRecords::new(self.inner.records(dataset), self.connection.clone())
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
    /// The entries of `dataset`'s rejection table, as
    /// [`crate::CrawlRun::rejected_records`], as an iterator that blocks
    /// the calling thread until the next one. Dropping it cancels the page
    /// being read.
    ///
    /// # Errors
    ///
    /// An item fails as [`crate::CrawlRun::rejected_records`]'s does,
    /// after which the iterator ends.
    pub fn rejected_records<I>(&self, dataset: &Dataset<I>) -> ReadRejectedRecords {
        ReadRejectedRecords::new(
            self.inner.rejected_records(dataset),
            self.connection.clone(),
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
    /// The Export of `dataset` in `format`, as [`crate::CrawlRun::export`],
    /// which `send()` sends.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::CrawlRun::export`].
    pub fn export<I>(&self, dataset: &Dataset<I>, format: ExportFormat) -> super::ExportDataset {
        super::ExportDataset::new(self.inner.export(dataset, format), self.connection.clone())
    }
}

/// The read of one Dataset's Records, as [`crate::ReadRecords`], as an
/// iterator that blocks the calling thread until the next one. It is sent
/// when first iterated; dropping it cancels the read.
#[must_use = "a read sends nothing until it is iterated"]
#[derive(Debug)]
pub struct ReadRecords<I> {
    /// The read, until first iterated.
    unsent: Option<crate::ReadRecords<I>>,
    /// The read, once sent.
    sent: Option<EventIter<DatasetRecord<I>>>,
    connection: blocking::ClientConnection,
}

impl<I> ReadRecords<I> {
    fn new(read: crate::ReadRecords<I>, connection: blocking::ClientConnection) -> Self {
        Self {
            unsent: Some(read),
            sent: None,
            connection,
        }
    }

    /// Reads the Records numbered past `after`, as
    /// [`crate::ReadRecords::after`]. Set before the read is first
    /// iterated.
    pub fn after(mut self, after: u64) -> Self {
        self.unsent = self.unsent.map(|read| read.after(after));
        self
    }
}

impl<I: DeserializeOwned + Send + Sync + 'static> Iterator for ReadRecords<I> {
    type Item = Result<DatasetRecord<I>, Error>;

    /// The next Record, blocking until it comes, or `None` once the read
    /// ended.
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(read) = self.unsent.take() {
            self.sent = Some(self.connection.iter(read.sent()));
        }
        self.sent.as_mut()?.next()
    }
}

/// The read of one Dataset's rejection table, as
/// [`crate::ReadRejectedRecords`], as an iterator that blocks the calling
/// thread until the next entry. It is sent when first iterated; dropping
/// it cancels the page being read.
#[must_use = "a read sends nothing until it is iterated"]
#[derive(Debug)]
pub struct ReadRejectedRecords {
    /// The read, until first iterated.
    unsent: Option<crate::ReadRejectedRecords>,
    /// The read, once sent.
    sent: Option<EventIter<RejectedRecord>>,
    connection: blocking::ClientConnection,
}

impl ReadRejectedRecords {
    fn new(read: crate::ReadRejectedRecords, connection: blocking::ClientConnection) -> Self {
        Self {
            unsent: Some(read),
            sent: None,
            connection,
        }
    }

    /// Reads the entries numbered past `after`, as
    /// [`crate::ReadRejectedRecords::after`]. Set before the read is first
    /// iterated.
    pub fn after(mut self, after: u64) -> Self {
        self.unsent = self.unsent.map(|read| read.after(after));
        self
    }
}

impl Iterator for ReadRejectedRecords {
    type Item = Result<RejectedRecord, Error>;

    /// The next entry, blocking until it comes, or `None` once the read
    /// ended.
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(read) = self.unsent.take() {
            self.sent = Some(self.connection.iter(read.sent()));
        }
        self.sent.as_mut()?.next()
    }
}

/// C-SEND-SYNC: a blocking crawl is shared with the threads that serve it,
/// and its Run and its start move between threads (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Crawl>();
    send_sync_static::<CrawlRun>();
    send_sync_static::<CrawlContext<()>>();
    send_static::<StartCrawl>();
    send_static::<Enqueue>();
    send_static::<EnqueueLinks>();
    send_sync_static::<ReadRecords<()>>();
    send_sync_static::<ReadRejectedRecords>();
};
