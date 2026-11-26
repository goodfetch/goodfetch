//! The HTTP tier's transport answering from the Run's `ResponseSource`
//! (ADR 0046): a fresh response the Response cache holds is served with no
//! request; a stale one is revalidated, its request sent with the
//! cache's `If-None-Match` and `If-Modified-Since`, and a 304 answering
//! them serves the stored body; any other answer is sent on as it arrives,
//! its body kept beside the read and stored once it ended whole. A body
//! cut, decoded by a browser or reached under a certificate exception is
//! never stored. A Run whose source only sends, `cache.mode: off` with no
//! recording, sends on its transport alone and never opens the cache.
//!
//! Each hop is judged as it leaves: the driver has added the headers its
//! Session presents, a `Cookie` among them, which keeps a credentialed
//! request out of the public partition; with no keeper, such a request is
//! neither answered from the cache nor stored. The partition is the Run's
//! person;
//! the proxy's exit country, the language and the device class stay empty
//! until the Run's Proxy and Identity profile give them.

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use goodfetch_engine::CALLER;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::response_cache::{
    CacheRequest, KeptOut, Partition, ResponseHead, ResponseSource, Source,
};
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::run::RunId;
use url::Url;

use crate::driver::{Body, Hop, Reuse, Sending, Sent, Transport};

/// What a Run's transports answer from before they send: its source, the
/// partition its responses are kept in, its ID and its clock.
#[derive(Clone, Debug)]
pub(crate) struct Caching {
    source: ResponseSource,
    partition: Partition,
    run_id: RunId,
    clock: Clock,
}

impl Caching {
    /// What `run`'s transports answer from, `source` its own; none when it
    /// only sends.
    pub(crate) fn of(source: ResponseSource, run: &RunIdentity) -> Option<Arc<Self>> {
        Self::new(
            source,
            Partition {
                person: CALLER.to_owned(),
                ..Partition::default()
            },
            run.id().clone(),
            run.clock(),
        )
    }

    /// What the transports of the Run `run_id` answer from: `source`, its
    /// responses kept in `partition` as `clock` reads the time; none when
    /// `source` only sends.
    pub(crate) fn new(
        source: ResponseSource,
        partition: Partition,
        run_id: RunId,
        clock: Clock,
    ) -> Option<Arc<Self>> {
        (!source.sends_only()).then(|| {
            Arc::new(Self {
                source,
                partition,
                run_id,
                clock,
            })
        })
    }

    /// `transport` answering from these sources first.
    pub(crate) fn over(self: &Arc<Self>, transport: Arc<dyn Transport>) -> Arc<dyn Transport> {
        Arc::new(Cached {
            transport,
            caching: Arc::clone(self),
        })
    }

    /// `sent`, the answer to `request`, its body kept as it is read and
    /// stored once it ended whole when the cache stores it.
    fn kept(self: &Arc<Self>, request: CacheRequest, sent: Sent) -> Sent {
        let head = ResponseHead {
            status: sent.status,
            headers: sent.headers.clone(),
        };
        let kept_out = sent
            .certificate_exception
            .is_some()
            .then_some(KeptOut::CertificateException);
        if sent.decoded
            || !self
                .source
                .stores(&self.partition, &request, &head, kept_out)
        {
            return sent;
        }
        Sent {
            body: Box::new(Teed {
                body: sent.body,
                read: BytesMut::new(),
                storing: Some(Storing {
                    caching: Arc::clone(self),
                    request,
                    head,
                }),
            }),
            ..sent
        }
    }
}

/// A transport answering from a Run's sources before it sends.
struct Cached {
    transport: Arc<dyn Transport>,
    caching: Arc<Caching>,
}

impl Transport for Cached {
    fn tier(&self) -> FetchTier {
        self.transport.tier()
    }

    fn proxy(&self) -> Option<Url> {
        self.transport.proxy()
    }

    fn presents(&self, url: &Url) -> Vec<(String, String)> {
        self.transport.presents(url)
    }

    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            let caching = &self.caching;
            let request = CacheRequest::new(
                hop.method.as_str(),
                &hop.url,
                &hop.headers,
                hop.body.clone().unwrap_or_default(),
            );
            let answered = caching
                .source
                .source(
                    None,
                    &caching.partition,
                    &request,
                    None,
                    caching.clock.wall(),
                )
                .await;
            let stale = match answered {
                Source::Routed(served) | Source::Recorded(served) => {
                    return Ok(whole(served.head, served.body));
                }
                Source::Fresh(hit) => return Ok(whole(hit.head, hit.body)),
                Source::Network => {
                    let sent = self.transport.send(hop, reuse).await?;
                    return Ok(caching.kept(request, sent));
                }
                Source::Stale(stale) => stale,
            };
            let mut conditional = hop.clone();
            conditional.headers.retain(|(name, _)| {
                !stale
                    .conditions
                    .iter()
                    .any(|(condition, _)| name.eq_ignore_ascii_case(condition))
            });
            conditional.headers.extend(stale.conditions.iter().cloned());
            let sent = self.transport.send(&conditional, reuse).await?;
            if sent.status != 304 {
                return Ok(caching.kept(request, sent));
            }
            let head = ResponseHead {
                status: sent.status,
                headers: sent.headers.clone(),
            };
            let revalidated = caching
                .source
                .revalidated(stale, &request, &head, caching.clock.wall())
                .await;
            if let Some(hit) = revalidated {
                return Ok(Sent {
                    reused: sent.reused,
                    ..whole(hit.head, hit.body)
                });
            }
            // The entry or its body is gone: the request is sent again
            // without the conditions, as the site was asked.
            drop(sent);
            let sent = self.transport.send(hop, reuse).await?;
            Ok(caching.kept(request, sent))
        })
    }
}

/// `head` and `body` as a hop's answer, its body whole.
fn whole(head: ResponseHead, body: Bytes) -> Sent {
    Sent {
        status: head.status,
        headers: head.headers,
        body: Box::new(Whole(Some(body).filter(|body| !body.is_empty()))),
        reused: false,
        decoded: false,
        certificate_exception: None,
    }
}

/// A body served whole from a source.
struct Whole(Option<Bytes>);

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let next = self.0.take();
        Box::pin(async move { Ok(next) })
    }
}

/// What a body stores once it ended whole.
struct Storing {
    caching: Arc<Caching>,
    request: CacheRequest,
    head: ResponseHead,
}

/// A body read on, its bytes kept until it ends whole, when they are
/// stored; dropped unended, as a cut or a read stopped early leaves it,
/// it stores nothing.
struct Teed {
    body: Box<dyn Body>,
    read: BytesMut,
    storing: Option<Storing>,
}

impl Body for Teed {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            let next = self.body.chunk().await?;
            match &next {
                Some(chunk) => self.read.extend_from_slice(chunk),
                None => {
                    if let Some(storing) = self.storing.take() {
                        let caching = &storing.caching;
                        caching
                            .source
                            .store(
                                &caching.partition,
                                &storing.request,
                                &storing.head,
                                std::mem::take(&mut self.read).freeze(),
                                None,
                                Some(&caching.run_id),
                                caching.clock.wall(),
                            )
                            .await;
                    }
                }
            }
            Ok(next)
        })
    }
}

#[cfg(test)]
mod tests;
