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
//!   (ADR 0090). A URL they disallow is refused with `robots_txt`, and a
//!   host whose Crawl-delay is above `site_preferences.max_crawl_delay`
//!   with `crawl_delay`. With `site_preferences.respect_robots_txt` off,
//!   robots.txt is neither read nor applied.
//! - **The delay between starts.** The request waits until the domain's
//!   last start, by any sharing run, is its Crawl-delay ago, and on its
//!   own domain key, or after a robots.txt it fetched, also
//!   `politeness.domain.delay`. There, its start is taken in the
//!   Politeness file at once, so two handlers' requests never read the
//!   same last start and go together; on another key, its Permit's rule
//!   already spaced it, and its start is recorded for every run sharing the
//!   domain.
//!
//! The page holds the lease through every wait (`LeaseCheck::permit_wait`),
//! which the crawl renews and names in the Run's live status. A request of
//! no HTTP(S) URL goes ungated.
//!
//! The same gate admits each hop after the first of a crawl request's
//! redirect chain on a document tier ([`ChainGate`]; ADR 0118), after the
//! Frontier's checks of the hop, in this order: a hop leaving the crawl's
//! scope ends the request `skipped` with `out_of_scope`, one to a host the
//! Run's domain policy refuses with `domain_policy`, and one to a key the
//! Frontier admitted for another request with `duplicate` (ADRs 0093, 0118
//! and 0120). A hop whose URL names an address a page may not reach in
//! place of a host ends it `skipped` with `domain_policy` too, before the
//! gate reads its robots.txt (ADR 0017); a refusal of the gate's ends it
//! `skipped` with its reason.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::admission::{Room, SecondCopies};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::robots_txt::{self, Fetch, Fetched, Judgment, Rule};
use goodfetch_engine::runs::Run;
use goodfetch_frontier::{DomainKeys, Scope};
use goodfetch_page::page_value::{
    Admission, DocumentResponse, Navigation, NavigationGate, Permit, Refusal,
};
use goodfetch_page::robots_txt::{Group, ProductToken, applicable_rules};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::page::SitePreference;
use goodfetch_protocol::settings::page_check::PageCheck;
use tokio::sync::oneshot;
use url::Url;

use crate::chain::{Gating, HopGate, Skip, Stop};
use crate::driver::{Hop, judged};

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
    /// `politeness.domain.adaptive_delay`: on, each hop's latency is
    /// recorded for its domain key, for every sharing run.
    pub adaptive_delay: bool,
    /// `site_preferences.respect_robots_txt`: off, robots.txt is neither
    /// read nor applied.
    pub respect_robots_txt: bool,
    /// `site_preferences.max_crawl_delay`, the longest Crawl-delay waited
    /// out; none waits out none.
    pub max_crawl_delay: Option<Duration>,
    /// The attempt's lease.
    pub lease: Arc<dyn LeaseCheck>,
    /// The Run's domain policy, which judges each redirect hop.
    pub egress: Arc<Policy>,
    /// The Page check its crawl request's Label declares of its own, which
    /// judges the browser pages the attempt opens in place of their
    /// Session's (ADR 0093).
    pub page_check: Option<PageCheck>,
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

impl Applying {
    /// Nothing: no rule disallows the URL and no Crawl-delay is asked.
    fn nothing() -> Self {
        Self {
            disallowed: None,
            crawl_delay: None,
        }
    }
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
        let (applying, fetched_at) = if hand_over.respect_robots_txt {
            self.robots_txt(url, &key).await
        } else {
            (Applying::nothing(), None)
        };
        if let Some(rule) = applying.disallowed {
            return Err(Refusal {
                reason: SitePreference::RobotsTxt,
                message: format!(
                    "The site's robots.txt disallows `{url}` to the crawl ({rule}), so the request never left."
                ),
            });
        }
        let crawl_delay = applying.crawl_delay.unwrap_or_default();
        if crawl_delay > Duration::ZERO
            && hand_over
                .max_crawl_delay
                .is_none_or(|max| crawl_delay > max)
        {
            let most = hand_over.max_crawl_delay.map_or_else(
                || "none".to_owned(),
                |max| format!("{} s", max.as_secs_f64()),
            );
            return Err(Refusal {
                reason: SitePreference::CrawlDelay,
                message: format!(
                    "The site's robots.txt asks for a Crawl-delay of {} s, more than `site_preferences.max_crawl_delay` waits out ({most}), so the request to `{url}` never left.",
                    crawl_delay.as_secs_f64()
                ),
            });
        }
        let start = self.start(&key, own, before, fetched_at, crawl_delay).await;
        if let Some(wait) = start.and_then(|start| start.duration_since(run.clock().wall()).ok()) {
            run.clock().sleep(wait).await;
        }
        Ok(permit)
    }

    /// When a request of `key` admitted now starts, taken for every run
    /// sharing the domain at once, so that two handlers' requests never
    /// read the same last start and go together (ADR 0086): after a
    /// robots.txt fetched at `fetched_at` for it, or on the attempt's own
    /// key, the delay between starts and the Crawl-delay after the last
    /// start; on another key, whose Permit its domain's rule already
    /// spaced, the Crawl-delay after the start `before` it.
    async fn start(
        &self,
        key: &str,
        own: bool,
        before: Option<SystemTime>,
        fetched_at: Option<SystemTime>,
        crawl_delay: Duration,
    ) -> Option<SystemTime> {
        let hand_over = &self.0;
        let run = &hand_over.run;
        let wall = run.clock().wall();
        let spacing = hand_over.domain_delay.max(crawl_delay);
        // The start its spacing runs from, when it takes its start at once.
        let after = fetched_at.or_else(|| own.then(|| wall.checked_sub(spacing).unwrap_or(wall)));
        let Some(politeness) = &hand_over.politeness else {
            // No run shares the domain: the request goes once its own
            // spacing passed.
            return [
                before.map(|before| before + crawl_delay),
                before.filter(|_| own).map(|before| before + spacing),
                fetched_at.map(|fetched| fetched + spacing),
            ]
            .into_iter()
            .flatten()
            .max();
        };
        if let Some(after) = after {
            return goodfetch_frontier::start_after(politeness, key, after, spacing)
                .await
                .inspect_err(|error| {
                    tracing::warn!(%error, run_id = %run.id(), "a handler's navigation start was not taken");
                })
                .ok();
        }
        let start = before
            .map(|before| before + crawl_delay)
            .filter(|start| *start > wall);
        if let Err(error) =
            goodfetch_frontier::started_again(politeness, key, start.unwrap_or(wall)).await
        {
            tracing::warn!(%error, run_id = %run.id(), "a handler's navigation start was not recorded");
        }
        start
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
            return (Applying::nothing(), None);
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

/// Whether the robots.txt `fetched` disallows `url` to a crawl whose
/// `site_preferences.robots_txt_user_agent` is `operator_token`, and by
/// which rule (ADRs 0086 and 0090): the groups for `goodfetch-foxhound` and
/// for the token apply, merged, else `*`'s; one that is unreachable, with
/// no earlier copy, disallows everything. A token that is no product token
/// names no group.
pub fn disallowing(fetched: &Fetched, operator_token: Option<&str>, url: &Url) -> Option<String> {
    let operator = operator_token.and_then(|token| ProductToken::new(token).ok());
    applying(fetched, operator.as_ref(), url).disallowed
}

/// The Crawl-delay the robots.txt `fetched` asks of a crawl whose
/// `site_preferences.robots_txt_user_agent` is `operator_token` (ADRs 0086
/// and 0090): the largest of the groups that apply, the groups for
/// `goodfetch-foxhound` and for the token merged, else `*`'s; `None` when
/// none asks for one, or with no file.
pub fn crawl_delay(fetched: &Fetched, operator_token: Option<&str>) -> Option<Duration> {
    let Fetched::File(file) = fetched else {
        return None;
    };
    let operator = operator_token.and_then(|token| ProductToken::new(token).ok());
    applying_groups(file, operator.as_ref())
        .into_iter()
        .filter_map(robots_txt::Group::crawl_delay)
        .max()
}

/// The groups of `file` that apply to a crawl whose operator token is
/// `operator` (ADRs 0086 and 0090).
fn applying_groups<'a>(
    file: &'a robots_txt::RobotsTxt,
    operator: Option<&ProductToken>,
) -> Vec<&'a robots_txt::Group> {
    let groups: Vec<Group<usize>> = file
        .groups()
        .iter()
        .enumerate()
        .map(|(at, group)| Group {
            user_agents: group.user_agents().to_vec(),
            rules: vec![at],
        })
        .collect();
    applicable_rules(&groups, operator)
        .into_iter()
        .filter_map(|at| file.groups().get(*at))
        .collect()
}

/// What applies of `fetched` to `url` for a crawl whose operator token is
/// `operator` (ADRs 0086 and 0090).
fn applying(fetched: &Fetched, operator: Option<&ProductToken>, url: &Url) -> Applying {
    match fetched {
        Fetched::NotFound(_) => Applying::nothing(),
        Fetched::Unreachable(reason) => Applying {
            disallowed: Some(format!("robots.txt unreachable: {reason}")),
            crawl_delay: None,
        },
        Fetched::File(file) => {
            let applying = applying_groups(file, operator);
            let rules: Vec<&Rule> = applying.iter().flat_map(|group| group.rules()).collect();
            let crawl_delay = applying
                .iter()
                .filter_map(|group| group.crawl_delay())
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

/// The gate of a crawl request's redirect chain on a document tier: the
/// attempt's navigation gate, after the Frontier's checks of each hop.
pub(crate) struct ChainGate {
    pub(crate) hand_over: Arc<HandOver>,
    /// The crawl request's ID.
    pub(crate) crawl_request: String,
}

impl ChainGate {
    /// What the Frontier says of `hop`: whether it leaves the scope, goes
    /// to a host the domain policy refuses or reaches a key admitted for
    /// another request. A run file that cannot answer leaves the hop to the
    /// navigation gate, and the transport's own judgment of it.
    async fn frontier(&self, hop: &Hop) -> goodfetch_frontier::Hop {
        let run = &self.hand_over.run;
        let (id, url, method, body) = frontier_hop(&self.crawl_request, hop);
        let egress = Arc::clone(&self.hand_over.egress);
        run.state()
            .write(move |connection| {
                let scope = Scope::read(connection)?;
                goodfetch_frontier::redirect_hop(
                    connection,
                    &scope,
                    &egress,
                    &id,
                    &url,
                    &method,
                    body.as_deref(),
                )
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, run_id = %run.id(), "a redirect hop was not checked by the Frontier");
                goodfetch_frontier::Hop::Follow
            })
    }
}

/// What the Frontier takes of `hop` of `crawl_request`.
fn frontier_hop(crawl_request: &str, hop: &Hop) -> (String, Url, HttpMethod, Option<Vec<u8>>) {
    (
        crawl_request.to_owned(),
        hop.url.clone(),
        HttpMethod::parse(hop.method.as_str()).unwrap_or_else(|_| HttpMethod::get()),
        hop.body.as_ref().map(|body| body.to_vec()),
    )
}

impl HopGate for ChainGate {
    fn admit<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>> {
        Box::pin(async move {
            let skipped = |reason, message: String| Err(Stop::Skipped(Skip { reason, message }));
            match self.frontier(hop).await {
                goodfetch_frontier::Hop::OutOfScope => {
                    return skipped(
                        SkipReason::OutOfScope,
                        format!("A redirect to `{}` leaves the crawl's scope.", hop.url),
                    );
                }
                goodfetch_frontier::Hop::DomainPolicy => {
                    return skipped(
                        SkipReason::DomainPolicy,
                        format!(
                            "A redirect to `{}` goes to a host the Run's domain policy refuses.",
                            hop.url
                        ),
                    );
                }
                goodfetch_frontier::Hop::Duplicate => {
                    return skipped(
                        SkipReason::Duplicate,
                        format!(
                            "A redirect to `{}` reaches a page the crawl admitted for another request.",
                            hop.url
                        ),
                    );
                }
                goodfetch_frontier::Hop::Follow => {}
            }
            // The crawl's HTTP tier connects directly: no proxy resolves
            // its hops.
            if let Err(refused) = judged(&hop.url, &self.hand_over.egress, true) {
                return skipped(
                    SkipReason::DomainPolicy,
                    format!(
                        "A redirect to `{}` is refused by the Run's egress policy: {refused}.",
                        hop.url
                    ),
                );
            }
            let _waiting = self.hand_over.lease.permit_wait(hop.url.as_str());
            match Gate::new(Arc::clone(&self.hand_over))
                .admitted(&hop.url)
                .await
            {
                Ok(permit) => Ok(permit),
                Err(refusal) => skipped(skip_reason(refusal.reason), refusal.message),
            }
        })
    }

    fn wait_until<'a>(&'a self, hop: &'a Hop, until: SystemTime) -> Gating<'a, ()> {
        Box::pin(async move {
            let _waiting = self.hand_over.lease.permit_wait(hop.url.as_str());
            let clock = self.hand_over.run.clock();
            if let Ok(wait) = until.duration_since(clock.wall()) {
                clock.sleep(wait).await;
            }
        })
    }

    /// Records the hop's send as the last start of its domain key for
    /// every run sharing it, when later than the start its admission took:
    /// what ran between them (robots.txt, the Frontier's checks, the
    /// hand-over) would otherwise bring the next request that much closer
    /// at the site than the delay (ADR 0086). It also moves the token the
    /// admission took from the domain's rate to the send, when later.
    fn sending<'a>(&'a self, hop: &'a Hop) -> Gating<'a, ()> {
        Box::pin(async move {
            let hand_over = &self.hand_over;
            let Some(politeness) = &hand_over.politeness else {
                return;
            };
            let run = &hand_over.run;
            let key = hand_over.keys.key(hop.url.as_str());
            if let Err(error) =
                goodfetch_frontier::started_again(politeness, &key, run.clock().wall()).await
            {
                tracing::warn!(%error, run_id = %run.id(), "a request's start at its send was not recorded");
            }
        })
    }

    /// Records the hop's latency for its domain key, for every run sharing
    /// it, while the Run's adaptive delay is on (spec #582's Settings).
    fn answered<'a>(&'a self, hop: &'a Hop, latency: Duration) -> Gating<'a, ()> {
        Box::pin(async move {
            let hand_over = &self.hand_over;
            let Some(politeness) = hand_over
                .politeness
                .as_ref()
                .filter(|_| hand_over.adaptive_delay)
            else {
                return;
            };
            let key = hand_over.keys.key(hop.url.as_str());
            if let Err(error) =
                goodfetch_frontier::latency_measured(politeness, &key, latency).await
            {
                tracing::warn!(%error, run_id = %hand_over.run.id(), "a response's latency was not recorded for its domain");
            }
        })
    }

    fn reached<'a>(&'a self, hop: &'a Hop) -> Gating<'a, ()> {
        Box::pin(async move {
            let run = &self.hand_over.run;
            let (id, url, method, body) = frontier_hop(&self.crawl_request, hop);
            let recorded = run
                .state()
                .write(move |connection| {
                    goodfetch_frontier::redirect_reached(
                        connection,
                        &id,
                        &url,
                        &method,
                        body.as_deref(),
                    )
                })
                .await;
            if let Err(error) = recorded {
                tracing::warn!(%error, run_id = %run.id(), "a redirect's final URL was not recorded");
            }
        })
    }
}

/// The skip a site preference's refusal of a hop ends its request with.
fn skip_reason(reason: SitePreference) -> SkipReason {
    match reason {
        SitePreference::RobotsTxt => SkipReason::RobotsTxt,
        SitePreference::CrawlDelay => SkipReason::CrawlDelay,
        SitePreference::UseSignals => SkipReason::UseSignals,
    }
}
