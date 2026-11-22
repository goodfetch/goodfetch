//! The handles a crawl declares and starts from (ADRs 0093 and 0103): its
//! Labels, its Datasets and the seeds of its Run, each carrying the number
//! of the crawl it belongs to, so a crawl tells its own from another's.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use goodfetch_collie_client::{Error, Schema, SchemaError};
use goodfetch_protocol::foxhound::frontier::{
    PAYLOAD_INVALID, PayloadInvalidDetails, PayloadIssue,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

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
    pub(super) record: PhantomData<fn(I)>,
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
    pub(super) url: String,
    pub(super) label: Arc<str>,
    pub(super) crawl: CrawlId,
    /// The Payload's JSON, or why it has none.
    pub(super) payload: Result<Value, Error>,
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
