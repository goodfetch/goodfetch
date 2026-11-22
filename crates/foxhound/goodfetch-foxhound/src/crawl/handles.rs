//! The handles a crawl declares and starts from (ADRs 0093 and 0103): its
//! Labels, its Datasets and the seeds of its Run, each carrying the number
//! of the crawl it belongs to, so a crawl tells its own from another's.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use goodfetch_collie_client::Error;
use serde::Serialize;
use serde_json::Value;

use super::{CrawlId, payload_invalid};

/// A Label a crawl declares (ADR 0093), whose requests carry a Payload of
/// type `P`: what a seed, an enqueue and a handler name. Cloning it is
/// cheap.
pub struct Label<P> {
    pub(super) name: Arc<str>,
    pub(super) crawl: CrawlId,
    pub(super) payload: PhantomData<fn() -> P>,
}

impl<P> Label<P> {
    /// The Label's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<P> Clone for Label<P> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            crawl: self.crawl,
            payload: PhantomData,
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
    /// The request of `url`, handled by `label`'s handler with `payload`.
    pub fn new<P: Serialize>(url: impl Into<String>, label: &Label<P>, payload: P) -> Self {
        Self {
            url: url.into(),
            label: Arc::clone(&label.name),
            crawl: label.crawl,
            payload: serde_json::to_value(payload).map_err(payload_invalid),
        }
    }
}
