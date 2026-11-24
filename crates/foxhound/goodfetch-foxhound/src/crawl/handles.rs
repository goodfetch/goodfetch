//! The handles a crawl declares and starts from (ADRs 0093 and 0103): its
//! Labels, its Datasets and the seeds of its Run, each carrying the number
//! of the crawl it belongs to, so a crawl tells its own from another's.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use goodfetch_collie_client::{Error, Schema, SchemaError, to_json};
use goodfetch_protocol::foxhound::crawl::{RequestSeed, SitemapRule, SitemapSeed};
use goodfetch_protocol::foxhound::dataset::{
    InvalidRecord, RECORD_INVALID, RecordData, RecordInvalidDetails, RecordIssue,
};
use goodfetch_protocol::foxhound::frontier::{
    HttpBody, HttpMethod, PAYLOAD_INVALID, PayloadInvalidDetails, PayloadIssue,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::declaration::RecordEncoder;
use super::{CrawlId, payload_invalid};

/// A Label a crawl declares (ADR 0093), whose requests carry a Payload of
/// type `P`: what a seed, an enqueue and a handler name. Cloning it is
/// cheap.
pub struct Label<P> {
    pub(super) name: Arc<str>,
    pub(super) crawl: CrawlId,
    /// The schema its Payloads are sent by, when its declaration gives
    /// one.
    pub(super) payload: Option<Arc<Schema<P>>>,
}

impl<P> Label<P> {
    /// The Label's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<P: Serialize + DeserializeOwned> Label<P> {
    /// `payload` as the JSON a request of the Label carries, or the
    /// `payload_invalid` that refuses it before anything is sent: one
    /// that does not serialize, and, by the Label's schema, one that is
    /// not JSON or does not read back as given.
    pub(super) fn encode(&self, payload: &P) -> Result<Value, Error> {
        match &self.payload {
            Some(schema) => schema
                .encode(payload)
                .map_err(|error| payload_refused(&self.name, &error)),
            None => serde_json::to_value(payload).map_err(payload_invalid),
        }
    }
}

impl<P> Clone for Label<P> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            crawl: self.crawl,
            payload: self.payload.clone(),
        }
    }
}

impl<P> fmt::Debug for Label<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Label").field(&self.name).finish()
    }
}

/// A Dataset a crawl declares (ADR 0103), whose Records are of type `I`:
/// what a handler pushes into. Cloning it is cheap.
pub struct Dataset<I> {
    pub(super) name: Arc<str>,
    pub(super) crawl: CrawlId,
    /// How a push writes its Records, when its declaration gives a schema.
    pub(super) encoder: Option<RecordEncoder<I>>,
    pub(super) record: PhantomData<fn(I)>,
}

impl<I> Dataset<I> {
    /// The Dataset's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<I: Serialize> Dataset<I> {
    /// `record` as the JSON a push sends, or the `record_invalid` that
    /// refuses it before anything is sent: one that is not JSON, and, by
    /// the Dataset's schema, one that does not read back as given.
    pub(super) fn encode(&self, record: &I) -> Result<RecordData, Error> {
        let json = match &self.encoder {
            Some(encode) => encode(record),
            None => to_json(record),
        }
        .map_err(|error| record_refused(&self.name, &error))?;
        Ok(RecordData::of(&json).expect("invariant: a JSON value writes as JSON"))
    }
}

impl<I> Clone for Dataset<I> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            crawl: self.crawl,
            encoder: self.encoder.clone(),
            record: PhantomData,
        }
    }
}

impl<I> fmt::Debug for Dataset<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Dataset").field(&self.name).finish()
    }
}

/// What a crawl starts from (ADRs 0093 and 0120): a request, of a URL,
/// the Label its handler is and its Payload, sent with a method and a
/// body and identified by a key of the program's own when it gives them;
/// or a sitemap, whose entries are each seeded with the Label of the
/// first rule whose pattern matches its URL.
#[derive(Debug)]
pub struct Seed {
    pub(super) seeded: Seeded,
}

/// What a [`Seed`] starts from.
#[derive(Debug)]
pub(super) enum Seeded {
    /// A request, with the crawl its Label is of, and its Payload's JSON
    /// or why it has none.
    Request {
        request: RequestSeed,
        crawl: CrawlId,
        payload: Result<Value, Error>,
    },
    /// A sitemap, with the crawl each rule's Label is of, and the request
    /// option given to it, which a sitemap seed does not take.
    Sitemap {
        sitemap: SitemapSeed,
        crawls: Vec<CrawlId>,
        refused: Option<&'static str>,
    },
}

impl Seed {
    /// The request of `url`, handled by `label`'s handler with `payload`,
    /// of the type `label` declares: a `GET` with no body, of priority 0,
    /// identified by its method, its URL's key and its body's hash.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a Payload is given by value, as the TypeScript and Python SDKs take it, and encoded at once"
    )]
    pub fn new<P: Serialize + DeserializeOwned>(
        url: impl Into<String>,
        label: &Label<P>,
        payload: P,
    ) -> Self {
        let payload = label.encode(&payload);
        Self {
            seeded: Seeded::Request {
                request: RequestSeed {
                    url: url.into(),
                    label: label.name.to_string(),
                    payload: None,
                    priority: 0,
                    forefront: false,
                    method: HttpMethod::default(),
                    body: None,
                    key: None,
                },
                crawl: label.crawl,
                payload,
            },
        }
    }

    /// The sitemap at `url`, or the sitemaps the site's robots.txt at
    /// `url` names, each of whose entries is seeded with the Label of the
    /// first of `rules` whose pattern, a glob as `crawl.scope.include`
    /// takes them, matches the entry's URL, and no Payload; an entry no
    /// rule matches is not added, with `no_rule`.
    pub fn sitemap<'a, S: Into<String>>(
        url: impl Into<String>,
        rules: impl IntoIterator<Item = (S, &'a Label<()>)>,
    ) -> Self {
        let (rules, crawls) = rules
            .into_iter()
            .map(|(pattern, label)| {
                (
                    SitemapRule {
                        pattern: pattern.into(),
                        label: label.name.to_string(),
                    },
                    label.crawl,
                )
            })
            .unzip();
        Self {
            seeded: Seeded::Sitemap {
                sitemap: SitemapSeed {
                    sitemap: url.into(),
                    rules,
                },
                crawls,
                refused: None,
            },
        }
    }

    /// The seed, sent with `method`.
    #[must_use]
    pub fn method(self, method: HttpMethod) -> Self {
        self.with_request("method", |request| request.method = method)
    }

    /// The seed, sent with `body`.
    #[must_use]
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let body = HttpBody::new(body.into());
        self.with_request("body", |request| request.body = Some(body))
    }

    /// The seed, of `priority` among the requests of its domain: higher
    /// goes first, and a priority never orders requests of different
    /// domains.
    #[must_use]
    pub fn priority(self, priority: i64) -> Self {
        self.with_request("priority", |request| request.priority = priority)
    }

    /// The seed, ahead of every request its domain holds pending, whatever
    /// their priority: of two such requests, the later goes first.
    #[must_use]
    pub fn forefront(self) -> Self {
        self.with_request("forefront", |request| request.forefront = true)
    }

    /// The seed, identified in the Frontier by `key` in place of its
    /// method, its URL's key and its body's hash: two requests of one key
    /// are one Crawl request, whatever their URLs.
    #[must_use]
    pub fn key(self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.with_request("key", |request| request.key = Some(key))
    }

    /// The seed with `set` applied to its request, or, for a sitemap seed,
    /// which takes no request option, with `option` kept to refuse it at
    /// the crawl's start.
    fn with_request(mut self, option: &'static str, set: impl FnOnce(&mut RequestSeed)) -> Self {
        match &mut self.seeded {
            Seeded::Request { request, .. } => set(request),
            Seeded::Sitemap { refused, .. } => {
                refused.get_or_insert(option);
            }
        }
        self
    }
}

/// The `payload_invalid` of a Payload for `label` its schema refused
/// before anything was sent: it is not JSON, or does not read back as
/// given; the details name the issue's JSON Pointer, as the engine's own.
fn payload_refused(label: &str, error: &SchemaError) -> Error {
    let path = match error {
        SchemaError::NotJson { path, .. } | SchemaError::ChangesOnReadBack { path, .. } => {
            path.clone()
        }
        _ => String::new(),
    };
    Error::of_sdk(
        &PAYLOAD_INVALID,
        &format!("the Payload for the Label `{label}` was not sent: {error}"),
        "Give a Payload that is JSON and that its type's `Deserialize` reads back as it was given.",
        &PayloadInvalidDetails {
            label: label.to_owned(),
            issues: vec![PayloadIssue {
                path,
                message: error.to_string(),
            }],
            limit: None,
            size: None,
            max_size: None,
        },
    )
}

/// The `record_invalid` of the one Record a push into `dataset` gives,
/// refused before anything was sent: it is not JSON, or its schema does
/// not read it back as given; the details name it at position 0 and the
/// issue by its JSON Pointer, as the engine's own.
fn record_refused(dataset: &str, error: &SchemaError) -> Error {
    let path = match error {
        SchemaError::NotJson { path, .. } | SchemaError::ChangesOnReadBack { path, .. } => {
            path.clone()
        }
        _ => String::new(),
    };
    Error::of_sdk(
        &RECORD_INVALID,
        &format!("the Dataset `{dataset}` did not stage the Record: {error}"),
        "Push a Record that is JSON and that its type's `Deserialize` reads back as it was given.",
        &RecordInvalidDetails {
            dataset: dataset.to_owned(),
            records: vec![InvalidRecord {
                position: 0,
                issues: vec![RecordIssue {
                    path,
                    message: error.to_string(),
                }],
            }],
        },
    )
}
