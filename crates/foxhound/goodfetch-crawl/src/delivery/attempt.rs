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
//! recorded for every run sharing the domain. What robots.txt allows is
//! read here and applied by the gate of #883.

use std::sync::Arc;

use std::collections::BTreeSet;

use goodfetch_engine::CallContext;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::robots_txt::{self, Fetch};
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{
    FetchTiers, HandOver, HandedOver, HttpTier, Requester, RobotsTxtFetch, TierChoice,
};
use goodfetch_frontier::{DomainKeys, EndMerged, PageSeen, SharedRobotsTxt};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    LEASE_LOST, WORK_ITEM_LEASED,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::frontier::HttpBody;
use goodfetch_protocol::foxhound::progress::InFlightPhase;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_DELAY, SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
};
use goodfetch_protocol::page::NAVIGATION_FAILED;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::PointInTime;
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
    choice: Option<TierChoice>,
    second_copies: SecondCopies,
}

impl Tiering {
    /// `choice` on the Run's `tiers` and `http` tier, its hedges taking
    /// `second_copies`; no choice hands the handler its request alone.
    pub(super) fn of(
        tiers: &FetchTiers,
        http: Option<&HttpTier>,
        choice: Option<TierChoice>,
        second_copies: &SecondCopies,
    ) -> Self {
        Self {
            tiers: tiers.clone(),
            http: http.cloned(),
            choice,
            second_copies: second_copies.clone(),
        }
    }

    /// The tier the attempt starts on, if any.
    pub(super) fn tier(&self) -> Option<FetchTier> {
        self.choice.as_ref().map(|choice| choice.tier)
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
/// copy has none to use. A fetch made here is a request of the domain:
/// the document then waits `politeness.domain.delay`, and its start is
/// recorded for every run sharing the domain.
async fn read_robots_txt(run: &Run, http: &HttpTier, leased: &Leased) {
    let Some(location) = goodfetch_frontier::Url::parse(&leased.crawl_request.url)
        .ok()
        .and_then(|url| robots_txt::location(&url))
    else {
        return;
    };
    let politeness = share_robots_txt(run).await;
    let (sending, sent) = oneshot::channel::<()>();
    let fetch: Fetch<'_> = Box::pin(async {
        let _ = sending.send(());
        http.robots_txt(run, &location).await
    });
    let _read = run.robots_txt().copy(&location, fetch).await;
    let delay = run
        .settings()
        .and_then(|settings| settings.resolved().duration(POLITENESS_DOMAIN_DELAY.name))
        .filter(|delay| !delay.is_zero());
    let (Ok(()), Some(delay)) = (sent.await, delay) else {
        return;
    };
    run.clock().sleep(delay).await;
    if let Some(politeness) = &politeness
        && let Err(error) =
            goodfetch_frontier::started_again(politeness, &leased.waiting.key, run.clock().wall())
                .await
    {
        tracing::warn!(%error, run_id = %run.id(), "a document's start after its robots.txt was not recorded");
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
    (leased, session, counted): (Leased, ServingSession, Counted),
    lease: WorkItemLease,
    (tiering, gating): (Tiering, Gating),
    in_flight: SharedInFlight,
) -> Attempt {
    let on = On {
        run: &run,
        context: &context,
        leased: &leased,
        session: &session,
        lease: &lease,
        gating: &gating,
        in_flight: &in_flight,
    };
    let mut merged = Vec::new();
    let answer = match (&tiering.choice, &tiering.http) {
        (None, _) => {
            let params = on.params(None, None);
            let handed_over = gating.hand_over(&run, &leased, &lease, &in_flight).await;
            let answer = handled(&run, context.client(), &params, &lease).await;
            drop(handed_over);
            answer
        }
        (Some(choice), _) if choice.tier == FetchTier::Browser => {
            on.browser(&tiering.tiers, choice, None).await
        }
        (Some(choice), Some(http)) => {
            let (answer, learned) = on.http(&tiering, http, choice).await;
            merged = learned;
            answer
        }
        (Some(_), None) => Answer::Failed {
            code: NAVIGATION_FAILED.name.to_owned(),
            retryable: true,
            message: "The crawl's HTTP tier could not be built, so the request was not fetched."
                .to_owned(),
            details: Map::new(),
        },
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

/// What one attempt runs its handler with.
struct On<'a> {
    run: &'a Run,
    context: &'a CallContext,
    leased: &'a Leased,
    session: &'a ServingSession,
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

    /// The attempt on the `http` tier: its origin's robots.txt read and its
    /// document fetched under the lease, the Frontier taught its page, and
    /// the handler given a page served from it, which a call the tier
    /// cannot serve escalates under `auto`. Answers how it ended and the
    /// pending requests the page's rules merged.
    async fn http(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        choice: &TierChoice,
    ) -> (Answer, Vec<(String, String)>) {
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
                read_robots_txt(run, http, leased).await;
                tiers.serve(http, run, &leased.crawl_request, &requester, choice).await
            } => document.map_err(|error| failed(&error)),
            _ = lease.ended() => Err(Answer::LeaseEnded),
        };
        let page_id = match fetched {
            Ok(page_id) => page_id,
            Err(answer) => return (answer, Vec::new()),
        };
        let merged = learn(run, tiers, &page_id, leased).await;
        record_tier(run, leased, choice, None).await;
        self.in_flight
            .lock()
            .await
            .went_on(run, &leased.crawl_request.id, InFlightPhase::Handling);
        let params = self.params(Some(FetchTier::Http), Some(page_id.clone()));
        let handed_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        let answer = handled(run, self.context.client(), &params, lease).await;
        drop(handed_over);
        let escalated = tiers.escalated(&page_id);
        tiers.close(&page_id);
        let answer = match escalated {
            // The first answer, whatever it says, is discarded.
            Some(call) if !matches!(answer, Answer::LeaseEnded | Answer::ClientGone) => {
                // The browser's run takes a whole lease of its own.
                lease.renew(lease.length());
                self.browser(tiers, choice, Some(call)).await
            }
            _ => answer,
        };
        (answer, merged)
    }

    /// The attempt on the `browser` tier: the Run's pages handed over,
    /// Collie's page opened and navigated to the request's URL through the
    /// gate under the lease, and the handler given it, closed once it
    /// answered. `escalated` names the call that moved the request here
    /// from a cheaper tier.
    async fn browser(
        &self,
        tiers: &FetchTiers,
        choice: &TierChoice,
        escalated: Option<&'static str>,
    ) -> Answer {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let handed_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        let opened = tokio::select! {
            biased;
            opened = tiers.browser(run.id(), &leased.crawl_request.url, self.context) => {
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
            forced: choice.forced.clone().filter(|_| escalated.is_none()),
        };
        record_tier(run, leased, &browser, escalated).await;
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
/// attempt, and whether it was forced, the engine's choice, or an
/// escalation from a cheaper tier by the call `escalated` (ADR 0023).
async fn record_tier(
    run: &Run,
    leased: &Leased,
    choice: &TierChoice,
    escalated: Option<&'static str>,
) {
    goodfetch_fetch_tier::record_tier(
        run,
        &leased.crawl_request.id,
        &leased.crawl_request.label,
        choice,
        escalated,
    )
    .await;
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
                    Answer::Failed { code, retryable, message, details }
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
