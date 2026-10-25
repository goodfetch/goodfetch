//! One attempt of a crawl's delivery (ADRs 0004, 0006 and 0024): its
//! `crawl_request.handle` request to a serving Connection, bounded by its
//! lease, and what its answer is. When the lease ends first, the request
//! is cancelled with `$/cancel` and an answer that still comes is a late
//! commit, refused with `lease_lost`. Its answer also says what the Session
//! that served it scored: a success, a block or a failed Page check.
//!
//! Its request runs as the crawl's tier choice planned it (the
//! `tier_choice` module): alone, on one tier, or as a sample of the sample
//! comparison, run on `http` and then on `browser`, the browser's run the
//! one whose answer counts, and whether the two runs matched classified
//! once the attempt settles.
//!
//! Under `fetch.tier_race` (ADR 0040; the `tier_race` module), an attempt
//! whose tier is not forced races instead: its document is fetched through
//! browser-fetch, and a full navigation of its URL in the same Session
//! starts its race's delay later, taking room beside the attempt's own as
//! a second copy does, one `requests` in Usage and its domain's delay. The
//! first page that passes the Page check, browser-fetch's only when it also
//! agrees with its Label's comparison, wins and is handed to the handler;
//! the other is cancelled: a browser-fetch in flight is dropped, its
//! connection closed, and a navigation not yet sent is never sent, while
//! one sent has its page closed, its load with it. Provenance records the race.
//! A browser-fetch whose document arrived gives its Label on its domain one
//! latency, which times its later races.
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
//! 0093). A handler that lets a `site_preferences_refused` its own
//! navigation raised escape ends its request `skipped` with that
//! refusal's reason, not `failed` (ADR 0093).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::CallContext;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{
    AddressCheck, Chosen, FetchTiers, HandOver, HttpTier, Judged, PAGE_CHECK_CALL, PooledSession,
    Requester, RobotsTxtFetch, Stop, TierChoice,
};
use goodfetch_frontier::{DomainKeys, sitemap};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    LEASE_LOST, WORK_ITEM_LEASED,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::progress::InFlightPhase;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_ADAPTIVE_DELAY, POLITENESS_DOMAIN_DELAY, SITE_PREFERENCES_MAX_CRAWL_DELAY,
    SITE_PREFERENCES_RESPECT_ROBOTS_TXT, SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
};
use goodfetch_protocol::page::{NAVIGATION_FAILED, PageId};
use goodfetch_protocol::settings::SESSION_PAGE_CHECK;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_session_pool::Effect;
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use crate::lease::{CommitError, WorkItemLease};
use crate::progress::SharedInFlight;
use crate::tier_choice::{self, AttemptPlan, Sampled};
use crate::tier_race::Raced;
use crate::work_items::Leased;

mod learning;
mod race;
mod site_preferences;
mod sitemap_fetch;

use learning::learn;
use site_preferences::{
    escaped_refusal, read_robots_txt, robots_txt_refusal, share_robots_txt, use_signals_refusal,
};

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
    /// The request ended `skipped` before its handler ran: the site's
    /// preferences refused it (its robots.txt, or an AI-use signal its
    /// response carries; ADRs 0086 and 0093), or a hop of its document's
    /// redirect chain left the scope, reached a key admitted for another
    /// request or was refused by the navigation gate (ADRs 0118 and 0120).
    Refused(SkipReason),
    /// Its origin's robots.txt answered 429 or 5xx, or no answer, with no
    /// earlier copy to apply, for the reason given: the request was not
    /// sent, and the failed fetch counts as one of its attempts (ADR 0086).
    RobotsTxtUnreachable(String),
    /// A sitemap's fetch read what its sitemap lists (ADR 0093): no
    /// handler ran.
    SitemapRead(sitemap::Document),
    /// The lease ended with no answer.
    LeaseEnded,
    /// The program's Connection ended.
    ClientGone,
}

/// The tier an attempt's request takes and what it is fetched with: the
/// Run's HTTP tier, the room its hedges take beside it, and the browser's
/// pages through the Run's `tiers`.
pub(super) struct Tiering {
    tiers: FetchTiers,
    http: Option<HttpTier>,
    plan: AttemptPlan,
    /// The Page check the request's Label declares of its own.
    page_check: Option<PageCheck>,
    second_copies: SecondCopies,
}

impl Tiering {
    /// `plan` on the Run's `tiers` and `http` tier, its hedges taking
    /// `second_copies`, the request's Label declaring `page_check`.
    pub(super) fn of(
        tiers: &FetchTiers,
        http: Option<&HttpTier>,
        plan: AttemptPlan,
        page_check: Option<PageCheck>,
        second_copies: &SecondCopies,
    ) -> Self {
        Self {
            tiers: tiers.clone(),
            http: http.cloned(),
            plan,
            page_check,
            second_copies: second_copies.clone(),
        }
    }

    /// The tier the attempt starts on, if any.
    pub(super) fn tier(&self) -> Option<FetchTier> {
        match &self.plan {
            AttemptPlan::Alone => None,
            AttemptPlan::On(choice) => Some(choice.tier),
            AttemptPlan::Sample(_) | AttemptPlan::Race(_) => Some(FetchTier::Http),
        }
    }
}

/// The tier a sample's cheaper run takes: `http`, chosen by no one.
const SAMPLED: TierChoice = TierChoice {
    tier: FetchTier::Http,
    forced: None,
};

/// The tier a race's browser-fetch serves its page on: `http`, sent from
/// inside the Session's browser, chosen by no one.
const RACED: TierChoice = SAMPLED;

/// The browser's tier, chosen by no one.
const BROWSER: TierChoice = TierChoice {
    tier: FetchTier::Browser,
    forced: None,
};

/// What an attempt's handler pages are gated with (the fetch tiers'
/// `gate`): the tiers that hand them over, room beside the Run's own
/// requests and how robots.txt is fetched; and the Page check of its
/// request's Label, which judges them in place of their Session's.
pub(super) struct Gating {
    pub(super) tiers: FetchTiers,
    pub(super) second_copies: SecondCopies,
    pub(super) robots: Option<RobotsTxtFetch>,
    /// The Run's domain policy.
    pub(super) egress: Arc<Policy>,
    /// The address check of the Run's HTTP tier, if it has one.
    pub(super) addresses: Option<AddressCheck>,
    pub(super) page_check: Option<PageCheck>,
}

impl Gating {
    /// What the attempt of `leased` holding `lease` gates its document's
    /// redirect hops with, then hands its Run's pages over with while its
    /// handler runs; `in_flight` names its waits.
    async fn hand_over(
        &self,
        run: &Run,
        leased: &Leased,
        lease: &WorkItemLease,
        in_flight: &SharedInFlight,
    ) -> Arc<HandOver> {
        let politeness = share_robots_txt(run).await;
        let settings = run.settings().map(|settings| settings.resolved().clone());
        let domain_delay = settings
            .as_ref()
            .and_then(|settings| settings.duration(POLITENESS_DOMAIN_DELAY.name))
            .unwrap_or_default();
        Arc::new(HandOver {
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
            adaptive_delay: settings.as_ref().is_some_and(|settings| {
                settings.boolean(POLITENESS_DOMAIN_ADAPTIVE_DELAY.name) == Some(true)
            }),
            pace: settings
                .as_ref()
                .map(goodfetch_frontier::Pace::of)
                .unwrap_or_default(),
            respect_robots_txt: settings.as_ref().is_none_or(|settings| {
                settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) != Some(false)
            }),
            max_crawl_delay: settings
                .as_ref()
                .and_then(|settings| settings.duration(SITE_PREFERENCES_MAX_CRAWL_DELAY.name)),
            lease: Arc::new(HandlerLease {
                lease: lease.clone(),
                run: run.clone(),
                request_id: leased.crawl_request.id.clone(),
                in_flight: Arc::clone(in_flight),
            }),
            egress: Arc::clone(&self.egress),
            addresses: self.addresses.clone(),
            page_check: self.page_check.clone(),
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
    /// The sample it was, with whether its two runs matched, once both
    /// answered.
    pub(super) sampled: Option<(Sampled, bool)>,
    /// The race it was, with its browser-fetch's latency, once its
    /// document arrived.
    pub(super) raced: Option<(Raced, Duration)>,
}

/// One attempt: the handler request, bounded by its lease, on the tier its
/// request takes. On `http` the request's document is fetched first and
/// the handler given a page served from it; a call that tier cannot serve
/// exactly under `auto` escalates the request to the browser, where the
/// handler runs again on Collie's page, neither a retry nor a failure, the
/// first answer discarded. On `browser` the engine opens Collie's page
/// for the handler, navigated to the request's URL through the gate. When
/// the lease ends first, the request is cancelled and its answer left to
/// [`late`].
pub(super) async fn attempt(
    run: Run,
    server: usize,
    context: CallContext,
    (leased, (session, in_browser), counted): (Leased, (ServingSession, PooledSession), Counted),
    lease: WorkItemLease,
    (tiering, gating): (Tiering, Gating),
    in_flight: SharedInFlight,
) -> Attempt {
    let on = On {
        run: &run,
        context: &context,
        leased: &leased,
        session: &session,
        in_browser: &in_browser,
        lease: &lease,
        gating: &gating,
        in_flight: &in_flight,
    };
    let mut merged = Vec::new();
    let mut sampled = None;
    let mut raced = None;
    let answer = match (&tiering.plan, &tiering.http) {
        (_, Some(http)) if leased.sitemap => on.sitemap(http).await,
        (_, None) if leased.sitemap => Answer::Failed {
            code: NAVIGATION_FAILED.name.to_owned(),
            retryable: true,
            message: "The crawl's HTTP tier could not be built, so the sitemap was not fetched."
                .to_owned(),
            details: Map::new(),
        },
        (AttemptPlan::Alone, _) => {
            let params = on.params(None, None);
            let hand_over = gating.hand_over(&run, &leased, &lease, &in_flight).await;
            let handed_over = gating.tiers.hand_over(hand_over);
            let answer = handled(&run, context.client(), &params, &lease).await;
            drop(handed_over);
            answer
        }
        (AttemptPlan::On(choice), _) if choice.tier == FetchTier::Browser => {
            on.browser(&tiering.tiers, choice, Chosen::AsChosen).await
        }
        (AttemptPlan::On(choice), Some(http)) => {
            let (answer, learned) = on.http(&tiering, http, choice).await;
            merged = learned;
            answer
        }
        (AttemptPlan::Sample(sample), Some(http)) => {
            let (answer, learned, matched) = on.sample(&tiering, http).await;
            merged = learned;
            sampled = matched.map(|matched| (sample.clone(), matched));
            answer
        }
        (AttemptPlan::Race(race), Some(http)) => {
            let (answer, learned, latency) = on.race(&tiering, http, race).await;
            merged = learned;
            raced = latency.map(|latency| (race.clone(), latency));
            answer
        }
        (AttemptPlan::On(_) | AttemptPlan::Sample(_) | AttemptPlan::Race(_), None) => {
            Answer::Failed {
                code: NAVIGATION_FAILED.name.to_owned(),
                retryable: true,
                message:
                    "The crawl's HTTP tier could not be built, so the request was not fetched."
                        .to_owned(),
                details: Map::new(),
            }
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
        sampled,
        raced,
    }
}

/// What a run on the `http` tier gave: the handler's answer and the call
/// that escalated its page, if one did.
struct HttpRun {
    answer: Answer,
    escalated: Option<&'static str>,
}

/// What one attempt runs its handler with.
struct On<'a> {
    run: &'a Run,
    context: &'a CallContext,
    leased: &'a Leased,
    session: &'a ServingSession,
    /// That Session as the browser tier opens its page in it.
    in_browser: &'a PooledSession,
    lease: &'a WorkItemLease,
    gating: &'a Gating,
    in_flight: &'a SharedInFlight,
}

impl On<'_> {
    /// The handler request of the attempt on `tier`, its page `page_id`.
    fn params(&self, tier: Option<FetchTier>, page_id: Option<PageId>) -> CrawlRequestHandleParams {
        CrawlRequestHandleParams {
            run_id: self.run.id().clone(),
            crawl_request: self.leased.crawl_request.clone(),
            attempt: self.leased.attempt,
            lease_epoch: self.leased.lease_epoch,
            session: self.session.clone(),
            tier,
            page_id,
        }
    }

    /// The attempt on the `http` tier: its run there ([`On::on_http`]),
    /// escalated to the browser at the first call the tier cannot serve
    /// under `auto`, in the same attempt, the first answer discarded.
    /// Answers how it ended and the pending requests the page's rules
    /// merged.
    async fn http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        choice: &TierChoice,
    ) -> (Answer, Vec<(String, String)>) {
        let (run, merged) = match self.on_http(tiering, http, choice, Chosen::AsChosen).await {
            Ok(ran) => ran,
            Err(ended) => return ended,
        };
        let answer = match run.escalated {
            // The first answer, whatever it says, is discarded.
            Some(call) if !matches!(run.answer, Answer::LeaseEnded | Answer::ClientGone) => {
                // The browser's run takes a whole lease of its own, and
                // stages anew: what the first run staged goes with it.
                self.lease.renew(self.lease.length());
                self.lease.restage();
                self.browser(&tiering.tiers, choice, Chosen::Escalated(call))
                    .await
            }
            _ => run.answer,
        };
        (answer, merged)
    }

    /// A sample of the crawl's sample comparison: its run on `http`, then,
    /// unless that run ended the attempt, its run on `browser` under a
    /// whole lease of its own, whose answer is the attempt's. Answers it,
    /// the pending requests the page's rules merged, and whether the two
    /// runs matched once the browser's answered with Records.
    async fn sample(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
    ) -> (Answer, Vec<(String, String)>, Option<bool>) {
        let (run, merged) = match self
            .on_http(tiering, http, &SAMPLED, Chosen::Sample(None))
            .await
        {
            Ok(ran) => ran,
            Err((answer, merged)) => return (answer, merged, None),
        };
        if matches!(run.answer, Answer::LeaseEnded | Answer::ClientGone) {
            return (run.answer, merged, None);
        }
        self.lease.renew(self.lease.length());
        self.lease.restage();
        let answer = self
            .browser(&tiering.tiers, &BROWSER, Chosen::Sample(run.escalated))
            .await;
        let matched = match (&answer, &run) {
            (
                Answer::Handled(CrawlRequestHandled {
                    records,
                    skipped: None,
                }),
                HttpRun {
                    answer:
                        Answer::Handled(CrawlRequestHandled {
                            records: cheaper,
                            skipped: None,
                        }),
                    escalated: None,
                },
            ) => Some(tier_choice::matches(Some(cheaper), records)),
            (
                Answer::Handled(CrawlRequestHandled {
                    records,
                    skipped: None,
                }),
                _,
            ) => Some(tier_choice::matches(None, records)),
            _ => None,
        };
        (answer, merged, matched)
    }

    /// One run on the `http` tier: its origin's robots.txt read and its
    /// document fetched under the lease, the Frontier taught its page, the
    /// tier recorded as `how` says, the page judged by its Label's Page
    /// check, else the Run's (ADR 0093), and the handler given a page
    /// served from it, closed once it answered. A page failing its expected
    /// page under `auto` escalates at `session.page_check` without its
    /// handler running. Answers the run and the pending requests the page's
    /// rules merged, or, when the attempt ended before its handler ran, its
    /// answer and those requests.
    async fn on_http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        choice: &TierChoice,
        how: Chosen,
    ) -> Result<(HttpRun, Vec<(String, String)>), (Answer, Vec<(String, String)>)> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let tiers = &tiering.tiers;
        let hand_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        // A Session that may move to the browser, under `auto`, sends its
        // HTTP from inside its browser, so its fingerprint and cookies stay
        // the Session's on both tiers (ADR 0020; pitfall H20).
        let in_browser;
        let http = if choice.forced.is_none() {
            in_browser = tiers.in_browser(http, run, self.in_browser, &hand_over, self.context);
            &in_browser
        } else {
            http
        };
        let page_id = self
            .document(tiering, http, choice, &hand_over)
            .await
            .map_err(|answer| (answer, Vec::new()))?;
        let merged = learn(run, tiers, &page_id, leased).await;
        record_tier(run, leased, choice, how).await;
        match judged(run, tiering, &page_id).await {
            Some(Judged::Passed) | None => {}
            Some(Judged::Failed(error)) => {
                tiers.close(&page_id);
                return Err((failed(&error), merged));
            }
            Some(Judged::Escalated) => {
                tiers.close(&page_id);
                // Its handler never ran: the run escalates at the Page
                // check, and the caller runs the request in the browser.
                let answer = Answer::Failed {
                    code: PAGE_CHECK_FAILED.to_owned(),
                    retryable: false,
                    message: "The page failed its expected page on the `http` tier.".to_owned(),
                    details: Map::new(),
                };
                return Ok((
                    HttpRun {
                        answer,
                        escalated: Some(PAGE_CHECK_CALL),
                    },
                    merged,
                ));
            }
        }
        self.in_flight
            .lock()
            .await
            .went_on(run, &leased.crawl_request.id, InFlightPhase::Handling);
        let params = self.params(Some(FetchTier::Http), Some(page_id.clone()));
        let handed_over = self.gating.tiers.hand_over(hand_over);
        let answer = handled(run, self.context.client(), &params, lease).await;
        drop(handed_over);
        let escalated = tiers.escalated(&page_id);
        tiers.close(&page_id);
        Ok((HttpRun { answer, escalated }, merged))
    }

    /// The document of the attempt's request fetched on `http` under the
    /// lease, served as `choice` says, once its origin's robots.txt was
    /// read, and opened as a served page, its pages gated with
    /// `hand_over`; one whose AI-use signals refuse the Run's Purpose is
    /// closed unhandled. Answers the page, or how the attempt ended.
    async fn document(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        choice: &TierChoice,
        hand_over: &Arc<HandOver>,
    ) -> Result<PageId, Answer> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let requester = Requester {
            request_id: &leased.crawl_request.id,
            session: &self.session.id,
            label: &leased.crawl_request.label,
            domain_key: &leased.waiting.key,
            second_copies: &tiering.second_copies,
            // No work item declares an Effect before area 9's Effect
            // journal (ADR 0106), and no crawl request is a Paid fetch
            // before ADR 0045's.
            declares_effect: false,
            paid: false,
        };
        let tiers = &tiering.tiers;
        let fetched = tokio::select! {
            biased;
            document = async {
                address_check(run, http, leased).await?;
                let read = read_robots_txt(run, http, leased).await;
                if let Some(refused) = robots_txt_refusal(run, leased, read.as_deref()) {
                    return Err(refused);
                }
                tiers
                    .serve(http, run, &leased.crawl_request, &requester, choice, hand_over)
                    .await
                    .map_err(|stop| match stop {
                        Stop::Failed(error) => failed(&error),
                        Stop::Skipped(skip) => {
                            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, reason = ?skip.reason, message = skip.message, "a redirect hop ended its request skipped");
                            Answer::Refused(skip.reason)
                        }
                    })
            } => document,
            _ = lease.ended() => Err(Answer::LeaseEnded),
        };
        let page_id = fetched?;
        if let Some(refused) = use_signals_refusal(run, tiers, &page_id) {
            tiers.close(&page_id);
            return Err(refused);
        }
        Ok(page_id)
    }

    /// The handler request of the attempt on `tier`, given the page
    /// `page_id`, bounded by its lease.
    async fn handle(&self, page_id: &PageId, tier: FetchTier) -> Answer {
        self.in_flight.lock().await.went_on(
            self.run,
            &self.leased.crawl_request.id,
            InFlightPhase::Handling,
        );
        let params = self.params(Some(tier), Some(page_id.clone()));
        handled(self.run, self.context.client(), &params, self.lease).await
    }

    /// The attempt on the `browser` tier: the Run's pages handed over,
    /// Collie's page opened and navigated to the request's URL through the
    /// gate under the lease, and the handler given it, closed once it
    /// answered. `how` says how the request came here: as chosen, as a
    /// sample's run, or by an escalation from a cheaper tier.
    async fn browser(&self, tiers: &FetchTiers, choice: &TierChoice, how: Chosen) -> Answer {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let hand_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        let handed_over = self.gating.tiers.hand_over(hand_over);
        let opened = tokio::select! {
            biased;
            opened = tiers.browser(run.id(), self.in_browser, &leased.crawl_request.url, self.context) => {
                opened.map_err(|error| failed(&error))
            }
            _ = lease.ended() => Err(Answer::LeaseEnded),
        };
        let page_id = match opened {
            Ok((page_id, _)) => page_id,
            Err(answer) => return answer,
        };
        let browser = TierChoice {
            tier: FetchTier::Browser,
            forced: choice.forced.clone().filter(|_| how == Chosen::AsChosen),
        };
        record_tier(run, leased, &browser, how).await;
        self.in_flight
            .lock()
            .await
            .went_on(run, &leased.crawl_request.id, InFlightPhase::Handling);
        let params = self.params(Some(FetchTier::Browser), Some(page_id.clone()));
        let answer = handled(run, self.context.client(), &params, lease).await;
        drop(handed_over);
        tiers.close_browser(&page_id).await;
        answer
    }
}

/// How the served page `page_id` fares against the Page check of its
/// request's Label, else the Run's (ADR 0093).
async fn judged(run: &Run, tiering: &Tiering, page_id: &PageId) -> Option<Judged> {
    let check = tiering.page_check.clone().or_else(|| run_page_check(run));
    match &check {
        Some(check) => tiering.tiers.judge(page_id, check).await,
        None => Some(Judged::Passed),
    }
}

/// The Run's own `session.page_check`, which judges the pages of a Label
/// that declares none (ADR 0093).
fn run_page_check(run: &Run) -> Option<PageCheck> {
    let settings = run.settings()?;
    let value = settings
        .resolved()
        .get(SESSION_PAGE_CHECK.name)?
        .value
        .clone();
    // The setting's shape was checked where it was set.
    serde_json::from_value::<Option<PageCheck>>(value).ok()?
}

/// Judges `leased`'s URL by the address check of `http`'s connections
/// before anything is sent for it, its robots.txt included (ADR 0017): a
/// host that stands for an address the Run's egress policy refuses ends
/// the request `skipped` with `domain_policy`, the policy's refusal and no
/// failure (spec #565).
async fn address_check(run: &Run, http: &HttpTier, leased: &Leased) -> Result<(), Answer> {
    let Ok(url) = goodfetch_frontier::Url::parse(&leased.crawl_request.url) else {
        return Ok(());
    };
    http.address_check().check(&url).await.map_err(|refused| {
        tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, %refused, "the address check refused a crawl request before anything was sent");
        Answer::Refused(SkipReason::DomainPolicy)
    })
}

/// An attempt that ended with `error` before its handler was asked.
fn failed(error: &CallError) -> Answer {
    Answer::Failed {
        code: error.code().to_owned(),
        retryable: error.retryable(),
        message: error.message().to_owned(),
        details: error.details().cloned().unwrap_or_default(),
    }
}

/// Records in the Run's Provenance the tier `choice` names for `leased`'s
/// attempt, and how it came there, as `how` says (ADR 0023).
async fn record_tier(run: &Run, leased: &Leased, choice: &TierChoice, how: Chosen) {
    goodfetch_fetch_tier::record_tier(
        run,
        &leased.crawl_request.id,
        &leased.crawl_request.label,
        choice,
        how,
    )
    .await;
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
        | Answer::SitemapRead(_)
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
