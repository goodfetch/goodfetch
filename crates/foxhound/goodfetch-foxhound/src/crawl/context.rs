//! What a handler is given for one attempt of a Crawl request (ADRs 0093
//! and 0109): the attempt, how the handler ended it itself, and the
//! context whose calls push Records and enqueue requests under its lease.

use std::future::Future;
use std::sync::{Arc, PoisonError};

use goodfetch_collie_client::{BoxError, ClientConnection, Error, Locator, Page};
use goodfetch_protocol::foxhound::crawl::{
    ATTEMPT_ENDED, CrawlRequestHandleParams, RETRY_REQUESTED,
};
use goodfetch_protocol::foxhound::dataset::{
    CRAWL_REQUEST_PUSH, CrawlRequestPushParams, CrawlRequestPushed,
};
use goodfetch_protocol::foxhound::error_code;
use goodfetch_protocol::foxhound::fetch::{RESPONSE_NOT_DOCUMENT, ResponseNotDocumentDetails};
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CRAWL_REQUEST_ENQUEUE_LINKS, CrawlRequestEnqueueLinksParams,
    CrawlRequestEnqueueParams, Enqueued, EnqueuedLinks, HttpMethod,
};
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{CrawlId, Dataset, Label, label_unknown};
use crate::sync::Mutex;

/// The `retry_requested` of a handler's `ctx.retry(message)` (ADR 0093).
pub(super) fn retry_requested(message: &str) -> Error {
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

/// The `response_not_document` of `ctx.page()` on an attempt the engine
/// served no page for.
fn response_not_document(attempt: &Attempt) -> Error {
    let content_type = attempt
        .params
        .response
        .as_ref()
        .and_then(|response| response.headers.get("content-type"))
        .cloned()
        .unwrap_or_default();
    Error::of_sdk(
        &RESPONSE_NOT_DOCUMENT,
        &format!(
            "the crawl request `{}`'s response is not a document, so its handler has no page",
            attempt.params.crawl_request.id
        ),
        "Read the response's body from the request's response instead of its page.",
        &ResponseNotDocumentDetails {
            content_type,
            call: "ctx.page".to_owned(),
            fix: "ctx.response".to_owned(),
        },
    )
}

/// How a handler ended its attempt itself, which the serve answers
/// whatever the handler then returned.
#[derive(Clone, Debug)]
pub(super) enum Ending {
    /// `ctx.skip(message)`: the request ends `skipped` with the reason
    /// `handler`.
    Skipped(String),
    /// `ctx.retry(message)`: the attempt ends with `retry_requested`.
    RetryRequested(String),
}

/// One attempt of a Crawl request, which its handler's context reads.
#[derive(Debug)]
pub(super) struct Attempt {
    connection: ClientConnection,
    /// The page `params` names, served from the request's document.
    page: Option<Page>,
    pub(super) params: CrawlRequestHandleParams,
    crawl: CrawlId,
    /// The names of the Labels the crawl declares, in order, which the
    /// serve reads once for every attempt.
    declared: Arc<[String]>,
    /// How the handler ended the attempt itself, once it did.
    ended: Mutex<Option<Ending>>,
}

impl Attempt {
    /// The attempt `params` names, of the crawl `crawl` whose Labels are
    /// `declared`, on `connection`, whose handler reads `page`.
    pub(super) fn new(
        connection: ClientConnection,
        page: Option<Page>,
        params: CrawlRequestHandleParams,
        crawl: CrawlId,
        declared: Arc<[String]>,
    ) -> Self {
        Self {
            connection,
            page,
            params,
            crawl,
            declared,
            ended: Mutex::new(None),
        }
    }

    /// How the handler ended the attempt itself, if it did.
    pub(super) fn ending(&self) -> Option<Ending> {
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
    pub(super) attempt: Arc<Attempt>,
    payload: P,
}

impl<P> CrawlContext<P> {
    /// The context of `attempt`, whose Payload reads as `payload`.
    pub(super) fn new(attempt: Arc<Attempt>, payload: P) -> Self {
        Self { attempt, payload }
    }

    /// The connection the attempt came on, which a blocking handler's
    /// calls wait on.
    pub(crate) fn connection(&self) -> &ClientConnection {
        &self.attempt.connection
    }

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

    /// The attempt's page, Collie's [`Page`] served from the request's
    /// document (ADRs 0085 and 0093), whose calls run under the attempt's
    /// lease and count in the crawl's own Run; the handler reads it, or
    /// finds links on it for [`CrawlContext::enqueue_links`], as any page.
    ///
    /// # Errors
    ///
    /// `response_not_document` when the request's response is no document
    /// the engine serves as a page.
    pub fn page(&self) -> Result<&Page, Error> {
        self.attempt
            .page
            .as_ref()
            .ok_or_else(|| response_not_document(&self.attempt))
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
        let pushed = self.send::<CrawlRequestPushed, _>(CRAWL_REQUEST_PUSH.name, params);
        async move { pushed.await.map(drop) }
    }

    /// Enqueues the request of `url`, handled by `label`'s handler with
    /// `payload` (`crawl_request.enqueue`), and answers what the Frontier
    /// did with it: added, or not, with why.
    ///
    /// # Errors
    ///
    /// `label_unknown` for a Label of another crawl, `payload_invalid` for
    /// a Payload that does not serialize, that its Label's schema refuses,
    /// naming each issue, or that is longer than `crawl.payload.max_size`,
    /// and `attempt_ended` once the attempt has ended.
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
        let params = self
            .own_payload(label, &payload)
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
            });
        self.send(CRAWL_REQUEST_ENQUEUE.name, params)
    }

    /// Enqueues the link of each element `links` matches on the attempt's
    /// page, each `href` resolved against the document's base URL, all
    /// handled by `label`'s handler with `payload`
    /// (`crawl_request.enqueue_links`), and answers what the Frontier did
    /// with each: added, not added with why, or no link.
    ///
    /// # Errors
    ///
    /// `label_unknown` for a Label of another crawl, `payload_invalid` for
    /// a Payload that does not serialize, that its Label's schema refuses,
    /// naming each issue, or that is longer than `crawl.payload.max_size`,
    /// and `attempt_ended` once the attempt has ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave some of the links enqueued or none.
    pub fn enqueue_links<Q: Serialize + DeserializeOwned>(
        &self,
        links: &Locator,
        label: &Label<Q>,
        payload: Q,
    ) -> impl Future<Output = Result<EnqueuedLinks, Error>> + Send + 'static {
        self.enqueue_links_of(links.id(), label, payload)
    }

    /// [`CrawlContext::enqueue_links`] of the locator `links` names, which
    /// a blocking handler's locator names too.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a Payload is given by value, as the TypeScript and Python SDKs take it, and encoded at once"
    )]
    pub(crate) fn enqueue_links_of<Q: Serialize + DeserializeOwned>(
        &self,
        links: &goodfetch_protocol::page::Locator,
        label: &Label<Q>,
        payload: Q,
    ) -> impl Future<Output = Result<EnqueuedLinks, Error>> + Send + 'static {
        let params =
            self.own_payload(label, &payload)
                .map(|payload| CrawlRequestEnqueueLinksParams {
                    run_id: self.attempt.params.run_id.clone(),
                    crawl_request_id: self.attempt.params.crawl_request.id.clone(),
                    lease_epoch: self.attempt.params.lease_epoch,
                    locator: links.clone(),
                    label: label.name.to_string(),
                    payload: Some(payload),
                    payloads: None,
                    include: Vec::new(),
                    exclude: Vec::new(),
                    limit: None,
                    priority: 0,
                });
        self.send(CRAWL_REQUEST_ENQUEUE_LINKS.name, params)
    }

    /// `payload`'s JSON for a request of `label`, or `label_unknown` for a
    /// Label of another crawl, or the `payload_invalid` of a Payload
    /// `label` refuses before anything is sent.
    fn own_payload<Q: Serialize + DeserializeOwned>(
        &self,
        label: &Label<Q>,
        payload: &Q,
    ) -> Result<Value, Error> {
        if label.crawl == self.attempt.crawl {
            label.encode(payload)
        } else {
            Err(label_unknown(&label.name, self.attempt.declared.to_vec()))
        }
    }

    /// The call of `method` with `params`, sent while the attempt runs,
    /// whose answer reads as `T`: what each of the attempt's own calls
    /// sends. It fails with what made `params`, or `attempt_ended` once
    /// the attempt has ended.
    fn send<T, C>(
        &self,
        method: &'static str,
        params: Result<C, Error>,
    ) -> impl Future<Output = Result<T, Error>> + Send + 'static
    where
        T: DeserializeOwned + Send + 'static,
        C: Serialize + Send + 'static,
    {
        let attempt = Arc::clone(&self.attempt);
        async move {
            let params = params?;
            attempt.ensure_running()?;
            attempt.connection.call::<T>(method, &params).await
        }
    }
}
