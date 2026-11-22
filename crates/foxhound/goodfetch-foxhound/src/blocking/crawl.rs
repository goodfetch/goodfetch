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
use goodfetch_protocol::foxhound::frontier::{Enqueued, EnqueuedLinks};
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Foxhound, Run, blocking};
use crate::{Dataset, Label, Seed};

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

    /// Declares the Label `name`, as [`crate::Crawl::label`] does.
    ///
    /// # Errors
    ///
    /// As [`crate::Crawl::label`].
    pub fn label<P>(&mut self, name: &str) -> Result<Label<P>, Error> {
        self.inner.label(name)
    }

    /// Declares the Dataset `name`, as [`crate::Crawl::dataset`] does.
    ///
    /// # Errors
    ///
    /// As [`crate::Crawl::dataset`].
    pub fn dataset<I>(&mut self, name: &str) -> Result<Dataset<I>, Error> {
        self.inner.dataset(name)
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

    /// Pushes `record` into `dataset`, as [`crate::CrawlContext::push`]
    /// does, and blocks until the engine has staged it.
    ///
    /// # Errors
    ///
    /// As [`crate::CrawlContext::push`].
    pub fn push<I: Serialize>(&self, dataset: &Dataset<I>, record: &I) -> Result<(), Error> {
        self.blocking_connection()
            .wait(self.inner.push(dataset, record))
    }

    /// Enqueues the request of `url`, as [`crate::CrawlContext::enqueue`]
    /// does, and blocks until the Frontier answers what it did with it.
    ///
    /// # Errors
    ///
    /// As [`crate::CrawlContext::enqueue`].
    pub fn enqueue<Q: Serialize>(
        &self,
        url: impl Into<String>,
        label: &Label<Q>,
        payload: Q,
    ) -> Result<Enqueued, Error> {
        self.blocking_connection()
            .wait(self.inner.enqueue(url, label, payload))
    }

    /// Enqueues the link of each element `links` matches, as
    /// [`crate::CrawlContext::enqueue_links`] does, and blocks until the
    /// Frontier answers what it did with each.
    ///
    /// # Errors
    ///
    /// As [`crate::CrawlContext::enqueue_links`].
    pub fn enqueue_links<Q: Serialize>(
        &self,
        links: &blocking::Locator,
        label: &Label<Q>,
        payload: Q,
    ) -> Result<EnqueuedLinks, Error> {
        self.blocking_connection()
            .wait(self.inner.enqueue_links_of(links.id(), label, payload))
    }

    /// The attempt's connection, blocking, whose runtime the calls run on
    /// while the handler's thread waits.
    fn blocking_connection(&self) -> blocking::ClientConnection {
        blocking::ClientConnection::from(self.inner.connection().clone())
    }
}

impl Foxhound {
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
};
