//! A crawl in Rust (ADRs 0093, 0109 and 0112): the program declares a
//! [`Crawl`], its Labels, their handlers and its Datasets, with no
//! connection; [`Foxhound::start_crawl`] starts its Run on the connection's
//! engine, the linked one by default (ADRs 0024 and 0087); and
//! [`Crawl::serve`] answers the engine's Crawl requests of that Run with
//! the handlers, each polled inside the serve's own future and caught at a
//! panic, until the Run ends and its report returns. A program that
//! answers its requests itself reads them from
//! [`CrawlRun::crawl_requests`], the stream beneath the serve. The SDK
//! spawns no task.

use std::any::Any;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt as _, StreamExt as _};
use goodfetch_collie_client::{BoxError, ClientConnection, Error, EventStream, HandlerCall, Run};
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CRAWL_START, CrawlRequestHandleParams, CrawlRequestHandled,
    CrawlStartParams, HANDLER_FAILED, HandlerFailedDetails, HandlerSkip, LABEL_DUPLICATE,
    LABEL_UNHANDLED, LABEL_UNKNOWN, LabelDeclaration as WireLabel, LabelUnknownDetails,
    RequestSeed, Seed as SeedRequest, Seeds,
};
use goodfetch_protocol::foxhound::dataset::{
    DATASET_DUPLICATE, DatasetDeclaration, DatasetDuplicateDetails,
};
use goodfetch_protocol::foxhound::error_code;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::{RunId, Tags};
use serde::de::DeserializeOwned;
use serde::{Deserialize as _, Serialize};
use serde_json::{Value, json};

use crate::Foxhound;
use crate::crawl_requests::CrawlRequests;
use crate::sync::AtomicU64;

mod context;
mod declaration;
mod handles;
mod run;
#[cfg(test)]
mod tests;

pub use context::CrawlContext;
use context::{Attempt, Ending, retry_requested};
pub use declaration::LabelDeclaration;
pub use handles::{Dataset, Label, Seed};
pub use run::{CrawlCall, CrawlRun};

/// The source of each crawl's own number, which its Labels and Datasets
/// carry, so a crawl tells its own from another's.
static NEXT_CRAWL: AtomicU64 = AtomicU64::new(0);

/// A crawl's own number within the process, which its Labels, Datasets
/// and seeds carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CrawlId(u64);

impl CrawlId {
    /// A number no other crawl of the process has.
    fn next() -> Self {
        Self(NEXT_CRAWL.fetch_add(1, Ordering::Relaxed))
    }
}

/// A handler's future, borrowing what the crawl's handlers borrow.
type HandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send + 'a>>;

/// A Label's handler, called once for each attempt: it reads the
/// attempt's Payload, then returns the handler's future. Shared, so a
/// blocking serve hands the connection's runtime a crawl of its own.
type Handler<'a> = Arc<dyn Fn(Delivered) -> Result<HandlerFuture<'a>, Error> + Send + Sync + 'a>;

/// One attempt the engine delivered, and its request's Payload, taken out
/// of its params so it is read without a copy.
struct Delivered {
    attempt: Arc<Attempt>,
    payload: Value,
}

/// A crawl the program declares (ADRs 0093 and 0112): its Labels, the
/// handler of each, and its Datasets. Declaring needs no connection, and a
/// mistake fails at once: a second Label of one name or a second handler
/// with `label_duplicate`, a handler for another crawl's Label with
/// `label_unknown`. Its lifetime is the one its handlers borrow for, so a
/// handler may borrow from the scope that serves the crawl (ADR 0112).
///
/// ```no_run
/// use goodfetch_foxhound::{ConnectOptions, Crawl, CrawlContext, Foxhound, Seed};
///
/// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct Book {
///     shelf: u32,
/// }
///
/// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
/// let mut crawl = Crawl::new();
/// let book = crawl.label::<Book>("book")?;
/// let titles = crawl.dataset::<String>("titles")?;
/// crawl.handle(&book, move |ctx: CrawlContext<Book>| async move {
///     ctx.push(&titles, &ctx.url().to_owned()).await?;
///     Ok(())
/// })?;
/// let run = foxhound
///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &book, Book { shelf: 1 })])
///     .await?;
/// let report = crawl.serve(&run).await?;
/// foxhound.close().await?;
/// # Ok(())
/// # }
/// ```
pub struct Crawl<'a> {
    id: CrawlId,
    /// Each Label in the order declared.
    labels: Vec<Declared<'a>>,
    datasets: Vec<DatasetDeclaration>,
}

/// A Label a crawl declares: its name, what `crawl.start` declares it
/// with, and its handler once attached.
#[derive(Clone)]
struct Declared<'a> {
    name: Arc<str>,
    wire: WireLabel,
    handler: Option<Handler<'a>>,
}

impl Default for Crawl<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Crawl<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let labels: Vec<(&str, bool)> = self
            .labels
            .iter()
            .map(|declared| (&*declared.name, declared.handler.is_some()))
            .collect();
        formatter
            .debug_struct("Crawl")
            .field("labels", &labels)
            .field("datasets", &self.datasets)
            .finish_non_exhaustive()
    }
}

impl<'a> Crawl<'a> {
    /// A crawl that declares nothing yet.
    pub fn new() -> Self {
        Self {
            id: CrawlId::next(),
            labels: Vec::new(),
            datasets: Vec::new(),
        }
    }

    /// The same crawl, its Labels, handlers and Datasets shared: what a
    /// blocking serve moves to the connection's runtime.
    pub(crate) fn shared(&self) -> Self {
        Self {
            id: self.id,
            labels: self.labels.clone(),
            datasets: self.datasets.clone(),
        }
    }

    /// Declares a Label, whose requests carry a Payload of type `P`, and
    /// returns it, the handle seeds, enqueues and its handler name it by:
    /// a bare name, `crawl.label::<P>(name)`, or a [`LabelDeclaration`]
    /// with the Label's Payload schema and settings (ADR 0099). A seed, an
    /// enqueue and a handler of the Label take a `P`, so the compiler
    /// refuses another type, `()` for a Label without a Payload; with a
    /// schema, the engine checks each Payload against it (ADR 0082).
    ///
    /// # Errors
    ///
    /// `label_duplicate` when the crawl already declares a Label of its
    /// name, and `setting_not_applicable` for a key of its settings a
    /// Label does not set; neither declares it.
    pub fn label<P: Serialize + DeserializeOwned>(
        &mut self,
        declaration: impl Into<LabelDeclaration<'a, P>>,
    ) -> Result<Label<P>, Error> {
        let (name, wire, schema) = declaration.into().into_parts()?;
        if self.labels.iter().any(|declared| *declared.name == *name) {
            return Err(Error::of_sdk(
                &LABEL_DUPLICATE,
                &format!("the crawl already declares the Label `{name}`"),
                "Declare each Label once, and use the Label the first `crawl.label` returned.",
                &json!({ "label": name }),
            ));
        }
        let name: Arc<str> = Arc::from(name);
        self.labels.push(Declared {
            name: Arc::clone(&name),
            wire,
            handler: None,
        });
        Ok(Label {
            name,
            crawl: self.id,
            payload: schema.map(Arc::new),
        })
    }

    /// Attaches `handler` to `label`: the serve calls it once for each
    /// attempt of a request of the Label, with the attempt's context, and
    /// the attempt commits what it pushed once it returns `Ok`. An `Err`,
    /// or a panic, fails the attempt with `handler_failed`, a Goodfetch
    /// [`Error`] keeping its own code (ADR 0109); a handler that called
    /// [`CrawlContext::skip`] or [`CrawlContext::retry`] ends it as it
    /// asked.
    ///
    /// # Errors
    ///
    /// `label_unknown` for a Label of another crawl, and `label_duplicate`
    /// when the Label already has a handler.
    pub fn handle<P, F, Fut>(&mut self, label: &Label<P>, handler: F) -> Result<(), Error>
    where
        P: DeserializeOwned + Send + 'a,
        F: FnOnce(CrawlContext<P>) -> Fut + Clone + Send + Sync + 'a,
        Fut: Future<Output = Result<(), BoxError>> + Send + 'a,
    {
        if label.crawl != self.id {
            return Err(self.label_unknown(&label.name));
        }
        let slot = self
            .labels
            .iter_mut()
            .find_map(|declared| (declared.name == label.name).then_some(&mut declared.handler))
            .expect("invariant: a Label of this crawl is declared on it");
        if slot.is_some() {
            return Err(Error::of_sdk(
                &LABEL_DUPLICATE,
                &format!("the Label `{}` already has a handler", label.name),
                "Attach one handler to each Label.",
                &json!({ "label": &*label.name }),
            ));
        }
        *slot = Some(Arc::new(move |Delivered { attempt, payload }| {
            let payload = P::deserialize(payload).map_err(payload_invalid)?;
            let handler = handler.clone();
            let future: HandlerFuture<'a> = Box::pin(handler(CrawlContext::new(attempt, payload)));
            Ok(future)
        }));
        Ok(())
    }

    /// Declares the Dataset `name`, whose Records are of type `I`, and
    /// returns it, the handle a handler pushes Records into.
    ///
    /// # Errors
    ///
    /// `dataset_duplicate` when the crawl already declares a Dataset
    /// `name`.
    pub fn dataset<I>(&mut self, name: &str) -> Result<Dataset<I>, Error> {
        if self.datasets.iter().any(|declared| declared.name == name) {
            return Err(Error::of_sdk(
                &DATASET_DUPLICATE,
                &format!("the crawl already declares the Dataset `{name}`"),
                "Declare each Dataset once, and use the Dataset the first `crawl.dataset` returned.",
                &DatasetDuplicateDetails {
                    dataset: name.to_owned(),
                },
            ));
        }
        self.datasets.push(DatasetDeclaration {
            name: name.to_owned(),
            schema: None,
            key: Vec::new(),
        });
        Ok(Dataset {
            name: Arc::from(name),
            crawl: self.id,
            record: PhantomData,
        })
    }

    /// Serves `run`, a Run this crawl started: answers each of the Run's
    /// Crawl requests with its Label's handler, as many at once as the
    /// engine sends, each polled inside this future and caught at a panic,
    /// which fails its attempt with `handler_failed` while the serve goes
    /// on; returns the Run's report once the Run ends (`run.report`).
    ///
    /// # Errors
    ///
    /// What `run.report` fails with, such as `closed` once the connection
    /// has ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops the serve: each attempt it was running,
    /// and each request of the Run the connection kept aside for it, is
    /// given back to the engine, which delivers it again, uncounted, to
    /// the Run's next serve, on this connection or on one that resumes the
    /// Run.
    pub async fn serve(&self, run: &CrawlRun) -> Result<CrawlRunReport, Error> {
        if let Some(error) = self.unhandled() {
            return Err(error);
        }
        let run_id = run.id().clone();
        let _forget = Forget {
            requests: Arc::clone(&run.requests),
            run_id: run_id.clone(),
        };
        let declared: Arc<[String]> = self.declared().into();
        let mut report = run.ended();
        let mut attempts = FuturesUnordered::new();
        let mut reading = true;
        let ended = loop {
            tokio::select! {
                biased;
                ended = &mut report => break ended,
                Some(()) = attempts.next(), if !attempts.is_empty() => {}
                call = run.requests.next(&run_id), if reading => match call {
                    Some(call) => attempts.push(self.attempt(call, run, &declared)),
                    None => reading = false,
                },
            }
        };
        drop(attempts);
        ended
    }

    /// Runs one attempt with its Label's handler and answers the engine:
    /// a `ctx.skip` or `ctx.retry` the handler called ends the attempt as
    /// it asked, whatever the handler then returned (ADR 0109).
    async fn attempt(&self, call: HandlerCall, run: &CrawlRun, declared: &Arc<[String]>) {
        let (outcome, ending) = match self.handler_of(&call, run, declared) {
            Ok((label, handler, delivered)) => {
                let held = Arc::clone(&delivered.attempt);
                let outcome = AssertUnwindSafe(async move { handler(delivered)?.await })
                    .catch_unwind()
                    .await;
                let outcome = outcome
                    .unwrap_or_else(|panic| Err(Box::new(panicked(&label, &*panic)) as BoxError));
                (outcome, held.ending())
            }
            Err(error) => (Err(Box::new(error) as BoxError), None),
        };
        sent(answer(call, ending, outcome));
    }

    /// The Label of `call`, its handler and the attempt it is handed, which
    /// the serve starts inside its caught future, so a panic while the
    /// handler's future is built is caught too.
    fn handler_of(
        &self,
        call: &HandlerCall,
        run: &CrawlRun,
        declared: &Arc<[String]>,
    ) -> Result<(Arc<str>, &Handler<'a>, Delivered), Error> {
        let delivered = delivered(call, &run.connection, &run.run, self.id, declared)
            .map_err(|error| params_unread(&error))?;
        let label = delivered.attempt.params.crawl_request.label.as_str();
        let Some(Declared {
            name,
            handler: Some(handler),
            ..
        }) = self.labels.iter().find(|declared| *declared.name == *label)
        else {
            return Err(label_unknown(label, declared.to_vec()));
        };
        Ok((Arc::clone(name), handler, delivered))
    }

    /// The `label_unhandled` of the crawl's first Label with no handler,
    /// if it has one.
    fn unhandled(&self) -> Option<Error> {
        let Declared { name, .. } = self
            .labels
            .iter()
            .find(|declared| declared.handler.is_none())?;
        Some(Error::of_sdk(
            &LABEL_UNHANDLED,
            &format!("the crawl attaches no handler to the Label `{name}`"),
            "Attach one with `crawl.handle(label, handler)`, or stop declaring the Label.",
            &json!({ "label": &**name }),
        ))
    }

    /// The names of the Labels the crawl declares, in order.
    fn declared(&self) -> Vec<String> {
        self.labels
            .iter()
            .map(|declared| declared.name.to_string())
            .collect()
    }

    /// The `label_unknown` of `label`, which the crawl does not declare.
    fn label_unknown(&self, label: &str) -> Error {
        label_unknown(label, self.declared())
    }

    /// What `crawl.start` takes to start the crawl from `seeds`. A crawl
    /// that attaches a handler to some Labels attaches one to each; one
    /// that attaches none is answered by the program itself, through
    /// [`CrawlRun::crawl_requests`].
    fn start_params(&self, seeds: Vec<Seed>) -> Result<CrawlStartParams, Error> {
        if self
            .labels
            .iter()
            .any(|declared| declared.handler.is_some())
            && let Some(error) = self.unhandled()
        {
            return Err(error);
        }
        let mut requests = Vec::with_capacity(seeds.len());
        for seed in seeds {
            if seed.crawl != self.id {
                return Err(self.label_unknown(&seed.label));
            }
            requests.push(SeedRequest::Request(RequestSeed {
                url: seed.url,
                label: seed.label.to_string(),
                payload: Some(seed.payload?),
                priority: 0,
                forefront: false,
                method: HttpMethod::default(),
                body: None,
                key: None,
            }));
        }
        let Some(seeds) = Seeds::new(requests) else {
            return Err(Error::refused(
                CRAWL_START.name,
                "a crawl starts from at least one seed",
                "Give `start_crawl` one seed or more.",
            ));
        };
        Ok(CrawlStartParams {
            run_id: None,
            tags: Tags::default(),
            seeds,
            labels: self
                .labels
                .iter()
                .map(|declared| (declared.name.to_string(), declared.wire.clone()))
                .collect(),
            datasets: self.datasets.clone(),
            extraction: None,
            hooks: Vec::new(),
            settings: serde_json::Map::new(),
            project: None,
            limits: Vec::new(),
            // The program's crawl waits on it, so it never detaches (ADR
            // 0102).
            detached: false,
        })
    }
}

/// The `label_unknown` of `label`, among the Labels `declared`.
fn label_unknown(label: &str, declared: Vec<String>) -> Error {
    Error::of_sdk(
        &LABEL_UNKNOWN,
        &format!("the crawl does not declare the Label `{label}`"),
        "Use a Label this crawl's `crawl.label` returned, or declare it on this crawl.",
        &LabelUnknownDetails {
            label: label.to_owned(),
            declared,
        },
    )
}

/// The `payload_invalid` of a Payload that does not serialize, or of a
/// delivered one that does not read as its Label's type.
fn payload_invalid(error: serde_json::Error) -> Error {
    Error::of_program(Box::new(error), error_code::PAYLOAD_INVALID)
}

/// The attempt `call` delivers on `connection`, of `run`, whose page it
/// reads, and of the crawl `crawl` whose Labels are `declared`, with its
/// request's Payload, `null` when it has none.
///
/// # Errors
///
/// Why its params do not read as this SDK's, which [`params_unread`]
/// refuses.
fn delivered(
    call: &HandlerCall,
    connection: &ClientConnection,
    run: &Run,
    crawl: CrawlId,
    declared: &Arc<[String]>,
) -> Result<Delivered, serde_json::Error> {
    let mut params = CrawlRequestHandleParams::deserialize(call.params())?;
    let payload = params.crawl_request.payload.take().unwrap_or(Value::Null);
    let page = params.page_id.clone().map(|page_id| run.page_of(page_id));
    let attempt = Arc::new(Attempt::new(
        connection.clone(),
        page,
        params,
        crawl,
        Arc::clone(declared),
    ));
    Ok(Delivered { attempt, payload })
}

/// Logs an answer to a Crawl request that did not reach the engine, whose
/// connection has ended: nothing waits for it.
fn sent(answered: Result<(), Error>) {
    if let Err(error) = answered {
        tracing::debug!(
            %error,
            "A Crawl request's answer did not reach the engine: its connection has ended."
        );
    }
}

/// The `invalid_params` of a Crawl request whose params do not read as
/// this SDK's, for `error`.
fn params_unread(error: &serde_json::Error) -> Error {
    Error::refused(
        CRAWL_REQUEST_HANDLE.name,
        &format!("its params do not read as this SDK's: {error}"),
        "Use an engine of this SDK's own version.",
    )
}

/// Answers the engine's `call` of an attempt that ended as `ending`, if
/// the program ended it itself, with `outcome`: a `ctx.skip` or
/// `ctx.retry` ends the attempt as it asked, whatever `outcome` is (ADR
/// 0109).
///
/// # Errors
///
/// `closed` once the connection has ended.
fn answer(
    call: HandlerCall,
    ending: Option<Ending>,
    outcome: Result<(), BoxError>,
) -> Result<(), Error> {
    match (ending, outcome) {
        (Some(Ending::Skipped(message)), _) => call.answer(
            serde_json::to_value(CrawlRequestHandled {
                records: Vec::new(),
                skipped: Some(HandlerSkip { message }),
            })
            .expect("invariant: a skip's answer serializes"),
        ),
        (Some(Ending::RetryRequested(message)), _) => {
            call.fail(retry_requested(&message), error_code::HANDLER_FAILED)
        }
        (None, Ok(())) => call.answer(
            serde_json::to_value(CrawlRequestHandled::default())
                .expect("invariant: an empty answer serializes"),
        ),
        (None, Err(error)) => call.fail(error, error_code::HANDLER_FAILED),
    }
}

/// Gives back, once the serve of a Run or its stream of Crawl requests
/// ends or is dropped, the Crawl requests the connection keeps aside for
/// that Run: the engine delivers them again to the Run's next reader.
struct Forget {
    requests: Arc<CrawlRequests>,
    run_id: RunId,
}

impl Drop for Forget {
    fn drop(&mut self) {
        self.requests.forget(&self.run_id);
    }
}

/// The `handler_failed` of a handler of the Label `label` that panicked
/// with `panic` (ADR 0109).
fn panicked(label: &str, panic: &(dyn Any + Send)) -> Error {
    let message = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic of no message".to_owned());
    Error::of_sdk(
        &HANDLER_FAILED,
        &format!("the handler of the Label `{label}` panicked: {message}"),
        "Fix the handler's panic, or return an error from it instead.",
        &HandlerFailedDetails {
            exception_type: "panic".to_owned(),
            exception_message: message,
            exception_stack: String::new(),
        },
    )
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
    /// Awaiting sends it; [`Crawl::serve`] then serves the Run. The crawl's
    /// declarations are checked before anything is sent.
    ///
    /// ```no_run
    /// use goodfetch_foxhound::{ConnectOptions, Crawl, CrawlContext, Foxhound, Seed};
    ///
    /// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
    /// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    /// let mut crawl = Crawl::new();
    /// let page = crawl.label::<()>("page")?;
    /// crawl.handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })?;
    /// let run = foxhound
    ///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &page, ())])
    ///     .await?;
    /// crawl.serve(&run).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Awaiting it fails with `label_unhandled` for a Label with no
    /// handler, `label_unknown` for a seed of another crawl's Label,
    /// `payload_invalid` for a Payload that does not serialize, or that
    /// its Label's schema refuses, naming each issue, `invalid_params`
    /// with no seed, and what `crawl.start` fails with.
    pub fn start_crawl(
        &self,
        crawl: &Crawl<'_>,
        seeds: impl IntoIterator<Item = Seed>,
    ) -> StartCrawl {
        StartCrawl::new(self.as_ref(), self.crawl_requests(), crawl, seeds)
    }
}

/// The start of a crawl's Run (`crawl.start`), which awaiting sends.
/// Nothing is sent until it is awaited.
#[must_use = "a crawl's start is sent only when it is awaited"]
#[derive(Debug)]
pub struct StartCrawl {
    connection: ClientConnection,
    requests: Arc<CrawlRequests>,
    crawl: CrawlId,
    declared: Arc<[String]>,
    /// The start's params, or why the crawl cannot start.
    params: Result<CrawlStartParams, Error>,
}

impl StartCrawl {
    /// The start of `crawl`'s Run from `seeds` on `connection`, whose
    /// Runs share `requests`.
    pub(crate) fn new(
        connection: &ClientConnection,
        requests: &Arc<CrawlRequests>,
        crawl: &Crawl<'_>,
        seeds: impl IntoIterator<Item = Seed>,
    ) -> Self {
        Self {
            connection: connection.clone(),
            requests: Arc::clone(requests),
            crawl: crawl.id,
            declared: crawl.declared().into(),
            params: crawl.start_params(seeds.into_iter().collect()),
        }
    }

    /// Names the Run for good, as [`Start::run_id`](crate::Start::run_id)
    /// does: a start with an existing Run ID resumes the Run if it was
    /// interrupted, or returns it as it ended (ADR 0102).
    pub fn run_id(mut self, run_id: &RunId) -> Self {
        if let Ok(params) = &mut self.params {
            params.run_id = Some(run_id.as_str().to_owned());
        }
        self
    }
}

impl IntoFuture for StartCrawl {
    type Output = Result<CrawlRun, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<CrawlRun, Error>> + Send + 'static>>;

    /// Opens the connection's stream of Crawl requests before the start
    /// is sent, so the engine's first request finds it. The start carries
    /// the project file the connection carries (ADR 0083).
    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            requests,
            crawl,
            declared,
            params,
        } = self;
        let started = params.and_then(|mut params| {
            params.project = connection.project().cloned();
            requests.open(&connection)?;
            Ok(connection.start_with(CRAWL_START.name, params))
        });
        Box::pin(async move {
            let run = started?.await?;
            Ok(CrawlRun {
                run,
                connection,
                requests,
                crawl,
                declared,
            })
        })
    }
}

/// C-SEND-SYNC: a crawl is shared with the futures that serve it, and its
/// Run, its start and its handles move between threads (ADRs 0081 and
/// 0112).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Crawl<'static>>();
    send_sync_static::<CrawlRun>();
    send_sync_static::<CrawlCall>();
    send_sync_static::<EventStream<CrawlCall>>();
    send_sync_static::<Label<()>>();
    send_sync_static::<LabelDeclaration<'static, ()>>();
    send_sync_static::<Dataset<()>>();
    send_static::<Seed>();
    send_static::<StartCrawl>();
    send_static::<<StartCrawl as IntoFuture>::IntoFuture>();
};
