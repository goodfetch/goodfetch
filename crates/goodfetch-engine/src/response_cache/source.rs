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

use crate::dns::HostMapping;
use crate::sync::ActiveEffects;

use super::{
    CacheRequest, EntryScope, Hit, Lookup, Partition, ResponseCache, ResponseCacheError,
    ResponseHead, Stale, WallAllowance, freshness, partition,
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
    /// A recorded response, shared with the index, and when it was originally
    /// recorded (ADR 0105, issue 802).
    Answered {
        /// The recorded head and body.
        served: Arc<Served>,
        /// When the response was recorded (`WARC-Date` or HTTP `Date`).
        recorded_at: Option<SystemTime>,
    },
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

/// A boxed future a [`Capture`] answers, returning a sequence number for
/// optional later truncation marking.
pub type Capturing<'a> = Pin<Box<dyn Future<Output = u64> + Send + 'a>>;

/// A Run's Response recording sink in `mode: "record"` (ADR 0105): redacts,
/// masks and keeps each response received so the WARC file can be written at
/// the Run's end.
pub trait Capture: Send + Sync + std::fmt::Debug {
    /// Captures one response exchange, returning its sequence number.
    fn capture(
        &self,
        request: CacheRequest,
        head: ResponseHead,
        body: Bytes,
        truncated: bool,
        now: SystemTime,
    ) -> Capturing<'_>;

    /// Marks an already-captured exchange as truncated (when the browser
    /// reports a disconnect after delivering partial bytes).
    fn mark_truncated(&self, sequence: u64);
}

/// Where a request's response comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A route fulfilled it.
    Routed(Served),
    /// The Run's Response recording holds it, along with when it was
    /// originally recorded (ADR 0105, issue 802).
    Recorded {
        /// The recorded head and body.
        served: Arc<Served>,
        /// When the response was recorded (`WARC-Date` or HTTP `Date`).
        recorded_at: Option<SystemTime>,
    },
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
    /// It was sent while a declared Effect's body ran (ADR 0106): never
    /// stored in any mode, and never served from the cache under `replay`.
    Effect,
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
    capture: Option<Arc<dyn Capture>>,
    /// The Session/Run's `network.resolve` mapping, built once so a replaying
    /// Run reaches hosts mapped to loopback live while answering every other
    /// host from its recording before any DNS lookup (ADR 0105, issue 801).
    resolve: Arc<HostMapping>,
    /// The Run's Wall allowance, built once from its settings so Wall-tagged
    /// entries are served only to Runs whose settings cover them (ADR 0052).
    wall_allowance: WallAllowance,
    /// The Run's in-memory count of active declared Effect bodies, read with
    /// a single atomic load and no I/O per request (ADRs 0046, 0106, 0140).
    active_effects: Option<ActiveEffects>,
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
            capture: None,
            resolve: Arc::new(HostMapping::default()),
            wall_allowance: WallAllowance::default(),
            active_effects: None,
        }
    }

    /// Attaches the Run's in-memory count of running declared Effect bodies,
    /// read with no lock, file open or spawn per request (ADRs 0046, 0106 and
    /// 0140, issue 825).
    #[must_use]
    pub fn active_effects(self, active_effects: ActiveEffects) -> Self {
        Self {
            active_effects: Some(active_effects),
            ..self
        }
    }

    /// Whether a declared Effect's body is currently running in this Run.
    #[must_use]
    pub fn in_effect(&self) -> bool {
        self.active_effects
            .as_ref()
            .is_some_and(ActiveEffects::is_running)
    }

    /// Attaches the Run's [`WallAllowance`], built once from its settings
    /// (ADR 0052, issue 803).
    #[must_use]
    pub fn wall_allowance(self, wall_allowance: WallAllowance) -> Self {
        Self {
            wall_allowance,
            ..self
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

    /// Attaches `resolve`, the Session or Run's `network.resolve` mapping,
    /// built once so loopback-mapped hosts bypass replay and reach the live
    /// test server (ADR 0105, issue 801).
    #[must_use]
    pub fn resolve(self, resolve: Arc<HostMapping>) -> Self {
        Self { resolve, ..self }
    }

    /// The source recording live responses into `capture` (`mode: "record"`):
    /// reads no cache entry, while still allowing the cache to store.
    #[must_use]
    pub fn capturing(self, capture: Arc<dyn Capture>) -> Self {
        Self {
            capture: Some(capture),
            ..self
        }
    }

    /// Whether it only sends: no recording, no capture, and the cache off.
    pub fn sends_only(&self) -> bool {
        matches!(self.cache, Keeping::Off) && self.recording.is_none() && self.capture.is_none()
    }

    /// Whether the Run replays a Response recording (ADR 0105): every
    /// unrouted request not mapped to loopback by `network.resolve` is
    /// answered from the recording or refused as `response_not_recorded`.
    pub fn replaying(&self) -> bool {
        self.recording.is_some()
    }

    /// Whether `url` is answered from the Run's Response recording rather than
    /// reached live: true when replaying a recording and `url`'s host is not
    /// mapped to loopback by `network.resolve` (ADR 0105, issue 801).
    pub fn replays_url(&self, url: &str) -> bool {
        if self.recording.is_none() {
            return false;
        }
        if self.resolve.is_empty() {
            return true;
        }
        let Some(host) = url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
        else {
            return true;
        };
        !self.resolve.maps_to_loopback(&host)
    }

    /// Whether the Run records its live responses into a Response recording
    /// (`mode: "record"`, ADR 0105): reads no cache entry, records every
    /// response, and may still store into the cache.
    pub fn recording_live(&self) -> bool {
        self.capture.is_some()
    }

    /// Whether a request sent with `method` may be answered or stored at
    /// all, so a transport reads what asking about it needs only then: the
    /// recording, `record` mode and `replay` take any method, `revalidate` a
    /// GET or a HEAD.
    pub fn takes(&self, method: &str) -> bool {
        self.recording.is_some()
            || self.capture.is_some()
            || match &self.cache {
                Keeping::Off => false,
                Keeping::Revalidate(_) => freshness::revalidates_method(method),
                Keeping::Replay(_) => true,
            }
    }

    /// Whether some response to `request` may be stored or recorded, before
    /// its head is known, so a browser pauses only those responses: always
    /// while recording, never while replaying a recording, and otherwise as
    /// the cache allows.
    pub fn may_store(&self, request: &CacheRequest, kept_out: Option<KeptOut>) -> bool {
        if self.capture.is_some() {
            return true;
        }
        self.recording.is_none()
            && kept_out.is_none()
            && !self.in_effect()
            && match &self.cache {
                Keeping::Off => false,
                Keeping::Revalidate(cache) => cache.storing() && freshness::revalidates(request),
                Keeping::Replay(cache) => cache.storing(),
            }
    }

    /// Records one captured response when in `mode: "record"`, returning its
    /// sequence number for optional truncation updates.
    pub async fn record(
        &self,
        request: CacheRequest,
        head: ResponseHead,
        body: Bytes,
        truncated: bool,
        now: SystemTime,
    ) -> Option<u64> {
        let capture = self.capture.as_ref()?;
        Some(capture.capture(request, head, body, truncated, now).await)
    }

    /// Marks an already-captured exchange as truncated.
    pub fn mark_recorded_truncated(&self, sequence: u64) {
        if let Some(capture) = &self.capture {
            capture.mark_truncated(sequence);
        }
    }

    /// Where the response to `request` in `partition` comes from at `now`:
    /// `routed`, the response a route fulfilled it with, if any, then the
    /// recording (whose miss is refused and never falls through to the
    /// cache or the network), then `Network` directly when in `mode: "record"`
    /// (never reading the cache), then the cache, then the network.
    pub async fn source(
        &self,
        routed: Option<Served>,
        partition: &Partition,
        request: &CacheRequest,
        kept_out: Option<KeptOut>,
        now: SystemTime,
    ) -> Source {
        self.resolve_source(routed, partition, request, kept_out, false, now)
            .await
    }

    /// As [`Self::source`], for a Wall refetch (ADR 0052): a refetch never
    /// reads the Response cache, so it never replays the copy that was
    /// withheld.
    pub async fn source_for_refetch(
        &self,
        routed: Option<Served>,
        partition: &Partition,
        request: &CacheRequest,
        kept_out: Option<KeptOut>,
        now: SystemTime,
    ) -> Source {
        self.resolve_source(routed, partition, request, kept_out, true, now)
            .await
    }

    async fn resolve_source(
        &self,
        routed: Option<Served>,
        partition: &Partition,
        request: &CacheRequest,
        kept_out: Option<KeptOut>,
        refetch: bool,
        now: SystemTime,
    ) -> Source {
        if let Some(served) = routed {
            return Source::Routed(served);
        }
        if let Some(recording) = &self.recording {
            if self.replays_url(&request.url) {
                return match recording.recorded(request).await {
                    Recorded::Answered {
                        served,
                        recorded_at,
                    } => Source::Recorded {
                        served,
                        recorded_at,
                    },
                    Recorded::Cut(served) => Source::RecordedCut(served),
                    Recorded::Missed(miss) => Source::NotRecorded(miss),
                };
            }
            // A host mapped to loopback by `network.resolve` during replay
            // reaches the live local test server directly, never the cache.
            return Source::Network;
        }
        if refetch || self.capture.is_some() || kept_out.is_some_and(KeptOut::bars_serving) {
            return Source::Network;
        }
        match &self.cache {
            Keeping::Off => Source::Network,
            Keeping::Revalidate(cache) => match cache
                .lookup_scoped(partition, request, &self.wall_allowance, false, now)
                .await
            {
                Lookup::Miss => Source::Network,
                Lookup::Fresh(hit) => Source::Fresh(hit),
                Lookup::Stale(stale) => Source::Stale(stale),
            },
            Keeping::Replay(cache) => {
                if self.in_effect() || kept_out == Some(KeptOut::Effect) {
                    return Source::Network;
                }
                match cache
                    .replayed_scoped(partition, request, &self.wall_allowance, false, now)
                    .await
                {
                    Some(hit) => Source::Fresh(hit),
                    None => Source::Network,
                }
            }
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
        if self.recording.is_some() || self.in_effect() {
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
        self.store_scoped(
            partition,
            request,
            head,
            body,
            kept_out,
            &EntryScope::default(),
            run_id,
            now,
        )
        .await
    }

    /// As [`Self::store`], tagging the stored entry with `scope` (Wall
    /// allowance, Wall refetch, or consent acceptance; ADRs 0048 and 0052).
    #[expect(
        clippy::too_many_arguments,
        reason = "the cache's own store arguments, with outcome and approval scope"
    )]
    pub async fn store_scoped(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        kept_out: Option<KeptOut>,
        scope: &EntryScope,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        if self.recording.is_some() || kept_out.is_some() || self.in_effect() {
            return false;
        }
        match &self.cache {
            Keeping::Off => false,
            Keeping::Revalidate(cache) => {
                cache
                    .store_scoped(partition, request, head, body, scope, run_id, now)
                    .await
            }
            Keeping::Replay(cache) => {
                cache
                    .store_replayed_scoped(partition, request, head, body, scope, run_id, now)
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
