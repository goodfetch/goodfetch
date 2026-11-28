//! `Caching` (ADR 0046): a Run's `ResponseSource` with the partition its
//! responses are kept in, its ID and its clock, which every tier's
//! transport consults alike, so an entry one tier stored is served to
//! another where the key matches. The HTTP tier puts it over its
//! transport; the browser's interception asks it at its cache stage. A
//! Session's own partition holds the responses it fetched with a cookie or
//! a credential, or that set one, until the Session ends; the Run's own
//! has no keeper, so such a response is neither served nor stored there.

use std::borrow::Cow;
use std::sync::Arc;

use bytes::Bytes;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunId;

use super::{
    CacheRequest, EntryScope, Hit, Keeper, KeptOut, Partition, ResponseHead, ResponseSource,
    Source, Stale,
};
use crate::clock::Clock;
use crate::mask::{Masks, Scope};

/// What a Run's transports answer from before they send: its source, the
/// partition its responses are kept in, its ID, its clock and, when
/// attached, the engine's secret `Masks` so requests carrying a resolved
/// secret are never stored.
#[derive(Clone, Debug)]
pub struct Caching {
    source: ResponseSource,
    partition: Partition,
    run_id: RunId,
    clock: Clock,
    masks: Option<Masks>,
}

impl Caching {
    /// What the transports of the Run `run_id` answer from: `source`, its
    /// responses kept in the partition of `person`, whom the Run's
    /// Connection acts for (the engine's `CALLER`), as `clock` reads the
    /// time; none when `source` only sends.
    pub fn of(
        source: ResponseSource,
        person: &str,
        run_id: RunId,
        clock: Clock,
    ) -> Option<Arc<Self>> {
        (!source.sends_only()).then(|| {
            Arc::new(Self {
                source,
                partition: Partition {
                    person: person.to_owned(),
                    ..Partition::default()
                },
                run_id,
                clock,
                masks: None,
            })
        })
    }

    /// The same source, also checking each request against `masks` so a
    /// request carrying a resolved secret is never stored on any tier
    /// (ADRs 0016 and 0046).
    #[must_use]
    pub fn with_masks(&self, masks: Masks) -> Arc<Self> {
        Arc::new(Self {
            masks: Some(masks),
            ..self.clone()
        })
    }

    /// The partition this transport reads and writes.
    #[must_use]
    pub fn partition(&self) -> &Partition {
        &self.partition
    }

    /// The same source in a partition with `country` as its proxy exit
    /// country (ADR 0046).
    #[must_use]
    pub fn with_country(&self, country: &str) -> Arc<Self> {
        Arc::new(Self {
            partition: Partition {
                country: country.to_owned(),
                ..self.partition.clone()
            },
            ..self.clone()
        })
    }

    /// Whether `request` carries any value resolved through the Run's
    /// Secrets, in its URL, headers or body (ADRs 0016 and 0046).
    #[must_use]
    pub fn carries_secret(&self, request: &CacheRequest) -> bool {
        let Some(masks) = &self.masks else {
            return false;
        };
        let carries = |bytes: &[u8]| matches!(masks.apply_bytes(bytes, Scope::Kept), Cow::Owned(_));
        carries(request.url.as_bytes())
            || request
                .headers
                .iter()
                .any(|(_, value)| carries(value.as_bytes()))
            || carries(request.body())
    }

    /// Whether the Run replays a Response recording, as
    /// [`ResponseSource::replaying`] says.
    pub fn replaying(&self) -> bool {
        self.source.replaying()
    }

    /// Whether `url` is answered from the Run's Response recording rather than
    /// reached live: true when replaying a recording and `url`'s host is not
    /// mapped to loopback by `network.resolve` (ADR 0105).
    pub fn replays_url(&self, url: &str) -> bool {
        self.source.replays_url(url)
    }

    /// Whether the Run records its live responses into a Response recording
    /// (`mode: "record"`, ADR 0105).
    pub fn recording(&self) -> bool {
        self.source.recording_live()
    }

    /// Records one captured response when in `mode: "record"`, returning its
    /// sequence number for optional truncation updates.
    pub async fn record(
        &self,
        request: CacheRequest,
        head: ResponseHead,
        body: Bytes,
        truncated: bool,
    ) -> Option<u64> {
        self.source
            .record(request, head, body, truncated, self.clock.wall())
            .await
    }

    /// Marks an already-captured exchange as truncated.
    pub fn mark_recorded_truncated(&self, sequence: u64) {
        self.source.mark_recorded_truncated(sequence);
    }

    /// Whether a request sent with `method` may be answered or stored, as
    /// [`ResponseSource::takes`] says.
    pub fn takes(&self, method: &str) -> bool {
        self.source.takes(method)
    }

    /// What the requests of the Run's Session `session` are answered from:
    /// these sources, the responses it fetches with a cookie or a
    /// credential, or that set one, kept in its own partition (ADR 0046).
    pub fn of_session(&self, session: &SessionId) -> Arc<Self> {
        Arc::new(Self {
            partition: Partition {
                keeper: Some(Keeper::Session {
                    run_id: self.run_id.clone(),
                    session_id: session.clone(),
                }),
                ..self.partition.clone()
            },
            ..self.clone()
        })
    }

    /// What the requests of a Session are answered from, built once when the
    /// Session opens (or when its proxy exit changes): its `keeper` (its
    /// Saved identity or itself), its proxy exit `country`, its Identity
    /// profile's `language` and its `device` class (ADR 0046).
    #[must_use]
    pub fn of_session_partition(
        &self,
        keeper: Keeper,
        country: Option<&str>,
        language: Option<&str>,
        device: Option<&str>,
    ) -> Arc<Self> {
        Arc::new(Self {
            partition: Partition {
                keeper: Some(keeper),
                person: self.partition.person.clone(),
                country: country.unwrap_or_default().to_owned(),
                language: language.unwrap_or_default().to_owned(),
                device: device.unwrap_or_default().to_owned(),
            },
            ..self.clone()
        })
    }

    /// Deletes what the Run's Session `session`, ended or retired, kept in
    /// its own partition; a cache that cannot be written keeps it, which a
    /// warning says, until the Run is discarded.
    ///
    /// # Cancel safety
    ///
    /// As [`ResponseSource::forget_session`]'s.
    pub async fn forget(&self, session: &SessionId) {
        if let Err(error) = self.source.forget_session(&self.run_id, session).await {
            tracing::warn!(run_id = %self.run_id, session_id = %session.0, %error, "a Session's Response cache entries were not deleted");
        }
    }

    /// Where the response to `request` comes from now, as
    /// [`ResponseSource::source`] says, with no route's answer.
    pub async fn source(&self, request: &CacheRequest, kept_out: Option<KeptOut>) -> Source {
        self.source
            .source(None, &self.partition, request, kept_out, self.clock.wall())
            .await
    }

    /// As [`Self::source`], for a Wall refetch (ADR 0052): never reads the
    /// Response cache.
    pub async fn source_for_refetch(
        &self,
        request: &CacheRequest,
        kept_out: Option<KeptOut>,
    ) -> Source {
        self.source
            .source_for_refetch(None, &self.partition, request, kept_out, self.clock.wall())
            .await
    }

    /// Whether a response with `head` to `request` would be stored, as
    /// [`ResponseSource::stores`] says.
    pub fn stores(
        &self,
        request: &CacheRequest,
        head: &ResponseHead,
        kept_out: Option<KeptOut>,
    ) -> bool {
        let kept_out = kept_out.or_else(|| {
            self.carries_secret(request)
                .then_some(KeptOut::ResolvedSecret)
        });
        self.source.stores(&self.partition, request, head, kept_out)
    }

    /// Whether some response to `request` may be stored, as
    /// [`ResponseSource::may_store`] says.
    pub fn may_store(&self, request: &CacheRequest, kept_out: Option<KeptOut>) -> bool {
        let kept_out = kept_out.or_else(|| {
            self.carries_secret(request)
                .then_some(KeptOut::ResolvedSecret)
        });
        self.source.may_store(request, kept_out)
    }

    /// Stores `body`, the whole answer with `head` to `request`, fetched
    /// by this Run now, as [`ResponseSource::store`] does: whether it was.
    pub async fn store(&self, request: &CacheRequest, head: &ResponseHead, body: Bytes) -> bool {
        self.store_scoped(request, head, body, &EntryScope::default())
            .await
    }

    /// As [`Self::store`], tagging the entry with `scope` (Wall allowance,
    /// Wall refetch, or consent acceptance; ADRs 0048 and 0052).
    pub async fn store_scoped(
        &self,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        scope: &EntryScope,
    ) -> bool {
        let kept_out = self
            .carries_secret(request)
            .then_some(KeptOut::ResolvedSecret);
        self.source
            .store_scoped(
                &self.partition,
                request,
                head,
                body,
                kept_out,
                scope,
                Some(&self.run_id),
                self.clock.wall(),
            )
            .await
    }

    /// The stored response `stale` names, freshened by the `304` with
    /// `head` that answered `request`'s conditions, as
    /// [`ResponseSource::revalidated`] gives it.
    pub async fn revalidated(
        &self,
        stale: Stale,
        request: &CacheRequest,
        head: &ResponseHead,
    ) -> Option<Hit> {
        self.source
            .revalidated(stale, request, head, self.clock.wall())
            .await
    }
}
