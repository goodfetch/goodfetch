//! One attempt of a crawl's delivery (ADRs 0004, 0006 and 0024): its
//! `crawl_request.handle` request to a serving Connection, bounded by its
//! lease, and what its answer is. When the lease ends first, the request
//! is cancelled with `$/cancel` and an answer that still comes is a late
//! commit, refused with `lease_lost`. Its answer also says what the Session
//! that served it scored: a success, a block or a failed Page check.
//!
//! Its request runs as the crawl's tier choice planned it (the
//! `tier_choice` module): alone, on one tier, or as a sample of the sample
//! comparison, run on `http`, on `light_browser` where the Run allows it,
//! and then on `browser`, the browser's run the one whose answer counts,
//! and whether the cheaper runs matched it classified once the attempt
//! settles. A run on `light_browser` is one on `http` whose page is served
//! from the document Lightpanda's run of the page's scripts left (ADR
//! 0020), escalated to the browser as `http`'s is.
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
//! recorded for every run sharing the domain, the lease renewed through
//! the wait as through a handler page's wait for a permit (ADR 0093). While
//! `site_preferences.respect_robots_txt` is on, a request robots.txt
//! disallows is never sent and ends `skipped` with `robots_txt`, before
//! any wait for its domain's delay; one whose robots.txt, or a redirect
//! hop's, cannot be read, with no earlier copy, is not sent either, the
//! failed fetch counting as one of its attempts and backing its domain
//! off, unless it is a hop's on another domain key, which says nothing of
//! the request's own; and it ends `skipped` with `robots_txt` once its
//! retries ran out.
//! While it is off, robots.txt is read only while
//! `site_preferences.mark_robots_txt_ignored` marks the Records taken
//! against it or the AI-use signals are honoured; otherwise it is neither
//! fetched, read nor stored (ADR 0138).
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
//!
//! Size: one attempt's way from its tier plan to its answer, each run on a
//! tier one method of [`On`], kept whole so every tier's run hands its
//! handler the same request, response and Provenance.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallContext;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::client::{ClientRequestError, Sent};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{
    Capture, Chosen, FetchFailure, FetchTiers, HandOver, HopChecks, HttpTier, Judged,
    PAGE_CHECK_CALL, PageFacts, PooledSession, Redirected, ReplayPlan, Request, Requester,
    RunHandOver, SitePreferences, Stop, TierChoice, Unfollowed, infer_plan, tier_serves,
};
use goodfetch_frontier::{DomainKeys, sitemap};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::foxhound::crawl::{
    ATTEMPT_ENDED, CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled,
    HANDLER_FAILED, LEASE_LOST, WORK_ITEM_LEASED,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::progress::InFlightPhase;
use goodfetch_protocol::foxhound::provenance::{CertificateExceptionMark, EncodingProvenance};
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_ADAPTIVE_DELAY, POLITENESS_DOMAIN_DELAY,
};
use goodfetch_protocol::page::{NAVIGATION_FAILED, PageId, PageResponse};
use goodfetch_protocol::settings::SESSION_PAGE_CHECK;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_session_pool::escalation::{self, Action, Met};
use goodfetch_session_pool::{Effect, SendsFrom};
use tokio::sync::{Mutex, oneshot};

use super::failure::{AttemptFailure, Why};
use super::watch::{self, Watch};
use crate::code_free::CodeFree;
use crate::lease::{CommitError, SoFar, WorkItemLease};
use crate::progress::SharedInFlight;
use crate::tier_choice::weighing::{self, StagedRun};
use crate::tier_choice::{AttemptPlan, Sampled, Weighed};
use crate::tier_race::Raced;
use crate::work_items::{Fetched, Leased};

mod decoy;
mod learning;
mod race;
mod site_preferences;
mod sitemap_fetch;

pub(super) use learning::{Taught, learns};
use learning::{learn, skipped};
use site_preferences::{escaped_refusal, share_robots_txt};

/// The Error code of a page the site refused, which Collie's page raises
/// with the Block cause in its details (ADRs 0035 and 0079), and the call
/// that moves a request to the browser when a Challenge escalates it.
const BLOCKED: &str = goodfetch_protocol::page::BLOCKED.name;

/// The Error code of a page that failed its Page check (ADR 0006), which
/// scores its Session down.
const PAGE_CHECK_FAILED: &str = goodfetch_protocol::page::PAGE_CHECK_FAILED.name;

/// How an attempt's document ended unserved: its answer, with the redirect
/// hops its chain followed before a hop ended it `skipped`, which still
/// teach (ADR 0120); none for any other ending.
pub(super) struct Unserved {
    pub(super) answer: Answer,
    pub(super) redirects: Arc<[Redirected]>,
}

impl From<Answer> for Unserved {
    fn from(answer: Answer) -> Self {
        Self {
            answer,
            redirects: Arc::new([]),
        }
    }
}

/// How an attempt ended, before its outcome is committed.
pub(super) enum Answer {
    /// The handler answered.
    Handled(CrawlRequestHandled),
    /// The handler failed, or answered no result.
    Failed(Box<AttemptFailure>),
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
    /// The same of a redirect hop on another domain key than the request's:
    /// counted alike, but it says nothing of the request's own domain.
    HopsRobotsTxtUnreachable(String),
    /// A sitemap's fetch read what its sitemap lists (ADR 0093): no
    /// handler ran.
    SitemapRead(sitemap::Document),
    /// The lease ended with no answer.
    LeaseEnded,
    /// The program's Connection ended.
    ClientGone,
}

impl Answer {
    /// The answer of an attempt that failed with `failure`.
    pub(super) fn failed(failure: AttemptFailure) -> Self {
        Self::Failed(Box::new(failure))
    }
}

/// The tier an attempt's request takes and what it is fetched with: the
/// Run's HTTP tier, the room its hedges take beside it, and the browser's
/// pages through the Run's `tiers`.
pub(super) struct Tiering {
    tiers: FetchTiers,
    http: Option<HttpTier>,
    plan: AttemptPlan,
    /// Whether a sample also runs on `light_browser`.
    light: bool,
    /// The Page check the request's Label declares of its own.
    page_check: Option<PageCheck>,
    second_copies: SecondCopies,
}

impl Tiering {
    /// `plan` on the Run's `tiers` and `http` tier, a sample also run on
    /// `light_browser` when `light`, its hedges taking `second_copies`,
    /// the request's Label declaring `page_check`.
    pub(super) fn of(
        tiers: &FetchTiers,
        http: Option<&HttpTier>,
        (plan, light): (AttemptPlan, bool),
        page_check: Option<PageCheck>,
        second_copies: &SecondCopies,
    ) -> Self {
        Self {
            tiers: tiers.clone(),
            http: http.cloned(),
            plan,
            light,
            page_check,
            second_copies: second_copies.clone(),
        }
    }

    /// The tier the attempt starts on, if any.
    pub(super) fn tier(&self) -> Option<FetchTier> {
        match &self.plan {
            AttemptPlan::Sitemap => None,
            AttemptPlan::On(choice) => Some(choice.tier),
            AttemptPlan::Replayed(_) => Some(FetchTier::DiscoveredApi),
            AttemptPlan::Sample(_) | AttemptPlan::Race(_) => Some(FetchTier::Http),
        }
    }
}

/// The tier a sample's cheaper run takes: `http`, chosen by no one.
const SAMPLED: TierChoice = TierChoice {
    tier: FetchTier::Http,
    forced: None,
};

/// The tier a sample's light run takes: `light_browser`, chosen by no
/// one.
const LIGHT_SAMPLED: TierChoice = TierChoice {
    tier: FetchTier::LightBrowser,
    forced: None,
};

/// The tier a sample's run replaying its browser's calls takes, and a
/// learned one's: `discovered_api`, chosen by no one.
const REPLAYED: TierChoice = TierChoice {
    tier: FetchTier::DiscoveredApi,
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

/// What every attempt of `run` hands over alike with its pages, built once
/// as its delivery opens, never per attempt (ADR 0138): its `settings` as
/// the navigation gate and the site's preferences read them, the
/// Politeness it shares, robots.txt and the AI-use signal files fetched on
/// `http`, the Run's HTTP tier, whose address check judges each redirect
/// hop, `egress`, its domain policy, and `hops`, how each redirect hop
/// asks the delivery loop of the visited set (ADR 0142).
pub(super) async fn run_hand_over(
    run: &Run,
    settings: &Resolved,
    http: Option<&HttpTier>,
    (egress, hops): (&Arc<Policy>, Arc<dyn HopChecks>),
) -> Arc<RunHandOver> {
    let site_preferences = SitePreferences::of(settings);
    // The Run reads robots.txt from the copy its user's runs share in the
    // Politeness file (ADR 0086); a crawl whose Politeness paces nothing
    // records no start, send or latency in it (ADR 0138).
    let paces = goodfetch_frontier::paces(settings);
    let shared = if paces
        || site_preferences.reads_robots_txt(Request::CrawlRequest)
        || site_preferences.reads_robots_txt(Request::Navigation)
    {
        share_robots_txt(run, settings).await
    } else {
        None
    };
    Arc::new(RunHandOver {
        run: run.clone(),
        keys: DomainKeys::of(settings),
        politeness: shared.clone().filter(|_| paces),
        shared_politeness: shared,
        robots: http.map(HttpTier::robots_txt_fetch),
        domain_delay: settings
            .get::<Option<Duration>>(&POLITENESS_DOMAIN_DELAY)
            .unwrap_or_default(),
        adaptive_delay: settings.get(&POLITENESS_DOMAIN_ADAPTIVE_DELAY),
        pace: goodfetch_frontier::Pace::of(settings),
        site_preferences,
        egress: Arc::clone(egress),
        hops,
        addresses: http.map(HttpTier::address_check),
    })
}

/// The lease check of the handler pages of the attempt of `leased`,
/// holding `lease` and watched by `watch` ([`HandlerLease`]), in the Run
/// `of_run` hands over; `in_flight` names its waits.
pub(super) fn handler_lease(
    of_run: &Arc<RunHandOver>,
    (leased, lease, watch): (&Leased, &WorkItemLease, &Watch),
    in_flight: &SharedInFlight,
) -> Arc<dyn LeaseCheck> {
    Arc::new(HandlerLease {
        lease: lease.clone(),
        watch: watch.clone(),
        of_run: Arc::clone(of_run),
        request_id: leased.crawl_request.id.clone(),
        in_flight: Arc::clone(in_flight),
    })
}

/// The lease check of an attempt's handler pages: the attempt's lease,
/// renewed while a page waits for a navigation gate's permit, the wait
/// named in the Run's live status, so a handler waiting for a permit is not
/// timed out meanwhile (ADR 0093). A panic in the renewal ends the
/// attempt ([`Watch`]). It shares its Run with every attempt's hand-over
/// rather than holding a clone of its own, which the loop would build and
/// the page's task drop once a page (#2243).
#[derive(Clone)]
struct HandlerLease {
    lease: WorkItemLease,
    watch: Watch,
    of_run: Arc<RunHandOver>,
    request_id: String,
    in_flight: SharedInFlight,
}

impl std::fmt::Debug for HandlerLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HandlerLease")
            .field("lease", &self.lease)
            .field("watch", &self.watch)
            .field("run", &self.of_run.run)
            .field("request_id", &self.request_id)
            .field("in_flight", &self.in_flight)
            .finish()
    }
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
        self.of_run.run.spawn(self.watch.over(async move {
            let id = &waiting.request_id;
            let run = &waiting.of_run.run;
            waiting.in_flight.lock().await.permit_wait_began(run, id);
            let (length, clock) = (waiting.lease.length(), run.clock());
            loop {
                waiting.lease.renew(length);
                tokio::select! {
                    _ = &mut stopped => break,
                    () = clock.sleep(length / RENEWALS_PER_LENGTH) => {}
                }
            }
            // The handler's timeout runs again from the wait's end.
            waiting.lease.renew(length);
            waiting.in_flight.lock().await.permit_wait_ended(run, id);
        }));
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
    /// What its page taught the Frontier, or teaches it with its outcome.
    pub(super) taught: Taught,
    /// The sample it was, with how its two runs weighed, once both
    /// answered.
    pub(super) sampled: Option<(Sampled, Weighed)>,
    /// The race it was, with its browser-fetch's latency, once its
    /// document arrived.
    pub(super) raced: Option<(Raced, Duration)>,
    /// The site's answer to its request's own document, when the engine
    /// fetched it, on any tier: its domain's Politeness reads it whatever
    /// the handler made of the page (the `backoff_classes` rule).
    pub(super) document: Option<PageResponse>,
    /// When its handler request last went out: the moment a handler's
    /// failure measures the time its site asked for against, so the time
    /// the handler and the engine took to report it never counts against
    /// the site's wait (ADR 0118). `None` when no handler request went.
    pub(super) handed_at: Option<SystemTime>,
    /// Whether a Challenge met on a tier short of the browser moved it to
    /// the browser, which its domain then learns (`escalation`).
    pub(super) moved: bool,
}

/// One attempt: the handler request, bounded by its lease, on the tier its
/// request takes. On `http` the request's document is fetched first and
/// the handler given a page served from it; a call that tier cannot serve
/// exactly under `auto` escalates the request to the browser, where the
/// handler runs again on Collie's page, neither a retry nor a failure, the
/// first answer discarded. On `browser` the engine opens Collie's page
/// for the handler, navigated to the request's URL through the gate. When
/// the lease ends first, the request is cancelled and its answer left to
/// [`late`]. A panic in the attempt, or in a task it started while it
/// runs, ends it failed with `engine_panicked` ([`Watch`]); what it holds
/// is given back as for any other end.
pub(super) async fn attempt(
    run: Run,
    server: usize,
    context: CallContext,
    (leased, (session, in_browser), counted): (Leased, (ServingSession, PooledSession), Counted),
    (lease, watch): (WorkItemLease, Watch),
    (tiering, (hand_over, code_free)): (Tiering, (Arc<HandOver>, Option<Arc<CodeFree>>)),
    in_flight: SharedInFlight,
) -> Attempt {
    let document = Mutex::new(None);
    let sent_at = Mutex::new(None);
    let moved = OnceLock::new();
    let on = On {
        document: &document,
        sent_at: &sent_at,
        moved: &moved,
        run: &run,
        context: &context,
        leased: &leased,
        session: &session,
        in_browser: &in_browser,
        http: tiering.http.as_ref(),
        lease: &lease,
        hand_over: &hand_over,
        code_free: code_free.as_deref(),
        in_flight: &in_flight,
        watch: &watch,
    };
    // Boxed: the attempt's future is large, and the watch holds it only
    // while it runs.
    let ran = watch
        .ended(Box::pin(async {
            let mut merged = Taught::default();
            let mut sampled = None;
            let mut raced = None;
            let answer = match (&tiering.plan, &tiering.http) {
                (AttemptPlan::Sitemap, Some(http)) => on.sitemap(http).await,
                (AttemptPlan::Sitemap, None) => Answer::failed(AttemptFailure::named(
                    NAVIGATION_FAILED.name,
                    true,
                    "The crawl's HTTP tier could not be built, so the sitemap was not fetched."
                        .to_owned(),
                )),
                (AttemptPlan::On(choice), _) if choice.tier == FetchTier::Browser => {
                    on.browser(&tiering.tiers, choice, Chosen::AsChosen).await
                }
                (AttemptPlan::On(choice), Some(http)) => {
                    let (answer, learned) = on.http(&tiering, http, choice, None).await;
                    merged = learned;
                    answer
                }
                (AttemptPlan::Replayed(plan), Some(http)) => {
                    let (answer, learned) = on.http(&tiering, http, &REPLAYED, Some(plan)).await;
                    merged = learned;
                    answer
                }
                (AttemptPlan::Sample(sample), Some(http)) => {
                    let (answer, learned, weighed) = Box::pin(on.sample(&tiering, http)).await;
                    merged = learned;
                    sampled = weighed.map(|weighed| (sample.clone(), weighed));
                    answer
                }
                (AttemptPlan::Race(race), Some(http)) => {
                    let (answer, learned, latency) = on.race(&tiering, http, race).await;
                    merged = learned;
                    raced = latency.map(|latency| (race.clone(), latency));
                    answer
                }
                (
                    AttemptPlan::On(_)
                    | AttemptPlan::Replayed(_)
                    | AttemptPlan::Sample(_)
                    | AttemptPlan::Race(_),
                    None,
                ) => Answer::failed(AttemptFailure::named(
                    NAVIGATION_FAILED.name,
                    true,
                    "The crawl's HTTP tier could not be built, so the request was not fetched."
                        .to_owned(),
                )),
            };
            (answer, merged, sampled, raced)
        }))
        .await;
    let (answer, taught, sampled, raced, ran) = match ran {
        Ok((answer, merged, sampled, raced)) => (answer, merged, sampled, raced, true),
        // What the attempt met before it panicked says nothing of its site:
        // its domain neither backs off nor learns from it.
        Err(panicked) => (
            Answer::failed(AttemptFailure::panicked(&panicked)),
            Taught::default(),
            None,
            None,
            false,
        ),
    };
    Attempt {
        document: document.into_inner().filter(|_| ran),
        handed_at: sent_at.into_inner().filter(|_| ran),
        moved: ran && moved.into_inner().is_some(),
        server,
        leased,
        _counted: counted,
        session,
        lease,
        answer,
        taught,
        sampled,
        raced,
    }
}

/// What a run on a document tier finds of its served page before the
/// Frontier learns from it or its handler runs, in the one order every
/// run checks it: a document only a guess could read first (ADR 0119), then
/// a Challenge the Block classification finds; each caller then runs the
/// Page check on a page that is neither ([`judged`]).
enum Found {
    /// The document declares no encoding and is not UTF-8: the call that
    /// escalates it to the browser, which reads it, under `auto`.
    Unreadable(&'static str),
    /// A Challenge of this cause, which checks the browser: under `auto`
    /// the request moves there, and under a forced tier it ends with this
    /// `blocked` (`escalation`). Other Block causes still reach the handler
    /// until the proxy-tier escalation ends them (#1137).
    Challenge(BlockCause, CallError),
    /// Neither: the Page check comes next.
    Readable,
}

/// A page a tier served for an attempt, held until its attempt closes it
/// ([`Served::close`]): dropped before, as the future that holds it is when
/// a race's navigation wins or its attempt is abandoned, it closes on its
/// tiers, so no served page outlives its attempt, nor holds its Run's
/// handles past the Run's end.
struct Served<'t> {
    tiers: &'t FetchTiers,
    /// The page, until it is closed.
    page_id: Option<PageId>,
}

impl Served<'_> {
    /// The page's ID.
    fn page_id(&self) -> &PageId {
        self.page_id
            .as_ref()
            .expect("invariant: a served page is held until it is closed")
    }

    /// Closes the page once its handler answered, as [`FetchTiers::close`]
    /// does, and answers what it held as it closed.
    fn close(mut self) -> Option<PageFacts> {
        let page_id = self
            .page_id
            .take()
            .expect("invariant: a served page is closed once");
        self.tiers.close(&page_id)
    }
}

impl Drop for Served<'_> {
    fn drop(&mut self) {
        if let Some(page_id) = self.page_id.take() {
            self.tiers.close(&page_id);
        }
    }
}

/// What a run on the `http` tier gave: the handler's answer, the call
/// that escalated its page, if one did, whether its page held a document,
/// the encoding it was read in (ADR 0119) and the URL its document came
/// from, its redirects followed (ADR 0118).
struct HttpRun {
    answer: Answer,
    escalated: Option<&'static str>,
    document: bool,
    encoding: Option<String>,
    url: Option<String>,
}

/// What a run on the `browser` tier gave: the handler's answer, the
/// encoding its document was read in, `document.characterSet`, and the URL
/// of the last main-frame document its page reached.
struct BrowserRun {
    answer: Answer,
    encoding: Option<String>,
    ended_at: Option<String>,
}

/// What one attempt runs its handler with.
struct On<'a> {
    run: &'a Run,
    context: &'a CallContext,
    leased: &'a Leased,
    session: &'a ServingSession,
    /// That Session as the browser tier opens its page in it.
    in_browser: &'a PooledSession,
    /// The Run's HTTP tier, when it could be built, whose address check
    /// judges the request's URL before a browser tier opens it.
    http: Option<&'a HttpTier>,
    lease: &'a WorkItemLease,
    /// What the attempt hands over with its Run's pages, built once as it
    /// was leased: each of its runs on a tier takes it whole.
    hand_over: &'a Arc<HandOver>,
    /// What a code-free crawl handles its requests with, in place of the
    /// program's handler; none when the program handles them.
    code_free: Option<&'a CodeFree>,
    in_flight: &'a SharedInFlight,
    /// The attempt's watch, over each task it starts on its Run while it
    /// runs.
    watch: &'a Watch,
    /// The site's answer to the request's own document, as the last run
    /// that fetched it met it, which its domain's Politeness reads (the
    /// `backoff_classes` rule).
    document: &'a Mutex<Option<PageResponse>>,
    /// When its handler request last went out ([`Attempt::handed_at`]).
    sent_at: &'a Mutex<Option<SystemTime>>,
    /// Whether a Challenge met on a tier short of the browser moved the
    /// request to the browser, set once it did.
    moved: &'a OnceLock<()>,
}

impl On<'_> {
    /// What the crawl does with the request, run as `choice` says, whose
    /// Session met a block of `cause`, the Block classification's or one
    /// the program's code named alike (`escalation`): a move to the
    /// browser is noted, for its domain to learn.
    fn on_block(&self, cause: BlockCause, choice: &TierChoice) -> Action {
        let action = escalation::on_block(
            cause,
            Met {
                tier: Some(choice.tier),
                forced: choice.forced.is_some(),
            },
        );
        if action == Action::MoveToBrowser {
            // Set once: a second move says nothing more.
            let _ = self.moved.set(());
        }
        action
    }

    /// The call that escalated a served page, as it closed once its
    /// handler answered (`closed`), if one did: `blocked` names a Challenge
    /// or a captcha the handler's own navigation met under `auto`, which
    /// moved the request to the browser, a move noted as [`On::on_block`]
    /// notes one, for its domain to learn (spec #600).
    fn escalated(&self, closed: Option<PageFacts>) -> Option<&'static str> {
        let escalated = closed.and_then(|facts| facts.escalated);
        if escalated == Some(BLOCKED) {
            // Set once: a second move says nothing more.
            let _ = self.moved.set(());
        }
        escalated
    }

    /// Whether `answer`, a handler's on a run as `choice` says, names a
    /// block that moves the request to the browser ([`On::on_block`]).
    fn moves(&self, answer: &Answer, choice: &TierChoice) -> bool {
        match answer {
            Answer::Failed(failure) => failure
                .block_cause()
                .is_some_and(|cause| self.on_block(cause, choice) == Action::MoveToBrowser),
            _ => false,
        }
    }

    /// The handler request of the attempt on `tier`, its page `page_id`,
    /// its request's own document answered `response`.
    fn params(
        &self,
        tier: Option<FetchTier>,
        page_id: Option<PageId>,
        response: Option<PageResponse>,
    ) -> CrawlRequestHandleParams {
        CrawlRequestHandleParams {
            run_id: self.run.id().clone(),
            crawl_request: self.leased.crawl_request.clone(),
            attempt: self.leased.attempt,
            lease_epoch: self.leased.lease_epoch,
            session: self.session.clone(),
            tier,
            page_id,
            response,
        }
    }

    /// The attempt on the `http` tier: its run there ([`On::on_http`]),
    /// escalated to the browser at the first call the tier cannot serve
    /// under `auto`, in the same attempt, the first answer discarded.
    /// Answers how it ended and the pending requests the page's rules
    /// merged. On `discovered_api`, its page replays `plan`'s calls.
    async fn http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        choice: &TierChoice,
        plan: Option<&ReplayPlan>,
    ) -> (Answer, Taught) {
        let (run, merged) = match self
            .on_http(tiering, http, (choice, plan), Chosen::AsChosen)
            .await
        {
            Ok(ran) => ran,
            Err(ended) => return ended,
        };
        let answer = match run.escalated {
            // The first answer, whatever it says, is discarded.
            Some(call) if !matches!(run.answer, Answer::LeaseEnded | Answer::ClientGone) => {
                // The browser's run takes a whole lease of its own, and
                // stages anew: what the first run staged goes with it.
                self.lease.renew(self.lease.length());
                drop(self.lease.restage());
                self.browser(&tiering.tiers, choice, Chosen::Escalated(call))
                    .await
            }
            // A Challenge the program's code named moves as one the rules
            // found does.
            None if self.moves(&run.answer, choice) => {
                self.lease.renew(self.lease.length());
                drop(self.lease.restage());
                self.browser(&tiering.tiers, choice, Chosen::Escalated(BLOCKED))
                    .await
            }
            _ => run.answer,
        };
        (answer, merged)
    }

    /// A sample of the crawl's sample comparison: its run on `http`, then,
    /// unless that run ended the attempt, its run on `browser` under a
    /// whole lease of its own, whose answer is the attempt's, matched or
    /// not. Answers it, the pending requests the page's rules merged, and
    /// how the two runs weighed once the browser's answered: what each run
    /// staged, the Records it pushed then those of its answer and the
    /// requests it enqueued, before any Hook (ADR 0103), the URL each ended
    /// on and the encoding each read in (ADRs 0118 and 0119). A browser run
    /// that met a consent wall keeps its Label on `browser` (ADR 0048).
    async fn sample(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
    ) -> (Answer, Taught, Option<Weighed>) {
        let (run, merged) = match self
            .on_http(tiering, http, (&SAMPLED, None), Chosen::Sample(None))
            .await
        {
            Ok(ran) => ran,
            Err((answer, merged)) => return (answer, merged, None),
        };
        // A page holding no document has nothing to compare, and the
        // browser would hold none either (ADR 0119): its answer stands.
        if matches!(run.answer, Answer::LeaseEnded | Answer::ClientGone) || !run.document {
            return (run.answer, merged, None);
        }
        // A Challenge its handler named moves the domain to the browser,
        // where the sample's browser run then serves the request.
        self.moves(&run.answer, &SAMPLED);
        self.lease.renew(self.lease.length());
        let cheaper = self.lease.restage();
        // Where the Run allows it, the sample runs on `light_browser` too,
        // before the browser, whose run stages what commits.
        let light = if tiering.light {
            match self
                .on_http(tiering, http, (&LIGHT_SAMPLED, None), Chosen::Sample(None))
                .await
            {
                Ok((light, _))
                    if matches!(light.answer, Answer::LeaseEnded | Answer::ClientGone) =>
                {
                    return (light.answer, merged, None);
                }
                Ok((light, _)) => {
                    self.lease.renew(self.lease.length());
                    Some((light, self.lease.restage()))
                }
                Err((Answer::LeaseEnded, _)) => return (Answer::LeaseEnded, merged, None),
                Err((Answer::ClientGone, _)) => return (Answer::ClientGone, merged, None),
                Err(_) => {
                    self.lease.renew(self.lease.length());
                    drop(self.lease.restage());
                    None
                }
            }
        } else {
            None
        };
        let capture = Capture::under(Limits::of_run(self.run.settings()));
        let browser = self
            .browser_run(
                &tiering.tiers,
                &BROWSER,
                Chosen::Sample(run.escalated),
                Some(&capture),
            )
            .await;
        let browser_staged = self.lease.pushed();
        let weighed = match &browser.answer {
            Answer::Failed(failure)
                if failure.block_cause() == Some(BlockCause::ConsentRequired) =>
            {
                Some(Weighed::Walled)
            }
            Answer::Handled(CrawlRequestHandled {
                records,
                skipped: None,
            }) => {
                let browser_records =
                    [browser_staged.records.records(), records.as_slice()].concat();
                let not_compared = self.not_compared();
                let browser_run = StagedRun {
                    records: &browser_records,
                    enqueues: &browser_staged.enqueues,
                    ended_at: browser.ended_at.as_deref(),
                    encoding: browser.encoding.as_deref(),
                };
                if matches_browser(&run, &cheaper, &browser_run, not_compared) {
                    Some(Weighed::Matched)
                } else {
                    let light = light.as_ref().is_some_and(|(light, staged)| {
                        matches_browser(light, staged, &browser_run, not_compared)
                    });
                    // `discovered_api` serves the document `http` did, so
                    // its run can only differ from that one where `http`
                    // escalated at a call only it serves: a network wait.
                    let replayed = match run.escalated {
                        Some(call)
                            if tier_serves(call, FetchTier::DiscoveredApi)
                                && !tier_serves(call, FetchTier::Http) =>
                        {
                            Box::pin(self.replayed(tiering, http, &capture, &browser_run)).await
                        }
                        _ => None,
                    };
                    Some(match replayed {
                        Some(plan) => Weighed::MatchedApi { plan, light },
                        None if light => Weighed::MatchedLight,
                        None => Weighed::Differed,
                    })
                }
            }
            _ => None,
        };
        (browser.answer, merged, weighed)
    }

    /// A sample's run on `discovered_api`, after its browser's, whose
    /// staging is set aside meanwhile and kept to commit: the calls the
    /// browser's page made, `capture`, read as a plan and replayed for
    /// the same request, its staging then discarded (ADRs 0020 and 0103).
    /// Answers the plan when the run staged what the browser's run
    /// `browser` did; `None` when the calls follow no plan or the run
    /// differed, escalated or failed.
    async fn replayed(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        capture: &Capture,
        browser: &StagedRun<'_>,
    ) -> Option<ReplayPlan> {
        let document = goodfetch_frontier::Url::parse(&self.leased.crawl_request.url).ok()?;
        let calls = capture.calls();
        let plan = infer_plan(&document, &calls);
        tracing::debug!(run_id = %self.run.id(), work_item = %self.leased.crawl_request.id, calls = calls.len(), ?plan, "a sample's browser calls were read as a plan");
        let plan = plan?;
        let browsers = self.lease.set_aside()?;
        self.lease.renew(self.lease.length());
        let ran = self
            .on_http(
                tiering,
                http,
                (&REPLAYED, Some(&plan)),
                Chosen::Sample(None),
            )
            .await;
        let staged = self.lease.restage();
        self.lease.put_back(browsers);
        let (run, _) = ran.ok()?;
        matches_browser(&run, &staged, browser, self.not_compared()).then_some(plan)
    }

    /// The fields of a Record that two runs of one attempt never hold
    /// alike, which a sample's comparison leaves out: a code-free crawl's
    /// ([`crate::code_free::NOT_COMPARED`]), none of a handler's.
    fn not_compared(&self) -> &'static BTreeSet<String> {
        static NONE: BTreeSet<String> = BTreeSet::new();
        if self.code_free.is_some() {
            &crate::code_free::NOT_COMPARED
        } else {
            &NONE
        }
    }

    /// One run on the `http` tier: its origin's robots.txt read and its
    /// document fetched under the lease, the tier recorded as `how` says,
    /// the page judged by its Label's Page check, else the Run's (ADR
    /// 0093), the Frontier then taught its page unless the run is a
    /// sample's, and the handler given a page
    /// served from it, closed once it answered. A page failing its expected
    /// page under `auto` escalates at `session.page_check` without its
    /// handler running. Answers the run and the pending requests the page's
    /// rules merged, or, when the attempt ended before its handler ran, its
    /// answer and those requests.
    async fn on_http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        (choice, plan): (&TierChoice, Option<&ReplayPlan>),
        how: Chosen,
    ) -> Result<(HttpRun, Taught), (Answer, Taught)> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let tiers = &tiering.tiers;
        let hand_over = self.hand_over;
        // The client the Session was drawn for (`tier_choice::sends_from`).
        let http = &match crate::tier_choice::sends_from(choice) {
            SendsFrom::Browser => {
                tiers.in_browser(http, run, self.in_browser, hand_over, self.context)
            }
            SendsFrom::Transport => http.keeping_cookies_of(&self.session.id),
        };
        let (served, facts) = self
            .document(tiering, http, choice, hand_over)
            .await
            .map_err(|Unserved { answer, redirects }| {
                // A sample's runs teach nothing, as [`Self::taught`] says.
                let taught = if matches!(how, Chosen::Sample(_)) {
                    Taught::default()
                } else {
                    skipped(run, leased, redirects)
                };
                (answer, taught)
            })?;
        // `served` is held until its handler answered: an attempt abandoned
        // while its handler runs, as an abort abandons it, closes its page
        // as it drops, so the page never holds the Run's handles past the
        // Run.
        let page_id = served.page_id().clone();
        *self.document.lock().await = facts.response.clone();
        // A `discovered_api` page replays its Label's calls (ADR 0020).
        if let Some(plan) = plan {
            tiers.replay_on(&page_id, plan.clone());
        }
        // Provenance names the tier that sent the document, whoever chose
        // it (ADR 0023).
        let sent = http.sent_on(choice.tier);
        if let Some(ended) = self
            .ended_unhandled(tiering, &page_id, &facts, (choice, sent, how))
            .await
        {
            return ended;
        }
        let merged = self.taught(tiers, &page_id, how).await;
        self.in_flight
            .lock()
            .await
            .went_on(run, &leased.crawl_request.id, InFlightPhase::Handling);
        let PageFacts {
            response,
            not_document,
            encoding,
            ..
        } = facts;
        let url = response.as_ref().map(|response| response.url.clone());
        let params = self.params(Some(choice.tier), Some(page_id.clone()), response);
        let handed_over = tiers.hand_over(Arc::clone(hand_over));
        let answer = handled(
            run,
            self.context,
            self.code_free,
            (&params, self.sent_at),
            lease,
        )
        .await;
        drop(handed_over);
        let escalated = self.escalated(served.close());
        Ok((
            HttpRun {
                answer,
                escalated,
                document: not_document.is_none(),
                encoding: encoding.map(|encoding| encoding.encoding),
                url,
            },
            merged,
        ))
    }

    /// What the run `how` names, its page `page_id` served and past its
    /// Page check, teaches the Frontier ([`learn`]), answering the pending
    /// requests a rule it taught merged. Only a run that may commit
    /// teaches: a sample's runs on a cheaper tier are discarded for the
    /// browser's, whatever they find, so they teach nothing (ADR 0020's
    /// #768 note).
    async fn taught(&self, tiers: &FetchTiers, page_id: &PageId, how: Chosen) -> Taught {
        if matches!(how, Chosen::Sample(_)) {
            return Taught::default();
        }
        learn(
            self.run,
            tiers,
            page_id,
            self.leased,
            self.code_free.is_some(),
        )
        .await
    }

    /// How the run `how` names on `choice` ends before its handler runs,
    /// its page `page_id` served with `facts`, by what it found of the page
    /// ([`Found`]), the page closed; `None` when its handler runs. Records
    /// the tier that sent the run's document either way, `sent`.
    async fn ended_unhandled(
        &self,
        tiering: &Tiering,
        page_id: &PageId,
        facts: &PageFacts,
        (choice, sent, how): (&TierChoice, FetchTier, Chosen),
    ) -> Option<Result<(HttpRun, Taught), (Answer, Taught)>> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let recorded = &TierChoice {
            tier: sent,
            ..choice.clone()
        };
        let tiers = &tiering.tiers;
        match self.found(tiering, page_id, facts).await {
            Found::Unreadable(call) => {
                tiers.close(page_id);
                record_tier(run, leased, lease, recorded, how, None, None).await;
                let message = "The document declares no encoding and is not UTF-8: only the browser reads it.";
                return Some(Ok((
                    escalated_unhandled(ATTEMPT_ENDED.name, message, call),
                    Taught::default(),
                )));
            }
            Found::Challenge(cause, error) => {
                tiers.close(page_id);
                record_tier(run, leased, lease, recorded, how, None, None).await;
                return Some(match self.on_block(cause, choice) {
                    Action::MoveToBrowser => Ok((
                        escalated_unhandled(
                            BLOCKED,
                            "A Challenge met on a tier short of the browser moved the request there.",
                            BLOCKED,
                        ),
                        Taught::default(),
                    )),
                    Action::Blocked => Err((failed(&error), Taught::default())),
                });
            }
            Found::Readable => {}
        }
        record_tier(
            run,
            leased,
            lease,
            recorded,
            how,
            facts.encoding.clone(),
            facts.certificate_exception.clone(),
        )
        .await;
        match judged(run, tiering, page_id).await {
            Some(Judged::Passed) | None => None,
            Some(Judged::Failed(error)) => {
                tiers.close(page_id);
                Some(Err((failed(&error), Taught::default())))
            }
            Some(Judged::Escalated) => {
                tiers.close(page_id);
                // Its handler never ran: the run escalates at the Page
                // check, and the caller runs the request in the browser.
                let message = "The page failed its expected page on a tier short of the browser.";
                Some(Ok((
                    escalated_unhandled(PAGE_CHECK_FAILED, message, PAGE_CHECK_CALL),
                    Taught::default(),
                )))
            }
        }
    }

    /// What a run finds of its served page `page_id`, whose `facts` it
    /// read as it was served, before the Frontier learns from it or its
    /// handler runs, checked in one order on every document tier, a tier
    /// race's browser-fetch included ([`Found`]).
    async fn found(&self, tiering: &Tiering, page_id: &PageId, facts: &PageFacts) -> Found {
        if let Some(call) = facts.escalated {
            return Found::Unreadable(call);
        }
        if let Some((block, error)) = tiering
            .tiers
            .block(page_id)
            .await
            .filter(|(block, _)| block.checks_the_browser())
        {
            return Found::Challenge(block.cause, error);
        }
        Found::Readable
    }

    /// The document of the attempt's request fetched on `http` under the
    /// lease, served as `choice` says, once its origin's robots.txt was
    /// read, and opened as a served page, its pages gated with
    /// `hand_over`; one whose AI-use signals refuse the Run's Purpose is
    /// closed unhandled. Answers the page, held from the moment it was
    /// served so that dropping the future closes it, with what it holds of
    /// its document, read once, or how the attempt ended.
    async fn document<'t>(
        &self,
        tiering: &'t Tiering,
        http: &HttpTier,
        choice: &TierChoice,
        hand_over: &Arc<HandOver>,
    ) -> Result<(Served<'t>, PageFacts), Unserved> {
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
                if let Ok(url) = goodfetch_frontier::Url::parse(&leased.crawl_request.url) {
                    self.site_preferences_judged(http, Request::CrawlRequest, &url)
                        .await?;
                }
                let served: Result<PageId, Unfollowed> = if choice.tier == FetchTier::LightBrowser {
                    let identity = self.context.identity();
                    let pinned_by = PinnedBy {
                        product: identity.product,
                        product_version: identity.product_version.clone(),
                    };
                    tiers
                        .light(
                            http,
                            run,
                            &leased.crawl_request,
                            choice,
                            hand_over,
                            (self.context.engine().home(), &pinned_by),
                        )
                        .await
                        .map_err(Unfollowed::from)
                } else {
                    tiers
                        .serve(http, run, &leased.crawl_request, &requester, choice, hand_over)
                        .await
                };
                served.map_err(|Unfollowed { stop, redirects }| match stop {
                    Stop::Failed(failure) => fetch_failed(&failure).into(),
                    Stop::RobotsTxtUnreachable {
                        message,
                        on_own_domain: true,
                    } => Answer::RobotsTxtUnreachable(message).into(),
                    Stop::RobotsTxtUnreachable {
                        message,
                        on_own_domain: false,
                    } => Answer::HopsRobotsTxtUnreachable(message).into(),
                    // A failed chain's hops are followed again by its
                    // retry; a skipped request's are taught now (ADR 0120).
                    Stop::Skipped(skip) => {
                        tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, reason = ?skip.reason, message = skip.message, "a redirect hop ended its request skipped");
                        Unserved {
                            answer: Answer::Refused(skip.reason),
                            redirects,
                        }
                    }
                })
            } => document,
            _ = lease.ended() => Err(Answer::LeaseEnded.into()),
        };
        let served = Served {
            tiers,
            page_id: Some(fetched?),
        };
        let facts = tiers
            .facts(served.page_id())
            .expect("invariant: a served page is open while it is held");
        if let Some(refused) = self.document_refusal(http, &facts.use_signals).await {
            return Err(refused.into());
        }
        Ok((served, facts))
    }

    /// The handler request of the attempt on `tier`, given the page
    /// `page_id`, whose request's own document answered `response`,
    /// bounded by its lease.
    async fn handle(
        &self,
        page_id: &PageId,
        tier: FetchTier,
        response: Option<PageResponse>,
    ) -> Answer {
        self.in_flight.lock().await.went_on(
            self.run,
            &self.leased.crawl_request.id,
            InFlightPhase::Handling,
        );
        let params = self.params(Some(tier), Some(page_id.clone()), response);
        handled(
            self.run,
            self.context,
            self.code_free,
            (&params, self.sent_at),
            self.lease,
        )
        .await
    }

    /// The attempt on the `browser` tier: the Run's pages handed over,
    /// Collie's page opened and navigated to the request's URL through the
    /// gate under the lease, and the handler given it, closed once it
    /// answered. `how` says how the request came here: as chosen, as a
    /// sample's run, or by an escalation from a cheaper tier.
    async fn browser(&self, tiers: &FetchTiers, choice: &TierChoice, how: Chosen) -> Answer {
        self.browser_run(tiers, choice, how, None).await.answer
    }

    /// [`On::browser`], answering also the encoding the browser read the
    /// request's own document in, its `document.characterSet`; its page's
    /// JSON calls are captured into `capture`, when given.
    async fn browser_run(
        &self,
        tiers: &FetchTiers,
        choice: &TierChoice,
        how: Chosen,
        capture: Option<&Capture>,
    ) -> BrowserRun {
        let ended = |answer| BrowserRun {
            answer,
            encoding: None,
            ended_at: None,
        };
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        // The address check judges the request's URL before the browser
        // opens anything, as on the HTTP tier (spec #565).
        if let Some(http) = self.http {
            let checked = tokio::select! {
                biased;
                checked = address_check(run, http, leased) => checked,
                _ = lease.ended() => Err(Answer::LeaseEnded),
            };
            if let Err(answer) = checked {
                return ended(answer);
            }
        }
        // A sample's `http` run was already sent: its browser run is
        // weighed like any other, so the request counts toward its Label's
        // choice (ADR 0035's #1153 note).
        if !matches!(how, Chosen::Sample(_))
            && let Some(decoy) = reached_through_trap(leased)
        {
            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "a page reached through a suspected trap link is a decoy on the browser tier");
            return ended(decoy);
        }
        let hand_over = match capture {
            Some(capture) => Arc::new(self.hand_over.capturing(capture.clone())),
            None => Arc::clone(self.hand_over),
        };
        let handed_over = tiers.hand_over(Arc::clone(&hand_over));
        let opened = tokio::select! {
            biased;
            opened = tiers.browser(&hand_over, self.in_browser, &leased.crawl_request.url, self.context) => {
                opened.map_err(|error| failed(&error))
            }
            _ = lease.ended() => Err(Answer::LeaseEnded),
        };
        let (page_id, navigated) = match opened {
            Ok(opened) => opened,
            Err(answer) => return ended(answer),
        };
        if let Some(refused) = refused_type(tiers, &page_id, run, leased).await {
            return ended(refused);
        }
        let browser = TierChoice {
            tier: FetchTier::Browser,
            forced: choice.forced.clone().filter(|_| how == Chosen::AsChosen),
        };
        let encoding = read_by_browser(tiers, &page_id, navigated.response.as_ref());
        self.document.lock().await.clone_from(&navigated.response);
        // A document the Session's egress relay reached under an
        // invalid-certificate exception marks its Records (ADR 0042).
        let certificate_exception = match &navigated.response {
            Some(response) => tiers.browser_exception(&page_id, &response.url).await,
            None => None,
        };
        record_tier(
            run,
            leased,
            lease,
            &browser,
            how,
            encoding.clone(),
            certificate_exception,
        )
        .await;
        self.in_flight
            .lock()
            .await
            .went_on(run, &leased.crawl_request.id, InFlightPhase::Handling);
        let params = self.params(
            Some(FetchTier::Browser),
            Some(page_id.clone()),
            navigated.response,
        );
        let answer = handled(
            run,
            self.context,
            self.code_free,
            (&params, self.sent_at),
            lease,
        )
        .await;
        drop(handed_over);
        let ended_at = tiers.browser_ended_at(&page_id);
        tiers.close_browser(&page_id).await;
        BrowserRun {
            answer,
            encoding: encoding.map(|encoding| encoding.encoding),
            ended_at,
        }
    }
}

/// The skip of the attempt whose browser page `page_id` the interception
/// stopped at its crawl request's own document, of a type its Label's
/// `crawl.content_types` does not take, before anything read its body
/// (ADR 0119): the page is closed and the request ends `skipped` with
/// `content_type`, as on the document tiers. `None` for any other page.
pub(super) async fn refused_type(
    tiers: &FetchTiers,
    page_id: &PageId,
    run: &Run,
    leased: &Leased,
) -> Option<Answer> {
    let refused = tiers.browser_refused(page_id)?;
    tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, content_type = refused, "the browser's interception stopped a document of a type its Label does not take");
    tiers.close_browser(page_id).await;
    Some(Answer::Refused(SkipReason::ContentType))
}

/// The encoding the browser read the request's own document of its page
/// `page_id` in, as its navigation's `response` names it,
/// `document.characterSet`, and where ADR 0119's order found it at the
/// interception; with no source where only the browser's guess read it,
/// which the escalation's call then names `detected` (ADR 0119).
pub(super) fn read_by_browser(
    tiers: &FetchTiers,
    page_id: &PageId,
    response: Option<&PageResponse>,
) -> Option<EncodingProvenance> {
    tiers.browser_encoding(
        page_id,
        response.and_then(|response| response.encoding.as_deref()),
    )
}

/// Whether the cheaper run `run`, which staged `staged`, staged what the
/// browser's run `browser` did: it answered and skipped nothing, never
/// escalated, and the weighing matched what each staged, the Records it
/// pushed then those of its answer and the requests it enqueued, before
/// any Hook (ADR 0103), the URL each ended on and the encoding each read
/// in (ADRs 0118 and 0119), the fields `not_compared` names left out of
/// every Record.
fn matches_browser(
    run: &HttpRun,
    staged: &SoFar,
    browser: &StagedRun<'_>,
    not_compared: &BTreeSet<String>,
) -> bool {
    let HttpRun {
        answer:
            Answer::Handled(CrawlRequestHandled {
                records,
                skipped: None,
            }),
        escalated: None,
        ..
    } = run
    else {
        return false;
    };
    let records = [staged.records.records(), records.as_slice()].concat();
    // The extraction's removal reports no field per attempt yet: the
    // fields left out are those the runs themselves never hold alike.
    weighing::weigh(
        &StagedRun {
            records: &records,
            enqueues: &staged.enqueues,
            ended_at: run.url.as_deref(),
            encoding: run.encoding.as_deref(),
        },
        browser,
        not_compared,
    )
}

/// A run on the `http` tier that escalated at `call` before its handler
/// ran, its answer failing with `code` and `message`, which the browser's
/// run replaces.
fn escalated_unhandled(code: &str, message: &str, call: &'static str) -> HttpRun {
    HttpRun {
        answer: Answer::failed(AttemptFailure::named(code, false, message.to_owned())),
        escalated: Some(call),
        document: true,
        encoding: None,
        url: None,
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
pub(super) fn run_page_check(run: &Run) -> Option<PageCheck> {
    let settings = run.settings()?;
    let value = settings
        .resolved()
        .effective(&SESSION_PAGE_CHECK)?
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

/// An attempt that ended with `error` before its handler was asked; a
/// page marked `noindex` that failed its Page check is a decoy instead
/// (ADR 0035).
fn failed(error: &CallError) -> Answer {
    ended(AttemptFailure::entered(
        error.code().to_owned(),
        error.retryable(),
        error.message().to_owned(),
        error.details().cloned().unwrap_or_default(),
    ))
}

/// An attempt whose fetch ended with `failure` before its handler was
/// asked; a decoy as [`failed`] says.
fn fetch_failed(failure: &FetchFailure) -> Answer {
    ended(AttemptFailure::of_fetch(failure))
}

/// The answer of an attempt that failed with `failure`: a page marked
/// `noindex` that failed its Page check is a decoy instead (ADR 0035).
fn ended(failure: AttemptFailure) -> Answer {
    match failure.why() {
        Why::PageCheckFailed {
            noindex: Some((url, status)),
        } => decoy::noindexed(url, *status),
        _ => Answer::failed(failure),
    }
}

/// The ending of `leased`'s request on a browser tier when it was added
/// through a link its served document's markup hides (ADR 0035): a
/// browser tier never follows a link a person cannot see, so its page,
/// never fetched, is a decoy.
fn reached_through_trap(leased: &Leased) -> Option<Answer> {
    leased
        .trap_link_suspected
        .then(|| decoy::reached_through_trap(&leased.crawl_request.url))
}

/// Records in the Run's Provenance the tier `choice` names for `leased`'s
/// attempt, how it came there, as `how` says (ADR 0023), and the
/// `encoding` its document was read in (ADR 0119), and notes both on
/// `lease`, with the `certificate_exception` its document's host was
/// reached through (ADR 0042), for the Provenance of each Record the
/// attempt commits.
async fn record_tier(
    run: &Run,
    leased: &Leased,
    lease: &WorkItemLease,
    choice: &TierChoice,
    how: Chosen,
    encoding: Option<EncodingProvenance>,
    certificate_exception: Option<CertificateExceptionMark>,
) {
    let tier = goodfetch_fetch_tier::tier_provenance(choice, how);
    let encoding = goodfetch_fetch_tier::record_tier(
        run,
        &leased.crawl_request.id,
        &leased.crawl_request.label,
        &tier,
        encoding,
    )
    .await;
    lease.fetched(Fetched {
        tier: Some(tier),
        encoding,
        certificate_exception,
        robots_txt: None,
    });
}

/// The handler request of one attempt, bounded by its lease: sent to the
/// program's handler through `context`'s Client, or, for a code-free crawl,
/// handled by the engine itself with `code_free` (ADR 0103). The moment it
/// goes out is kept in `sent_at`.
async fn handled(
    run: &Run,
    context: &CallContext,
    code_free: Option<&CodeFree>,
    (params, sent_at): (&CrawlRequestHandleParams, &Mutex<Option<SystemTime>>),
    lease: &WorkItemLease,
) -> Answer {
    *sent_at.lock().await = Some(run.clock().wall());
    if let Some(code_free) = code_free {
        return tokio::select! {
            biased;
            handled = code_free.handled(params, run.point_in_time()) => match handled {
                Ok((handled, fields)) => {
                    lease.fields_read(fields);
                    Answer::Handled(handled)
                }
                Err(error) => failed(&error),
            },
            _ = lease.ended() => Answer::LeaseEnded,
        };
    }
    match context
        .client()
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
                        None => Answer::failed(AttemptFailure::entered(
                            code, retryable, message, details,
                        )),
                    }
                }
                Err(ClientRequestError::Malformed(message)) => {
                    Answer::failed(AttemptFailure::named(HANDLER_FAILED.name, false, message))
                }
                Err(_) => Answer::ClientGone,
            },
            _ = lease.ended() => {
                sent.cancel().await;
                run.spawn(watch::after(run.metrics(), late(run.clone(), lease.clone(), sent)));
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
            if let Ok(handled) = answer {
                match lease.commit(run.state(), run.secrets().mask().masks(), handled.records, run.point_in_time()).await {
                    Err(CommitError::LeaseLost) => run.record_outcome(LEASE_LOST.name),
                    Err(CommitError::Refused(refused)) => {
                        tracing::debug!(code = refused.code(), run_id = %run.id(), "a late answer's Records were refused");
                    }
                    Err(CommitError::RunState(error)) => {
                        tracing::debug!(%error, run_id = %run.id(), "a late answer was not written");
                    }
                    Ok(_) => {}
                }
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
        Answer::Failed(failure) => match failure.why() {
            Why::Blocked(cause) => Effect::Blocked(*cause),
            Why::PageCheckFailed { .. } => Effect::PageCheckFailed,
            _ => Effect::Neutral,
        },
        Answer::Handled(CrawlRequestHandled {
            skipped: Some(_), ..
        })
        | Answer::Refused(_)
        | Answer::RobotsTxtUnreachable(_)
        | Answer::HopsRobotsTxtUnreachable(_)
        | Answer::SitemapRead(_)
        | Answer::LeaseEnded
        | Answer::ClientGone => Effect::Neutral,
    }
}
