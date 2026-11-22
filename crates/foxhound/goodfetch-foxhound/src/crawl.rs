//! A crawl in Rust (ADRs 0093, 0109 and 0112): the program declares a
//! [`Crawl`], its Labels, their handlers and its Datasets, with no
//! connection; [`Foxhound::start_crawl`] starts its Run on the connection's
//! engine, the linked one by default (ADRs 0024 and 0087); and
//! [`Crawl::serve`] answers the engine's Crawl requests of that Run with
//! the handlers, each polled inside the serve's own future and caught at a
//! panic, until the Run ends and its report returns. The SDK spawns no
//! task.

use std::any::Any;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::pin::{Pin, pin};
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt as _, StreamExt as _};
use goodfetch_collie_client::{BoxError, ClientConnection, Error, HandlerCall, Run};
use goodfetch_protocol::foxhound::crawl::{
    ATTEMPT_ENDED, CRAWL_REQUEST_HANDLE, CRAWL_START, CrawlRequestHandleParams,
    CrawlRequestHandled, CrawlStartParams, HANDLER_FAILED, HandlerFailedDetails, HandlerSkip,
    LABEL_DUPLICATE, LABEL_UNHANDLED, LABEL_UNKNOWN, LabelDeclaration as WireLabel,
    LabelUnknownDetails, RETRY_REQUESTED, RequestSeed, Seed as SeedRequest, Seeds,
};
use goodfetch_protocol::foxhound::dataset::{
    CRAWL_REQUEST_PUSH, CrawlRequestPushParams, CrawlRequestPushed, DATASET_DUPLICATE,
    DatasetDeclaration, DatasetDuplicateDetails,
};
use goodfetch_protocol::foxhound::error_code;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CrawlRequestEnqueueParams, Enqueued, HttpMethod,
};
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::{RUN_REPORT, RunId, RunParams, Tags};
use serde::de::DeserializeOwned;
use serde::{Deserialize as _, Serialize};
use serde_json::{Value, json};

use crate::Foxhound;
use crate::crawl_requests::CrawlRequests;
use crate::sync::{AtomicU64, Mutex};

mod label;
#[cfg(test)]
mod tests;

pub use label::{Label, LabelDeclaration};

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
/// attempt's Payload, then returns the handler's future.
type Handler<'a> = Box<dyn Fn(Arc<Attempt>) -> Result<HandlerFuture<'a>, Error> + Send + Sync + 'a>;

/// A crawl the program declares (ADRs 0093 and 0112): its Labels, the
/// handler of each, and its Datasets. Declaring needs no connection, and a
/// mistake fails at once: a second Label of one name or a second handler
/// with `label_duplicate`, a handler for another crawl's Label with
/// `label_unknown`. Its lifetime is the one its handlers borrow for, so a
/// handler may borrow from the scope that serves the crawl (ADR 0112).
///
/// ```no_run
/// use goodfetch_foxhound::schemars::{self, JsonSchema};
/// use goodfetch_foxhound::{ConnectOptions, Crawl, CrawlContext, Foxhound, Seed};
///
/// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
/// #[derive(JsonSchema, serde::Serialize, serde::Deserialize)]
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

/// A Label as its crawl declares it: its name, its Payload's JSON Schema,
/// and its handler once attached.
struct Declared<'a> {
    name: Arc<str>,
    payload: Value,
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

    /// Declares a Label, whose requests carry a Payload of type `P`, and
    /// returns it, the handle seeds, enqueues and its handler name it by:
    /// `crawl.label::<P>(name)` with the schema `schemars` derives for `P`,
    /// or a [`LabelDeclaration`] with a schema of its own (ADRs 0082 and
    /// 0099). The engine receives the schema and checks each of the
    /// Label's Payloads against it at its enqueue, so a seed, an enqueue
    /// and a handler take a `P`, and the compiler refuses another type,
    /// `()` for a Label without a Payload.
    ///
    /// # Errors
    ///
    /// `label_duplicate` when the crawl already declares a Label of its
    /// name.
    pub fn label<P: Serialize + DeserializeOwned>(
        &mut self,
        declaration: impl Into<LabelDeclaration<P>>,
    ) -> Result<Label<P>, Error> {
        let LabelDeclaration { name, payload } = declaration.into();
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
            payload: payload.wire_schema().clone(),
            handler: None,
        });
        Ok(Label {
            name,
            crawl: self.id,
            payload: Arc::new(payload),
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
        *slot = Some(Box::new(move |attempt: Arc<Attempt>| {
            let payload = attempt.params.crawl_request.payload.as_ref();
            let payload =
                P::deserialize(payload.unwrap_or(&Value::Null)).map_err(payload_invalid)?;
            let handler = handler.clone();
            let future: HandlerFuture<'a> = Box::pin(handler(CrawlContext { attempt, payload }));
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
        let run_id = run.id().clone();
        let _forget = Forget {
            requests: &run.requests,
            run_id: &run_id,
        };
        let declared: Arc<[String]> = self.declared().into();
        let report = run
            .connection
            .call::<CrawlRunReport>(
                RUN_REPORT.name,
                &RunParams {
                    run_id: run_id.clone(),
                },
            )
            .no_timeout();
        let mut report = pin!(report.into_future());
        let mut attempts = FuturesUnordered::new();
        let mut reading = true;
        let ended = loop {
            tokio::select! {
                biased;
                ended = &mut report => break ended,
                Some(()) = attempts.next(), if !attempts.is_empty() => {}
                call = run.requests.next(&run_id), if reading => match call {
                    Some(call) => attempts.push(self.attempt(call, &run.connection, &declared)),
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
    async fn attempt(
        &self,
        call: HandlerCall,
        connection: &ClientConnection,
        declared: &Arc<[String]>,
    ) {
        let (outcome, ending) = match self.handler_of(&call, connection, declared) {
            Ok((label, handler, attempt)) => {
                let held = Arc::clone(&attempt);
                let outcome = AssertUnwindSafe(async move { handler(attempt)?.await })
                    .catch_unwind()
                    .await;
                let outcome = outcome
                    .unwrap_or_else(|panic| Err(Box::new(panicked(&label, &*panic)) as BoxError));
                (outcome, held.ending())
            }
            Err(error) => (Err(Box::new(error) as BoxError), None),
        };
        let answered = match (ending, outcome) {
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
        };
        if let Err(error) = answered {
            tracing::debug!(
                %error,
                "A Crawl request's answer did not reach the engine: its connection has ended."
            );
        }
    }

    /// The Label of `call`, its handler and the attempt it is handed, which
    /// the serve starts inside its caught future, so a panic while the
    /// handler's future is built is caught too.
    fn handler_of(
        &self,
        call: &HandlerCall,
        connection: &ClientConnection,
        declared: &Arc<[String]>,
    ) -> Result<(Arc<str>, &Handler<'a>, Arc<Attempt>), Error> {
        let params = CrawlRequestHandleParams::deserialize(call.params()).map_err(|error| {
            Error::refused(
                CRAWL_REQUEST_HANDLE.name,
                &format!("its params do not read as this SDK's: {error}"),
                "Use an engine of this SDK's own version.",
            )
        })?;
        let label = params.crawl_request.label.as_str();
        let Some(Declared {
            name,
            handler: Some(handler),
            ..
        }) = self.labels.iter().find(|declared| *declared.name == *label)
        else {
            return Err(label_unknown(label, declared.to_vec()));
        };
        let attempt = Arc::new(Attempt {
            connection: connection.clone(),
            params,
            crawl: self.id,
            declared: Arc::clone(declared),
            ended: Mutex::new(None),
        });
        Ok((Arc::clone(name), handler, attempt))
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

    /// What `crawl.start` takes to start the crawl from `seeds`.
    fn start_params(&self, seeds: Vec<Seed>) -> Result<CrawlStartParams, Error> {
        if let Some(Declared { name, .. }) = self
            .labels
            .iter()
            .find(|declared| declared.handler.is_none())
        {
            return Err(Error::of_sdk(
                &LABEL_UNHANDLED,
                &format!("the crawl attaches no handler to the Label `{name}`"),
                "Attach one with `crawl.handle(label, handler)`, or stop declaring the Label.",
                &json!({ "label": &**name }),
            ));
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
                .map(|declared| {
                    let wire = WireLabel {
                        payload: Some(declared.payload.clone()),
                        ..WireLabel::default()
                    };
                    (declared.name.to_string(), wire)
                })
                .collect(),
            datasets: self.datasets.clone(),
            extraction: None,
            hooks: Vec::new(),
            settings: serde_json::Map::new(),
            project: None,
            limits: Vec::new(),
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

/// The `payload_invalid` of a delivered Payload that does not read as its
/// Label's type.
fn payload_invalid(error: serde_json::Error) -> Error {
    Error::of_program(Box::new(error), error_code::PAYLOAD_INVALID)
}

/// Gives back, once the serve of a Run ends or is dropped, the Crawl
/// requests the connection keeps aside for that Run: the engine delivers
/// them again to the Run's next serve.
struct Forget<'r> {
    requests: &'r CrawlRequests,
    run_id: &'r RunId,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.requests.forget(self.run_id);
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

/// The `retry_requested` of a handler's `ctx.retry(message)` (ADR 0093).
fn retry_requested(message: &str) -> Error {
    Error::of_sdk(
        &RETRY_REQUESTED,
        &format!("the handler asked to retry its attempt: {message}"),
        "The request is retried under `crawl.retry.max`: raise it for more attempts.",
        &json!({ "message": message }),
    )
}

/// The `attempt_ended` of a call on an attempt that already ended.
fn attempt_ended(attempt: &Attempt) -> Error {
    Error::of_sdk(
        &ATTEMPT_ENDED,
        &format!(
            "the attempt {} of the crawl request `{}` already ended",
            attempt.params.attempt, attempt.params.crawl_request.id
        ),
        "Make the call before the handler skips, retries or returns.",
        &json!({ "crawl_request_id": &attempt.params.crawl_request.id }),
    )
}

/// How a handler ended its attempt itself, which the serve answers
/// whatever the handler then returned.
#[derive(Clone, Debug)]
enum Ending {
    /// `ctx.skip(message)`: the request ends `skipped` with the reason
    /// `handler`.
    Skipped(String),
    /// `ctx.retry(message)`: the attempt ends with `retry_requested`.
    RetryRequested(String),
}

/// A Dataset a crawl declares (ADR 0103), whose Records are of type `I`:
/// what a handler pushes into. Cloning it is cheap.
pub struct Dataset<I> {
    name: Arc<str>,
    crawl: CrawlId,
    record: PhantomData<fn(I)>,
}

impl<I> Dataset<I> {
    /// The Dataset's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<I> Clone for Dataset<I> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            crawl: self.crawl,
            record: PhantomData,
        }
    }
}

impl<I> fmt::Debug for Dataset<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Dataset").field(&self.name).finish()
    }
}

/// A request a crawl starts from: a URL, the Label its handler is, and
/// its Payload.
#[derive(Debug)]
pub struct Seed {
    url: String,
    label: Arc<str>,
    crawl: CrawlId,
    /// The Payload's JSON, or why it has none.
    payload: Result<Value, Error>,
}

impl Seed {
    /// The request of `url`, handled by `label`'s handler with `payload`,
    /// of the type `label` declares.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a Payload is given by value, as the TypeScript and Python SDKs take it, and encoded at once"
    )]
    pub fn new<P: Serialize + DeserializeOwned>(
        url: impl Into<String>,
        label: &Label<P>,
        payload: P,
    ) -> Self {
        Self {
            url: url.into(),
            label: Arc::clone(&label.name),
            crawl: label.crawl,
            payload: label.encode(&payload),
        }
    }
}

impl Foxhound {
    /// The start of `crawl`'s Run from `seeds` (`crawl.start`), which
    /// awaiting sends; [`Crawl::serve`] then serves it. The crawl's
    /// declarations are checked first, before anything is sent.
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
    /// `payload_invalid` for a Payload that is not JSON, or that its
    /// Label's schema refuses, naming each issue, `invalid_params` with no
    /// seed, and what `crawl.start` fails with.
    pub fn start_crawl(
        &self,
        crawl: &Crawl<'_>,
        seeds: impl IntoIterator<Item = Seed>,
    ) -> StartCrawl {
        StartCrawl {
            connection: self.as_ref().clone(),
            requests: Arc::clone(self.crawl_requests()),
            params: crawl.start_params(seeds.into_iter().collect()),
        }
    }
}

/// The start of a crawl's Run (`crawl.start`), which awaiting sends.
/// Nothing is sent until it is awaited.
#[must_use = "a crawl's start is sent only when it is awaited"]
#[derive(Debug)]
pub struct StartCrawl {
    connection: ClientConnection,
    requests: Arc<CrawlRequests>,
    /// The start's params, or why the crawl cannot start.
    params: Result<CrawlStartParams, Error>,
}

impl StartCrawl {
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
    /// is sent, so the engine's first request finds it.
    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            requests,
            params,
        } = self;
        let started = params.and_then(|params| {
            requests.open(&connection)?;
            Ok(connection.start_with(CRAWL_START.name, params))
        });
        Box::pin(async move {
            let run = started?.await?;
            Ok(CrawlRun {
                run,
                connection,
                requests,
            })
        })
    }
}

/// A crawl's Run, which [`Crawl::serve`] serves: the [`Run`] it is, and
/// the connection's Crawl requests its serve reads. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct CrawlRun {
    run: Run,
    connection: ClientConnection,
    requests: Arc<CrawlRequests>,
}

impl CrawlRun {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        self.run.id()
    }

    /// The Run, with every member a Run has, such as `close`.
    pub fn run(&self) -> &Run {
        &self.run
    }
}

/// One attempt of a Crawl request, which its handler's context reads.
#[derive(Debug)]
struct Attempt {
    connection: ClientConnection,
    params: CrawlRequestHandleParams,
    crawl: CrawlId,
    /// The names of the Labels the crawl declares, in order, which the
    /// serve reads once for every attempt.
    declared: Arc<[String]>,
    /// How the handler ended the attempt itself, once it did.
    ended: Mutex<Option<Ending>>,
}

impl Attempt {
    /// How the handler ended the attempt itself, if it did.
    fn ending(&self) -> Option<Ending> {
        self.ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ends the attempt as `ending` and returns the error the handler
    /// returns for it: `retry_requested` for a retry, `attempt_ended` for
    /// a skip. Once the handler ended it, the attempt stays as it was and
    /// the call is `attempt_ended`.
    fn end(&self, ending: Ending) -> BoxError {
        let mut ended = self.ended.lock().unwrap_or_else(PoisonError::into_inner);
        let error = match (&*ended, &ending) {
            (None, Ending::RetryRequested(message)) => retry_requested(message),
            (None, Ending::Skipped(_)) | (Some(_), _) => attempt_ended(self),
        };
        ended.get_or_insert(ending);
        Box::new(error)
    }

    /// `attempt_ended` once the handler ended the attempt itself.
    fn ensure_running(&self) -> Result<(), Error> {
        match self.ending() {
            Some(_) => Err(attempt_ended(self)),
            None => Ok(()),
        }
    }
}

/// What a handler is given for one attempt of a request of its Label
/// (ADR 0093): the request, its Payload of type `P`, and the calls that
/// push Records and enqueue requests under the attempt's lease.
#[derive(Debug)]
pub struct CrawlContext<P> {
    attempt: Arc<Attempt>,
    payload: P,
}

impl<P> CrawlContext<P> {
    /// The request's Payload.
    pub fn payload(&self) -> &P {
        &self.payload
    }

    /// The request's URL.
    pub fn url(&self) -> &str {
        &self.attempt.params.crawl_request.url
    }

    /// The crawl request's ID within its Run.
    pub fn id(&self) -> &str {
        &self.attempt.params.crawl_request.id
    }

    /// The request's Label.
    pub fn label(&self) -> &str {
        &self.attempt.params.crawl_request.label
    }

    /// The attempt's number, 1 for the first.
    pub fn attempt(&self) -> u32 {
        self.attempt.params.attempt
    }

    /// The Run's ID.
    pub fn run_id(&self) -> &RunId {
        &self.attempt.params.run_id
    }

    /// Skips the request: it ends `skipped`, with the reason `handler` and
    /// `message`, and what the attempt pushed or enqueued is discarded. The
    /// handler returns what this returns, `return Err(ctx.skip(…))`; the
    /// attempt ends from the call itself, so one swallowed still ends it,
    /// and every later call on the context fails with `attempt_ended`.
    #[must_use = "a skip is returned from the handler: `return Err(ctx.skip(…))`"]
    pub fn skip(&self, message: &str) -> BoxError {
        self.attempt.end(Ending::Skipped(message.to_owned()))
    }

    /// Ends the attempt with `retry_requested`, retryable: the request is
    /// delivered again under `crawl.retry.max`, and ends `failed` with that
    /// error once the retries run out. What the attempt pushed or enqueued
    /// is discarded. As with [`CrawlContext::skip`], the handler returns
    /// what this returns, and the attempt ends from the call itself.
    #[must_use = "a retry is returned from the handler: `return Err(ctx.retry(…))`"]
    pub fn retry(&self, message: &str) -> BoxError {
        self.attempt.end(Ending::RetryRequested(message.to_owned()))
    }

    /// Pushes `record` into `dataset` (`crawl_request.push`): it is staged
    /// with the attempt and commits once the handler returns `Ok`.
    ///
    /// # Errors
    ///
    /// `record_invalid` for a Record that does not serialize or that its
    /// Dataset refuses, `dataset_unknown` for a Dataset the crawl does not
    /// declare, and `attempt_ended` once the attempt has ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the Record staged or not.
    pub fn push<I: Serialize>(
        &self,
        dataset: &Dataset<I>,
        record: &I,
    ) -> impl Future<Output = Result<(), Error>> + Send + 'static {
        let params = serde_json::to_value(record)
            .map_err(|error| Error::of_program(Box::new(error), error_code::RECORD_INVALID))
            .map(|record| CrawlRequestPushParams {
                run_id: self.attempt.params.run_id.clone(),
                crawl_request_id: self.attempt.params.crawl_request.id.clone(),
                lease_epoch: self.attempt.params.lease_epoch,
                dataset: dataset.name.to_string(),
                records: vec![record],
            });
        let attempt = Arc::clone(&self.attempt);
        async move {
            let params = params?;
            attempt.ensure_running()?;
            attempt
                .connection
                .call::<CrawlRequestPushed>(CRAWL_REQUEST_PUSH.name, &params)
                .await
                .map(drop)
        }
    }

    /// Enqueues the request of `url`, handled by `label`'s handler with
    /// `payload` (`crawl_request.enqueue`), and answers what the Frontier
    /// did with it: added, or not, with why.
    ///
    /// # Errors
    ///
    /// `label_unknown` for a Label of another crawl, `payload_invalid` for
    /// a Payload that is not JSON, that its Label's schema refuses, naming
    /// each issue, or that is longer than `crawl.payload.max_size`, and
    /// `attempt_ended` once the attempt has ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the request enqueued or not.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a Payload is given by value, as the TypeScript and Python SDKs take it, and encoded at once"
    )]
    pub fn enqueue<Q: Serialize + DeserializeOwned>(
        &self,
        url: impl Into<String>,
        label: &Label<Q>,
        payload: Q,
    ) -> impl Future<Output = Result<Enqueued, Error>> + Send + 'static {
        let params = if label.crawl == self.attempt.crawl {
            label
                .encode(&payload)
                .map(|payload| CrawlRequestEnqueueParams {
                    run_id: self.attempt.params.run_id.clone(),
                    crawl_request_id: self.attempt.params.crawl_request.id.clone(),
                    lease_epoch: self.attempt.params.lease_epoch,
                    url: url.into(),
                    label: label.name.to_string(),
                    payload: Some(payload),
                    priority: 0,
                    forefront: false,
                    method: HttpMethod::default(),
                    body: None,
                    key: None,
                    document: None,
                })
        } else {
            Err(label_unknown(&label.name, self.attempt.declared.to_vec()))
        };
        let attempt = Arc::clone(&self.attempt);
        async move {
            let params = params?;
            attempt.ensure_running()?;
            attempt
                .connection
                .call::<Enqueued>(CRAWL_REQUEST_ENQUEUE.name, &params)
                .await
        }
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
    send_sync_static::<Label<()>>();
    send_sync_static::<Dataset<()>>();
    send_static::<Seed>();
    send_static::<StartCrawl>();
    send_static::<<StartCrawl as IntoFuture>::IntoFuture>();
};
