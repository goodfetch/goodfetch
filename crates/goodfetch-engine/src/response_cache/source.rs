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

use super::{
    CacheRequest, Hit, Lookup, Partition, ResponseCache, ResponseHead, Stale, freshness, partition,
};

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
    /// The cache holds it fresh, or replays it: served with no request.
    Fresh(Hit),
    /// The cache holds it stale: the request is sent with its conditions,
    /// and a 304 answering it is handed to [`ResponseSource::revalidated`].
    Stale(Stale),
    /// Nothing holds it: the request is sent as it is.
    Network,
}

/// Why a response is never stored, read from the fetch's own outcome
/// (ADRs 0045, 0046, 0106, 0118): what it is must not be served again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptOut {
    /// A Block cause classified it.
    Blocked,
    /// A soft block classified it.
    SoftBlocked,
    /// A Page check failed on it.
    FailedPageCheck,
    /// Its body was cut short.
    CutBody,
    /// A Paid fetch got it: never stored, and never served from the cache
    /// either, so every Paid fetch reaches its provider.
    Paid,
    /// Its request carries a resolved secret.
    ResolvedSecret,
    /// It was fetched under `trusted_cas`, `client_certificates` or
    /// `accept_invalid_certs`.
    CertificateException,
}

impl KeptOut {
    /// Whether the cache does not answer the request either: a Paid fetch
    /// alone.
    fn bars_serving(self) -> bool {
        self == Self::Paid
    }
}

/// The sources one Run's requests are answered from, which its clones
/// share. The default only sends.
#[derive(Clone, Debug, Default)]
pub struct ResponseSource {
    cache: Keeping,
    recording: Option<Arc<dyn Recording>>,
}

/// How a Run uses the cache, as its `cache.mode` says.
#[derive(Clone, Debug, Default)]
enum Keeping {
    /// `off`: never read nor written.
    #[default]
    Off,
    /// `revalidate`: under RFC 9111.
    Revalidate(ResponseCache),
    /// `replay`: everything stored and served again with no request.
    Replay(ResponseCache),
}

impl ResponseSource {
    /// The source of a Run whose `cache.mode` is `mode`, `cache` the
    /// user's: with `off` it never reads nor writes the cache.
    pub fn of(cache: &ResponseCache, mode: CacheMode) -> Self {
        Self {
            cache: match mode {
                CacheMode::Off => Keeping::Off,
                CacheMode::Revalidate => Keeping::Revalidate(cache.clone()),
                CacheMode::Replay => Keeping::Replay(cache.clone()),
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
        matches!(self.cache, Keeping::Off) && self.recording.is_none()
    }

    /// Where the response to `request` in `partition` comes from at `now`:
    /// `routed`, the response a route fulfilled it with, if any, then the
    /// recording, then the cache, then the network. An earlier source's
    /// answer leaves the later ones unasked. The cache is not asked for a
    /// request `kept_out` says it does not answer: a Paid fetch.
    pub async fn source(
        &self,
        routed: Option<Served>,
        partition: &Partition,
        request: &CacheRequest,
        kept_out: Option<KeptOut>,
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
        if kept_out.is_some_and(KeptOut::bars_serving) {
            return Source::Network;
        }
        match &self.cache {
            Keeping::Off => Source::Network,
            Keeping::Revalidate(cache) => match cache.lookup(partition, request, now).await {
                Lookup::Miss => Source::Network,
                Lookup::Fresh(hit) => Source::Fresh(hit),
                Lookup::Stale(stale) => Source::Stale(stale),
            },
            Keeping::Replay(cache) => match cache.replayed(partition, request, now).await {
                Some(hit) => Source::Fresh(hit),
                None => Source::Network,
            },
        }
    }

    /// [`ResponseCache::revalidated`]; none unless under `revalidate`.
    pub async fn revalidated(
        &self,
        stale: Stale,
        request: &CacheRequest,
        head: &ResponseHead,
        now: SystemTime,
    ) -> Option<Hit> {
        match &self.cache {
            Keeping::Revalidate(cache) => cache.revalidated(stale, request, head, now).await,
            Keeping::Off | Keeping::Replay(_) => None,
        }
    }

    /// Whether a response with `head` to `request` in `partition` would be
    /// stored, so its body is kept as it arrives: never with the cache off,
    /// while the engine runs without it, while its filesystem is low on
    /// space, when `kept_out` names a reason, nor when no partition keeps
    /// it.
    pub fn stores(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        kept_out: Option<KeptOut>,
    ) -> bool {
        let variant = match &self.cache {
            Keeping::Off => return false,
            Keeping::Revalidate(cache) => {
                cache.storing().then(|| freshness::storable(request, head))
            }
            Keeping::Replay(cache) => cache.storing().then(|| freshness::variant(request, head)),
        };
        kept_out.is_none()
            && variant.flatten().is_some()
            && partition::kept_in(partition.keeper.as_ref(), request, head).is_some()
    }

    /// [`ResponseCache::store`] under `revalidate`,
    /// [`ResponseCache::store_replayed`] under `replay`; nothing with the
    /// cache off, nor when `kept_out` names a reason.
    #[expect(
        clippy::too_many_arguments,
        reason = "the cache's own store arguments, with the outcome that keeps a response out"
    )]
    pub async fn store(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        kept_out: Option<KeptOut>,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        if kept_out.is_some() {
            return false;
        }
        match &self.cache {
            Keeping::Off => false,
            Keeping::Revalidate(cache) => {
                cache
                    .store(partition, request, head, body, run_id, now)
                    .await
            }
            Keeping::Replay(cache) => {
                cache
                    .store_replayed(partition, request, head, body, run_id, now)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests;
