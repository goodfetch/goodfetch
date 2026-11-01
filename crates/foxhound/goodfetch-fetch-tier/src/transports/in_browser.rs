//! Browser-fetch (ADR 0020; spec #579's `transports`): the transport of a
//! Session that may move to the browser, which sends each hop from inside
//! that Session's browser context through Collie's browser-fetch tier, the
//! one `page.request` sends through, so the site sees one TLS and HTTP/2
//! fingerprint and one cookie jar for the whole Session, whichever tier
//! fetches (pitfall H20).
//!
//! The browser follows no redirect for it (a 3xx answers as it came), and
//! hands its body over whole with its content codings decoded, so the
//! driver only checks it ([`Sent::decoded`]) and its timeouts bound the
//! whole fetch from the hop's head on. The browser asks for its own
//! content codings, so the driver's `Accept-Encoding` is not sent over
//! them, and resends a hop the site never processed itself, so its
//! failures are [`TransportFailure::Browser`], never sent again; the
//! egress policy judges each URL before the browser is asked, as the
//! other transports judge it.

use std::sync::Arc;

use bytes::Bytes;
use goodfetch_engine::CallContext;
use goodfetch_engine::egress::{self, Judgment, Policy, RequestKind};
use goodfetch_page::page_value::{
    BrowserPages, EngineFetch, FetchedBody, PageValue, PooledSession,
};
use goodfetch_protocol::run::RunId;

use crate::driver::{Body, BrowserFailure, Hop, Reuse, Sending, Sent, Transport, TransportFailure};

/// Browser-fetch, sending from the browser context of one pool Session.
pub(crate) struct BrowserFetch {
    pub(crate) pages: Arc<dyn BrowserPages>,
    pub(crate) run_id: RunId,
    pub(crate) pooled: PooledSession,
    /// The value of the pages it sends from: the attempt's lease, and a
    /// gate that admits at once, since the chain gates each hop itself.
    pub(crate) value: PageValue,
    /// The call whose Connection's Client its pages are reached from.
    pub(crate) context: CallContext,
    pub(crate) egress: Policy,
}

impl Transport for BrowserFetch {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            let request = egress::Request {
                url: hop.url.as_str(),
                kind: RequestKind::Fetch,
            };
            if let Judgment::Refused(refused) = egress::judge(&request, &self.egress) {
                return Err(TransportFailure::Egress(refused));
            }
            let request = EngineFetch {
                url: hop.url.to_string(),
                method: hop.method.to_string(),
                headers: hop
                    .headers
                    .iter()
                    .filter(|(name, _)| !name.eq_ignore_ascii_case("accept-encoding"))
                    .cloned()
                    .collect(),
                body: hop.body.clone(),
            };
            let fetched = self
                .pages
                .fetch(
                    &self.run_id,
                    &self.pooled,
                    self.value.clone(),
                    &request,
                    &self.context,
                )
                .await
                .map_err(|error| TransportFailure::Browser(BrowserFailure(error)))?;
            Ok(Sent {
                status: fetched.status,
                headers: fetched.headers,
                body: Box::new(Handed(Some(fetched.body))),
                reused: false,
                decoded: true,
                certificate_exception: None,
            })
        })
    }
}

/// A body the browser handed over whole, or cut.
struct Handed(Option<FetchedBody>);

impl Body for Handed {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let next = match self.0.take() {
            Some(FetchedBody::Whole(bytes)) => Ok(Some(Bytes::from(bytes))),
            None => Ok(None),
            // The driver names the body cut (ADR 0118).
            Some(FetchedBody::Cut { .. }) => Err(TransportFailure::ConnectionClosed),
        };
        Box::pin(async move { next })
    }
}
