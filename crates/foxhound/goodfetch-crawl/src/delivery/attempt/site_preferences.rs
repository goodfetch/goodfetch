//! What a crawl request's attempt does with the answers of the one judge
//! of a site's preferences (ADRs 0086, 0093 and 0138;
//! `goodfetch_fetch_tier::SitePreferences`): its origin's robots.txt and
//! the AI-use signal files read from the own transport of a Session of the
//! Run's pool (`HandOver::site_file`, #2422), its Crawl-delay recorded for the
//! request's domain key, a refusal turned into the attempt's answer, a
//! Record marked when the crawl ignores the robots.txt that disallows it,
//! the document's start after that robots.txt waited out, and a handler's
//! escaped `site_preferences_refused`.

use std::sync::Arc;

use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{
    AfterRobotsTxt, NotAllowed, Reading, Refusal, RefusedBy, Request, UseSignals, skip_reason,
};
use goodfetch_frontier::SharedRobotsTxt;
use goodfetch_frontier::Url;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::page::{SITE_PREFERENCES_REFUSED, SitePreference};
use serde_json::{Map, Value};

use super::{Answer, On, failed};

/// The Politeness the crawl's Run shares, under `settings`, once it
/// opened: from then on the Run reads robots.txt from the copy its user's
/// runs share there (ADR 0086). A Politeness file that cannot open leaves
/// the Run its own copy.
pub(super) async fn share_robots_txt(run: &Run, settings: &Resolved) -> Option<Politeness> {
    match run.politeness(settings).await {
        Ok((politeness, _)) => {
            let shared = politeness.clone();
            run.share_robots_txt(|| Arc::new(SharedRobotsTxt::new(shared, run.clock())));
            Some(politeness)
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "the shared robots.txt copy is out of reach; the crawl reads its own");
            None
        }
    }
}

impl On<'_> {
    /// What the site's preferences say of the attempt's `request` to
    /// `url` before it is sent (`SitePreferences::judged`), its origin's
    /// robots.txt read (`HandOver::site_file`) when something uses it, and robots.txt
    /// itself whatever the crawl respects for a sitemap seed that is one.
    /// While the crawl respects robots.txt, the Crawl-delay it asks for is
    /// recorded for the request's domain key, for every run sharing the
    /// domain, before the request is judged. A refused request is never
    /// sent: its answer says why. One the crawl fetches although robots.txt
    /// disallows it marks the Records it commits. A request let go then
    /// waits for its start after the robots.txt it read
    /// (`Reading::start_after`): `politeness.domain.delay`, or the
    /// Crawl-delay when it is longer, after that robots.txt fetch's
    /// response arrived, so the site never receives the document sooner
    /// than that after the robots.txt request. The lease is held through
    /// that wait, a permit's wait for the domain's delay: it counts against
    /// no timeout and the Run's live status names it (ADR 0093). Answers
    /// the robots.txt it read.
    pub(super) async fn site_preferences_judged(
        &self,
        request: Request,
        url: &Url,
    ) -> Result<Option<Reading>, Answer> {
        let (run, leased, of_run) = (self.run, self.leased, &self.hand_over.of_run);
        let preferences = &of_run.site_preferences;
        let key = &leased.waiting.key;
        let reads = preferences.reads_robots_txt(request);
        let robots_seed =
            request == Request::Sitemap && goodfetch_frontier::sitemap::is_robots_txt(url);
        if robots_seed
            && !reads
            && let Some(settings) = run.settings()
        {
            // A crawl that reads no robots.txt for its requests did not
            // share the copy as it started (ADR 0086).
            share_robots_txt(run, settings.resolved()).await;
        }
        let reading = if reads || robots_seed {
            preferences
                .read_robots_txt(run, url, |location| self.hand_over.site_file(location))
                .await
        } else {
            None
        };
        if let (Some(reading), Some(politeness)) = (&reading, &of_run.shared_politeness)
            && preferences.respects_robots_txt()
            && let Err(error) = goodfetch_frontier::crawl_delay_read(
                politeness,
                key,
                reading.location().as_str(),
                preferences.crawl_delay_asked(reading),
            )
            .await
        {
            tracing::warn!(%error, run_id = %run.id(), "a host's Crawl-delay was not recorded for its domain");
        }
        let allowed = preferences
            .judged(request, url, reading.as_ref(), |location| {
                self.hand_over.site_file(location)
            })
            .await
            .map_err(|not_allowed| match not_allowed {
                NotAllowed::Refused(refusal) => {
                    answer(run, leased.crawl_request.id.as_str(), refusal)
                }
                NotAllowed::PastALimit(limit) => failed(&limit.error()),
            })?;
        if allowed.ignored {
            self.lease.robots_txt_ignored();
        }
        let spacing = of_run
            .domain_delay
            .max(allowed.crawl_delay.unwrap_or_default());
        if let Some(read) = &reading
            && let AfterRobotsTxt::At(start) = read
                .start_after(of_run.politeness.as_ref(), run, key, spacing)
                .await
            && let Ok(wait) = start.duration_since(run.clock().wall())
        {
            let _waiting = self.hand_over.lease.permit_wait(&leased.crawl_request.url);
            run.clock().sleep(wait).await;
        }
        Ok(reading)
    }

    /// What the AI-use signals the response of the attempt's served page
    /// carried, `signals`, in its headers and its document's markup, say of
    /// it (`SitePreferences::document_judged`), the RSL license they name
    /// read once for the Run (`HandOver::site_file`): nothing when none refuses the
    /// Run's `site_preferences.purpose`, else the attempt's answer, its
    /// handler never run.
    pub(super) async fn document_refusal(&self, signals: &UseSignals) -> Option<Answer> {
        let (run, leased) = (self.run, self.leased);
        self.hand_over
            .of_run
            .site_preferences
            .document_judged(&leased.crawl_request.url, signals, |location| {
                self.hand_over.site_file(location)
            })
            .await
            .err()
            .map(|refusal| answer(run, leased.crawl_request.id.as_str(), refusal))
    }
}

/// The answer of an attempt whose request a site's preferences refused:
/// one whose robots.txt could not be read is retried as a failed fetch,
/// backing its domain off; any other ends `skipped` with its reason.
fn answer(run: &Run, work_item: &str, refusal: Refusal) -> Answer {
    tracing::debug!(run_id = %run.id(), work_item, by = ?refusal.by, message = refusal.message, "a site preference refused a crawl request");
    match refusal.by {
        RefusedBy::UnreachableRobotsTxt => Answer::RobotsTxtUnreachable(refusal.message),
        by => Answer::Refused(by.skip_reason()),
    }
}

/// The skip reason of a handler's failure that is a `site_preferences_refused`
/// its own navigation raised and it let escape: the request ends `skipped`
/// with that refusal's reason, not `failed`, since the site asked not to be
/// crawled there (ADR 0093).
pub(super) fn escaped_refusal(code: &str, details: &Map<String, Value>) -> Option<SkipReason> {
    if code != SITE_PREFERENCES_REFUSED.name {
        return None;
    }
    let reason = details.get("reason").cloned()?;
    serde_json::from_value::<SitePreference>(reason)
        .ok()
        .map(skip_reason)
}
