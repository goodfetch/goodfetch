//! An attempt's tier race (`fetch.tier_race`; ADR 0040; the crate's
//! `tier_race` module): browser-fetch and a navigation started its race's
//! delay later in the same Session, the first page that can win handed to
//! the handler, the other cancelled, both counted, the race recorded. A
//! request a hidden link reached races with no navigation: a browser tier
//! never follows such a link (ADR 0035).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::CallContext;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::{Room, SecondCopies};
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{
    Chosen, FetchTiers, HttpTier, Judged, PooledSession, Race, RaceEnd, TierChoice, record_race,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::page::{PageId, PageResponse};
use goodfetch_protocol::usage::REQUESTS;
use serde_json::Map;
use tokio::sync::oneshot;

use super::{
    Answer, BLOCKED, BROWSER, On, PAGE_CHECK_FAILED, RACED, Served, Tiering, failed, judged, learn,
    reached_through_trap, read_by_browser, record_tier, refused_type,
};
use crate::lease::WorkItemLease;
use crate::tier_race::Raced;

/// The tier the Provenance of a race browser-fetch won names: the
/// `browser_fetch` its race names the winner, chosen by no one.
const BROWSER_FETCH: TierChoice = TierChoice {
    tier: FetchTier::BrowserFetch,
    forced: None,
};

impl On<'_> {
    /// A race of the attempt's tiers (`fetch.tier_race`, ADR 0040):
    /// browser-fetch at once and a navigation `raced`'s delay later, the
    /// first page that can win handed to the handler, the other cancelled
    /// (the module's doc). A handler that escalates browser-fetch's page
    /// runs again in the browser, as on `http`. Answers how it ended, the
    /// pending requests the page's rules merged, and browser-fetch's
    /// latency once its document arrived.
    pub(super) async fn race(
        &self,
        tiering: &Tiering,
        http: &HttpTier,
        raced: &Raced,
    ) -> (Answer, Vec<(String, String)>, Option<Duration>) {
        let (run, leased, lease) = (self.run, self.leased, self.lease);
        let tiers = &tiering.tiers;
        let started = run.clock().now();
        let hand_over = self
            .gating
            .hand_over(run, leased, lease, self.in_flight)
            .await;
        // The navigation's page takes the attempt's value as it opens.
        let handed_over = self.gating.tiers.hand_over(Arc::clone(&hand_over));
        let fetching = tiers.in_browser(http, run, self.in_browser, &hand_over, self.context);
        let (cancel, cancelled) = oneshot::channel::<()>();
        let (navigated, mut navigation) = oneshot::channel::<Navigated>();
        let (sending, mut sent) = oneshot::channel::<()>();
        // A request a hidden link reached is a decoy on any browser tier:
        // its race opens no navigation, which loses as a browser run of it
        // ends.
        let decoy = reached_through_trap(leased);
        if decoy.is_none() {
            run.spawn(
                Navigation {
                    run: run.clone(),
                    tiers: tiers.clone(),
                    pooled: self.in_browser.clone(),
                    url: leased.crawl_request.url.clone(),
                    context: self.context.clone(),
                    second_copies: tiering.second_copies.clone(),
                    domain_key: leased.waiting.key.clone(),
                    at: started + raced.delay,
                }
                .run(cancelled, sending, navigated),
            );
        }
        let document = Box::pin(async {
            let served = self
                .document(tiering, &fetching, &RACED, &hand_over)
                .await?;
            let page_id = served.page_id();
            let latency = run.clock().now().duration_since(started);
            // A document only a guess could read never wins: the
            // browser reads it (ADR 0119).
            // Nor does a Challenge: the navigation meets it in the
            // browser, and the domain learns the move (`escalation`).
            let challenged = match tiers.block(page_id).await {
                Some((block, _)) if block.checks_the_browser() => {
                    self.on_block(block.cause, &RACED);
                    true
                }
                _ => false,
            };
            let passed = tiers.escalated(page_id).is_none()
                && !challenged
                && matches!(
                    judged(run, tiering, page_id).await,
                    Some(Judged::Passed) | None
                );
            Ok::<_, Answer>((served, latency, passed && raced.agrees))
        });
        let ended = first_to_win(document, (&mut navigation, decoy.map(Lost::Decoy)), lease).await;
        // The loser is cancelled: browser-fetch's connection closed as its
        // future dropped, and a navigation not yet sent is never sent.
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
                browser_fetch: ended.browser_fetch(),
                navigation: ended.navigation(sent.try_recv().is_ok()),
            },
        )
        .await;
        let latency = ended.latency;
        let (answer, merged) = self.after_race(tiers, ended).await;
        drop(handed_over);
        (answer, merged, latency)
    }

    /// The attempt once its race ended as `ended` says: the winner's page
    /// handed to the handler, recorded as the race's, or how it ended.
    async fn after_race(
        &self,
        tiers: &FetchTiers,
        ended: RaceEnded,
    ) -> (Answer, Vec<(String, String)>) {
        let (run, leased) = (self.run, self.leased);
        match ended.won {
            Won::Ended(answer) => (answer, Vec::new()),
            Won::Neither => {
                let answer = match ended.navigation_lost {
                    Some(Lost::Failed(answer) | Lost::Decoy(answer)) => Some(answer),
                    _ => ended.fetch_lost,
                };
                (
                    answer.expect("invariant: a race neither run won ended with a failure"),
                    Vec::new(),
                )
            }
            Won::BrowserFetch(page_id) => {
                let merged = learn(run, tiers, &page_id, leased).await;
                let encoding = tiers.encoding(&page_id);
                let certificate_exception = tiers.certificate_exception(&page_id);
                record_tier(
                    run,
                    leased,
                    self.lease,
                    &BROWSER_FETCH,
                    Chosen::Raced,
                    encoding,
                    certificate_exception,
                )
                .await;
                let response = tiers.response(&page_id);
                let answer = self.handle(&page_id, FetchTier::Http, response).await;
                let escalated = tiers.escalated(&page_id);
                tiers.close(&page_id);
                let answer = match escalated {
                    Some(call) if !matches!(answer, Answer::LeaseEnded | Answer::ClientGone) => {
                        // The browser's run takes a whole lease of its own, and
                        // stages anew: what the first run staged goes with it.
                        self.lease.renew(self.lease.length());
                        self.lease.restage();
                        self.browser(tiers, &BROWSER, Chosen::Escalated(call)).await
                    }
                    None if self.moves(&answer, &RACED) => {
                        self.lease.renew(self.lease.length());
                        self.lease.restage();
                        self.browser(tiers, &BROWSER, Chosen::Escalated(BLOCKED))
                            .await
                    }
                    _ => answer,
                };
                (answer, merged)
            }
            Won::Navigation(page_id, response) => {
                if let Some(refused) = refused_type(tiers, &page_id, run, leased).await {
                    return (refused, Vec::new());
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
                    &BROWSER,
                    Chosen::Raced,
                    encoding,
                    certificate_exception,
                )
                .await;
                let answer = self.handle(&page_id, FetchTier::Browser, response).await;
                tiers.close_browser(&page_id).await;
                (answer, Vec::new())
            }
        }
    }
}

/// How a tier race ended: which run won, and how each that lost did.
struct RaceEnded {
    won: Won,
    /// Browser-fetch's latency, once its document arrived.
    latency: Option<Duration>,
    /// Browser-fetch's failure, once it lost.
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

impl RaceEnded {
    fn browser_fetch(&self) -> RaceEnd {
        match (&self.won, &self.fetch_lost) {
            (Won::BrowserFetch(_), _) => RaceEnd::Won,
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

/// Waits for the first of a race's `document`, browser-fetch's page with
/// its latency and whether it can win, and `navigation` that can win,
/// unless it lost before it started (`lost`), or for both to lose, or for
/// `lease` to end; `document` is dropped, and so cancelled, before it
/// answers, closing a page it served. A browser-fetch page that cannot win
/// is closed.
async fn first_to_win<'t>(
    mut document: std::pin::Pin<
        Box<impl Future<Output = Result<(Served<'t>, Duration, bool), Answer>>>,
    >,
    (navigation, lost): (&mut oneshot::Receiver<Navigated>, Option<Lost>),
    lease: &WorkItemLease,
) -> RaceEnded {
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
                Ok((served, took, true)) => {
                    ended.latency = Some(took);
                    break Won::BrowserFetch(served.keep());
                }
                Ok((served, took, false)) => {
                    ended.latency = Some(took);
                    drop(served);
                    ended.fetch_lost = Some(Answer::Failed {
                        code: PAGE_CHECK_FAILED.to_owned(),
                        retryable: false,
                        message: "The page browser-fetch served failed its expected page or its Label's comparison, and the race's navigation passed no page either.".to_owned(),
                        details: Map::new(),
                    });
                }
                Err(answer @ Answer::Failed { .. }) => ended.fetch_lost = Some(answer),
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
enum Won {
    /// Browser-fetch's page, served on `http`.
    BrowserFetch(PageId),
    /// The navigation's page, in the browser, and its document's
    /// response.
    Navigation(PageId, Option<PageResponse>),
    /// Neither: the attempt ends with the navigation's failure, else
    /// browser-fetch's.
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

/// How long a race's navigation whose domain is full waits before it asks
/// for room again: the Politeness file wakes only the admission's own
/// waiters.
const FULL_RETRY: Duration = Duration::from_millis(250);

/// A race's navigation, run as a task of its own: cancelled, it is never
/// sent, or its page closes, its load with it.
struct Navigation {
    run: Run,
    tiers: FetchTiers,
    pooled: PooledSession,
    url: String,
    context: CallContext,
    second_copies: SecondCopies,
    domain_key: String,
    /// When it starts: the race's delay after browser-fetch started.
    at: tokio::time::Instant,
}

impl Navigation {
    /// Waits for its start, then for room beside the attempt's own as a
    /// second copy, asking again every [`FULL_RETRY`] while its domain is
    /// full, as the navigation gate does, charges one request in Usage and navigates, unless
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
            match self.second_copies.take(&self.domain_key, clock.now()).await {
                Room::Taken(copy) => break copy,
                Room::Until(moment) => until = moment,
                // Room frees as the domain's requests end; the race's end
                // cancels the wait.
                Room::Full => until = clock.now() + FULL_RETRY,
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
            opened = self.tiers.browser(self.run.id(), &self.pooled, &self.url, &self.context) => Some(opened),
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
