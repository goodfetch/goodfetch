//! Step 5 of a page's pipeline: the Run's Response cache and a replaying
//! Run's Response recording (ADR 0046), asked through its Session's
//! `Caching`, the engine's, which the HTTP tier's transport asks alike, so
//! an entry one tier stored is served to the other where the key matches.
//!
//! A request is judged as it leaves, with the Session's cookies the
//! browser would add, which keep a credentialed request in the Session's
//! own partition. A fresh entry, or one the recording holds, answers it
//! with no request, its content coding decoded, since the browser takes a
//! fulfilled body as it is; a stale one sends it with the cache's
//! conditions, and a 304 answering them serves the stored body. A response
//! the cache stores pauses on its way back, and its body, as the browser
//! decoded it, is stored with its head less its `Content-Encoding` and
//! `Content-Length`, so either tier serves it as it reads. A request to a
//! host reached through a certificate exception, or carrying a resolved
//! secret, is answered from the cache only where its outcome allows, and
//! never stored.
//!
//! A request a service worker answers never pauses on its page, so it is a
//! miss. A main-frame navigation whose page has a navigation gate or a
//! document reader (a handler's page), and a service worker's request when
//! a gate may judge the navigation it carries, go on to the network: the
//! gate and the reader judge what the site sends.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use data_encoding::BASE64;
use goodfetch_engine::mask::Scope;
use goodfetch_engine::response_cache::{CacheRequest, KeptOut, ResponseHead, Source, Stale};
use goodfetch_engine::response_limits::{Decoding, Limits};
use goodfetch_protocol::page::PageResponse;

use super::{COOKIE, Held, PageContext, Sender, lock};
use crate::cdp::driver::cdp_response;
use crate::cdp::{ContinueRequest, Header, NetworkId, PausedId, PausedRequest};
use crate::proxy::{self, PROXY_STATUS};

/// The response header naming a body's content codings.
const CONTENT_ENCODING: &str = "content-encoding";

/// The response headers a body served decoded no longer matches.
const DECODED_AWAY: [&str; 2] = [CONTENT_ENCODING, "content-length"];

/// A request sent on to the network whose response the cache may store,
/// or answers once a 304 revalidated its stale entry.
pub(super) struct Storing {
    /// Its network ID, by which its failure drops it.
    pub(super) network_id: Option<NetworkId>,
    request: CacheRequest,
    kept_out: Option<KeptOut>,
    /// The stale entry its conditions name.
    stale: Option<Stale>,
    /// Whether this request is a document navigation.
    navigation: bool,
}

impl Storing {
    /// Whether the load the network ID `network_id` names sent it.
    pub(super) fn sent_as(&self, network_id: &str) -> bool {
        self.network_id.as_ref().map(NetworkId::as_str) == Some(network_id)
    }
}

/// A request as the cache is asked about it.
struct Asked {
    request: CacheRequest,
    kept_out: Option<KeptOut>,
}

impl PageContext {
    /// Answers `held`, which `command` sends on, from the Session's
    /// sources when one holds its response: whether it did. Else, when
    /// the cache holds it stale, `command` carries its conditions; and when
    /// its response may be stored, `command` pauses it on its way back.
    pub(super) async fn served_from_cache(
        &self,
        held: &Held,
        command: &mut ContinueRequest,
    ) -> bool {
        if !self.consults_cache(held) {
            return false;
        }
        let Some(caching) = self.state.caching.clone() else {
            return false;
        };
        if !caching.takes(held.method()) {
            return false;
        }
        let Some(asked) = self.asked(held, command).await else {
            return false;
        };
        let stale = match caching.source(&asked.request, asked.kept_out).await {
            Source::Routed(served) | Source::Recorded(served) => {
                return self.serve(held, served.head, &served.body).await;
            }
            Source::Fresh(hit) => return self.serve(held, hit.head, &hit.body).await,
            Source::Network if caching.may_store(&asked.request, asked.kept_out) => None,
            Source::Network => return false,
            Source::Stale(stale) => {
                command.headers = Some(conditional(sent(held, command), &stale.conditions));
                Some(stale)
            }
        };
        command.intercept_response = true;
        lock(&self.storing).insert(
            held.paused.id().clone(),
            Storing {
                network_id: held.paused.network_id().cloned(),
                request: asked.request,
                kept_out: asked.kept_out,
                stale,
                navigation: held.navigation,
            },
        );
        false
    }

    /// Lets the response paused at `paused` go on, stored first when
    /// `storing` waits for it and the cache stores it; a 304 answering a
    /// stale entry's conditions is answered with the stored body instead.
    pub(super) async fn stored_or_sent_on(&self, paused: &PausedRequest, storing: Option<Storing>) {
        let storing =
            storing.filter(|_| paused.responded() && !cdp_response::redirects(paused.status()));
        if let Some(storing) = storing
            && self.answered_from_store(paused, storing).await
        {
            return;
        }
        self.continue_response(paused.id()).await;
    }

    /// Whether the cache's sources answer `held`'s requests: a page's
    /// navigation its gate or its reader judges, and a service worker's
    /// request whose navigation a gate may judge, are the site's to answer.
    fn consults_cache(&self, held: &Held) -> bool {
        let gated = self.driver.value().gate().is_some();
        if held.navigation {
            return !gated && self.driver.reader().is_none();
        }
        !(gated && matches!(self.sender, Sender::ServiceWorker { .. }))
    }

    /// `held`, sent on with `command`, as the cache is asked about it, the
    /// cookies the browser adds for its URL included; none when its body
    /// or the browser's cookies cannot be read.
    async fn asked(&self, held: &Held, command: &ContinueRequest) -> Option<Asked> {
        let url = url::Url::parse(held.url()).ok()?;
        let body = match &command.post_data {
            Some(encoded) => BASE64.decode(encoded.as_bytes()).ok()?,
            None if held.paused.has_body() => held.paused.body()?,
            None => Vec::new(),
        };
        let cookies = match self.cdp.cookie_header(held.url()).await {
            Ok(cookies) => cookies,
            Err(failure) => {
                tracing::debug!(
                    ?failure,
                    "the browser named no cookies, so the request goes on past the cache"
                );
                return None;
            }
        };
        let not_cookie = |name: &str| !name.eq_ignore_ascii_case(COOKIE);
        let mut headers: Vec<(String, String)> = match &command.headers {
            Some(sent) => sent
                .iter()
                .filter(|header| not_cookie(&header.name))
                .map(|header| (header.name.clone(), header.value.clone()))
                .collect(),
            None => held
                .paused
                .headers()
                .iter()
                .filter(|(name, _)| not_cookie(name))
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        };
        if !cookies.is_empty() {
            headers.push((COOKIE.to_owned(), cookies));
        }
        let kept_out = self.kept_out(&url, &headers, &body);
        Some(Asked {
            request: CacheRequest::new(held.method(), &url, &headers, Bytes::from(body)),
            kept_out,
        })
    }

    /// Why a response to the request to `url` with `headers` and `body`
    /// is never stored: its host is reached through a certificate
    /// exception, or it carries a resolved secret.
    fn kept_out(
        &self,
        url: &url::Url,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Option<KeptOut> {
        let excepted = url.scheme() == "https"
            && url
                .host_str()
                .zip(self.driver.relay())
                .is_some_and(|(host, relay)| relay.excepts(&host.to_ascii_lowercase()));
        if excepted {
            return Some(KeptOut::CertificateException);
        }
        let masks = self.run.secrets.mask().masks();
        let carries = |bytes: &[u8]| matches!(masks.apply_bytes(bytes, Scope::Kept), Cow::Owned(_));
        let secret = carries(url.as_str().as_bytes())
            || headers.iter().any(|(_, value)| carries(value.as_bytes()))
            || carries(body);
        secret.then_some(KeptOut::ResolvedSecret)
    }

    /// Answers `held` with `stored` and `body`, a stored response: whether
    /// it did. A main-frame document is handed to its page as the site's,
    /// and a service worker's request is traced as one it sent.
    async fn serve(&self, held: &Held, stored: ResponseHead, body: &[u8]) -> bool {
        let Some((status, headers, decoded)) = self.decoded(held.url(), stored, body) else {
            return false;
        };
        if held.navigation {
            if let Some(on_document) = &self.state.on_document {
                on_document.received(held.url(), &self.state.engine_requests);
            }
            *lock(&self.document) = Arc::from(held.url());
        }
        self.fulfill_whole(held.paused.id(), status, headers, &decoded)
            .await;
        self.trace_service_worker(held, None);
        true
    }

    /// Stores the response paused at `paused` for `storing`'s request,
    /// or answers it with the stored body a 304 revalidated: whether it
    /// answered it.
    async fn answered_from_store(&self, paused: &PausedRequest, storing: Storing) -> bool {
        let Some(caching) = self.state.caching.as_ref() else {
            return false;
        };
        let status = paused.status();
        let head = ResponseHead {
            status,
            headers: paused.response_header_list(),
        };
        if status == 304
            && let Some(stale) = storing.stale
        {
            let Some(hit) = caching.revalidated(stale, &storing.request, &head).await else {
                tracing::debug!(
                    url.origin = goodfetch_engine::log::origin(paused.url()),
                    "a stale entry left the cache before its 304 came: the 304 goes on as it came"
                );
                return false;
            };
            return self
                .answer(paused.id(), paused.url(), hit.head, &hit.body)
                .await;
        }
        if !caching.stores(&storing.request, &head, storing.kept_out) {
            return false;
        }
        // The browser gives the body decoded: the head stored says so.
        if let Some(body) = cdp_response::body_if_whole(&self.driver, paused.id().as_str()).await {
            if self.is_blocked(paused.url(), &head, &body, storing.navigation)
                && !caching.stores(&storing.request, &head, Some(KeptOut::Blocked))
            {
                return false;
            }
            let head = ResponseHead {
                status,
                headers: without_codings(head.headers),
            };
            caching
                .store(&storing.request, &head, Bytes::from(body))
                .await;
        }
        false
    }

    /// Whether the Product's Block classifier names a Block cause for this
    /// site response, keeping challenge and refusal pages out of the Response
    /// cache with no extra document read (ADRs 0046, 0053, 0140; #871).
    fn is_blocked(&self, url: &str, head: &ResponseHead, body: &[u8], navigation: bool) -> bool {
        let html = head.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value.to_ascii_lowercase().contains("html")
        });
        if !navigation && head.status < 400 && !html {
            return false;
        }
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in &head.headers {
            headers
                .entry(name.to_ascii_lowercase())
                .and_modify(|existing| {
                    existing.push('\n');
                    existing.push_str(value);
                })
                .or_insert_with(|| value.clone());
        }
        let party = self.driver.proxy().map_or(proxy::Party::Site, |proxy| {
            let proxy_status = headers.get(PROXY_STATUS).map(String::as_str);
            proxy::party(proxy.response(url, proxy_status))
        });
        if party == proxy::Party::Proxy {
            return false;
        }
        let response = PageResponse {
            url: url.to_owned(),
            status: head.status,
            status_text: String::new(),
            headers,
            encoding: None,
            request_id: String::new(),
            from_service_worker: false,
        };
        let matched: Vec<&str> = self
            .state
            .blocks
            .markers(head.status)
            .into_iter()
            .filter(|marker| contains_ascii_case_insensitive(body, marker.as_bytes()))
            .collect();
        let markup = matched.join("\n");
        self.state
            .blocks
            .classify(&response, party, &markup)
            .is_some()
    }

    /// Answers the request `id` to `url` with `head` and `body`: whether
    /// it did.
    async fn answer(&self, id: &PausedId, url: &str, head: ResponseHead, body: &[u8]) -> bool {
        let Some((status, headers, decoded)) = self.decoded(url, head, body) else {
            return false;
        };
        self.fulfill_whole(id, status, headers, &decoded).await;
        true
    }

    /// The status, headers and body the browser is answered with for
    /// `head` and `body`, a stored response to `url`: its content codings
    /// decoded under the Run's limits; none when they do not decode.
    fn decoded(
        &self,
        url: &str,
        head: ResponseHead,
        body: &[u8],
    ) -> Option<(u16, Vec<Header>, Vec<u8>)> {
        let codings = head
            .headers
            .iter()
            .filter(|(name, _)| name == CONTENT_ENCODING)
            .map(|(_, value)| value.as_str());
        let mut decoding =
            Decoding::new(url, codings, Limits::of_run(self.run.identity().settings()));
        let decoded = decoding.push(body).and_then(|()| decoding.finish());
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(failure) => {
                tracing::debug!(
                    ?failure,
                    url.origin = goodfetch_engine::log::origin(url),
                    "a stored body did not decode: the request goes on to the network"
                );
                return None;
            }
        };
        let headers = without_codings(head.headers)
            .into_iter()
            .map(|(name, value)| Header { name, value })
            .collect();
        Some((head.status, headers, decoded))
    }
}

/// `headers`, names in lower case, less those a decoded body no longer
/// matches.
fn without_codings(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(name, _)| !DECODED_AWAY.contains(&name.as_str()))
        .collect()
}

/// The headers `held` leaves with as `command` sends it on, without the
/// cookies the browser adds.
fn sent(held: &Held, command: &ContinueRequest) -> Vec<Header> {
    command.headers.clone().unwrap_or_else(|| {
        held.paused
            .headers()
            .iter()
            .map(|(name, value)| Header {
                name: name.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    })
}

/// `sent` with `conditions` in place of any header of their names.
fn conditional(mut sent: Vec<Header>, conditions: &[(String, String)]) -> Vec<Header> {
    sent.retain(|header| {
        !conditions
            .iter()
            .any(|(name, _)| header.name.eq_ignore_ascii_case(name))
    });
    sent.extend(conditions.iter().map(|(name, value)| Header {
        name: name.clone(),
        value: value.clone(),
    }));
    sent
}

/// Whether `haystack` contains `needle`, matching ASCII letters
/// case-insensitively without allocating a lowercased copy of `haystack`.
fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests;
