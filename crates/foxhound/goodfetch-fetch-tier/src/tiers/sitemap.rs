//! A sitemap's fetch on a Run's HTTP tier (ADR 0093): the one request a
//! crawl's sitemap seed sends itself, its redirects followed as a crawl
//! request's are, under the domain policy the tier's transport enforces,
//! and never measured against the scope.

use std::time::Duration;

use bytes::Bytes;
use goodfetch_engine::CallError;
use goodfetch_page::page_value::Permit;
use goodfetch_protocol::foxhound::crawl::{RESPONSE_NOT_OK, RESPONSE_UNAVAILABLE};
use url::Url;

use super::HttpTier;
use crate::chain::{Gating, HopGate, Skip, Stop, follow};
use crate::driver::{FetchFailure, Hop, fetch};
use crate::gate::{ConditionalRevisit, HandOver};

/// Response headers of a syndication source fetch relevant to revisit
/// scheduling (`ETag`, `Last-Modified`, caching floors; ADR 0139).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceHeaders {
    /// `ETag` validator header, if returned.
    pub etag: Option<String>,
    /// `Last-Modified` validator header, if returned.
    pub last_modified: Option<String>,
    /// `Cache-Control` header, if returned.
    pub cache_control: Option<String>,
    /// `Expires` header, if returned.
    pub expires: Option<String>,
    /// `Retry-After` header, if returned.
    pub retry_after: Option<String>,
    /// `Date` header, if returned.
    pub date: Option<String>,
}

impl SourceHeaders {
    fn from_pairs(pairs: &[(String, String)]) -> Self {
        let find = |target: &str| {
            pairs
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(target))
                .map(|(_, value)| value.clone())
        };
        Self {
            etag: find("etag"),
            last_modified: find("last-modified"),
            cache_control: find("cache-control"),
            expires: find("expires"),
            retry_after: find("retry-after"),
            date: find("date"),
        }
    }
}

/// Outcome of fetching a syndication feed with optional conditional validators
/// (ADR 0139).
#[derive(Clone, Debug)]
pub enum SyndicationFetched {
    /// The server returned a 2xx feed body.
    Modified {
        /// The decoded feed body.
        body: Bytes,
        /// Response validators and caching floor headers.
        headers: SourceHeaders,
    },
    /// The server answered `304 Not Modified` to our conditional request.
    NotModified {
        /// Updated validators / caching headers from the 304 response, if any.
        headers: SourceHeaders,
    },
}

impl HttpTier {
    /// Fetches the sitemap at `url` for `hand_over`'s Run (ADR 0093): a
    /// `GET` whose redirects are followed as a crawl request's are, under
    /// the Run's domain policy, which its transport enforces on every hop,
    /// and never the scope; each hop's send recorded as its domain's start
    /// and its answer's latency for the adaptive delay, through the
    /// attempt's `hand_over`, as a page's hops are (ADR 0086); answers its
    /// body, its content codings decoded.
    ///
    /// # Errors
    ///
    /// The fetch driver's and the chain's errors; `response_unavailable`
    /// for a 408, 429 or 5xx and `response_not_ok` for any other status
    /// but a 2xx, each naming the `url` and the `status`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the fetch's connection.
    pub async fn sitemap(&self, url: &Url, hand_over: &HandOver) -> Result<Bytes, FetchFailure> {
        match self.source_fetch("sitemap", url, hand_over, None).await? {
            SyndicationFetched::Modified { body, .. } => Ok(body),
            SyndicationFetched::NotModified { .. } => Err(CallError::new(
                &RESPONSE_NOT_OK,
                "unexpected 304 Not Modified on a sitemap fetch",
                &serde_json::json!({ "url": url.as_str() }),
            )
            .into()),
        }
    }

    /// Fetches the syndication feed at `url` for `hand_over`'s Run (ADRs 0093
    /// and 0139): same transport, redirect and Politeness rules as
    /// [`Self::sitemap`], optionally sending `If-None-Match` /
    /// `If-Modified-Since` from `conditional` and returning `304 Not Modified`
    /// as [`SyndicationFetched::NotModified`].
    ///
    /// # Errors
    ///
    /// The fetch driver's and the chain's errors; `response_unavailable` for
    /// 408, 429 or 5xx and `response_not_ok` for any other non-2xx status.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the fetch's connection.
    pub async fn syndication(
        &self,
        url: &Url,
        hand_over: &HandOver,
        conditional: Option<&ConditionalRevisit>,
    ) -> Result<SyndicationFetched, FetchFailure> {
        self.source_fetch("syndication feed", url, hand_over, conditional)
            .await
    }

    async fn source_fetch(
        &self,
        kind: &str,
        url: &Url,
        hand_over: &HandOver,
        conditional: Option<&ConditionalRevisit>,
    ) -> Result<SyndicationFetched, FetchFailure> {
        let run = &hand_over.run().identity;
        let clock = run.clock();
        let gate = SitemapHops(hand_over);
        let mut hop = Hop::get(url.clone());
        if let Some(cond) = conditional {
            if let Some(etag) = &cond.etag {
                hop.headers.push(("if-none-match".to_owned(), etag.clone()));
            }
            if let Some(last_modified) = &cond.last_modified {
                hop.headers
                    .push(("if-modified-since".to_owned(), last_modified.clone()));
            }
        }
        let fetched = match follow(
            hop,
            &self.following,
            &gate,
            (&clock, run.owner(), run.metrics().counted_work()),
            |hop| async move { fetch(self.transport.as_ref(), &hop, &self.bounds, &clock).await },
        )
        .await
        {
            Ok(fetched) => fetched,
            Err(Stop::NotModified { headers, .. }) if conditional.is_some() => {
                return Ok(SyndicationFetched::NotModified {
                    headers: SourceHeaders::from_pairs(&headers),
                });
            }
            Err(stop) => {
                return Err(match stop {
                    Stop::Failed(failure) => failure,
                    // The open gate skips no hop, and reads no robots.txt.
                    Stop::Skipped(Skip { message, .. })
                    | Stop::RobotsTxtUnreachable { message, .. } => CallError::new(
                        &RESPONSE_NOT_OK,
                        message,
                        &serde_json::json!({ "url": url.as_str() }),
                    )
                    .into(),
                    Stop::NotModified { .. } => CallError::new(
                        &RESPONSE_NOT_OK,
                        format!("unexpected 304 Not Modified on a {kind} fetch"),
                        &serde_json::json!({ "url": url.as_str() }),
                    )
                    .into(),
                });
            }
        };
        let status = fetched.status;
        if (200..300).contains(&status) {
            return Ok(SyndicationFetched::Modified {
                body: fetched.body,
                headers: SourceHeaders::from_pairs(&fetched.headers),
            });
        }
        let details = serde_json::json!({ "url": fetched.url.as_str(), "status": status });
        let unavailable = status == 408 || status == 429 || (500..600).contains(&status);
        Err(FetchFailure::from(if unavailable {
            CallError::new(
                &RESPONSE_UNAVAILABLE,
                format!("The {kind} {} answered {status}.", fetched.url),
                &details,
            )
        } else {
            CallError::new(
                &RESPONSE_NOT_OK,
                format!(
                    "The {kind} {} answered {status}, not a {kind}.",
                    fetched.url
                ),
                &details,
            )
        }))
    }
}

/// The hops of a sitemap's fetch after its first: each sent at once, under
/// the domain policy its transport enforces, with no scope to leave and no
/// handler page whose gate would admit it; a redirect's `Retry-After`
/// waited out on the Run's clock; each hop's send, the first's included,
/// and its answer's latency recorded through the attempt's hand-over.
struct SitemapHops<'h>(&'h HandOver);

impl HopGate for SitemapHops<'_> {
    fn admit<'a>(&'a self, _hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>> {
        Box::pin(async { Ok(Permit::unlimited()) })
    }

    fn wait_until<'a>(&'a self, _hop: &'a Hop, until: std::time::SystemTime) -> Gating<'a, ()> {
        Box::pin(async move {
            let clock = self.0.run().identity.clock();
            if let Ok(wait) = until.duration_since(clock.wall()) {
                clock.sleep(wait).await;
            }
        })
    }

    fn unparsable_retry_after<'a>(
        &'a self,
        hop: &'a Hop,
        status: u16,
        value: &'a str,
    ) -> Gating<'a, ()> {
        Box::pin(
            self.0
                .unparsable_retry_after(hop.url.as_str(), status, value),
        )
    }

    fn sending<'a>(&'a self, hop: &'a Hop, lead: bool) -> Gating<'a, Result<(), Stop>> {
        Box::pin(self.0.sending(hop, lead))
    }

    fn answered<'a>(&'a self, hop: &'a Hop, latency: Duration) -> Gating<'a, ()> {
        Box::pin(self.0.of_run.answered(hop, latency))
    }

    fn reached<'a>(&'a self, _hop: &'a Hop) -> Gating<'a, Result<(), Stop>> {
        Box::pin(async { Ok(()) })
    }
}
