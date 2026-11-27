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
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::response_cache::CacheMode;
use goodfetch_protocol::run::RunId;

use super::{
    CacheRequest, Hit, Lookup, Partition, ResponseCache, ResponseCacheError, ResponseHead, Stale,
    freshness, partition,
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

/// A request a replaying Run's Response recording had no response for
/// (ADR 0105): refused, never sent to the cache or the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotRecorded {
    /// The request's method.
    pub method: String,
    /// The request's URL, without its fragment.
    pub url: String,
    /// The Response recording's path, as the start named it.
    pub recording: String,
    /// The recorded URLs of the same host and path, in the order recorded.
    pub nearest: Vec<String>,
}

/// What a replaying Run's Response recording answers a request with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// A recorded response, shared with the index.
    Answered(Arc<Served>),
    /// A recorded response whose body broke off: replays as
    /// `response_truncated`.
    Cut(Arc<Served>),
    /// No recorded response: refused as `response_not_recorded`.
    Missed(NotRecorded),
}

/// A boxed future a [`Recording`] answers, so it stays object-safe.
pub type Replaying<'a> = Pin<Box<dyn Future<Output = Recorded> + Send + 'a>>;

/// A Run's Response recording as its source reads it (ADR 0105): the
/// response it recorded for a request, or why it missed.
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
    Recorded(Arc<Served>),
    /// The Run's Response recording holds it, marked cut (`WARC-Truncated`):
    /// replays as `response_truncated`.
    RecordedCut(Arc<Served>),
    /// The Run replays a Response recording that lacks it: refused, never
    /// sent to the cache or the network.
    NotRecorded(NotRecorded),
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

    /// Whether the Run replays a Response recording (ADR 0105): every
    /// unrouted request is answered from the recording or refused as
    /// `response_not_recorded`, and nothing reaches the cache or the site.
    pub fn replaying(&self) -> bool {
        self.recording.is_some()
    }

    /// Whether a request sent with `method` may be answered or stored at
    /// all, so a transport reads what asking about it needs only then: the
    /// recording and `replay` take any method, `revalidate` a GET or a
    /// HEAD.
    pub fn takes(&self, method: &str) -> bool {
        self.recording.is_some()
            || match &self.cache {
                Keeping::Off => false,
                Keeping::Revalidate(_) => freshness::revalidates_method(method),
                Keeping::Replay(_) => true,
            }
    }

    /// Whether some response to `request` may be stored, before its head
    /// is known, so a browser pauses only those responses: never while
    /// replaying a recording, when the cache is off, when `kept_out` names
    /// a reason, nor under `revalidate` for a method other than GET/HEAD.
    pub fn may_store(&self, request: &CacheRequest, kept_out: Option<KeptOut>) -> bool {
        self.recording.is_none()
            && kept_out.is_none()
            && match &self.cache {
                Keeping::Off => false,
                Keeping::Revalidate(cache) => cache.storing() && freshness::revalidates(request),
                Keeping::Replay(cache) => cache.storing(),
            }
    }

    /// Where the response to `request` in `partition` comes from at `now`:
    /// `routed`, the response a route fulfilled it with, if any, then the
    /// recording (whose miss is refused and never falls through to the
    /// cache or the network), then the cache, then the network. An earlier
    /// source's answer leaves the later ones unasked. The cache is not
    /// asked for a request `kept_out` says it does not answer: a Paid fetch.
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
        if let Some(recording) = &self.recording {
            return match recording.recorded(request).await {
                Recorded::Answered(served) => Source::Recorded(served),
                Recorded::Cut(served) => Source::RecordedCut(served),
                Recorded::Missed(miss) => Source::NotRecorded(miss),
            };
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
    /// stored, so its body is kept as it arrives: never while replaying a
    /// recording, with the cache off, while the engine runs without it,
    /// while its filesystem is low on space, when `kept_out` names a
    /// reason, nor when no partition keeps it.
    pub fn stores(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        kept_out: Option<KeptOut>,
    ) -> bool {
        if self.recording.is_some() {
            return false;
        }
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
    /// [`ResponseCache::store_replayed`] under `replay`; nothing while
    /// replaying a recording, with the cache off, nor when `kept_out` names
    /// a reason.
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
        if self.recording.is_some() || kept_out.is_some() {
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

    /// [`ResponseCache::forget_session`] in either mode; nothing with the
    /// cache off, which kept nothing.
    ///
    /// # Errors
    ///
    /// As [`ResponseCache::forget_session`]'s.
    ///
    /// # Cancel safety
    ///
    /// As [`ResponseCache::forget_session`]'s.
    pub async fn forget_session(
        &self,
        run_id: &RunId,
        session_id: &SessionId,
    ) -> Result<(), ResponseCacheError> {
        match &self.cache {
            Keeping::Off => Ok(()),
            Keeping::Revalidate(cache) | Keeping::Replay(cache) => {
                cache.forget_session(run_id, session_id).await
            }
        }
    }
}

#[cfg(test)]
mod tests;
