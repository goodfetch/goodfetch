//! A sitemap's fetch as one attempt (ADR 0093): the attempt of a sitemap
//! seed's work item, which reads its origin's robots.txt as a page's does,
//! fetches its sitemap on the Run's HTTP tier and parses it, running no
//! handler.

use goodfetch_engine::robots_txt::Fetched;
use goodfetch_fetch_tier::HttpTier;
use goodfetch_frontier::sitemap;
use goodfetch_protocol::foxhound::crawl::RESPONSE_NOT_OK;
use serde_json::{Map, Value};

use super::{Answer, On, failed, read_robots_txt, robots_txt_refusal};

impl On<'_> {
    /// The fetch of a sitemap seed's sitemap, or of the robots.txt whose
    /// `Sitemap:` lines it reads (ADR 0093), under the lease, on the Run's
    /// HTTP tier, whatever `fetch.tier` says: its origin's robots.txt read
    /// first, which may refuse it as it refuses a page, and the sitemap
    /// parsed, no handler run, each hop's send recorded as its domain's
    /// start and its latency for the adaptive delay, for every run sharing
    /// it, through the attempt's hand-over as a page's hops are. A
    /// robots.txt seed reads the copy its origin's robots.txt read gave,
    /// with no second fetch.
    pub(super) async fn sitemap(&self, http: &HttpTier) -> Answer {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        tokio::select! {
            biased;
            answer = async {
                let read = read_robots_txt(run, http, leased).await;
                if let Some(refused) = robots_txt_refusal(run, leased, read.as_deref()) {
                    return refused;
                }
                let Ok(url) = goodfetch_frontier::Url::parse(&leased.crawl_request.url) else {
                    return unreadable(&leased.crawl_request.url, "its URL does not parse");
                };
                if sitemap::is_robots_txt(&url) {
                    return match read.as_deref() {
                        Some(Fetched::File(file)) => Answer::SitemapRead(sitemap::Document::Sitemaps(
                            file.sitemaps().to_vec(),
                        )),
                        Some(Fetched::Unreachable(reason)) => Answer::RobotsTxtUnreachable(format!(
                            "The site's robots.txt could not be read ({reason}), so its sitemaps were not read."
                        )),
                        Some(Fetched::NotFound(_)) | None => {
                            Answer::SitemapRead(sitemap::Document::Sitemaps(Vec::new()))
                        }
                    };
                }
                let hand_over = self.gating.hand_over(run, leased, lease, self.in_flight).await;
                match http.sitemap(&url, &hand_over).await {
                    Ok(body) => match sitemap::parse(&body) {
                        Ok(document) => Answer::SitemapRead(document),
                        Err(why) => unreadable(url.as_str(), &why.0),
                    },
                    Err(error) => failed(&error),
                }
            } => answer,
            _ = lease.ended() => Answer::LeaseEnded,
        }
    }
}

/// A sitemap's fetch whose file at `url` is no sitemap, for `why`: not
/// retried, since the site answers the same again (#909 reports it).
fn unreadable(url: &str, why: &str) -> Answer {
    let mut details = Map::new();
    details.insert("url".to_owned(), Value::from(url));
    Answer::Failed {
        code: RESPONSE_NOT_OK.name.to_owned(),
        retryable: false,
        message: format!("The sitemap {url} could not be read: {why}."),
        details,
    }
}
