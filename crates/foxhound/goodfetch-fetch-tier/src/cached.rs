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

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use goodfetch_engine::response_cache::{CacheRequest, Caching, KeptOut, ResponseHead, Source};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use url::Url;

use crate::driver::{Body, Hop, Reuse, Sending, Sent, Transport};

/// `transport` answering from `caching`'s sources first.
pub(crate) fn over(caching: &Arc<Caching>, transport: Arc<dyn Transport>) -> Arc<dyn Transport> {
    Arc::new(Cached {
        transport,
        caching: Arc::clone(caching),
    })
}

/// `sent`, the answer to `request`, its body kept as it is read and stored
/// in `caching` once it ended whole when the cache stores it.
fn kept(caching: &Arc<Caching>, request: CacheRequest, sent: Sent) -> Sent {
    let head = ResponseHead {
        status: sent.status,
        headers: sent.headers.clone(),
    };
    let kept_out = sent
        .certificate_exception
        .is_some()
        .then_some(KeptOut::CertificateException);
    if sent.decoded || !caching.stores(&request, &head, kept_out) {
        return sent;
    }
    Sent {
        body: Box::new(Teed {
            body: sent.body,
            read: BytesMut::new(),
            storing: Some(Storing {
                caching: Arc::clone(caching),
                request,
                head,
            }),
        }),
        ..sent
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
            let stale = match caching.source(&request, None).await {
                Source::Routed(served) | Source::Recorded(served) => {
                    return Ok(whole(served.head, served.body));
                }
                Source::Fresh(hit) => return Ok(whole(hit.head, hit.body)),
                Source::Network => {
                    let sent = self.transport.send(hop, reuse).await?;
                    return Ok(kept(caching, request, sent));
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
                return Ok(kept(caching, request, sent));
            }
            let head = ResponseHead {
                status: sent.status,
                headers: sent.headers.clone(),
            };
            if let Some(hit) = caching.revalidated(stale, &request, &head).await {
                return Ok(Sent {
                    reused: sent.reused,
                    ..whole(hit.head, hit.body)
                });
            }
            // The entry or its body is gone: the request is sent again
            // without the conditions, as the site was asked.
            drop(sent);
            let sent = self.transport.send(hop, reuse).await?;
            Ok(kept(caching, request, sent))
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
                        storing
                            .caching
                            .store(
                                &storing.request,
                                &storing.head,
                                std::mem::take(&mut self.read).freeze(),
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
