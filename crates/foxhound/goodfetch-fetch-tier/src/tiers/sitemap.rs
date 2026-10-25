//! A sitemap's fetch on a Run's HTTP tier (ADR 0093): the one request a
//! crawl's sitemap seed sends itself, its redirects followed as a crawl
//! request's are, under the domain policy the tier's transport enforces,
//! and never measured against the scope.

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::runs::Run;
use goodfetch_frontier::DomainKeys;
use goodfetch_page::page_value::Permit;
use goodfetch_protocol::foxhound::crawl::{RESPONSE_NOT_OK, RESPONSE_UNAVAILABLE};
use url::Url;

use super::HttpTier;
use crate::chain::{Gating, HopGate, Stop, follow};
use crate::driver::{Hop, fetch};

impl HttpTier {
    /// Fetches the sitemap at `url` for `run` (ADR 0093): a `GET` whose
    /// redirects are followed as a crawl request's are, under the Run's
    /// domain policy, which its transport enforces on every hop, and never
    /// the scope; each hop's send recorded as its domain's start in
    /// `politeness`, the Politeness the Run shares, under the Run's domain
    /// `keys`, as a page's hops are (ADR 0086); answers its body, its
    /// content codings decoded.
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
    pub async fn sitemap(
        &self,
        run: &Run,
        url: &Url,
        (politeness, keys): (Option<&Politeness>, &DomainKeys),
    ) -> Result<Vec<u8>, CallError> {
        let clock = run.clock();
        let gate = SitemapHops {
            clock: clock.clone(),
            politeness,
            keys,
            run,
        };
        let fetched = follow(
            Hop::get(url.clone()),
            &self.following,
            &gate,
            &clock,
            |hop| {
                let clock = clock.clone();
                async move { fetch(self.transport.as_ref(), &hop, &self.deadlines, &clock).await }
            },
        )
        .await
        .map_err(|stop| match stop {
            Stop::Failed(error) => error,
            // The open gate skips no hop.
            Stop::Skipped(skip) => CallError::new(
                &RESPONSE_NOT_OK,
                skip.message,
                &serde_json::json!({ "url": url.as_str() }),
            ),
        })?;
        let status = fetched.status;
        if (200..300).contains(&status) {
            return Ok(fetched.body);
        }
        let details = serde_json::json!({ "url": fetched.url.as_str(), "status": status });
        let unavailable = status == 408 || status == 429 || (500..600).contains(&status);
        Err(if unavailable {
            CallError::new(
                &RESPONSE_UNAVAILABLE,
                format!("The sitemap {} answered {status}.", fetched.url),
                &details,
            )
        } else {
            CallError::new(
                &RESPONSE_NOT_OK,
                format!(
                    "The sitemap {} answered {status}, not a sitemap.",
                    fetched.url
                ),
                &details,
            )
        })
    }
}

/// The hops of a sitemap's fetch after its first: each sent at once, under
/// the domain policy its transport enforces, with no scope to leave and no
/// handler page whose gate would admit it; a redirect's `Retry-After`
/// waited out on `clock`; each hop's send, the first's included, recorded
/// as its domain's start in `politeness`.
struct SitemapHops<'h> {
    clock: Clock,
    politeness: Option<&'h Politeness>,
    keys: &'h DomainKeys,
    run: &'h Run,
}

impl HopGate for SitemapHops<'_> {
    fn admit<'a>(&'a self, _hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>> {
        Box::pin(async { Ok(Permit::unlimited()) })
    }

    fn wait_until<'a>(&'a self, _hop: &'a Hop, until: std::time::SystemTime) -> Gating<'a, ()> {
        Box::pin(async move {
            if let Ok(wait) = until.duration_since(self.clock.wall()) {
                self.clock.sleep(wait).await;
            }
        })
    }

    fn sending<'a>(&'a self, hop: &'a Hop) -> Gating<'a, ()> {
        Box::pin(async move {
            let Some(politeness) = self.politeness else {
                return;
            };
            let key = self.keys.key(hop.url.as_str());
            if let Err(error) =
                goodfetch_frontier::started_again(politeness, &key, self.clock.wall()).await
            {
                tracing::warn!(%error, run_id = %self.run.id(), "a sitemap's start at its send was not recorded");
            }
        })
    }

    fn reached<'a>(&'a self, _hop: &'a Hop) -> Gating<'a, ()> {
        Box::pin(async {})
    }
}
