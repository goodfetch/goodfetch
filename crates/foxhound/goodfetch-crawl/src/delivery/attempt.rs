//! One attempt of a crawl's delivery (ADRs 0004, 0006 and 0024): its
//! `crawl_request.handle` request to a serving Connection, bounded by its
//! lease, and what its answer is. When the lease ends first, the request
//! is cancelled with `$/cancel` and an answer that still comes is a late
//! commit, refused with `lease_lost`. Its answer also says what the Session
//! that served it scored: a success, a block or a failed Page check.
//!
//! Under `fetch.tier: http` (spec #579), the attempt first fetches its
//! request's document on the Run's HTTP tier, under the same lease, and
//! hands the handler the page served from it, closed once the attempt
//! ends; a fetch that fails ends the attempt with its error, as a
//! handler's failure would, and sends the handler nothing. The fetch is
//! sent from the attempt's Session, whose identical requests in flight it
//! may share, and hedged under its domain key's room (ADR 0040). Before
//! it, the attempt reads its origin's robots.txt from the copy the user's
//! runs share (ADR 0086), which the crawl's Run reads from then on in
//! place of its own; when that copy has none to use, the robots.txt is
//! fetched under the attempt's own Permit of its domain key, and the
//! document then waits the domain's delay between starts, its start
//! recorded for every run sharing the domain. While
//! `site_preferences.respect_robots_txt` is on, a request robots.txt
//! disallows is never sent and ends `skipped` with `robots_txt`; one whose
//! robots.txt cannot be read, with no earlier copy, is not sent either, the
//! failed fetch counting as one of its attempts and backing its domain
//! off, and it ends `skipped` with `robots_txt` once its retries ran out.
//! The Crawl-delay the robots.txt asks for is recorded for the request's
//! domain key, where every sharing run's admission folds it into the
//! delay between starts; after a robots.txt fetched here the document
//! waits it too. A Crawl-delay above `site_preferences.max_crawl_delay`
//! is not waited: the request is never sent and ends `skipped` with
//! `crawl_delay` (ADR 0086). While `site_preferences.respect_use_signals` is on, a document whose
//! response carries an AI-use signal refusing the Run's Purpose is closed
//! unhandled, its request `skipped` with `use_signals` (ADRs 0086 and
//! 0093).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use std::collections::BTreeSet;

use goodfetch_engine::CallError;
use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::robots_txt::{self, Fetch, Fetched};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{
    FetchTiers, HandOver, HandedOver, HttpTier, Purpose, Requester, RobotsTxtFetch,
};
use goodfetch_frontier::{DomainKeys, EndMerged, PageSeen, SharedRobotsTxt};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    LEASE_LOST, WORK_ITEM_LEASED,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::frontier::HttpBody;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::progress::InFlightPhase;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_DELAY, SITE_PREFERENCES_MAX_CRAWL_DELAY, SITE_PREFERENCES_RESPECT_ROBOTS_TXT,
    SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
};
use goodfetch_protocol::page::{PageId, SITE_PREFERENCES_REFUSED, SitePreference};
use goodfetch_protocol::run::PointInTime;
use goodfetch_protocol::site_preferences::{
    SITE_PREFERENCES_PURPOSE, SITE_PREFERENCES_RESPECT_USE_SIGNALS,
};
use goodfetch_session_pool::Effect;
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use crate::lease::{CommitError, WorkItemLease};
use crate::progress::SharedInFlight;
use crate::work_items::{self, Leased};

/// The Error code of a page the site refused, which Collie's page raises
/// with the Block cause in its details (ADRs 0035 and 0079); declared with
/// the page's errors (#711).
const BLOCKED: &str = "blocked";

/// The Error code of a page that failed its Page check (ADR 0006), which
/// scores its Session down; declared with the Page check (area 5).
const PAGE_CHECK_FAILED: &str = "page_check_failed";

/// How an attempt ended, before its outcome is committed.
pub(super) enum Answer {
    /// The handler answered.
    Handled(CrawlRequestHandled),
    /// The handler failed, or answered no result.
    Failed {
        code: String,
        retryable: bool,
        message: String,
        details: Map<String, Value>,
    },
    /// The site's preferences refused the request before its handler ran:
    /// its robots.txt, or an AI-use signal its response carries (ADRs 0086
    /// and 0093).
    Refused(SkipReason),
    /// Its origin's robots.txt answered 429 or 5xx, or no answer, with no
    /// earlier copy to apply, for the reason given: the request was not
    /// sent, and the failed fetch counts as one of its attempts (ADR 0086).
    RobotsTxtUnreachable(String),
    /// The lease ended with no answer.
    LeaseEnded,
    /// The program's Connection ended.
    ClientGone,
}

/// What an attempt fetches its request's document with under `fetch.tier:
/// http`: the Run's HTTP tier, and the room its hedges take beside it.
pub(super) struct DocumentFetch {
    tiers: FetchTiers,
    http: HttpTier,
    second_copies: SecondCopies,
}

impl DocumentFetch {
    /// The Run's `tiers` and `http` tier, its hedges taking
    /// `second_copies`.
    pub(super) fn of(tiers: &FetchTiers, http: &HttpTier, second_copies: &SecondCopies) -> Self {
        Self {
            tiers: tiers.clone(),
            http: http.clone(),
            second_copies: second_copies.clone(),
        }
    }
}

/// What an attempt's handler pages are gated with (the fetch tiers'
/// `gate`): the tiers that hand them over, room beside the Run's own
/// requests and how robots.txt is fetched.
pub(super) struct Gating {
    pub(super) tiers: FetchTiers,
    pub(super) second_copies: SecondCopies,
    pub(super) robots: Option<RobotsTxtFetch>,
}

impl Gating {
    /// Hands the Run's pages over to the attempt of `leased` holding
    /// `lease`, while its handler runs; `in_flight` names its waits.
    async fn hand_over(
        &self,
        run: &Run,
        leased: &Leased,
        lease: &WorkItemLease,
        in_flight: &SharedInFlight,
    ) -> HandedOver {
        let politeness = share_robots_txt(run).await;
        let settings = run.settings().map(|settings| settings.resolved().clone());
        let domain_delay = settings
            .as_ref()
            .and_then(|settings| settings.duration(POLITENESS_DOMAIN_DELAY.name))
            .unwrap_or_default();
        self.tiers.hand_over(HandOver {
            run: run.clone(),
            domain_key: leased.waiting.key.clone(),
            keys: settings.as_ref().map(DomainKeys::of).unwrap_or_default(),
            second_copies: self.second_copies.clone(),
            politeness,
            robots: self.robots.clone(),
            operator_token: settings.as_ref().and_then(|settings| {
                settings
                    .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
                    .as_str()
                    .map(str::to_owned)
            }),
            domain_delay,
            lease: Arc::new(HandlerLease {
                lease: lease.clone(),
                run: run.clone(),
                request_id: leased.crawl_request.id.clone(),
                in_flight: Arc::clone(in_flight),
            }),
        })
    }
}

/// The lease check of an attempt's handler pages: the attempt's lease,
/// renewed while a page waits for a navigation gate's permit, the wait
/// named in the Run's live status, so a handler waiting for a permit is not
/// timed out meanwhile (ADR 0093).
#[derive(Clone, Debug)]
struct HandlerLease {
    lease: WorkItemLease,
    run: Run,
    request_id: String,
    in_flight: SharedInFlight,
}

/// How often a permit's wait renews its lease, as a share of its length.
const RENEWALS_PER_LENGTH: u32 = 4;

impl LeaseCheck for HandlerLease {
    fn check(&self) -> Result<(), LeaseEnded> {
        self.lease.check()
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        self.lease.withheld(ended, method)
    }

    fn permit_wait(&self, _url: &str) -> PermitWait {
        let (stop, mut stopped) = oneshot::channel::<()>();
        let waiting = self.clone();
        self.run.spawn(async move {
            let id = &waiting.request_id;
            waiting
                .in_flight
                .lock()
                .await
                .permit_wait_began(&waiting.run, id);
            let (length, clock) = (waiting.lease.length(), waiting.run.clock());
            loop {
                waiting.lease.renew(length);
                tokio::select! {
                    _ = &mut stopped => break,
                    () = clock.sleep(length / RENEWALS_PER_LENGTH) => {}
                }
            }
            // The handler's timeout runs again from the wait's end.
            waiting.lease.renew(length);
            waiting
                .in_flight
                .lock()
                .await
                .permit_wait_ended(&waiting.run, id);
        });
        PermitWait::holding(stop)
    }
}

/// The Politeness the crawl's Run shares, once it opened: from then on
/// the Run reads robots.txt from the copy its user's runs share there
/// (ADR 0086). A Politeness file that cannot open leaves the Run its own
/// copy.
async fn share_robots_txt(run: &Run) -> Option<Politeness> {
    let settings = run.settings()?.resolved();
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

/// Reads the robots.txt of `leased`'s origin, as the crawl's Run reads it:
/// from the copy its user's runs share, fetched on `http` only when the
/// copy has none to use. The Crawl-delay it asks for is recorded for the
/// request's domain key, for every run sharing the domain. A fetch made
/// here is a request of the domain: the document then starts
/// `politeness.domain.delay` after it, or the Crawl-delay when the crawl
/// waits it out and it is longer, and no sooner than that after the start
/// another sharing run took meanwhile; that start is taken in the
/// Politeness file for every run sharing the domain. Answers what applies, or `None` for a URL with no
/// robots.txt.
async fn read_robots_txt(run: &Run, http: &HttpTier, leased: &Leased) -> Option<Arc<Fetched>> {
    let location = goodfetch_frontier::Url::parse(&leased.crawl_request.url)
        .ok()
        .and_then(|url| robots_txt::location(&url))?;
    let politeness = share_robots_txt(run).await;
    let (sending, sent) = oneshot::channel::<SystemTime>();
    let fetch: Fetch<'_> = Box::pin(async {
        let _ = sending.send(run.clock().wall());
        http.robots_txt(run, &location).await
    });
    let read = run.robots_txt().copy(&location, fetch).await;
    let Some(settings) = run.settings().map(|settings| settings.resolved().clone()) else {
        return Some(read);
    };
    let asked = crawl_delay_asked(&settings, &read);
    if let Some(politeness) = &politeness
        && let Err(error) = goodfetch_frontier::crawl_delay_read(
            politeness,
            &leased.waiting.key,
            location.as_str(),
            asked,
        )
        .await
    {
        tracing::warn!(%error, run_id = %run.id(), "a host's Crawl-delay was not recorded for its domain");
    }
    let delay = settings
        .duration(POLITENESS_DOMAIN_DELAY.name)
        .max(asked.filter(|asked| waits_out(&settings, *asked)))
        .filter(|delay| !delay.is_zero());
    let (Ok(fetched_at), Some(delay)) = (sent.await, delay) else {
        return Some(read);
    };
    let start = match &politeness {
        Some(politeness) => {
            goodfetch_frontier::start_after(politeness, &leased.waiting.key, fetched_at, delay)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, run_id = %run.id(), "a document's start after its robots.txt was not recorded");
                    fetched_at + delay
                })
        }
        None => fetched_at + delay,
    };
    if let Ok(wait) = start.duration_since(run.clock().wall()) {
        run.clock().sleep(wait).await;
    }
    Some(read)
}

/// The Crawl-delay the robots.txt `read` asks of a crawl under `settings`,
/// its operator token's group merged with Foxhound's (ADR 0086).
fn crawl_delay_asked(settings: &Resolved, read: &Fetched) -> Option<Duration> {
    let operator_token = settings
        .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
        .as_str();
    goodfetch_fetch_tier::crawl_delay(read, operator_token)
}

/// Whether a crawl under `settings` waits out the Crawl-delay `asked`:
/// while `site_preferences.respect_robots_txt` is on, up to
/// `site_preferences.max_crawl_delay` (ADR 0086).
fn waits_out(settings: &Resolved, asked: Duration) -> bool {
    settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) != Some(false)
        && settings
            .duration(SITE_PREFERENCES_MAX_CRAWL_DELAY.name)
            .is_some_and(|max| asked <= max)
}

/// What the AI-use signals the response of the served page `page_id`
/// carried say of its request while `site_preferences.respect_use_signals`
/// is on (ADRs 0086 and 0093): nothing when none refuses the Run's
/// `site_preferences.purpose`, else the attempt's answer, its handler
/// never run.
fn use_signals_refusal(run: &Run, tiers: &FetchTiers, page_id: &PageId) -> Option<Answer> {
    let settings = run.settings()?.resolved();
    if settings.boolean(SITE_PREFERENCES_RESPECT_USE_SIGNALS.name) != Some(true) {
        return None;
    }
    let purpose = settings
        .value(SITE_PREFERENCES_PURPOSE.name)
        .as_str()
        .and_then(Purpose::named)?;
    tiers
        .use_signals(page_id)
        .refusing(purpose)
        .map(|_| Answer::Refused(SkipReason::UseSignals))
}

/// What the robots.txt `read` for `leased`'s request says of it while
/// `site_preferences.respect_robots_txt` is on (ADR 0086): nothing when
/// it allows it and asks for no Crawl-delay above
/// `site_preferences.max_crawl_delay`, else the attempt's answer, sent
/// nothing.
fn robots_txt_refusal(run: &Run, leased: &Leased, read: Option<&Fetched>) -> Option<Answer> {
    let settings = run.settings()?.resolved();
    if settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) == Some(false) {
        return None;
    }
    let read = read?;
    if let Fetched::Unreachable(reason) = read {
        return Some(Answer::RobotsTxtUnreachable(format!(
            "The site's robots.txt could not be read ({reason}), and no earlier copy applies, so the request was not sent."
        )));
    }
    let url = goodfetch_frontier::Url::parse(&leased.crawl_request.url).ok()?;
    let operator_token = settings
        .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
        .as_str();
    if goodfetch_fetch_tier::disallowing(read, operator_token, &url).is_some() {
        return Some(Answer::Refused(SkipReason::RobotsTxt));
    }
    crawl_delay_asked(settings, read)
        .filter(|asked| !waits_out(settings, *asked))
        .map(|_| Answer::Refused(SkipReason::CrawlDelay))
}

pub(super) struct Attempt {
    /// The server, by its place in the delivery's list, its request went to.
    pub(super) server: usize,
    pub(super) leased: Leased,
    /// Its count in `goodfetch.work_item.leased`, until it drops.
    _counted: Counted,
    /// The Session of the crawl's Session pool that served it.
    pub(super) session: ServingSession,
    pub(super) lease: WorkItemLease,
    pub(super) answer: Answer,
    /// The pending requests a rule its page taught merged, which it ended
    /// `skipped` with `duplicate`, by ID with their domain key: they leave
    /// their queues.
    pub(super) merged: Vec<(String, String)>,
}

/// One attempt: the handler request, bounded by its lease. When the lease
/// ends first, the request is cancelled and its answer left to [`late`].
pub(super) async fn attempt(
    run: Run,
    server: usize,
    client: Client,
    (leased, session, counted): (Leased, ServingSession, Counted),
    lease: WorkItemLease,
    (fetching, gating): (Option<DocumentFetch>, Gating),
    in_flight: SharedInFlight,
) -> Attempt {
    let fetched = match &fetching {
        None => Ok(None),
        Some(fetch) => {
            let requester = Requester {
                request_id: &leased.crawl_request.id,
                session: &session.id,
                label: &leased.crawl_request.label,
                domain_key: &leased.waiting.key,
                second_copies: &fetch.second_copies,
                // No work item declares an Effect before area 9's Effect
                // journal (ADR 0106), and no crawl request is a Paid fetch
                // before ADR 0045's.
                declares_effect: false,
                paid: false,
            };
            tokio::select! {
            biased;
            document = async {
                let read = read_robots_txt(&run, &fetch.http, &leased).await;
                if let Some(refused) = robots_txt_refusal(&run, &leased, read.as_deref()) {
                    return Err(refused);
                }
                fetch
                    .tiers
                    .serve(&fetch.http, &run, &leased.crawl_request, &requester)
                    .await
                    .map(Some)
                    .map_err(|error| Answer::Failed {
                        code: error.code().to_owned(),
                        retryable: error.retryable(),
                        message: error.message().to_owned(),
                        details: error.details().cloned().unwrap_or_default(),
                    })
            } => document,
            _ = lease.ended() => Err(Answer::LeaseEnded),
            }
        }
    };
    let mut merged = Vec::new();
    let fetched = fetched.and_then(|document| match (&fetching, &document) {
        (Some(fetch), Some(page_id)) => match use_signals_refusal(&run, &fetch.tiers, page_id) {
            Some(refused) => {
                fetch.tiers.close(page_id);
                Err(refused)
            }
            None => Ok(document),
        },
        _ => Ok(document),
    });
    let answer = match fetched {
        Err(answer) => answer,
        Ok(document) => {
            if let (Some(fetch), Some(page_id)) = (&fetching, &document) {
                merged = learn(&run, &fetch.tiers, page_id, &leased).await;
            }
            if document.is_some() {
                in_flight.lock().await.went_on(
                    &run,
                    &leased.crawl_request.id,
                    InFlightPhase::Handling,
                );
            }
            let params = CrawlRequestHandleParams {
                run_id: run.id().clone(),
                crawl_request: leased.crawl_request.clone(),
                attempt: leased.attempt,
                lease_epoch: leased.lease_epoch,
                session: session.clone(),
                tier: document.as_ref().map(|_| FetchTier::Http),
                page_id: document.clone(),
            };
            let handed_over = gating.hand_over(&run, &leased, &lease, &in_flight).await;
            let answer = handled(&run, &client, &params, &lease).await;
            drop(handed_over);
            if let (Some(fetch), Some(page_id)) = (&fetching, &document) {
                fetch.tiers.close(page_id);
            }
            answer
        }
    };
    Attempt {
        server,
        leased,
        _counted: counted,
        session,
        lease,
        answer,
        merged,
    }
}

/// Hands the Frontier the page `page_id` the tiers fetched for `leased`'s
/// crawl request, with its page signature and its `rel=canonical`, before
/// its handler runs, so the links the handler then enqueues count against
/// what the page taught (ADRs 0005 and 0120); in the same commit, each
/// pending request a rule it taught merges ends `skipped` with
/// `duplicate`. Answers those requests, by ID with the domain key they
/// wait under, which leave their queues. A run file that cannot take it
/// loses only what the page would have taught.
async fn learn(
    run: &Run,
    tiers: &FetchTiers,
    page_id: &PageId,
    leased: &Leased,
) -> Vec<(String, String)> {
    let Some(signed) = tiers.signature(page_id).await else {
        return Vec::new();
    };
    let Ok(url) = goodfetch_frontier::Url::parse(&signed.url) else {
        return Vec::new();
    };
    let request = leased.crawl_request.clone();
    let ended_at = run.point_in_time();
    let learned = run
        .state()
        .write(move |connection| {
            let mut merged = Merged {
                ended_at,
                ended: Vec::new(),
            };
            goodfetch_frontier::learn(
                connection,
                PageSeen {
                    crawl_request: &request.id,
                    url: &url,
                    method: &request.method,
                    body: request.body.as_ref().map(HttpBody::bytes),
                    signature: signed.signature,
                    canonical: signed.canonical.as_ref(),
                },
                &mut merged,
            )?;
            Ok(merged.ended)
        })
        .await;
    learned.unwrap_or_else(|error| {
        tracing::warn!(%error, run_id = %run.id(), "the Frontier could not learn from a fetched page");
        Vec::new()
    })
}

/// The crawl's side of a rule learned later: the pending requests it
/// merged, ended at `ended_at`, by ID with their domain key.
struct Merged {
    ended_at: PointInTime,
    ended: Vec<(String, String)>,
}

impl EndMerged for Merged {
    fn end_merged(
        &mut self,
        connection: &rusqlite::Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<String>> {
        let ended = work_items::end_merged(connection, crawl_requests, &self.ended_at)?;
        Ok(ended
            .into_iter()
            .map(|(id, url, key)| {
                if let Some(key) = key {
                    self.ended.push((id, key));
                }
                url
            })
            .collect())
    }
}

/// The skip reason of a handler's failure that is a `site_preferences_refused`
/// its own navigation raised and it let escape: the request ends `skipped`
/// with that refusal's reason, not `failed`, since the site asked not to be
/// crawled there (ADR 0093).
fn escaped_refusal(code: &str, details: &Map<String, Value>) -> Option<SkipReason> {
    if code != SITE_PREFERENCES_REFUSED.name {
        return None;
    }
    let reason = details.get("reason").cloned()?;
    match serde_json::from_value::<SitePreference>(reason).ok()? {
        SitePreference::RobotsTxt => Some(SkipReason::RobotsTxt),
        SitePreference::CrawlDelay => Some(SkipReason::CrawlDelay),
        SitePreference::UseSignals => Some(SkipReason::UseSignals),
    }
}

/// The handler request of one attempt, bounded by its lease.
async fn handled(
    run: &Run,
    client: &Client,
    params: &CrawlRequestHandleParams,
    lease: &WorkItemLease,
) -> Answer {
    match client
        .send::<CrawlRequestHandled>(&CRAWL_REQUEST_HANDLE, params)
        .await
    {
        Err(_) => Answer::ClientGone,
        Ok(mut sent) => tokio::select! {
            biased;
            answer = sent.answer() => match answer {
                Ok(handled) => Answer::Handled(handled),
                Err(ClientRequestError::Failed { code, retryable, message, details }) => {
                    match escaped_refusal(&code, &details) {
                        Some(reason) => Answer::Refused(reason),
                        None => Answer::Failed { code, retryable, message, details },
                    }
                }
                Err(ClientRequestError::Malformed(message)) => Answer::Failed {
                    code: HANDLER_FAILED.name.to_owned(),
                    retryable: false,
                    message,
                    details: Map::new(),
                },
                Err(_) => Answer::ClientGone,
            },
            _ = lease.ended() => {
                sent.cancel().await;
                run.spawn(late(run.clone(), lease.clone(), sent));
                Answer::LeaseEnded
            }
        },
    }
}

/// One more attempt in `goodfetch.work_item.leased`, until the answer
/// drops.
pub(super) fn leased_count(run: &Run) -> Counted {
    let counter = run
        .metrics()
        .meter()
        .i64_up_down_counter(WORK_ITEM_LEASED.name)
        .with_description(WORK_ITEM_LEASED.description)
        .with_unit(WORK_ITEM_LEASED.unit)
        .build();
    Counted::new(counter, Vec::new())
}

/// The answer of an attempt whose lease ended first: a result is a late
/// commit, refused with `lease_lost` and its Records discarded; a failure,
/// such as the cancel the Client honoured, changes nothing.
async fn late(run: Run, lease: WorkItemLease, mut sent: Sent<CrawlRequestHandled>) {
    tokio::select! {
        biased;
        () = run.cancellation().cancelled() => {}
        answer = sent.answer() => {
            if let Ok(handled) = answer
                && let Err(CommitError::LeaseLost) =
                    lease.commit(run.state(), handled.records, run.point_in_time()).await
            {
                run.record_outcome(LEASE_LOST.name);
            }
        }
    }
}

/// What an attempt's answer says of the Session that served it: a
/// committed answer is a success, a block or a failed Page check scores it
/// down, and any other ending, a skip or the handler's own failure, says
/// nothing of it.
pub(super) fn effect(answer: &Answer) -> Effect {
    match answer {
        Answer::Handled(CrawlRequestHandled { skipped: None, .. }) => Effect::Success,
        Answer::Failed { code, details, .. } => match block_cause(code, details) {
            Some(cause) => Effect::Blocked(cause),
            None if code == PAGE_CHECK_FAILED => Effect::PageCheckFailed,
            None => Effect::Neutral,
        },
        Answer::Handled(CrawlRequestHandled {
            skipped: Some(_), ..
        })
        | Answer::Refused(_)
        | Answer::RobotsTxtUnreachable(_)
        | Answer::LeaseEnded
        | Answer::ClientGone => Effect::Neutral,
    }
}

/// The Block cause of a handler's failure with the Error code `blocked`,
/// which its details name.
pub(super) fn block_cause(code: &str, details: &Map<String, Value>) -> Option<BlockCause> {
    if code != BLOCKED {
        return None;
    }
    details
        .get("cause")
        .and_then(|cause| serde_json::from_value(cause.clone()).ok())
}
