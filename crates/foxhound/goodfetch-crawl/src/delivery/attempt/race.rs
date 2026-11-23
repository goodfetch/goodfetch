//! An attempt's tier race (`fetch.tier_race`; ADR 0040; the crate's
//! `tier_race` module): its first request on the document tier and a
//! navigation started its race's delay later, the first page that can win
//! handed to the handler, the other cancelled, both counted, the race
//! recorded. The first request leaves from where its Label learned to
//! leave from (#2421): from the own transport of a Session drawn for it
//! beside the attempt's when its Label learned to send its HTTP from
//! there, else through browser-fetch from the hop page of the attempt's
//! Session, which the navigation runs in either way. A request a hidden
//! link reached races with no navigation: a browser tier never follows
//! such a link (ADR 0035).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::CallContext;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{
    Chosen, FetchTiers, HandOver, HttpTier, Judged, PageFacts, PooledSession, Race, RaceEnd,
    TierChoice, record_race, room_taken, tier_provenance,
};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::hook::CrawlHookStage;
use goodfetch_protocol::page::{PageId, PageResponse};
use goodfetch_protocol::usage::REQUESTS;
use goodfetch_session_pool::SendsFrom;
use tokio::sync::oneshot;

use super::{
    Answer, AttemptFailure, BLOCKED, BROWSER, FETCHED, Found, On, PAGE_CHECK_FAILED, Served,
    Tiering, failed, judged, reached_through_trap, read_by_browser, record_tier, refused_type,
};
use crate::lease::WorkItemLease;
use crate::tier_race::Raced;

impl On<'_> {
    /// A race of the attempt's tiers (`fetch.tier_race`, ADR 0040): its
    /// first request at once and a navigation `raced`'s delay later, the
    /// first page that can win handed to the handler, the other cancelled
    /// (the module's doc). A handler that escalates the first request's
    /// page runs again in the browser, in the attempt's Session, as on
    /// `http`. Answers how it ended, `raced` with its first request's
    /// latency once its document arrived, and whether the attempt's answer
    /// is that of a first request sent from a Session's own transport.
    pub(super) async fn race(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        raced: &Raced,
    ) -> (Answer, (Option<(Raced, Duration)>, bool)) {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        // The race is one fetch of the request: its before-request Hooks run
        // once, before either side leaves (ADR 0104).
        if let Some(answer) = self.stages(&[CrawlHookStage::BeforeRequest]).await {
            return (answer, (None, false));
        }
        let tiers = &tiering.tiers;
        let (first, http) = match self.sent_from(tiering, http) {
            Ok(sent_from) => sent_from,
            Err(answer) => return (answer, (None, false)),
        };
        let started = run.clock().now();
        let hand_over = self.hand_over;
        // The navigation's page takes the attempt's value as it opens.
        let handed_over = tiers.hand_over(Arc::clone(hand_over));
        let fetching = first.client(tiers, http);
        // Provenance names the tier that sent the first request (#2421).
        let sent_on = fetching.sent_on(FetchTier::Http);
        let (cancel, cancelled) = oneshot::channel::<()>();
        let (navigated, mut navigation) = oneshot::channel::<Navigated>();
        let (sending, mut sent) = oneshot::channel::<()>();
        // A request a hidden link reached is a decoy on any browser tier:
        // its race opens no navigation, which loses as a browser run of it
        // ends.
        let decoy = reached_through_trap(leased);
        if decoy.is_none() {
            run.spawn(
                self.watch.over(
                    Navigation {
                        run: run.clone(),
                        tiers: tiers.clone(),
                        hand_over: Arc::clone(hand_over),
                        pooled: self.in_browser.clone(),
                        crawl_request: leased.crawl_request.clone(),
                        context: self.context.clone(),
                        second_copies: tiering.second_copies.clone(),
                        domain_key: leased.waiting.key.clone(),
                        at: started + raced.delay,
                    }
                    .run(cancelled, sending, navigated),
                ),
            );
        }
        let document = Box::pin(async {
            // A raced chain a hop ends `skipped` teaches nothing: the
            // navigation may still serve the request.
            let (served, facts) = first
                .document(tiering, &fetching, &FETCHED, hand_over)
                .await
                .map_err(|unserved| unserved.answer)?;
            let latency = run.clock().now().duration_since(started);
            // The first request's page is checked as a run on `http`
            // checks its own. A document only a guess could read never
            // wins: the browser reads it (ADR 0119). Nor does a Challenge:
            // the navigation meets it in the browser, and the domain
            // learns the move (`escalation`).
            let passed = match first
                .found(tiering, served.page_id(), &facts, (&FETCHED, Chosen::Raced))
                .await
            {
                Found::Unreadable(_) => false,
                Found::Blocked(cause, _) => {
                    first.on_block(&cause, &FETCHED, Chosen::Raced);
                    false
                }
                Found::Readable => matches!(
                    judged(run, tiering, served.page_id()).await,
                    Some(Judged::Passed) | None
                ),
            };
            Ok::<_, Answer>((served, facts, latency, passed && raced.agrees))
        });
        let ended = first_to_win(document, (&mut navigation, decoy.map(Lost::Decoy)), lease).await;
        // The loser is cancelled: the first request's connection closed as
        // its future dropped, and a navigation not yet sent is never sent.
        drop(cancel);
        if !matches!(ended.won, Won::Navigation(..)) {
            navigation.close();
            if let Ok(Navigated::Opened(page_id, _)) = navigation.try_recv() {
                tiers.close_browser(&page_id).await;
            }
        }
        record_race(
            run,
            &Race {
                request_id: &leased.crawl_request.id,
                label: &leased.crawl_request.label,
                after: raced.delay,
                first_sent_on: sent_on,
                first: ended.first(),
                navigation: ended.navigation(sent.try_recv().is_ok()),
            },
        )
        .await;
        let latency = ended.latency;
        let (answer, by_first) = self.after_race(&first, (tiers, sent_on), ended).await;
        drop(handed_over);
        let from_transport = by_first && first.sends_from == SendsFrom::Transport;
        let latency = latency.map(|latency| (raced.clone(), latency));
        (answer, (latency, from_transport))
    }

    /// The attempt once its race ended as `ended` says: the winner's page
    /// handed to the handler, recorded as the race's, its first request
    /// as `first` sent it on `sent_on`, or how it ended. Answers whether
    /// its answer is the first request's own, neither the navigation's
    /// nor an escalation's.
    async fn after_race(
        &self,
        first: &On<'_>,
        (tiers, sent_on): (&FetchTiers, FetchTier),
        ended: RaceEnded<'_>,
    ) -> (Answer, bool) {
        let (run, leased) = (self.run, self.leased);
        match ended.won {
            Won::Ended(answer) => (answer, false),
            Won::Neither => {
                let (answer, by_first) = match ended.navigation_lost {
                    Some(Lost::Failed(answer) | Lost::Decoy(answer)) => (Some(answer), false),
                    _ => (ended.fetch_lost, true),
                };
                (
                    answer.expect("invariant: a race neither run won ended with a failure"),
                    by_first,
                )
            }
            Won::First(served, facts) => {
                // `served` is held until its handler answered, as
                // `On::on_http` holds its own: an attempt abandoned while
                // its handler runs, as an abort abandons it, closes its
                // page as it drops, so the page never holds the Run's
                // handles past the Run.
                let page_id = served.page_id();
                self.lessons
                    .learn(tiers, page_id, &leased.crawl_request)
                    .await;
                record_tier(
                    run,
                    leased,
                    self.lease,
                    tier_provenance(
                        &TierChoice {
                            tier: sent_on,
                            forced: None,
                        },
                        Chosen::Raced,
                    ),
                    facts.encoding,
                    facts.certificate_exception,
                )
                .await;
                let answer = first.handle(page_id, FetchTier::Http, facts.response).await;
                let escalated = self.escalated(served.close());
                // An escalation runs again in the browser, in the attempt's
                // Session, whatever sent the first request.
                let (answer, by_first) = match escalated {
                    Some(call) if !answer.unanswered() => {
                        // The browser's run takes a whole lease of its own, and
                        // stages anew: what the first run staged goes with it.
                        self.lease.renew(self.lease.length());
                        self.lease.restage();
                        let answer = self.browser(tiers, &BROWSER, Chosen::Escalated(call)).await;
                        (answer, false)
                    }
                    None if self.moves(&answer, &FETCHED, Chosen::Raced) => {
                        self.lease.renew(self.lease.length());
                        self.lease.restage();
                        let answer = self
                            .browser(tiers, &BROWSER, Chosen::Escalated(BLOCKED))
                            .await;
                        (answer, false)
                    }
                    _ => (answer, true),
                };
                (answer, by_first)
            }
            Won::Navigation(page_id, response) => {
                if let Some(refused) = refused_type(tiers, &page_id, run, leased).await {
                    return (refused, false);
                }
                let encoding = read_by_browser(tiers, &page_id, response.as_ref());
                let certificate_exception = match &response {
                    Some(response) => tiers.browser_exception(&page_id, &response.url).await,
                    None => None,
                };
                record_tier(
                    run,
                    leased,
                    self.lease,
                    tier_provenance(&BROWSER, Chosen::Raced),
                    encoding,
                    certificate_exception,
                )
                .await;
                let answer = self.handle(&page_id, FetchTier::Browser, response).await;
                tiers.close_browser(&page_id).await;
                (answer, false)
            }
        }
    }
}

/// How a tier race ended: which run won, and how each that lost did.
struct RaceEnded<'t> {
    won: Won<'t>,
    /// The first request's latency, once its document arrived.
    latency: Option<Duration>,
    /// The first request's failure, once it lost.
    fetch_lost: Option<Answer>,
    /// How the navigation lost, once it did.
    navigation_lost: Option<Lost>,
}

/// How a race's navigation lost.
enum Lost {
    /// With this failure.
    Failed(Answer),
    /// Never sent.
    NotSent,
    /// Never started: its request is a decoy on a browser tier, which ends
    /// as this answer says.
    Decoy(Answer),
}

impl RaceEnded<'_> {
    /// How the first request ended.
    fn first(&self) -> RaceEnd {
        match (&self.won, &self.fetch_lost) {
            (Won::First(..), _) => RaceEnd::Won,
            (_, Some(_)) => RaceEnd::Failed,
            _ => RaceEnd::Cancelled,
        }
    }

    /// How the navigation ended, `sent` or not.
    fn navigation(&self, sent: bool) -> RaceEnd {
        match (&self.won, &self.navigation_lost) {
            (Won::Navigation(..), _) => RaceEnd::Won,
            (_, Some(Lost::Failed(_))) => RaceEnd::Failed,
            (_, Some(Lost::NotSent | Lost::Decoy(_))) => RaceEnd::NotSent,
            _ if sent => RaceEnd::Cancelled,
            _ => RaceEnd::NotSent,
        }
    }
}

/// Waits for the first of a race's `document`, its first request's page
/// with its latency and whether it can win, and `navigation` that can win,
/// unless it lost before it started (`lost`), or for both to lose, or for
/// `lease` to end; `document` is dropped, and so cancelled, before it
/// answers, closing a page it served. A first request's page that cannot
/// win is closed.
async fn first_to_win<'t>(
    mut document: std::pin::Pin<
        Box<impl Future<Output = Result<(Served<'t>, PageFacts, Duration, bool), Answer>>>,
    >,
    (navigation, lost): (&mut oneshot::Receiver<Navigated>, Option<Lost>),
    lease: &WorkItemLease,
) -> RaceEnded<'t> {
    let mut ended = RaceEnded {
        won: Won::Neither,
        latency: None,
        fetch_lost: None,
        navigation_lost: lost,
    };
    ended.won = loop {
        tokio::select! {
            biased;
            _ = lease.ended() => break Won::Ended(Answer::LeaseEnded),
            fetched = &mut document, if ended.fetch_lost.is_none() => match fetched {
                Ok((served, facts, took, true)) => {
                    ended.latency = Some(took);
                    break Won::First(served, Box::new(facts));
                }
                Ok((served, _, took, false)) => {
                    ended.latency = Some(took);
                    drop(served);
                    ended.fetch_lost = Some(Answer::failed(AttemptFailure::named(
                        PAGE_CHECK_FAILED,
                        false,
                        "The page the race's first request served failed its expected page or its Label's comparison, and the race's navigation passed no page either.".to_owned(),
                    )));
                }
                Err(answer @ Answer::Failed(_)) => ended.fetch_lost = Some(answer),
                Err(answer) => break Won::Ended(answer),
            },
            answered = &mut *navigation, if ended.navigation_lost.is_none() => match answered {
                Ok(Navigated::Opened(page_id, response)) => break Won::Navigation(page_id, response),
                Ok(Navigated::Failed(error)) => ended.navigation_lost = Some(Lost::Failed(failed(&error))),
                Ok(Navigated::NotSent) | Err(_) => ended.navigation_lost = Some(Lost::NotSent),
            },
        }
        if ended.fetch_lost.is_some() && ended.navigation_lost.is_some() {
            break Won::Neither;
        }
    };
    ended
}

/// Which run of a tier race won, if one did.
enum Won<'t> {
    /// The first request's page, served on `http` and held until its
    /// handler answered, and what it held of its document as it was
    /// served.
    First(Served<'t>, Box<PageFacts>),
    /// The navigation's page, in the browser, and its document's
    /// response.
    Navigation(PageId, Option<PageResponse>),
    /// Neither: the attempt ends with the navigation's failure, else the
    /// first request's.
    Neither,
    /// The attempt ended before either won: its lease, its Connection, or
    /// a refusal that holds for both, such as its robots.txt.
    Ended(Answer),
}

/// What a race's navigation answered.
enum Navigated {
    /// Its page, navigated, passed the Page check, and its document's
    /// response.
    Opened(PageId, Option<PageResponse>),
    /// It failed, its Page check included.
    Failed(CallError),
    /// It was never sent: Usage had none left.
    NotSent,
}

/// A race's navigation, run as a task of its own: cancelled, it is never
/// sent, or its page closes, its load with it.
struct Navigation {
    run: Run,
    tiers: FetchTiers,
    /// The attempt's hand-over, whose value its page takes.
    hand_over: Arc<HandOver>,
    pooled: PooledSession,
    /// The crawl request it fetches, with its method and body.
    crawl_request: CrawlRequest,
    context: CallContext,
    second_copies: SecondCopies,
    domain_key: String,
    /// When it starts: the race's delay after the first request started.
    at: tokio::time::Instant,
}

impl Navigation {
    /// Waits for its start, then for room beside the attempt's own as a
    /// second copy, asking again as the navigation gate does
    /// ([`room_taken`]), charges one request in Usage and navigates, unless
    /// `cancelled` resolves first; tells `sending` once it is sent,
    /// answers on `navigated`, and closes the page of a navigation no one
    /// waits for any more.
    async fn run(
        self,
        mut cancelled: oneshot::Receiver<()>,
        sending: oneshot::Sender<()>,
        navigated: oneshot::Sender<Navigated>,
    ) {
        let clock = self.run.clock();
        let mut until = self.at;
        let copy = loop {
            tokio::select! {
                biased;
                _ = &mut cancelled => return,
                () = clock.sleep_until(until) => {}
            }
            // Room frees as the domain's requests end; the race's end
            // cancels the wait.
            let room = self.second_copies.take(&self.domain_key, clock.now()).await;
            match room_taken(room, clock.now()) {
                Ok(copy) => break copy,
                Err(at) => until = at,
            }
        };
        if !matches!(
            cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ) || self.run.charge(REQUESTS, 1).await.is_err()
        {
            self.second_copies.give_back(copy).await;
            let _ = navigated.send(Navigated::NotSent);
            return;
        }
        let _ = sending.send(());
        let opened = tokio::select! {
            biased;
            // Dropped, the navigation's page closes, its load with it.
            _ = &mut cancelled => None,
            opened = self.tiers.browser(&self.hand_over, &self.pooled, &self.crawl_request, &self.context) => Some(opened),
        };
        self.second_copies.give_back(copy).await;
        let Some(opened) = opened else {
            return;
        };
        match opened {
            Ok((page_id, answered)) => {
                if let Err(Navigated::Opened(page_id, _)) =
                    navigated.send(Navigated::Opened(page_id, answered.response))
                {
                    self.tiers.close_browser(&page_id).await;
                }
            }
            Err(error) => {
                let _ = navigated.send(Navigated::Failed(error));
            }
        }
    }
}
