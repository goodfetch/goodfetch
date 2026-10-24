//! What Foxhound sets on a handler's pages (spec #579's `gate`; ADRs 0086,
//! 0087, 0093, 0118 and 0121): Collie's `NavigationGate`, filled with the
//! per-domain Politeness and the robots.txt of the host each navigation
//! goes to, and Collie's `LeaseCheck`, the attempt's own.
//!
//! While an attempt's handler runs, the attempt hands its Run's pages over
//! ([`FetchTiers::hand_over`](crate::FetchTiers::hand_over)): every page
//! that opens in the Run's Sessions meanwhile takes the value of the
//! attempt handed over last, and every page opened from it takes its
//! opener's. The gate then admits each navigation, redirect hop,
//! `page.request` and service worker's navigation request of such a page:
//!
//! - **Its permit.** A request to the attempt's own domain key goes under
//!   the Permit the attempt's admission holds, its document having arrived
//!   before the handler ran, so the domain keeps one request in flight; any
//!   other domain key takes a Permit of its own beside the Run's requests,
//!   as a hedge's second copy does (ADR 0040), waiting while the domain is
//!   full or its delay has not passed, and given back once the permit is
//!   released.
//! - **robots.txt.** The host's robots.txt is read through the copy the
//!   user's runs share (ADR 0086), fetched under that permit when the copy
//!   has none to use; the groups for `goodfetch-foxhound` and for
//!   `site_preferences.robots_txt_user_agent` apply, merged, else `*`'s
//!   (ADR 0090). A URL they disallow is refused with `robots_txt`.
//! - **The delay between starts.** The request waits until the domain's
//!   last start, by any sharing run, is its Crawl-delay ago, and on its
//!   own domain key also `politeness.domain.delay`; its start is then
//!   recorded for every run sharing the domain.
//!
//! The page holds the lease through every wait (`LeaseCheck::permit_wait`),
//! which the crawl renews and names in the Run's live status. A request of
//! no HTTP(S) URL goes ungated.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::admission::{Room, SecondCopies};
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::robots_txt::{self, Fetch, Fetched, Judgment, Rule};
use goodfetch_engine::runs::Run;
use goodfetch_frontier::DomainKeys;
use goodfetch_page::page_value::{
    Admission, DocumentResponse, Navigation, NavigationGate, Permit, Refusal,
};
use goodfetch_page::robots_txt::{Group, ProductToken, applicable_rules};
use goodfetch_protocol::page::SitePreference;
use tokio::sync::oneshot;
use url::Url;

/// Fetches the robots.txt at a location for a Run, as the crawl's HTTP
/// tier does (`HttpTier::robots_txt_fetch`).
pub type RobotsTxtFetch = Arc<dyn Fn(Run, Url) -> Fetch<'static> + Send + Sync>;

/// How long a navigation whose domain is full waits before it asks for a
/// Permit again: the Politeness file wakes only the admission's own
/// waiters.
const FULL_RETRY: Duration = Duration::from_millis(250);

/// What an attempt hands over with its Run's pages while its handler runs.
pub struct HandOver {
    /// The attempt's Run.
    pub run: Run,
    /// The domain key of its crawl request, whose Permit its admission
    /// holds.
    pub domain_key: String,
    /// How the Run keys its requests' domains.
    pub keys: DomainKeys,
    /// Room for requests beside those the Run admitted.
    pub second_copies: SecondCopies,
    /// The Politeness the Run shares, once it opened.
    pub politeness: Option<Politeness>,
    /// How robots.txt is fetched when the shared copy has none.
    pub robots: Option<RobotsTxtFetch>,
    /// `site_preferences.robots_txt_user_agent`, when set: a value that is
    /// no product token names no group.
    pub operator_token: Option<String>,
    /// `politeness.domain.delay`.
    pub domain_delay: Duration,
    /// The attempt's lease.
    pub lease: Arc<dyn LeaseCheck>,
}

/// The navigation gate of the pages one attempt handed over.
pub(crate) struct Gate(Arc<HandOver>);

impl Gate {
    pub(crate) fn new(hand_over: Arc<HandOver>) -> Self {
        Self(hand_over)
    }
}

impl fmt::Debug for Gate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Gate")
            .field("run", self.0.run.id())
            .field("domain_key", &self.0.domain_key)
            .finish_non_exhaustive()
    }
}

/// The lease check of the pages one attempt handed over: the attempt's.
pub(crate) struct Lease(Arc<HandOver>);

impl Lease {
    pub(crate) fn new(hand_over: Arc<HandOver>) -> Self {
        Self(hand_over)
    }
}

impl fmt::Debug for Lease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.lease.fmt(formatter)
    }
}

impl LeaseCheck for Lease {
    fn check(&self) -> Result<(), LeaseEnded> {
        self.0.lease.check()
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        self.0.lease.withheld(ended, method)
    }

    fn permit_wait(&self, url: &str) -> PermitWait {
        self.0.lease.permit_wait(url)
    }
}

impl NavigationGate for Gate {
    fn admit<'a>(&'a self, navigation: &'a Navigation) -> Admission<'a> {
        Box::pin(async move {
            match Url::parse(&navigation.url) {
                Ok(url) if matches!(url.scheme(), "http" | "https") => self.admitted(&url).await,
                _ => Ok(Permit::unlimited()),
            }
        })
    }

    fn judge_document(&self, _response: &DocumentResponse<'_>) -> Result<(), Refusal> {
        Ok(())
    }
}

/// What applies of a host's robots.txt.
struct Applying {
    /// Whether it disallows the URL, by which rule.
    disallowed: Option<String>,
    /// The largest Crawl-delay of the groups that apply.
    crawl_delay: Option<Duration>,
}

impl Gate {
    /// Admits a request to `url`, or refuses it.
    pub(crate) async fn admitted(&self, url: &Url) -> Result<Permit, Refusal> {
        let hand_over = &self.0;
        let run = &hand_over.run;
        let key = hand_over.keys.key(url.as_str());
        let own = key == hand_over.domain_key;
        let before = self.last_start(&key).await;
        let permit = if own {
            Permit::unlimited()
        } else {
            self.beside(&key).await
        };
        let (applying, fetched_at) = self.robots_txt(url, &key).await;
        if let Some(rule) = applying.disallowed {
            return Err(Refusal {
                reason: SitePreference::RobotsTxt,
                message: format!(
                    "The site's robots.txt disallows `{url}` to the crawl ({rule}), so the request never left."
                ),
            });
        }
        let crawl_delay = applying.crawl_delay.unwrap_or_default();
        let ready = [
            before.map(|before| before + crawl_delay),
            before
                .filter(|_| own)
                .map(|before| before + hand_over.domain_delay),
            fetched_at.map(|fetched| fetched + hand_over.domain_delay.max(crawl_delay)),
        ]
        .into_iter()
        .flatten()
        .max();
        let wall = run.clock().wall();
        if let Some(wait) = ready.and_then(|ready| ready.duration_since(wall).ok()) {
            run.clock().sleep(wait).await;
        }
        if let Some(politeness) = &hand_over.politeness
            && let Err(error) =
                goodfetch_frontier::started_again(politeness, &key, run.clock().wall()).await
        {
            tracing::warn!(%error, run_id = %run.id(), "a handler's navigation start was not recorded");
        }
        Ok(permit)
    }

    /// When the last request of `key` started, by any sharing run.
    async fn last_start(&self, key: &str) -> Option<SystemTime> {
        let politeness = self.0.politeness.as_ref()?;
        goodfetch_frontier::last_start(politeness, key)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, run_id = %self.0.run.id(), "a domain's last start could not be read");
                None
            })
    }

    /// A Permit of `key` beside the Run's own requests, once the domain
    /// has room, given back once released.
    async fn beside(&self, key: &str) -> Permit {
        let run = &self.0.run;
        loop {
            match self.0.second_copies.take(key, run.clock().now()).await {
                Room::Taken(copy) => {
                    let (run, second_copies) = (run.clone(), self.0.second_copies.clone());
                    return Permit::new(move || {
                        run.spawn(async move { second_copies.give_back(copy).await });
                    });
                }
                Room::Until(moment) => run.clock().sleep_until(moment).await,
                Room::Full => run.clock().sleep(FULL_RETRY).await,
            }
        }
    }

    /// What applies of `url`'s robots.txt, read from the copy the Run
    /// shares, and when it was fetched for this request, if it was.
    async fn robots_txt(&self, url: &Url, key: &str) -> (Applying, Option<SystemTime>) {
        let hand_over = &self.0;
        let run = &hand_over.run;
        let Some(location) = robots_txt::location(url) else {
            return (
                Applying {
                    disallowed: None,
                    crawl_delay: None,
                },
                None,
            );
        };
        let (sending, sent) = oneshot::channel::<SystemTime>();
        let fetch: Fetch<'_> = Box::pin(async {
            let _ = sending.send(run.clock().wall());
            match &hand_over.robots {
                Some(fetch) => fetch(run.clone(), location.clone()).await,
                None => robots_txt::Answer::Unreachable(
                    "the crawl has no transport to fetch robots.txt with".to_owned(),
                ),
            }
        });
        let fetched = run.robots_txt().copy(&location, fetch).await;
        let fetched_at = sent.await.ok();
        if let (Some(at), Some(politeness)) = (fetched_at, &hand_over.politeness)
            && let Err(error) = goodfetch_frontier::started_again(politeness, key, at).await
        {
            tracing::warn!(%error, run_id = %run.id(), "a robots.txt fetch's start was not recorded");
        }
        let operator = hand_over
            .operator_token
            .as_deref()
            .and_then(|token| ProductToken::new(token).ok());
        (applying(&fetched, operator.as_ref(), url), fetched_at)
    }
}

/// What applies of `fetched` to `url` for a crawl whose operator token is
/// `operator` (ADRs 0086 and 0090).
fn applying(fetched: &Fetched, operator: Option<&ProductToken>, url: &Url) -> Applying {
    match fetched {
        Fetched::NotFound(_) => Applying {
            disallowed: None,
            crawl_delay: None,
        },
        Fetched::Unreachable(reason) => Applying {
            disallowed: Some(format!("robots.txt unreachable: {reason}")),
            crawl_delay: None,
        },
        Fetched::File(file) => {
            let groups: Vec<Group<usize>> = file
                .groups()
                .iter()
                .enumerate()
                .map(|(at, group)| Group {
                    user_agents: group.user_agents().to_vec(),
                    rules: vec![at],
                })
                .collect();
            let applying: Vec<usize> = applicable_rules(&groups, operator)
                .into_iter()
                .copied()
                .collect();
            let rules: Vec<&Rule> = applying
                .iter()
                .filter_map(|at| file.groups().get(*at))
                .flat_map(robots_txt::Group::rules)
                .collect();
            let crawl_delay = applying
                .iter()
                .filter_map(|at| file.groups().get(*at)?.crawl_delay())
                .max();
            let disallowed = match robots_txt::judge(rules, &robots_txt::target(url)) {
                Judgment::Disallowed(rule) => Some(rule.to_string()),
                Judgment::Allowed(_) => None,
            };
            Applying {
                disallowed,
                crawl_delay,
            }
        }
    }
}
