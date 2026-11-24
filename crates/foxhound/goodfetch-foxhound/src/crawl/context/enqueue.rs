//! A handler's enqueues (ADRs 0093, 0099 and 0120): the builders
//! `ctx.enqueue` and `ctx.enqueue_links` return, whose options are builder
//! methods named after each and which awaiting sends, and the Payload an
//! `enqueue_links` gives its links, one for every link or a function of
//! each, ADR 0099's one name for both forms.

use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_collie_client::Error;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CRAWL_REQUEST_ENQUEUE_LINKS, CRAWL_REQUEST_LINKS,
    CrawlRequestEnqueueLinksParams, CrawlRequestEnqueueParams, CrawlRequestLinks,
    CrawlRequestLinksParams, Enqueued, EnqueuedLinks, HttpBody, HttpMethod, MatchedLink,
    PAYLOAD_INVALID, PayloadInvalidDetails, PayloadIssue,
};
use goodfetch_protocol::page::Locator;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::Attempt;
use crate::crawl::{Label, label_unknown};

/// The Payload [`CrawlContext::enqueue_links`](crate::CrawlContext::enqueue_links)
/// gives the links it adds: one Payload for every link, which any Payload
/// converts into, or [`LinkPayload::with`] a function of each.
pub struct LinkPayload<Q>(Given<Q>);

/// A function that answers each link's Payload, or `None` to leave the
/// link out.
type AnswerOf<Q> = Box<dyn FnMut(&MatchedLink) -> Option<Q> + Send>;

/// How a [`LinkPayload`] was given.
enum Given<Q> {
    /// One Payload for every link.
    Every(Q),
    /// A function of each link.
    Each(AnswerOf<Q>),
}

impl<Q> LinkPayload<Q> {
    /// The Payload `payload_of` answers for each element the locator
    /// matched, in document order, as `crawl_request.links` reads it, or
    /// `None` to leave its link out, which the answer reports as
    /// `skipped_by_payload`. The function runs on the handler's task, once
    /// for each element, before any link is sent.
    pub fn with(payload_of: impl FnMut(&MatchedLink) -> Option<Q> + Send + 'static) -> Self {
        Self(Given::Each(Box::new(payload_of)))
    }
}

impl<Q> From<Q> for LinkPayload<Q> {
    /// `payload`, for every link.
    fn from(payload: Q) -> Self {
        Self(Given::Every(payload))
    }
}

impl<Q: fmt::Debug> fmt::Debug for LinkPayload<Q> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Given::Every(payload) => formatter.debug_tuple("LinkPayload").field(payload).finish(),
            Given::Each(_) => formatter.write_str("LinkPayload::with(..)"),
        }
    }
}

/// `label`'s name when it is one of `attempt`'s crawl, or its
/// `label_unknown`.
fn own_label<Q>(attempt: &Attempt, label: &Label<Q>) -> Result<String, Error> {
    if label.crawl == attempt.crawl {
        Ok(label.name.to_string())
    } else {
        Err(label_unknown(&label.name, attempt.declared.to_vec()))
    }
}

/// The enqueue of one request from inside a handler's attempt
/// (`crawl_request.enqueue`), which awaiting sends. Each request option is
/// a method named after it; nothing is sent until it is awaited.
#[must_use = "an enqueue is sent only when it is awaited"]
#[derive(Debug)]
pub struct Enqueue {
    attempt: Arc<Attempt>,
    /// The enqueue's params, or why it cannot be sent.
    params: Result<CrawlRequestEnqueueParams, Error>,
}

impl Enqueue {
    /// The enqueue, on `attempt`, of the request of `url` under `label`
    /// with `payload`, of the default method, priority and identity.
    pub(super) fn new<Q: Serialize + DeserializeOwned>(
        attempt: &Arc<Attempt>,
        url: String,
        label: &Label<Q>,
        payload: &Q,
    ) -> Self {
        let params = own_label(attempt, label).and_then(|name| {
            Ok(CrawlRequestEnqueueParams {
                run_id: attempt.params.run_id.clone(),
                crawl_request_id: attempt.params.crawl_request.id.clone(),
                lease_epoch: attempt.params.lease_epoch,
                url,
                label: name,
                payload: Some(label.encode(payload)?),
                priority: 0,
                forefront: false,
                method: HttpMethod::default(),
                body: None,
                key: None,
                document: None,
            })
        });
        Self {
            attempt: Arc::clone(attempt),
            params,
        }
    }

    /// The request, sent with `method`; `GET` by default. Any method is a
    /// read (ADR 0079).
    pub fn method(self, method: HttpMethod) -> Self {
        self.with(|params| params.method = method)
    }

    /// The request, sent with `body`.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let body = HttpBody::new(body.into());
        self.with(|params| params.body = Some(body))
    }

    /// The request, of `priority` among the requests of its domain: higher
    /// goes first, and a priority never orders requests of different
    /// domains.
    pub fn priority(self, priority: i64) -> Self {
        self.with(|params| params.priority = priority)
    }

    /// The request, ahead of every request its domain holds pending,
    /// whatever their priority: of two such requests, the later goes
    /// first.
    pub fn forefront(self) -> Self {
        self.with(|params| params.forefront = true)
    }

    /// The request, identified in the Frontier by `key` in place of its
    /// method, its URL's key and its body's hash: two requests of one key
    /// are one Crawl request, whatever their URLs.
    pub fn key(self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.with(|params| params.key = Some(key))
    }

    /// The enqueue with `set` applied to its params.
    fn with(mut self, set: impl FnOnce(&mut CrawlRequestEnqueueParams)) -> Self {
        if let Ok(params) = &mut self.params {
            set(params);
        }
        self
    }
}

impl IntoFuture for Enqueue {
    type Output = Result<Enqueued, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Enqueued, Error>> + Send + 'static>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.attempt.call(CRAWL_REQUEST_ENQUEUE.name, self.params))
    }
}

/// A Payload function encoded for its Label: the JSON of each link's
/// answer, given its position, `null` for one left out.
type PayloadOf = Box<dyn FnMut(usize, &MatchedLink) -> Result<Value, Error> + Send>;

/// The enqueue of every link a locator matches from inside a handler's
/// attempt (`crawl_request.enqueue_links`), which awaiting sends. Each
/// option is a method named after it; nothing is sent until it is awaited.
#[must_use = "an enqueue of links is sent only when it is awaited"]
pub struct EnqueueLinks {
    attempt: Arc<Attempt>,
    /// The enqueue's params, or why it cannot be sent.
    params: Result<CrawlRequestEnqueueLinksParams, Error>,
    /// The Payload function each link's Payload is the answer of, when the
    /// enqueue was given one, which `crawl_request.links` feeds first.
    payload_of: Option<PayloadOf>,
}

impl EnqueueLinks {
    /// The enqueue, on `attempt`, of the link of each element `locator`
    /// matches under `label` with `payload`, with no pattern, limit or
    /// priority.
    pub(super) fn new<Q: Serialize + DeserializeOwned + 'static>(
        attempt: &Arc<Attempt>,
        locator: &Locator,
        label: &Label<Q>,
        payload: LinkPayload<Q>,
    ) -> Self {
        let mut payload_of = None;
        let params = own_label(attempt, label).and_then(|name| {
            let payload = match payload.0 {
                Given::Every(payload) => Some(label.encode(&payload)?),
                Given::Each(mut each) => {
                    let label = label.clone();
                    let encoded: PayloadOf = Box::new(move |at, link| match each(link) {
                        Some(payload) => kept_payload(&label, at, &payload),
                        None => Ok(Value::Null),
                    });
                    payload_of = Some(encoded);
                    None
                }
            };
            Ok(CrawlRequestEnqueueLinksParams {
                run_id: attempt.params.run_id.clone(),
                crawl_request_id: attempt.params.crawl_request.id.clone(),
                lease_epoch: attempt.params.lease_epoch,
                locator: locator.clone(),
                label: name,
                payload,
                payloads: None,
                include: Vec::new(),
                exclude: Vec::new(),
                limit: None,
                priority: 0,
            })
        });
        Self {
            attempt: Arc::clone(attempt),
            params,
            payload_of,
        }
    }

    /// The links, each matching one of `patterns`, globs as
    /// `crawl.scope.include`'s, which only narrow the crawl's scope.
    pub fn include<I>(self, patterns: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let patterns = patterns.into_iter().map(Into::into).collect();
        self.with(|params| params.include = patterns)
    }

    /// The links, each matching none of `patterns`, besides the crawl's
    /// scope.
    pub fn exclude<I>(self, patterns: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let patterns = patterns.into_iter().map(Into::into).collect();
        self.with(|params| params.exclude = patterns)
    }

    /// At most `limit` links added: once they are, each later element
    /// whose link the Frontier would judge is not added, with
    /// `link_limit`.
    pub fn limit(self, limit: u64) -> Self {
        self.with(|params| params.limit = Some(limit))
    }

    /// The links, of `priority` among the requests of their domain, as
    /// [`Enqueue::priority`].
    pub fn priority(self, priority: i64) -> Self {
        self.with(|params| params.priority = priority)
    }

    /// The enqueue with `set` applied to its params.
    fn with(mut self, set: impl FnOnce(&mut CrawlRequestEnqueueLinksParams)) -> Self {
        if let Ok(params) = &mut self.params {
            set(params);
        }
        self
    }
}

impl fmt::Debug for EnqueueLinks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnqueueLinks")
            .field("params", &self.params)
            .field("payload_of", &self.payload_of.as_ref().map(|_| ".."))
            .finish_non_exhaustive()
    }
}

impl IntoFuture for EnqueueLinks {
    type Output = Result<EnqueuedLinks, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<EnqueuedLinks, Error>> + Send + 'static>>;

    /// With a Payload function, reads each element the locator matches
    /// (`crawl_request.links`) and sends the function's answer for each as
    /// `payloads`, in that order; a refused answer fails the call before
    /// any link is sent.
    fn into_future(self) -> Self::IntoFuture {
        let Self {
            attempt,
            params,
            payload_of,
        } = self;
        Box::pin(async move {
            let mut params = params?;
            if let Some(mut payload_of) = payload_of {
                let read = CrawlRequestLinksParams {
                    run_id: params.run_id.clone(),
                    crawl_request_id: params.crawl_request_id.clone(),
                    lease_epoch: params.lease_epoch,
                    locator: params.locator.clone(),
                };
                let read: CrawlRequestLinks = Arc::clone(&attempt)
                    .call(CRAWL_REQUEST_LINKS.name, Ok(read))
                    .await?;
                let payloads = read
                    .links
                    .iter()
                    .enumerate()
                    .map(|(at, link)| payload_of(at, link))
                    .collect::<Result<_, _>>()?;
                params.payloads = Some(payloads);
            }
            attempt
                .call(CRAWL_REQUEST_ENQUEUE_LINKS.name, Ok(params))
                .await
        })
    }
}

/// The JSON of `payload`, a Payload function's answer for the link at
/// `at` that keeps it, or its `payload_invalid`: one `label` refuses, or
/// one that is `null`, which would leave the link out, as a Label of no
/// Payload's `()` is.
fn kept_payload<Q: Serialize + DeserializeOwned>(
    label: &Label<Q>,
    at: usize,
    payload: &Q,
) -> Result<Value, Error> {
    let encoded = label.encode(payload)?;
    if !encoded.is_null() {
        return Ok(encoded);
    }
    Err(Error::of_sdk(
        &PAYLOAD_INVALID,
        &format!(
            "the Payload function's answer for the link at {at} is `null`, which no link of the Label `{}` takes",
            label.name
        ),
        "Give a Label without a Payload one Payload for every link, `()`, and answer `None` to leave a link out.",
        &PayloadInvalidDetails {
            label: label.name.to_string(),
            issues: vec![PayloadIssue {
                path: String::new(),
                message: format!("the answer for the link at {at} is `null`"),
            }],
            limit: None,
            size: None,
            max_size: None,
        },
    ))
}
