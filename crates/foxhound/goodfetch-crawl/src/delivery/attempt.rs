//! One attempt of a crawl's delivery (ADRs 0004, 0006 and 0024): its
//! `crawl_request.handle` request to a serving Connection, bounded by its
//! lease, and what its answer is. When the lease ends first, the request
//! is cancelled with `$/cancel` and an answer that still comes is a late
//! commit, refused with `lease_lost`. A request the program gives back,
//! answering `call_cancelled` by the program with nothing closed under it
//! (ADR 0102), ends its attempt released, uncounted. Its answer also says
//! what the Session that served it scored: a success, a block or a failed
//! Page check.
//!
//! Its request runs as the crawl's tier choice planned it (the
//! `tier_choice` module): alone, on one tier, or as a sample of the sample
//! comparison, run on `http`, on `light_browser` where the Run allows it,
//! and then on `browser`, the browser's run the one whose answer counts,
//! and whether the cheaper runs matched it classified once the attempt
//! settles. A run on `light_browser` is one on `http` whose page is served
//! from the document Lightpanda's run of the page's scripts left (ADR
//! 0020), escalated to the browser as `http`'s is. Under
//! `fetch.http_from: learned` (#2417), a sample whose `http` run differed
//! from the browser's runs once more after it, on `browser_fetch`, from the
//! hop page of the attempt's own browser Session.
//!
//! Under `fetch.tier_race` (ADR 0040; the `tier_race` module), an attempt
//! whose tier is not forced races instead: its document is fetched by a
//! first request on the document tier, and a full navigation of its URL in
//! the attempt's Session starts its race's delay later, taking room beside
//! the attempt's own as a second copy does, one `requests` in Usage and its
//! domain's delay. The first request leaves from where its Label learned
//! to leave from (#2421): under `fetch.http_from: learned`, from the own
//! transport of a Session drawn for it beside the attempt's when its Label
//! learned to send its HTTP from there, else through browser-fetch from
//! the hop page of the attempt's Session. The first page that passes the
//! Page check, the first request's only when it also agrees with its
//! Label's comparison, wins and is handed to the handler; the other is
//! cancelled: a first request in flight is dropped, its connection closed,
//! and a navigation not yet sent is never sent, while one sent has its page
//! closed, its load with it. Provenance records the race and names the tier
//! that sent the first request. A first request whose document arrived
//! gives its Label on its domain one latency, which times its later races.
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
use goodfetch_engine::deadline::Enclosing;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{
    Capture, Chosen, FetchFailure, FetchTiers, HandOver, HopChecks, HttpTier, Judged,
    PAGE_CHECK_CALL, PageFacts, PooledSession, Redirected, ReplayPlan, Request, Requester,
    RunHandOver, SitePreferences, Stop, TierChoice, Unfollowed, infer_plan, light_browser_unsent,
    tier_provenance, tier_serves,
};
use goodfetch_frontier::{DomainKeys, sitemap};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, StoppedBy};
use goodfetch_protocol::foxhound::crawl::{
    ATTEMPT_ENDED, CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled,
    HANDLER_FAILED, LEASE_LOST, WORK_ITEM_LEASED,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::progress::InFlightPhase;
use goodfetch_protocol::foxhound::provenance::{
    CertificateExceptionMark, EncodingProvenance, TierProvenance,
};
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_ADAPTIVE_DELAY, POLITENESS_DOMAIN_DELAY,
};
use goodfetch_protocol::page::{NAVIGATION_FAILED, PageId, PageResponse};
use goodfetch_protocol::settings::SESSION_PAGE_CHECK;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_session_pool::escalation::{self, Action, Met};
use goodfetch_session_pool::{Effect, PoolKey, PoolSettings, SendsFrom};
use serde_json::{Map, Value};
use tokio::sync::{Mutex, oneshot};

use super::failure::{AttemptFailure, Why};
use super::watch::{self, Watch};
use crate::code_free::CodeFree;
use crate::hooks::Hooks;
use crate::lease::{CommitError, SoFar, WorkItemLease};
use crate::progress::SharedInFlight;
use crate::run_state::{Asking, Escalated};
use crate::tier_choice::weighing::{self, StagedRun};
use crate::tier_choice::{AttemptPlan, Between, Replay, Sampled, Weighed};
use crate::tier_race::Raced;
use crate::work_items::{Fetched, Leased};

mod decoy;
mod learning;
mod race;
mod record_stage;
mod site_preferences;
mod sitemap_fetch;

pub(super) use learning::{Lessons, learns};
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
    /// The program gave the request back unhandled, its own stop (ADRs
    /// 0093 and 0102): its work item waits again, the attempt uncounted.
    Released,
}

impl Answer {
    /// The answer of an attempt that failed with `failure`.
    pub(super) fn failed(failure: AttemptFailure) -> Self {
        Self::Failed(Box::new(failure))
    }

    /// Whether the attempt ended with no answer of its handler to run
    /// again on another tier: its lease ended, its Connection ended or its
    /// program gave its request back.
    pub(super) fn unanswered(&self) -> bool {
        matches!(self, Self::LeaseEnded | Self::ClientGone | Self::Released)
    }
}

/// What an attempt is handled with beside its tier: what it hands over
/// with its Run's pages, what a code-free crawl handles it with, none when
/// the program does, and the Run's Hooks in code.
pub(super) type Handling = (Arc<HandOver>, Option<Arc<CodeFree>>, Arc<Hooks>);

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
    /// The client the attempt's Session sends from.
    sends_from: SendsFrom,
    /// The Session a sample's `http` run, or a race's first request, is
    /// sent from when its Label sends its HTTP from the transport
    /// (`fetch.http_from`, #2421): drawn beside the attempt's, sharing
    /// nothing with it.
    transport: Option<TransportSession>,
    /// The Session drawn only to read the attempt's site files from its
    /// own transport, when neither the attempt's Session nor `transport`
    /// sends from one (#2422).
    site_files: Option<ServingSession>,
    /// Where an escalation draws the Session it runs again in, when the
    /// attempt's Session sends from its own transport (#2416).
    escalating: Option<Escalating>,
}

/// What an attempt whose Session sends from its own transport asks the
/// crawl's Run state through, as it escalates to the browser, for a
/// Session whose client is its browser (#2416): the pool its Label draws
/// from among the crawl's `labels`, under the Run's pool `settings`.
pub(super) struct Escalating {
    pub(super) asking: Asking,
    pub(super) labels: std::sync::Arc<crate::labels::Declared>,
    pub(super) pool: PoolKey,
    pub(super) settings: PoolSettings,
}

/// The Session a sample's `http` run, or a race's first request, is sent
/// from, from its own transport, as it would run in the browser, with the
/// Run's HTTP tier as it sends on it: `None` when that tier could not be
/// built for it, which fails the sample or the race.
pub(super) struct TransportSession {
    pub(super) session: ServingSession,
    pub(super) in_browser: PooledSession,
    pub(super) http: Option<HttpTier>,
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
            sends_from: SendsFrom::Browser,
            transport: None,
            site_files: None,
            escalating: None,
        }
    }

    /// The same, its Session sending from `sends_from`, and a sample's
    /// `http` run, or a race's first request, sent from `transport`'s
    /// Session when given.
    pub(super) fn sending(
        mut self,
        sends_from: SendsFrom,
        transport: Option<TransportSession>,
    ) -> Self {
        self.sends_from = sends_from;
        self.transport = transport;
        self
    }

    /// The same, its site files read from `site_files`' Session when
    /// given.
    pub(super) fn reading_site_files(mut self, site_files: Option<ServingSession>) -> Self {
        self.site_files = site_files;
        self
    }

    /// The same, an escalation drawing through `escalating` the Session it
    /// runs again in, when given.
    pub(super) fn escalating(mut self, escalating: Option<Escalating>) -> Self {
        self.escalating = escalating;
        self
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

/// The tier a browser-fetch the engine chose serves its page on, a
/// race's, a sample's under `fetch.http_from: learned` and a Label's
/// learned onto `browser_fetch`: `http`, sent from inside the Session's
/// browser, chosen by no one. Provenance names `browser_fetch`, the tier
/// that sent it; its handler is told `http`, the tier its page serves as.
const FETCHED: TierChoice = SAMPLED;

/// The browser's tier, chosen by no one.
const BROWSER: TierChoice = TierChoice {
    tier: FetchTier::Browser,
    forced: None,
};

/// What every attempt of `run` hands over alike with its pages, built once
/// as its delivery opens, never per attempt (ADR 0138): its `settings` as
/// the navigation gate and the site's preferences read them, the
/// Politeness it shares, `http`, the Run's HTTP tier, whose address check
/// judges each redirect hop, `egress`, its domain policy, and `hops`, how each redirect hop
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
/// timed out meanwhile (ADR 0093), and the end of the handler call its
/// pages' calls inherit (ADR 0080). A panic in the renewal ends the
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

impl LeaseCheck for HandlerLease {
    fn check(&self) -> Result<(), LeaseEnded> {
        self.lease.check()
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        self.lease.withheld(ended, method)
    }

    fn permit_wait(&self, _url: &str) -> PermitWait {
        let (stop, stopped) = oneshot::channel::<()>();
        let waiting = self.clone();
        self.of_run.run.spawn(self.watch.over(async move {
            let id = &waiting.request_id;
            let run = &waiting.of_run.run;
            waiting.in_flight.lock().await.permit_wait_began(run, id);
            waiting.lease.held_through_permit_wait(stopped).await;
            waiting.in_flight.lock().await.permit_wait_ended(run, id);
        }));
        PermitWait::holding(stop)
    }

    fn enclosing(&self) -> Option<Enclosing> {
        self.lease.enclosing()
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
    /// The Session a sample's `http` run, or a race's first request, was
    /// sent from, from its own transport, when its Label sends its HTTP
    /// from there.
    pub(super) transport: Option<ServingSession>,
    /// Whether its answer is that of its race's first request, sent from
    /// `transport`'s Session (#2421), which that Session's score then
    /// takes in place of its own Session's.
    pub(super) transport_answered: bool,
    /// The Session drawn only to read its site files, when one was.
    pub(super) site_files: Option<ServingSession>,
    /// The Session whose client is its browser that it ran again in, when
    /// it escalated from its Session's own transport (#2416): its answer
    /// is this Session's.
    pub(super) escalated: Option<ServingSession>,
    pub(super) lease: WorkItemLease,
    pub(super) answer: Answer,
    /// The sample it was, with how its two runs weighed, once both
    /// answered.
    pub(super) sampled: Option<(Sampled, Weighed)>,
    /// The race it was, with its first request's latency, once its
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
    /// Whether its site refused the request its Session's own transport
    /// sent on the tier its Label's comparison chose, which drops that
    /// choice and retries the request as a sample (`escalation`, #2419).
    pub(super) refused: bool,
}

/// The answer of an attempt that needs the crawl's HTTP tier, which could
/// not be built: its `what` was not fetched, retried as a failed
/// navigation.
fn without_http_tier(what: &str) -> Answer {
    Answer::failed(AttemptFailure::named(
        NAVIGATION_FAILED.name,
        true,
        format!("The crawl's HTTP tier could not be built, so the {what} was not fetched."),
    ))
}

/// The choice a request on `choice` is served under: a Label learned onto
/// `browser_fetch` is served as `http` from its Session's browser (#2417).
fn served_as(choice: &TierChoice) -> &TierChoice {
    match choice {
        TierChoice {
            tier: FetchTier::BrowserFetch,
            forced: None,
        } => &FETCHED,
        choice => choice,
    }
}

/// One attempt: the handler request, bounded by its lease, on the tier its
/// request takes. On `http` the request's document is fetched first and
/// the handler given a page served from it; a call that tier cannot serve
/// exactly under `auto` escalates the request to the browser, where the
/// handler runs again on Collie's page, neither a retry nor a failure, the
/// first answer discarded. On `browser` the engine opens Collie's page
/// for the handler, navigated to the request's URL through the gate. When
/// the lease ends first, the request is cancelled and its answer left to
/// [`late`]. Once its handler answered, the Records it is about to commit
/// go through the Run's Record-stage Hooks in code ([`record_stage`]). A
/// panic in the attempt, or in a task it started while it runs, ends it
/// failed with `engine_panicked` ([`Watch`]); what it holds
/// is given back as for any other end.
pub(super) async fn attempt(
    run: Run,
    server: usize,
    context: CallContext,
    (leased, (session, in_browser), counted): (Leased, (ServingSession, PooledSession), Counted),
    (lease, watch): (WorkItemLease, Watch),
    (tiering, (hand_over, code_free, hooks)): (Tiering, Handling),
    (in_flight, lessons): (SharedInFlight, &Lessons),
) -> Attempt {
    let (document, sent_at) = (Mutex::new(None), Mutex::new(None));
    let (moved, refused) = (OnceLock::new(), OnceLock::new());
    let escalated = OnceLock::new();
    let on = On {
        document: &document,
        escalated_to: &escalated,
        sent_at: &sent_at,
        moved: &moved,
        refused: &refused,
        run: &run,
        context: &context,
        leased: &leased,
        session: &session,
        in_browser: &in_browser,
        http: tiering.http.as_ref(),
        sends_from: tiering.sends_from,
        lease: &lease,
        hand_over: &hand_over,
        code_free: code_free.as_deref(),
        in_flight: &in_flight,
        lessons,
        watch: &watch,
    };
    // Boxed: the attempt's future is large, and the watch holds it only
    // while it runs.
    let ran = watch
        .ended(Box::pin(async {
            let mut sampled = None;
            // Its race's latency, and whether a transport's first request answered.
            let mut raced = (None, false);
            let answer = match (&tiering.plan, &tiering.http) {
                (AttemptPlan::Sitemap, Some(http)) => on.sitemap(http).await,
                (AttemptPlan::Sitemap, None) => without_http_tier("sitemap"),
                (AttemptPlan::On(choice), _) if choice.tier == FetchTier::Browser => {
                    on.browser(&tiering.tiers, choice, Chosen::AsChosen).await
                }
                (AttemptPlan::On(choice), Some(http)) => {
                    on.http(&tiering, http, served_as(choice), None).await
                }
                (AttemptPlan::Replayed(replay), Some(http)) => {
                    on.http(&tiering, http, &REPLAYED, Some(&replay.plan)).await
                }
                (AttemptPlan::Sample(sample), Some(http)) => {
                    let (answer, weighed) =
                        Box::pin(on.sample(&tiering, http, sample.browser_fetch)).await;
                    sampled = weighed.map(|weighed| (sample.clone(), weighed));
                    answer
                }
                (AttemptPlan::Race(race), Some(http)) => {
                    let (answer, ran) = on.race(&tiering, http, race).await;
                    raced = ran;
                    answer
                }
                (
                    AttemptPlan::On(_)
                    | AttemptPlan::Replayed(_)
                    | AttemptPlan::Sample(_)
                    | AttemptPlan::Race(_),
                    None,
                ) => without_http_tier("request"),
            };
            // The Record stage runs once, on the answer that counts.
            let answer = match answer {
                Answer::Handled(handled)
                    if handled.skipped.is_none() && hooks.at_record_stage() =>
                {
                    record_stage::passed(&on, &hooks, handled).await
                }
                answer => answer,
            };
            (answer, sampled, raced)
        }))
        .await;
    let (answer, sampled, (raced, transport_answered), ran) = match ran {
        Ok((answer, sampled, raced)) => (answer, sampled, raced, true),
        // What the attempt met before it panicked says nothing of its site:
        // its domain neither backs off nor learns from it.
        Err(panicked) => (
            Answer::failed(AttemptFailure::panicked(&panicked)),
            None,
            (None, false),
            false,
        ),
    };
    Attempt {
        document: document.into_inner().filter(|_| ran),
        handed_at: sent_at.into_inner().filter(|_| ran),
        moved: ran && moved.into_inner().is_some(),
        refused: ran && refused.into_inner().is_some(),
        server,
        leased,
        _counted: counted,
        session,
        transport: tiering.transport.map(|drawn| drawn.session),
        transport_answered: ran && transport_answered,
        site_files: tiering.site_files,
        escalated: escalated.into_inner().map(|(session, _)| session),
        lease,
        answer,
        sampled,
        raced,
    }
}

/// What a run on a document tier finds of its served page before the
/// Frontier learns from it or its handler runs, in the one order every
/// run checks it: a document only a guess could read first (ADR 0119), then
/// a block the Block classification finds that the run stops at; each
/// caller then runs the Page check on a page that is neither ([`judged`]).
enum Found {
    /// The document declares no encoding and is not UTF-8: the call that
    /// escalates it to the browser, which reads it, under `auto`.
    Unreadable(&'static str),
    /// A block of this cause the run stops at, with the `blocked` it ends
    /// with where it is not moved (`escalation`): a Challenge, which checks
    /// the browser, so that under `auto` the request moves there and under
    /// a forced tier it ends; or an IP block met by the Session's own
    /// transport on the tier its Label's comparison chose, which drops that
    /// choice (#2419). Other blocks still reach the handler until the
    /// proxy-tier escalation ends them (#1137).
    Blocked(BlockCause, CallError),
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
#[derive(Clone, Copy)]
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
    /// The client the Session sends its HTTP from.
    sends_from: SendsFrom,
    lease: &'a WorkItemLease,
    /// What the attempt hands over with its Run's pages, built once as it
    /// was leased: each of its runs on a tier takes it whole.
    hand_over: &'a Arc<HandOver>,
    /// What a code-free crawl handles its requests with, in place of the
    /// program's handler; none when the program handles them.
    code_free: Option<&'a CodeFree>,
    in_flight: &'a SharedInFlight,
    /// Where its page notes what it teaches the Frontier.
    lessons: &'a Lessons,
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
    /// Whether its site refused the request its Session's own transport
    /// sent on the tier its Label's comparison chose, set once it did
    /// ([`Attempt::refused`]).
    refused: &'a OnceLock<()>,
    /// The Session whose client is its browser that the request ran again
    /// in, as that Session runs in the browser, set once its attempt
    /// escalated from a Session's own transport (#2416).
    escalated_to: &'a OnceLock<(ServingSession, PooledSession)>,
}

impl On<'_> {
    /// Where the run `how` names on `choice` met a block (`escalation`):
    /// from the Session's own transport when it sends from there and the
    /// run is on the tier its Label's comparison chose, not a sample's,
    /// an escalation's or a race's.
    fn met(&self, choice: &TierChoice, how: Chosen) -> Met {
        Met {
            tier: Some(choice.tier),
            forced: choice.forced.is_some(),
            from_transport: self.sends_from == SendsFrom::Transport && how == Chosen::AsChosen,
        }
    }

    /// What the crawl does with the request, run as `how` names on
    /// `choice`, whose Session met a block of `cause`, the Block
    /// classification's or one the program's code named alike
    /// (`escalation`): a move to the browser is noted, for its domain to
    /// learn, and a refusal of its transport, for its Label to be sampled
    /// again.
    fn on_block(&self, cause: &BlockCause, choice: &TierChoice, how: Chosen) -> Action {
        let action = escalation::on_block(cause, self.met(choice, how));
        // Set once: a second move or refusal says nothing more.
        match action {
            Action::MoveToBrowser => {
                let _ = self.moved.set(());
            }
            Action::Sample => {
                let _ = self.refused.set(());
            }
            Action::Blocked => {}
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

    /// Whether `answer`, a handler's on a run as `how` names on `choice`,
    /// names a block that moves the request to the browser
    /// ([`On::on_block`]); one that refused its transport ends it as it is.
    fn moves(&self, answer: &Answer, choice: &TierChoice, how: Chosen) -> bool {
        match answer {
            Answer::Failed(failure) => failure
                .block_cause()
                .is_some_and(|cause| self.on_block(cause, choice, how) == Action::MoveToBrowser),
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
    /// under `auto`, in the same attempt, the first answer discarded
    /// ([`On::escalate`]).
    /// Answers how it ended. On `discovered_api`, its page replays
    /// `plan`'s calls.
    async fn http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        choice: &TierChoice,
        plan: Option<&ReplayPlan>,
    ) -> Answer {
        // Lightpanda sends a bodiless `GET` alone: under `auto` any other
        // request goes to the browser before it is sent (issue 2471).
        if choice.tier == FetchTier::LightBrowser
            && choice.forced.is_none()
            && let Some(call) = light_browser_unsent(&self.leased.crawl_request)
        {
            return self.escalate(tiering, choice, call).await;
        }
        let run = match self
            .on_http(tiering, http, (choice, plan), Chosen::AsChosen)
            .await
        {
            Ok(ran) => ran,
            Err(ended) => return ended,
        };
        match run.escalated {
            // The first answer, whatever it says, is discarded.
            Some(call) if !run.answer.unanswered() => {
                // The browser's run takes a whole lease of its own, and
                // stages anew: what the first run staged goes with it.
                self.lease.renew(self.lease.length());
                drop(self.lease.restage());
                self.escalate(tiering, choice, call).await
            }
            // A Challenge the program's code named moves as one the rules
            // found does; an IP block it named on the transport is decided
            // as one they found is, the answer standing.
            None if self.moves(&run.answer, choice, Chosen::AsChosen) => {
                self.lease.renew(self.lease.length());
                drop(self.lease.restage());
                self.escalate(tiering, choice, BLOCKED).await
            }
            _ => run.answer,
        }
    }

    /// The request run again on the browser, escalated at `call` from a
    /// run as `choice` says: in the attempt's Session when it sends from
    /// its browser (`fetch.http_from: browser`), else in a Session whose
    /// client is its browser, drawn for it from its Label's pool, which
    /// carries nothing of the first (#2416). A failed answer, retried, when
    /// the crawl ended before that Session was drawn.
    async fn escalate(&self, tiering: &Tiering, choice: &TierChoice, call: &'static str) -> Answer {
        let how = Chosen::Escalated(call);
        let escalating = match &tiering.escalating {
            Some(escalating) if self.sends_from == SendsFrom::Transport => escalating,
            _ => return self.browser(&tiering.tiers, choice, how).await,
        };
        // The loop answers from memory, at once: the attempt waits for it
        // whatever its lease, so the Session it draws is always settled.
        let drawn = escalating
            .asking
            .escalated(Escalated {
                labels: std::sync::Arc::clone(&escalating.labels),
                pool: escalating.pool.clone(),
                domain_key: self.leased.waiting.key.clone(),
                settings: escalating.settings,
                now: self.run.clock().wall(),
            })
            .await;
        let Ok(drawn) = drawn else {
            return Answer::failed(AttemptFailure::named(
                NAVIGATION_FAILED.name,
                true,
                "The crawl ended before a browser Session was drawn for the escalated request, so it was not fetched again."
                    .to_owned(),
            ));
        };
        // An attempt escalates once, so nothing was set before.
        let (session, in_browser) = self
            .escalated_to
            .get_or_init(|| (drawn.session, drawn.in_browser));
        On {
            session,
            in_browser,
            sends_from: SendsFrom::Browser,
            ..*self
        }
        .browser(&tiering.tiers, choice, how)
        .await
    }

    /// The client the Session was drawn for (`AttemptPlan::sends_from`)
    /// on the Run's HTTP tier `http`: the hop page of its browser, or its
    /// own transport, keeping its cookies.
    fn client(&self, tiers: &FetchTiers, http: &HttpTier) -> HttpTier {
        match self.sends_from {
            SendsFrom::Browser => tiers.in_browser(
                http,
                self.run,
                self.in_browser,
                self.hand_over,
                self.context,
            ),
            SendsFrom::Transport => http.keeping_cookies_of(&self.session.id),
        }
    }

    /// Where a sample's `http` run, or a race's first request, is sent
    /// from, on the Run's HTTP tier as it sends there: the Session drawn
    /// for it from the transport when there is one, else the attempt's own
    /// on `http`; a failed answer when that Session's tier could not be
    /// built.
    fn sent_from<'s>(
        &'s self,
        tiering: &'s Tiering,
        http: &'s HttpTier,
    ) -> Result<(On<'s>, &'s HttpTier), Answer> {
        let Some(sample) = &tiering.transport else {
            return Ok((*self, http));
        };
        let Some(http) = &sample.http else {
            return Err(Answer::failed(AttemptFailure::named(
                NAVIGATION_FAILED.name,
                true,
                "The crawl's HTTP tier could not be built for the Session drawn to send the request from its own transport, so the request was not fetched."
                    .to_owned(),
            )));
        };
        Ok((
            On {
                session: &sample.session,
                in_browser: &sample.in_browser,
                http: Some(http),
                sends_from: SendsFrom::Transport,
                ..*self
            },
            http,
        ))
    }

    /// A sample of the crawl's sample comparison: its run on `http`, sent
    /// from the Session drawn for it when its Label sends its HTTP from the
    /// transport (`fetch.http_from`), else from the attempt's, then, unless
    /// that run ended the attempt, its run on `browser` under a whole lease
    /// of its own, whose answer is the attempt's, matched or not. Answers
    /// it and how the two runs weighed once the browser's answered: what each run
    /// staged, the Records it pushed then those of its answer and the
    /// requests it enqueued, before any Hook (ADR 0103), the URL each ended
    /// on and the encoding each read in (ADRs 0118 and 0119). A browser run
    /// that met a consent wall keeps its Label on `browser` (ADR 0048).
    /// When `browser_fetch` (`fetch.http_from: learned`), a sample whose
    /// `http` run differed runs once more on `browser_fetch`, and one whose
    /// replay differed from the transport replays once more from the
    /// browser ([`Self::replays`]).
    async fn sample(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        browser_fetch: bool,
    ) -> (Answer, Option<Weighed>) {
        let (sender, sent_on) = match self.sent_from(tiering, http) {
            Ok(sender) => sender,
            Err(answer) => return (answer, None),
        };
        let run = match sender
            .on_http(tiering, sent_on, (&SAMPLED, None), Chosen::Sample(None))
            .await
        {
            Ok(ran) => ran,
            Err(answer) => return (answer, None),
        };
        // A page holding no document has nothing to compare, and the
        // browser would hold none either (ADR 0119): its answer stands.
        if run.answer.unanswered() || !run.document {
            return (run.answer, None);
        }
        // A Challenge its handler named moves the domain to the browser,
        // where the sample's browser run then serves the request.
        self.moves(&run.answer, &SAMPLED, Chosen::Sample(None));
        self.lease.renew(self.lease.length());
        let cheaper = self.lease.restage();
        let light = match self.light_sampled(tiering, http).await {
            Ok(light) => light,
            Err(ended) => return (ended, None),
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
                if failure.block_cause() == Some(&BlockCause::ConsentRequired) =>
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
                            Box::pin(self.replays(
                                tiering,
                                (&sender, sent_on),
                                (http, browser_fetch),
                                &capture,
                                &browser_run,
                            ))
                            .await
                        }
                        _ => None,
                    };
                    let fetch = browser_fetch
                        && Box::pin(self.aside(tiering, http, (&FETCHED, None), &browser_run))
                            .await;
                    Some(Weighed::Differed(Between {
                        api: replayed,
                        fetch,
                        light,
                    }))
                }
            }
            _ => None,
        };
        (browser.answer, weighed)
    }

    /// A sample's runs on `discovered_api`, after its browser's: the calls
    /// the browser's page made, `capture`, read as a plan and replayed for
    /// the same request ([`Self::aside`]; ADRs 0020 and 0103), from
    /// `sender`, the Session its `http` run was sent from, over `sent_on`,
    /// and, where `browser_fetch` (`fetch.http_from: learned`) and that
    /// replay left from the transport and differed, once more from the hop
    /// page of the attempt's own browser Session, over `http` (#2420).
    /// Answers the plan and the client of the first run that staged what
    /// the browser's run `browser` did; `None` when the calls follow no
    /// plan or every run differed, escalated or failed.
    async fn replays(
        &self,
        tiering: &Tiering,
        (sender, sent_on): (&On<'_>, &HttpTier),
        (http, browser_fetch): (&HttpTier, bool),
        capture: &Capture,
        browser: &StagedRun<'_>,
    ) -> Option<Replay> {
        let document = goodfetch_frontier::Url::parse(&self.leased.crawl_request.url).ok()?;
        let calls = capture.calls();
        let plan = infer_plan(&document, &calls);
        tracing::debug!(run_id = %self.run.id(), work_item = %self.leased.crawl_request.id, calls = calls.len(), ?plan, "a sample's browser calls were read as a plan");
        let plan = plan?;
        let from = if sender
            .aside(tiering, sent_on, (&REPLAYED, Some(&plan)), browser)
            .await
        {
            sender.sends_from
        } else if browser_fetch
            && sender.sends_from == SendsFrom::Transport
            && self
                .aside(tiering, http, (&REPLAYED, Some(&plan)), browser)
                .await
        {
            self.sends_from
        } else {
            return None;
        };
        Some(Replay { plan, from })
    }

    /// A sample's run on `choice`, replaying `plan` where it names one,
    /// after its browser's, whose staging is set aside meanwhile and kept
    /// to commit, its own staging then discarded: on `discovered_api`
    /// ([`Self::replays`]), or on `browser_fetch` under `fetch.http_from:
    /// learned` (#2417), the same request sent from the hop page of the
    /// attempt's own browser Session. Answers whether the run staged what
    /// the browser's run `browser` did; not when it differed, escalated or
    /// failed.
    async fn aside(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        (choice, plan): (&TierChoice, Option<&ReplayPlan>),
        browser: &StagedRun<'_>,
    ) -> bool {
        let Some(browsers) = self.lease.set_aside() else {
            return false;
        };
        self.lease.renew(self.lease.length());
        let ran = self
            .on_http(tiering, http, (choice, plan), Chosen::Sample(None))
            .await;
        let staged = self.lease.restage();
        self.lease.put_back(browsers);
        ran.is_ok_and(|run| matches_browser(&run, &staged, browser, self.not_compared()))
    }

    /// Where the Run allows it, the sample's run on `light_browser`, before
    /// the browser, whose run stages what commits; `None` where it is not
    /// allowed, cannot send the request or failed, and the answer that
    /// ends the attempt where its lease ended or its client left.
    async fn light_sampled(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
    ) -> Result<Option<(HttpRun, SoFar)>, Answer> {
        // Lightpanda sends a bodiless `GET` alone (issue 2471).
        if !tiering.light || light_browser_unsent(&self.leased.crawl_request).is_some() {
            return Ok(None);
        }
        match self
            .on_http(tiering, http, (&LIGHT_SAMPLED, None), Chosen::Sample(None))
            .await
        {
            Ok(light) if light.answer.unanswered() => Err(light.answer),
            Ok(light) => {
                self.lease.renew(self.lease.length());
                Ok(Some((light, self.lease.restage())))
            }
            Err(ended) if ended.unanswered() => Err(ended),
            Err(_) => {
                self.lease.renew(self.lease.length());
                drop(self.lease.restage());
                Ok(None)
            }
        }
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
    /// 0093), its lesson then noted with the Frontier unless the run is a
    /// sample's, and the handler given a page
    /// served from it, closed once it answered. A page failing its expected
    /// page under `auto` escalates at `session.page_check` without its
    /// handler running. Answers the run or, when the attempt ended before
    /// its handler ran, its answer.
    async fn on_http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        (choice, plan): (&TierChoice, Option<&ReplayPlan>),
        how: Chosen,
    ) -> Result<HttpRun, Answer> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let tiers = &tiering.tiers;
        let hand_over = self.hand_over;
        let http = &self.client(tiers, http);
        let (served, facts) = self
            .document(tiering, http, choice, hand_over)
            .await
            .map_err(|Unserved { answer, redirects }| {
                // A sample's runs teach nothing, as [`Self::learn`] says.
                if !matches!(how, Chosen::Sample(_)) {
                    self.lessons.skipped(&redirects);
                }
                answer
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
        // it (ADR 0023), and the one a replay's calls are sent on (#2420).
        let recorded = TierProvenance {
            calls_sent_on: plan.map(|_| http.sent_on(FetchTier::Http)),
            ..tier_provenance(
                &TierChoice {
                    tier: http.sent_on(choice.tier),
                    ..choice.clone()
                },
                how,
            )
        };
        if let Some(ended) = self
            .ended_unhandled(tiering, &page_id, &facts, (choice, how), &recorded)
            .await
        {
            return ended;
        }
        self.learn(tiers, &page_id, how).await;
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
        Ok(HttpRun {
            answer,
            escalated,
            document: not_document.is_none(),
            encoding: encoding.map(|encoding| encoding.encoding),
            url,
        })
    }

    /// Notes what the run `how` names, its page `page_id` served and past
    /// its Page check, teaches the Frontier ([`Lessons::learn`]). Only a
    /// run that may commit teaches: a sample's runs on a cheaper tier are
    /// discarded for the browser's, whatever they find, so they teach
    /// nothing (ADR 0020's #768 note).
    async fn learn(&self, tiers: &FetchTiers, page_id: &PageId, how: Chosen) {
        if !matches!(how, Chosen::Sample(_)) {
            self.lessons
                .learn(tiers, page_id, &self.leased.crawl_request)
                .await;
        }
    }

    /// How the run `how` names on `choice` ends before its handler runs,
    /// its page `page_id` served with `facts`, by what it found of the page
    /// ([`Found`]), the page closed; `None` when its handler runs. Records
    /// the run's tier either way, `recorded`.
    async fn ended_unhandled(
        &self,
        tiering: &Tiering,
        page_id: &PageId,
        facts: &PageFacts,
        (choice, how): (&TierChoice, Chosen),
        recorded: &TierProvenance,
    ) -> Option<Result<HttpRun, Answer>> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let tiers = &tiering.tiers;
        match self.found(tiering, page_id, facts, (choice, how)).await {
            Found::Unreadable(call) => {
                tiers.close(page_id);
                record_tier(run, leased, lease, recorded.clone(), None, None).await;
                let message = "The document declares no encoding and is not UTF-8: only the browser reads it.";
                return Some(Ok(escalated_unhandled(ATTEMPT_ENDED.name, message, call)));
            }
            Found::Blocked(cause, error) => {
                tiers.close(page_id);
                record_tier(run, leased, lease, recorded.clone(), None, None).await;
                return Some(match self.on_block(&cause, choice, how) {
                    Action::MoveToBrowser => Ok(escalated_unhandled(
                        BLOCKED,
                        "A Challenge met on a tier short of the browser moved the request there.",
                        BLOCKED,
                    )),
                    Action::Sample | Action::Blocked => Err(failed(&error)),
                });
            }
            Found::Readable => {}
        }
        record_tier(
            run,
            leased,
            lease,
            recorded.clone(),
            facts.encoding.clone(),
            facts.certificate_exception.clone(),
        )
        .await;
        match judged(run, tiering, page_id).await {
            Some(Judged::Passed) | None => None,
            Some(Judged::Failed(error)) => {
                tiers.close(page_id);
                Some(Err(failed(&error)))
            }
            Some(Judged::Escalated) => {
                tiers.close(page_id);
                // Its handler never ran: the run escalates at the Page
                // check, and the caller runs the request in the browser.
                let message = "The page failed its expected page on a tier short of the browser.";
                Some(Ok(escalated_unhandled(
                    PAGE_CHECK_FAILED,
                    message,
                    PAGE_CHECK_CALL,
                )))
            }
        }
    }

    /// What the run `how` names on `choice` finds of its served page
    /// `page_id`, whose `facts` it read as it was served, before the
    /// Frontier learns from it or its handler runs, checked in one order on
    /// every document tier, a tier race's first request included
    /// ([`Found`]).
    async fn found(
        &self,
        tiering: &Tiering,
        page_id: &PageId,
        facts: &PageFacts,
        (choice, how): (&TierChoice, Chosen),
    ) -> Found {
        if let Some(call) = facts.escalated {
            return Found::Unreadable(call);
        }
        if let Some((block, error)) = tiering.tiers.block(page_id).await.filter(|(block, _)| {
            block.checks_the_browser()
                || escalation::on_block(&block.cause, self.met(choice, how)) == Action::Sample
        }) {
            return Found::Blocked(block.cause, error);
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
                    self.site_preferences_judged(Request::CrawlRequest, &url)
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
                    // retry; a skipped request's are noted now (ADR 0120).
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
        if let Some(refused) = self.document_refusal(&facts.use_signals).await {
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
            opened = tiers.browser(&hand_over, self.in_browser, &leased.crawl_request, self.context) => {
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
            tier_provenance(&browser, how),
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

/// Records in the Run's Provenance the `tier` of `leased`'s attempt and
/// how it came there ([`tier_provenance`]; ADR 0023), and the
/// `encoding` its document was read in (ADR 0119), and notes both on
/// `lease`, with the `certificate_exception` its document's host was
/// reached through (ADR 0042), for the Provenance of each Record the
/// attempt commits.
async fn record_tier(
    run: &Run,
    leased: &Leased,
    lease: &WorkItemLease,
    tier: TierProvenance,
    encoding: Option<EncodingProvenance>,
    certificate_exception: Option<CertificateExceptionMark>,
) {
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
        .send_noted::<CrawlRequestHandled>(&CRAWL_REQUEST_HANDLE, params, |id| {
            lease.handler_sent(id.clone());
        })
        .await
    {
        Err(_) => Answer::ClientGone,
        Ok(mut sent) => tokio::select! {
            biased;
            answer = sent.answer() => match answer {
                Ok(handled) => Answer::Handled(handled),
                Err(ClientRequestError::Failed { code, details, .. }) if given_back(&code, &details) => {
                    Answer::Released
                }
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

/// Whether a handler's failure `code` with `details` gives its request
/// back: a `call_cancelled` the program raised by stopping the call itself,
/// nothing closed under it, as dropping `serve` or one `CrawlCall` answers
/// (ADR 0102). A `call_cancelled` the browser raised, or one that closed
/// the page under the handler, is the handler's failure.
fn given_back(code: &str, details: &Map<String, Value>) -> bool {
    code == CALL_CANCELLED.name
        && serde_json::from_value::<CallCancelledDetails>(Value::Object(details.clone())).is_ok_and(
            |details| {
                matches!(
                    details,
                    CallCancelledDetails {
                        by: StoppedBy::Program,
                        closed: None,
                    }
                )
            },
        )
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
            Why::Blocked(cause) => Effect::Blocked(cause.clone()),
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
        | Answer::ClientGone
        | Answer::Released => Effect::Neutral,
    }
}
