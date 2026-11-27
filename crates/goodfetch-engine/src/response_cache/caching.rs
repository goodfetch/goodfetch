//! `Caching` (ADR 0046): a Run's `ResponseSource` with the partition its
//! responses are kept in, its ID and its clock, which every tier's
//! transport consults alike, so an entry one tier stored is served to
//! another where the key matches. The HTTP tier puts it over its
//! transport; the browser's interception asks it at its cache stage. A
//! Session's own partition holds the responses it fetched with a cookie or
//! a credential, or that set one, until the Session ends; the Run's own
//! has no keeper, so such a response is neither served nor stored there.

use std::sync::Arc;

use bytes::Bytes;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunId;

use super::{
    CacheRequest, Hit, Keeper, KeptOut, Partition, ResponseHead, ResponseSource, Source, Stale,
};
use crate::clock::Clock;

/// What a Run's transports answer from before they send: its source, the
/// partition its responses are kept in, its ID and its clock.
#[derive(Clone, Debug)]
pub struct Caching {
    source: ResponseSource,
    partition: Partition,
    run_id: RunId,
    clock: Clock,
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
            })
        })
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

    /// Whether a response with `head` to `request` would be stored, as
    /// [`ResponseSource::stores`] says.
    pub fn stores(
        &self,
        request: &CacheRequest,
        head: &ResponseHead,
        kept_out: Option<KeptOut>,
    ) -> bool {
        self.source.stores(&self.partition, request, head, kept_out)
    }

    /// Whether some response to `request` may be stored, as
    /// [`ResponseSource::may_store`] says.
    pub fn may_store(&self, request: &CacheRequest, kept_out: Option<KeptOut>) -> bool {
        self.source.may_store(request, kept_out)
    }

    /// Stores `body`, the whole answer with `head` to `request`, fetched
    /// by this Run now, as [`ResponseSource::store`] does: whether it was.
    pub async fn store(&self, request: &CacheRequest, head: &ResponseHead, body: Bytes) -> bool {
        self.source
            .store(
                &self.partition,
                request,
                head,
                body,
                None,
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
