//! `ResponseSource` (ADR 0046): the one value the engine's interception and
//! the fetch transports consult for a request's response, in a fixed
//! order: a fulfilling route (ADR 0085), then the Run's Response recording
//! (ADR 0105), then the Response cache when `cache.mode` is not `off`,
//! then the network. A Run's source is built once from its settings; one
//! built with the cache off and no recording only sends, and never opens
//! the cache.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use goodfetch_protocol::response_cache::CacheMode;
use goodfetch_protocol::run::RunId;

use super::{CacheRequest, Hit, Lookup, Partition, ResponseCache, ResponseHead, Stale, freshness};

/// A response served whole without the network: a fulfilling route's or
/// the Run's Response recording's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Served {
    /// Its head.
    pub head: ResponseHead,
    /// Its body, as it would arrive: no content coding decoded.
    pub body: Bytes,
}

/// A boxed future a [`Recording`] answers, so it stays object-safe.
pub type Replaying<'a> = Pin<Box<dyn Future<Output = Option<Served>> + Send + 'a>>;

/// A Run's Response recording as its source reads it (ADR 0105): the
/// response it recorded for a request, if any.
pub trait Recording: Send + Sync + std::fmt::Debug {
    /// What it recorded for `request`.
    fn recorded<'a>(&'a self, request: &'a CacheRequest) -> Replaying<'a>;
}

/// Where a request's response comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A route fulfilled it.
    Routed(Served),
    /// The Run's Response recording holds it.
    Recorded(Served),
    /// The cache holds it fresh: served with no request.
    Fresh(Hit),
    /// The cache holds it stale: the request is sent with its conditions,
    /// and a 304 answering it is handed to [`ResponseSource::revalidated`].
    Stale(Stale),
    /// Nothing holds it: the request is sent as it is.
    Network,
}

/// The sources one Run's requests are answered from, which its clones
/// share. The default only sends.
#[derive(Clone, Debug, Default)]
pub struct ResponseSource {
    /// The cache, unless `cache.mode` is `off`.
    cache: Option<ResponseCache>,
    recording: Option<Arc<dyn Recording>>,
}

impl ResponseSource {
    /// The source of a Run whose `cache.mode` is `mode`, `cache` the
    /// user's: with `off` it never reads nor writes the cache.
    pub fn of(cache: &ResponseCache, mode: CacheMode) -> Self {
        Self {
            cache: match mode {
                CacheMode::Off => None,
                CacheMode::Revalidate => Some(cache.clone()),
            },
            recording: None,
        }
    }

    /// The source answering from the Run's Response `recording` ahead of
    /// the cache.
    #[must_use]
    pub fn recording(self, recording: Arc<dyn Recording>) -> Self {
        Self {
            recording: Some(recording),
            ..self
        }
    }

    /// Whether it only sends: no recording, and the cache off.
    pub fn sends_only(&self) -> bool {
        self.cache.is_none() && self.recording.is_none()
    }

    /// Where the response to `request` in `partition` comes from at `now`:
    /// `routed`, the response a route fulfilled it with, if any, then the
    /// recording, then the cache, then the network. An earlier source's
    /// answer leaves the later ones unasked.
    pub async fn source(
        &self,
        routed: Option<Served>,
        partition: &Partition,
        request: &CacheRequest,
        now: SystemTime,
    ) -> Source {
        if let Some(served) = routed {
            return Source::Routed(served);
        }
        if let Some(recording) = &self.recording
            && let Some(served) = recording.recorded(request).await
        {
            return Source::Recorded(served);
        }
        let Some(cache) = &self.cache else {
            return Source::Network;
        };
        match cache.lookup(partition, request, now).await {
            Lookup::Miss => Source::Network,
            Lookup::Fresh(hit) => Source::Fresh(hit),
            Lookup::Stale(stale) => Source::Stale(stale),
        }
    }

    /// [`ResponseCache::revalidated`]; none with the cache off.
    pub async fn revalidated(
        &self,
        stale: Stale,
        request: &CacheRequest,
        head: &ResponseHead,
        now: SystemTime,
    ) -> Option<Hit> {
        self.cache
            .as_ref()?
            .revalidated(stale, request, head, now)
            .await
    }

    /// Whether a response with `head` to `request` would be stored, so its
    /// body is kept as it arrives: never with the cache off, or while the
    /// engine runs without it.
    pub fn stores(&self, request: &CacheRequest, head: &ResponseHead) -> bool {
        self.cache
            .as_ref()
            .is_some_and(|cache| !cache.running_without())
            && freshness::storable(request, head).is_some()
    }

    /// [`ResponseCache::store`]; nothing with the cache off.
    pub async fn store(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        match &self.cache {
            Some(cache) => {
                cache
                    .store(partition, request, head, body, run_id, now)
                    .await
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests;
