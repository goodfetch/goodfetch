//! A crawl for a program that does not use async (ADRs 0081 and 0093):
//! [`Crawl`] declares as [`crate::Crawl`] does, its handlers plain
//! functions, `Fn + Send + Sync + 'static`, each attempt's run on one of
//! the connection's tracked blocking threads, never on its runtime's
//! workers, as many at once as the engine sends; [`Crawl::serve`] blocks
//! the calling thread until the Run ends and returns its report.

use std::fmt;
use std::future::IntoFuture as _;
use std::sync::Arc;

use goodfetch_collie_client::{BoxError, Error};
use goodfetch_protocol::foxhound::dataset::{DatasetRecord, RECORD, RUN_RECORDS, RecordRead};
use goodfetch_protocol::foxhound::frontier::{Enqueued, EnqueuedLinks, HttpMethod};
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Foxhound, Run, blocking};
use crate::{Dataset, DatasetDeclaration, Label, LabelDeclaration, LinkPayload, Seed};

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

    /// The attempt's number, 1 for the first.
    pub fn attempt(&self) -> u32 {
        self.inner.attempt()
    }

    /// The Run's ID.
    pub fn run_id(&self) -> &RunId {
        self.inner.run_id()
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
    pub fn start_crawl(&self, crawl: &Crawl, seeds: impl IntoIterator<Item = Seed>) -> StartCrawl {
        StartCrawl {
            start: crate::StartCrawl::new(
                self.linked.connection(),
                &self.crawl_requests,
                &crawl.inner,
                seeds,
            ),
            connection: self.connection.clone(),
        }
    }
}

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
    inner: crate::CrawlRun,
    run: Run,
    connection: blocking::ClientConnection,
}

impl CrawlRun {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        self.inner.id()
    }

    /// The Run, with every member a blocking Run has, such as `close`.
    pub fn run(&self) -> &Run {
        &self.run
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
    pub fn records<I>(
        &self,
        dataset: &Dataset<I>,
    ) -> impl Iterator<Item = Result<DatasetRecord, Error>> + Send + 'static {
        self.connection
            .stream::<RecordRead>(
                RUN_RECORDS.name,
                &self.inner.records_params(dataset),
                RECORD,
            )
            .map(|read| read.map(|read| read.record))
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
};
