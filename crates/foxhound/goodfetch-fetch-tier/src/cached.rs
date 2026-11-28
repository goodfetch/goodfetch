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
//! request out of the public partition. A pool Session that sends its HTTP
//! itself keeps such a response in its own partition, which no other
//! Session reads and which leaves when it retires; the Run's own transport
//! has no keeper, so such a request is neither answered from the cache nor
//! stored there. The partition is the Run's person; the proxy's exit
//! country, the language and the device class stay empty until the Run's
//! Proxy and Identity profile give them. The Run's `Caching` is the
//! engine's, which the browser's interception asks alike, so an entry one
//! tier stored is served to the other.
#![expect(
    clippy::disallowed_types,
    reason = "DeferredStores locks its state only to push, mark or drain in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::sync::{Arc, Mutex, PoisonError};

use bytes::{Bytes, BytesMut};
use goodfetch_engine::response_cache::{CacheRequest, Caching, KeptOut, ResponseHead, Source};
use goodfetch_engine::response_recording::not_recorded_error;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::provenance::ResponseOrigin;
use url::Url;

use crate::driver::{Body, CallFailure, Hop, Reuse, Sending, Sent, Transport, TransportFailure};

/// One completed HTTP response held in memory until Block classification and
/// Page check pass (ADRs 0006 and 0046, issue 795).
struct PendingStore {
    caching: Arc<Caching>,
    request: CacheRequest,
    head: ResponseHead,
    body: Bytes,
}

/// Deferred Response cache entries and kept-out flag for one document fetch.
#[derive(Default)]
struct DeferredState {
    pending: Vec<PendingStore>,
    kept_out: bool,
}

/// Completed responses of one document fetch waiting for post-load checks
/// before entering the Response cache (ADRs 0006 and 0046, issue 795).
#[derive(Clone, Default)]
pub(crate) struct DeferredStores(Arc<Mutex<DeferredState>>);

impl DeferredStores {
    fn push(&self, pending: PendingStore) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.kept_out {
            state.pending.push(pending);
        }
    }

    /// Marks the document kept out (Block cause or failed Page check) and
    /// drops any held responses so they never enter the Response cache.
    pub(crate) fn keep_out(&self) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.kept_out = true;
        state.pending.clear();
    }

    /// Commits every held response to the Response cache unless marked kept
    /// out by Block classification or Page check.
    pub(crate) async fn commit(&self) {
        let pending = {
            let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if state.kept_out {
                state.pending.clear();
                return;
            }
            std::mem::take(&mut state.pending)
        };
        for entry in pending {
            entry
                .caching
                .store(&entry.request, &entry.head, entry.body)
                .await;
        }
    }
}

/// `transport` answering from `caching`'s sources first.
pub fn over(caching: &Arc<Caching>, transport: Arc<dyn Transport>) -> Arc<dyn Transport> {
    Arc::new(Cached {
        transport,
        caching: Arc::clone(caching),
        deferred: None,
    })
}

/// `transport` answering from `caching`'s sources first, holding completed
/// responses in `deferred` until the caller commits them after Block
/// classification and Page check pass (ADRs 0006 and 0046, issue 795).
pub(crate) fn over_deferred(
    caching: &Arc<Caching>,
    transport: Arc<dyn Transport>,
    deferred: DeferredStores,
) -> Arc<dyn Transport> {
    Arc::new(Cached {
        transport,
        caching: Arc::clone(caching),
        deferred: Some(deferred),
    })
}

/// `sent`, the answer to `request`, its body kept as it is read and stored
/// in `caching` (and/or recorded when in `mode: "record"`) once it ends.
fn kept(
    caching: &Arc<Caching>,
    deferred: Option<&DeferredStores>,
    request: CacheRequest,
    sent: Sent,
) -> Sent {
    let head = ResponseHead {
        status: sent.status,
        headers: sent.headers.clone(),
    };
    let kept_out = (sent.certificate_exception.is_some() || sent.custom_tls)
        .then_some(KeptOut::CertificateException);
    let store_in_cache = !sent.decoded && caching.stores(&request, &head, kept_out);
    let record_live = caching.recording();
    if !store_in_cache && !record_live {
        return sent;
    }
    Sent {
        body: Box::new(Teed {
            body: sent.body,
            read: BytesMut::new(),
            storing: Some(Storing {
                caching: Arc::clone(caching),
                deferred: deferred.cloned(),
                request,
                head,
                store_in_cache,
                record_live,
            }),
        }),
        ..sent
    }
}

/// A transport answering from a Run's sources before it sends.
struct Cached {
    transport: Arc<dyn Transport>,
    caching: Arc<Caching>,
    deferred: Option<DeferredStores>,
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
            let stale = match caching.source(&request, None).await {
                Source::Routed(served) => {
                    return Ok(whole(served.head, served.body, None));
                }
                Source::Recorded {
                    served,
                    recorded_at,
                } => {
                    let origin = recorded_at
                        .map(|recorded_at| ResponseOrigin::ResponseRecording { recorded_at });
                    return Ok(whole(served.head.clone(), served.body.clone(), origin));
                }
                Source::RecordedCut(served) => {
                    return Ok(cut_body(served.head.clone(), served.body.clone()));
                }
                Source::NotRecorded(miss) => {
                    return Err(TransportFailure::Recorded(CallFailure(not_recorded_error(
                        &miss,
                    ))));
                }
                Source::Fresh(hit) => {
                    return Ok(whole(
                        hit.head,
                        hit.body,
                        Some(ResponseOrigin::ResponseCache {
                            fetched_at: hit.fetched_at,
                            run_id: hit.run_id,
                        }),
                    ));
                }
                Source::Network => {
                    let sent = self.transport.send(hop, reuse).await?;
                    return Ok(kept(caching, self.deferred.as_ref(), request, sent));
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
                return Ok(kept(caching, self.deferred.as_ref(), request, sent));
            }
            let head = ResponseHead {
                status: sent.status,
                headers: sent.headers.clone(),
            };
            if let Some(hit) = caching.revalidated(stale, &request, &head).await {
                return Ok(Sent {
                    reused: sent.reused,
                    ..whole(
                        hit.head,
                        hit.body,
                        Some(ResponseOrigin::ResponseCache {
                            fetched_at: hit.fetched_at,
                            run_id: hit.run_id,
                        }),
                    )
                });
            }
            // The entry or its body is gone: the request is sent again
            // without the conditions, as the site was asked.
            drop(sent);
            let sent = self.transport.send(hop, reuse).await?;
            Ok(kept(caching, self.deferred.as_ref(), request, sent))
        })
    }
}

/// `head` and `body` as a hop's answer, its body whole.
fn whole(head: ResponseHead, body: Bytes, origin: Option<ResponseOrigin>) -> Sent {
    Sent {
        status: head.status,
        headers: head.headers,
        body: Box::new(Whole(Some(body).filter(|body| !body.is_empty()))),
        reused: false,
        decoded: false,
        certificate_exception: None,
        custom_tls: false,
        origin,
    }
}

/// `head` and `body` as a hop's answer whose body broke off: yields its
/// recorded bytes, then fails with `ConnectionClosed` so the driver reports
/// `response_truncated` (ADR 0118).
fn cut_body(head: ResponseHead, body: Bytes) -> Sent {
    Sent {
        status: head.status,
        headers: head.headers,
        body: Box::new(Cut(Some(body).filter(|body| !body.is_empty()))),
        reused: false,
        decoded: false,
        certificate_exception: None,
        custom_tls: false,
        origin: None,
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

/// A recorded body marked truncated: yields any partial bytes then reports
/// a closed connection.
struct Cut(Option<Bytes>);

impl Body for Cut {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let next = self.0.take();
        Box::pin(async move {
            match next {
                Some(bytes) => Ok(Some(bytes)),
                None => Err(TransportFailure::ConnectionClosed),
            }
        })
    }
}

/// What a body stores and/or records once it ends.
struct Storing {
    caching: Arc<Caching>,
    deferred: Option<DeferredStores>,
    request: CacheRequest,
    head: ResponseHead,
    store_in_cache: bool,
    record_live: bool,
}

/// A body read on, its bytes kept until it ends whole (when they are stored
/// and/or recorded) or breaks off (when a recording Run records the cut
/// response with `WARC-Truncated: disconnect` and the cache stores nothing).
struct Teed {
    body: Box<dyn Body>,
    read: BytesMut,
    storing: Option<Storing>,
}

impl Body for Teed {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            match self.body.chunk().await {
                Ok(Some(chunk)) => {
                    self.read.extend_from_slice(&chunk);
                    Ok(Some(chunk))
                }
                Ok(None) => {
                    if let Some(storing) = self.storing.take() {
                        let body = std::mem::take(&mut self.read).freeze();
                        if storing.record_live {
                            storing
                                .caching
                                .record(
                                    storing.request.clone(),
                                    storing.head.clone(),
                                    body.clone(),
                                    false,
                                )
                                .await;
                        }
                        if storing.store_in_cache {
                            if let Some(deferred) = storing.deferred {
                                deferred.push(PendingStore {
                                    caching: storing.caching,
                                    request: storing.request,
                                    head: storing.head,
                                    body,
                                });
                            } else {
                                storing
                                    .caching
                                    .store(&storing.request, &storing.head, body)
                                    .await;
                            }
                        }
                    }
                    Ok(None)
                }
                Err(error) => {
                    if let Some(storing) = self.storing.take()
                        && storing.record_live
                    {
                        let body = std::mem::take(&mut self.read).freeze();
                        storing
                            .caching
                            .record(storing.request, storing.head, body, true)
                            .await;
                    }
                    Err(error)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
