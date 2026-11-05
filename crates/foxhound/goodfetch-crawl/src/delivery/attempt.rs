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
//! disallows is never sent and ends `skipped` with `robots_txt`; one whose
//! robots.txt cannot be read, with no earlier copy, is not sent either, the
//! failed fetch counting as one of its attempts and backing its domain
//! off, and it ends `skipped` with `robots_txt` once its retries ran out.
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

use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallContext;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::client::{ClientRequestError, Sent};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::RunSettings;
use goodfetch_fetch_tier::{
    AddressCheck, Capture, Chosen, ContentTypes, FetchAdmission, FetchTiers, HandOver, HttpTier,
    Judged, PAGE_CHECK_CALL, PooledSession, ReplayPlan, Requester, RobotsTxtFetch, SignalFiles,
    Stop, TierChoice, infer_plan, tier_serves,
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
    POLITENESS_DOMAIN_ADAPTIVE_DELAY, POLITENESS_DOMAIN_DELAY, SITE_PREFERENCES_MAX_CRAWL_DELAY,
    SITE_PREFERENCES_RESPECT_ROBOTS_TXT, SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
};
use goodfetch_protocol::page::{NAVIGATION_FAILED, PageId, PageResponse};
use goodfetch_protocol::settings::SESSION_PAGE_CHECK;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_session_pool::Effect;
use goodfetch_session_pool::escalation::{self, Action, Met};
use serde_json::{Map, Value};
use tokio::sync::{Mutex, oneshot};

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

use learning::learn;
use site_preferences::{
    escaped_refusal, honoured_purpose, read_robots_txt, reads_robots_txt, respects_robots_txt,
    robots_txt_ignored, robots_txt_refusal, share_robots_txt, use_signals_declared,
    use_signals_refusal,
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
            AttemptPlan::Alone => None,
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
    /// The main-content step the request's Label chooses with
    /// `content.extractor`, over the Run's.
    pub(super) extractor: Option<goodfetch_fetch_tier::BuiltIn>,
    /// The encoding `fetch.encoding` forces on the request's own
    /// document: its Label's, else the Run's (ADR 0119).
    pub(super) encoding: Option<&'static encoding_rs::Encoding>,
    /// The types `crawl.content_types` lets the request's own document
    /// be: its Label's, else the Run's (ADR 0119).
    pub(super) content_types: ContentTypes,
    /// What a code-free crawl handles its requests with, in place of the
    /// program's handler; none when the program handles them.
    pub(super) code_free: Option<Arc<CodeFree>>,
    /// The AI-use signal files the Run fetched, `TDMRep`'s well-known
    /// file and RSL licenses, each once for all its requests.
    pub(super) signal_files: SignalFiles,
    /// The attempt's HTTP admission, given back once its document is
    /// fetched, else once it ends.
    pub(super) fetch_admission: FetchAdmission,
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
        let settings = run.settings().map(RunSettings::resolved);
        // A crawl whose Politeness paces nothing records no start, send or
        // latency in it (ADR 0138).
        let politeness = if settings.is_none_or(goodfetch_frontier::paces) {
            share_robots_txt(run).await
        } else {
            None
        };
        let domain_delay = settings
            .and_then(|settings| settings.duration(POLITENESS_DOMAIN_DELAY.name))
            .unwrap_or_default();
        Arc::new(HandOver {
            run: run.clone(),
            domain_key: leased.waiting.key.clone(),
            keys: settings.map(DomainKeys::of).unwrap_or_default(),
            second_copies: self.second_copies.clone(),
            politeness,
            robots: self.robots.clone(),
            operator_token: settings.and_then(|settings| {
                settings
                    .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
                    .as_str()
                    .map(str::to_owned)
            }),
            domain_delay,
            adaptive_delay: settings.is_some_and(|settings| {
                settings.boolean(POLITENESS_DOMAIN_ADAPTIVE_DELAY.name) == Some(true)
            }),
            pace: settings
                .map(goodfetch_frontier::Pace::of)
                .unwrap_or_default(),
            respect_robots_txt: settings.is_none_or(|settings| {
                settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) != Some(false)
            }),
            max_crawl_delay: settings
                .and_then(|settings| settings.duration(SITE_PREFERENCES_MAX_CRAWL_DELAY.name)),
            use_signals: settings.and_then(honoured_purpose),
            signal_files: self.signal_files.clone(),
            lease: Arc::new(HandlerLease {
                lease: lease.clone(),
                run: run.clone(),
                request_id: leased.crawl_request.id.clone(),
                in_flight: Arc::clone(in_flight),
            }),
            fetch_admission: self.fetch_admission.clone(),
            egress: Arc::clone(&self.egress),
            addresses: self.addresses.clone(),
            page_check: self.page_check.clone(),
            extractor: self.extractor,
            encoding: self.encoding,
            content_types: self.content_types.clone(),
            capture: None,
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
            let params = on.params(None, None, None);
            let hand_over = gating.hand_over(&run, &leased, &lease, &in_flight).await;
            let handed_over = gating.tiers.hand_over(hand_over);
            let answer = handled(
                &run,
                &context,
                gating.code_free.as_deref(),
                (&params, &sent_at),
                &lease,
            )
            .await;
            drop(handed_over);
            answer
        }
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
            let (answer, learned, weighed) = on.sample(&tiering, http).await;
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
        ) => Answer::Failed {
            code: NAVIGATION_FAILED.name.to_owned(),
            retryable: true,
            message: "The crawl's HTTP tier could not be built, so the request was not fetched."
                .to_owned(),
            details: Map::new(),
        },
    };
    Attempt {
        document: document.into_inner(),
        handed_at: sent_at.into_inner(),
        moved: moved.into_inner().is_some(),
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

/// A page a tier served for an attempt, held until it is handed out
/// ([`Served::keep`]): dropped before, as the future that holds it is when
/// a race's navigation wins, it closes on its tiers, so no served page
/// outlives its attempt, nor holds its Run's handles past the Run's end.
struct Served<'t> {
    tiers: &'t FetchTiers,
    /// The page, until it is handed out.
    page_id: Option<PageId>,
}

impl Served<'_> {
    /// The page's ID.
    fn page_id(&self) -> &PageId {
        self.page_id
            .as_ref()
            .expect("invariant: a served page is held until it is handed out")
    }

    /// Hands the page out: its holder closes it from now on.
    fn keep(mut self) -> PageId {
        self.page_id
            .take()
            .expect("invariant: a served page is handed out once")
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
    gating: &'a Gating,
    in_flight: &'a SharedInFlight,
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

    /// The call that escalated the served page `page_id` once its handler
    /// answered, if one did: `blocked` names a Challenge or a captcha the
    /// handler's own navigation met under `auto`, which moved the request
    /// to the browser, a move noted as [`On::on_block`] notes one, for its
    /// domain to learn (spec #600).
    fn escalated(&self, tiers: &FetchTiers, page_id: &PageId) -> Option<&'static str> {
        let escalated = tiers.escalated(page_id);
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
            Answer::Failed { code, details, .. } => block_cause(code, details)
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
    ) -> (Answer, Vec<(String, String)>) {
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
    ) -> (Answer, Vec<(String, String)>, Option<Weighed>) {
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
        let capture = Capture::default();
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
            Answer::Failed { code, details, .. } if walled(code, details) => Some(Weighed::Walled),
            Answer::Handled(CrawlRequestHandled {
                records,
                skipped: None,
            }) => {
                let browser_records =
                    [browser_staged.records.records(), records.as_slice()].concat();
                let browser_run = StagedRun {
                    records: &browser_records,
                    enqueues: &browser_staged.enqueues,
                    ended_at: browser.ended_at.as_deref(),
                    encoding: browser.encoding.as_deref(),
                };
                if matches_browser(&run, &cheaper, &browser_run) {
                    Some(Weighed::Matched)
                } else {
                    let light = light.as_ref().is_some_and(|(light, staged)| {
                        matches_browser(light, staged, &browser_run)
                    });
                    // `discovered_api` serves the document `http` did, so
                    // its run can only differ from that one where `http`
                    // escalated at a call only it serves: a network wait.
                    let replayed = match run.escalated {
                        Some(call)
                            if tier_serves(call, FetchTier::DiscoveredApi)
                                && !tier_serves(call, FetchTier::Http) =>
                        {
                            self.replayed(tiering, http, &capture, &browser_run).await
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
        matches_browser(&run, &staged, browser).then_some(plan)
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
    ) -> Result<(HttpRun, Vec<(String, String)>), (Answer, Vec<(String, String)>)> {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let tiers = &tiering.tiers;
        let hand_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        // A Session that may move to the browser, under `auto`, sends its
        // HTTP from inside its browser, so its fingerprint and cookies stay
        // the Session's on both tiers (ADR 0020; pitfall H20), and so does
        // one forced onto `browser_fetch`.
        let in_browser;
        let http = if choice.forced.is_none() || choice.tier == FetchTier::BrowserFetch {
            in_browser = tiers.in_browser(http, run, self.in_browser, &hand_over, self.context);
            &in_browser
        } else {
            http
        };
        let page_id = self
            .document(tiering, http, choice, &hand_over)
            .await
            .map_err(|answer| (answer, Vec::new()))?
            .keep();
        *self.document.lock().await = tiers.response(&page_id);
        // A `discovered_api` page replays its Label's calls (ADR 0020).
        if let Some(plan) = plan {
            tiers.replay_on(&page_id, plan.clone());
        }
        // A document only a guess could read escalates at once under
        // `auto`, before the Frontier learns from it or its handler runs
        // (ADR 0119).
        if let Some(call) = tiers.escalated(&page_id) {
            tiers.close(&page_id);
            record_tier(run, leased, lease, choice, how, None, None).await;
            let message =
                "The document declares no encoding and is not UTF-8: only the browser reads it.";
            return Ok((
                escalated_unhandled(ATTEMPT_ENDED.name, message, call),
                Vec::new(),
            ));
        }
        if let Some(challenged) = self.challenged(tiers, &page_id, choice, how).await {
            return challenged;
        }
        let encoding = tiers.encoding(&page_id);
        let certificate_exception = tiers.certificate_exception(&page_id);
        record_tier(
            run,
            leased,
            lease,
            choice,
            how,
            encoding.clone(),
            certificate_exception,
        )
        .await;
        match judged(run, tiering, &page_id).await {
            Some(Judged::Passed) | None => {}
            Some(Judged::Failed(error)) => {
                tiers.close(&page_id);
                return Err((failed(&error), Vec::new()));
            }
            Some(Judged::Escalated) => {
                tiers.close(&page_id);
                // Its handler never ran: the run escalates at the Page
                // check, and the caller runs the request in the browser.
                let message = "The page failed its expected page on a tier short of the browser.";
                return Ok((
                    escalated_unhandled(PAGE_CHECK_FAILED, message, PAGE_CHECK_CALL),
                    Vec::new(),
                ));
            }
        }
        let merged = self.taught(tiers, &page_id, how).await;
        self.in_flight
            .lock()
            .await
            .went_on(run, &leased.crawl_request.id, InFlightPhase::Handling);
        let document = tiers.not_document(&page_id).is_none();
        let url = tiers.response(&page_id).map(|response| response.url);
        let params = self.params(
            Some(choice.tier),
            Some(page_id.clone()),
            tiers.response(&page_id),
        );
        let handed_over = self.gating.tiers.hand_over(hand_over);
        let answer = handled(
            run,
            self.context,
            self.gating.code_free.as_deref(),
            (&params, self.sent_at),
            lease,
        )
        .await;
        drop(handed_over);
        let escalated = self.escalated(tiers, &page_id);
        tiers.close(&page_id);
        Ok((
            HttpRun {
                answer,
                escalated,
                document,
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
    async fn taught(
        &self,
        tiers: &FetchTiers,
        page_id: &PageId,
        how: Chosen,
    ) -> Vec<(String, String)> {
        if matches!(how, Chosen::Sample(_)) {
            return Vec::new();
        }
        learn(self.run, tiers, page_id, self.leased).await
    }

    /// How the run on `choice`, its page `page_id` served, ends when the
    /// Block classification finds a Challenge on it: under `auto` it moves
    /// to the browser, before the Frontier learns from its page or its
    /// handler runs, and under a forced tier it ends `blocked`
    /// (`escalation`); `None` for any other page. Other Block causes still
    /// reach the handler until the proxy-tier escalation ends them (#1137).
    async fn challenged(
        &self,
        tiers: &FetchTiers,
        page_id: &PageId,
        choice: &TierChoice,
        how: Chosen,
    ) -> Option<Result<(HttpRun, Vec<(String, String)>), (Answer, Vec<(String, String)>)>> {
        let (block, error) = tiers
            .block(page_id)
            .await
            .filter(|(block, _)| block.checks_the_browser())?;
        tiers.close(page_id);
        record_tier(self.run, self.leased, self.lease, choice, how, None, None).await;
        Some(match self.on_block(block.cause, choice) {
            Action::MoveToBrowser => Ok((
                escalated_unhandled(
                    BLOCKED,
                    "A Challenge met on a tier short of the browser moved the request there.",
                    BLOCKED,
                ),
                Vec::new(),
            )),
            Action::Blocked => Err((failed(&error), Vec::new())),
        })
    }

    /// The document of the attempt's request fetched on `http` under the
    /// lease, served as `choice` says, once its origin's robots.txt was
    /// read, and opened as a served page, its pages gated with
    /// `hand_over`; one whose AI-use signals refuse the Run's Purpose is
    /// closed unhandled. Answers the page, held from the moment it was
    /// served so that dropping the future closes it, or how the attempt
    /// ended.
    async fn document<'t>(
        &self,
        tiering: &'t Tiering,
        http: &HttpTier,
        choice: &TierChoice,
        hand_over: &Arc<HandOver>,
    ) -> Result<Served<'t>, Answer> {
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
                let read = if reads_robots_txt(run) {
                    read_robots_txt(run, http, leased, &*hand_over.lease).await
                } else {
                    None
                };
                if let Some(refused) = robots_txt_refusal(run, leased, read.as_deref()) {
                    return Err(refused);
                }
                if robots_txt_ignored(run, leased, read.as_deref()) {
                    lease.robots_txt_ignored();
                }
                if let Some(refused) = use_signals_declared(
                    run,
                    http,
                    leased,
                    read.as_deref(),
                    &self.gating.signal_files,
                )
                .await
                {
                    return Err(refused);
                }
                let served = if choice.tier == FetchTier::LightBrowser {
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
                            (self.context.runs().home(), &pinned_by),
                        )
                        .await
                } else {
                    tiers
                        .serve(http, run, &leased.crawl_request, &requester, choice, hand_over)
                        .await
                };
                served
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
        let served = Served {
            tiers,
            page_id: Some(fetched?),
        };
        if let Some(refused) = use_signals_refusal(
            run,
            (tiers, http),
            served.page_id(),
            &self.gating.signal_files,
        )
        .await
        {
            return Err(refused);
        }
        Ok(served)
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
            self.gating.code_free.as_deref(),
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
        let mut hand_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        if let Some(capture) = capture
            && let Some(owned) = Arc::get_mut(&mut hand_over)
        {
            owned.capture = Some(capture.clone());
        }
        let handed_over = self.gating.tiers.hand_over(Arc::clone(&hand_over));
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
            self.gating.code_free.as_deref(),
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
/// in (ADRs 0118 and 0119).
fn matches_browser(run: &HttpRun, staged: &SoFar, browser: &StagedRun<'_>) -> bool {
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
    // No field is named removed yet: the extraction's removal reports none
    // per attempt.
    weighing::weigh(
        &StagedRun {
            records: &records,
            enqueues: &staged.enqueues,
            ended_at: run.url.as_deref(),
            encoding: run.encoding.as_deref(),
        },
        browser,
        &std::collections::BTreeSet::new(),
    )
}

/// Whether a run that failed with `code` and `details` met a consent
/// wall: `blocked` with `consent_required` (ADR 0048).
fn walled(code: &str, details: &Map<String, Value>) -> bool {
    code == BLOCKED
        && details.get("cause").and_then(Value::as_str)
            == Some(BlockCause::ConsentRequired.as_str())
}

/// A run on the `http` tier that escalated at `call` before its handler
/// ran, its answer failing with `code` and `message`, which the browser's
/// run replaces.
fn escalated_unhandled(code: &str, message: &str, call: &'static str) -> HttpRun {
    HttpRun {
        answer: Answer::Failed {
            code: code.to_owned(),
            retryable: false,
            message: message.to_owned(),
            details: Map::new(),
        },
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

/// An attempt that ended with `error` before its handler was asked; a
/// page marked `noindex` that failed its Page check is a decoy instead
/// (ADR 0035).
fn failed(error: &CallError) -> Answer {
    let details = error.details().cloned().unwrap_or_default();
    if error.code() == PAGE_CHECK_FAILED
        && details.get("noindex").and_then(Value::as_bool) == Some(true)
    {
        let url = details
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let status = details
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .unwrap_or_default();
        return decoy::noindexed(url, status);
    }
    Answer::Failed {
        code: error.code().to_owned(),
        retryable: error.retryable(),
        message: error.message().to_owned(),
        details,
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
            handled = code_free.handled(params, context, run.point_in_time()) => match handled {
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
                    lease.commit(run.state(), run.secrets().mask().masks(), handled.records, run.point_in_time()).await
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
