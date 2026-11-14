//! The HTTP tier's redundant requests (ADRs 0040, 0045, 0086 and 0106;
//! RT-20), one Run's each, so nothing is shared across runs.
//!
//! **Coalescing** (`fetch.coalesce`, on by default): a crawl request that
//! [`repeatable`] allows, identical to one in flight from the same Session
//! (the same method, URL, headers and body), waits for that one's answer
//! instead of sending its own; Provenance records the response shared. A
//! first request that ends with no answer, cancelled, leaves the next to
//! send its own.
//!
//! **Hedges** (`fetch.hedge.enabled`, off by default): a request
//! [`repeatable`] allows that has not answered within its route's
//! threshold ([`Latencies`]) sends a second copy on a new connection
//! through the same transport, and so the Session's own exit, once the Run
//! has room for it: its hedge budget ([`Budget`]), a Permit of its domain
//! key and a share of the rate cap ([`SecondCopies`]), an HTTP admission of
//! the Resource governor, and a request charged in Usage. The first copy
//! that answers wins, the other cancelled; Provenance records the hedge.

#![expect(
    clippy::disallowed_types,
    reason = "each table is read and written in one step, before any `.await` and never under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use goodfetch_engine::admission::{Room, SecondCopies};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::fetch::{
    FETCH_COALESCE, FETCH_HEDGE_ENABLED, FETCH_HEDGE_MAX_PERCENT,
};
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::usage::REQUESTS;
use serde_json::json;
use tokio::sync::watch;

use crate::content_types::Accepts;
use crate::driver::{
    FetchBounds, FetchFailure, Fetched, Hop, OwnDocument, Transport, fetch_accepting, fetch_anew,
};
use crate::hedging::{Budget, Latencies, repeatable};

/// Provenance's action for a response a request shared with an identical
/// one in flight.
const RESPONSE_SHARED: &str = "response_shared";

/// Provenance's action for a hedged request.
const HEDGED: &str = "hedged";

/// What a crawl request is sent for, beside its hop: what decides whether
/// it may be hedged or coalesced, and the room a hedge takes.
pub struct Requester<'a> {
    /// The crawl request's ID.
    pub request_id: &'a str,
    /// The Session it is sent from, whose identity an identical request
    /// shares only within it.
    pub session: &'a SessionId,
    /// Its route: its Label on its domain key (ADR 0093).
    pub label: &'a str,
    /// Its domain key, under which a hedge takes its Permit.
    pub domain_key: &'a str,
    /// The room a hedge takes beside the request.
    pub second_copies: &'a SecondCopies,
    /// Whether its work item declared an Effect (ADR 0106).
    pub declares_effect: bool,
    /// Whether it is a Paid fetch (ADR 0045).
    pub paid: bool,
}

/// A fetch's outcome, which an identical request in flight may share.
type Outcome = Result<Fetched, FetchFailure>;

/// What one Run's HTTP tier keeps of its redundant requests.
pub(crate) struct Redundant {
    /// `None` when `fetch.hedge.enabled` is off.
    hedges: Option<Hedges>,
    coalesces: bool,
    in_flight: Mutex<BTreeMap<Identity, InFlight>>,
}

struct Hedges {
    latencies: Mutex<Latencies>,
    budget: Mutex<Budget>,
}

/// A request in flight that identical ones may wait for.
struct InFlight {
    request_id: String,
    answered: watch::Receiver<Option<Outcome>>,
}

/// What makes two requests identical: their Session, method, URL, headers
/// in order and body.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    session: SessionId,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Bytes>,
    /// The types its handler takes: requests that take others never share
    /// an answer.
    accepts: Option<Accepts>,
}

impl Identity {
    fn of(session: &SessionId, hop: &Hop, accepts: Option<&Accepts>) -> Self {
        Self {
            session: session.clone(),
            method: hop.method.as_str().to_owned(),
            url: hop.url.as_str().to_owned(),
            headers: hop
                .headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .collect(),
            body: hop.body.clone(),
            accepts: accepts.cloned(),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Redundant {
    /// A Run's, under its settings.
    pub(crate) fn of(settings: &Resolved) -> Self {
        let hedges = settings.get::<bool>(&FETCH_HEDGE_ENABLED).then(|| Hedges {
            latencies: Mutex::new(Latencies::default()),
            budget: Mutex::new(Budget::new(settings.get(&FETCH_HEDGE_MAX_PERCENT))),
        });
        Self {
            hedges,
            coalesces: settings.get(&FETCH_COALESCE),
            in_flight: Mutex::new(BTreeMap::new()),
        }
    }

    /// Fetches `hop` through `transport` for `requester` of `run`: the
    /// answer of an identical request in flight from its Session when one
    /// is, else its own, hedged when its route is slow, of its `own`
    /// document as [`fetch_accepting`] says. Requests whose
    /// `fetch.encoding` differs share an answer: each reads its text under
    /// its own.
    ///
    /// # Errors
    ///
    /// The fetch driver's.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes its connections and gives a hedge's room
    /// back; identical requests waiting for it send their own.
    pub(crate) async fn fetch(
        &self,
        transport: &dyn Transport,
        bounds: &FetchBounds,
        run: &Run,
        (hop, own): (&Hop, Option<OwnDocument<'_>>),
        requester: &Requester<'_>,
    ) -> Outcome {
        let repeatable = repeatable(&hop.method, requester.declares_effect, requester.paid);
        if !(self.coalesces && repeatable) {
            return self
                .sent(transport, bounds, run, (hop, own), requester, repeatable)
                .await;
        }
        let identity = Identity::of(requester.session, hop, own.map(|own| own.accepts));
        loop {
            match self.join_or_lead(&identity, requester.request_id) {
                Turn::Join(request_id, mut answered) => {
                    let outcome = answered
                        .wait_for(Option::is_some)
                        .await
                        .ok()
                        .and_then(|outcome| outcome.clone());
                    if let Some(outcome) = outcome {
                        record_shared(run, hop, requester, &request_id).await;
                        return outcome;
                    }
                }
                Turn::Lead(lead) => {
                    let outcome = self
                        .sent(transport, bounds, run, (hop, own), requester, repeatable)
                        .await;
                    lead.answer(outcome.clone());
                    return outcome;
                }
            }
        }
    }

    /// The request in flight identical to `identity` to wait for, or the
    /// lead for `request_id` when none is.
    fn join_or_lead(&self, identity: &Identity, request_id: &str) -> Turn<'_> {
        let mut in_flight = lock(&self.in_flight);
        if let Some(first) = in_flight.get(identity) {
            return Turn::Join(first.request_id.clone(), first.answered.clone());
        }
        let (answer, answered) = watch::channel(None);
        in_flight.insert(
            identity.clone(),
            InFlight {
                request_id: request_id.to_owned(),
                answered,
            },
        );
        Turn::Lead(Lead {
            redundant: self,
            identity: identity.clone(),
            answer,
        })
    }

    /// Sends `hop` itself, hedged when hedges are on, it is `repeatable`
    /// and its route has a threshold.
    async fn sent(
        &self,
        transport: &dyn Transport,
        bounds: &FetchBounds,
        run: &Run,
        (hop, own): (&Hop, Option<OwnDocument<'_>>),
        requester: &Requester<'_>,
        repeatable: bool,
    ) -> Outcome {
        let clock = run.clock();
        if let Some(hedges) = &self.hedges {
            lock(&hedges.budget).sent();
        }
        let Some(hedges) = self.hedges.as_ref().filter(|_| repeatable) else {
            return fetch_accepting(transport, hop, bounds, &clock, own).await;
        };
        let route = format!("{}@{}", requester.label, requester.domain_key);
        let threshold = lock(&hedges.latencies).threshold(&route);
        let started = clock.now();
        let first = fetch_accepting(transport, hop, bounds, &clock, own);
        tokio::pin!(first);
        let outcome = match threshold {
            None => first.await,
            Some(threshold) => {
                let early = tokio::select! {
                    biased;
                    outcome = &mut first => Some(outcome),
                    () = clock.sleep_until(started + threshold) => None,
                };
                match early {
                    Some(outcome) => outcome,
                    None => {
                        hedged(
                            hedges,
                            transport,
                            bounds,
                            run,
                            (hop, own),
                            requester,
                            threshold,
                            first,
                        )
                        .await
                    }
                }
            }
        };
        if outcome.is_ok() {
            lock(&hedges.latencies).record(&route, clock.now().duration_since(started));
        }
        outcome
    }
}

/// A request's turn among identical ones in flight.
enum Turn<'a> {
    /// Wait for the answer of the request in flight, by its ID.
    Join(String, watch::Receiver<Option<Outcome>>),
    /// Send it, and answer those that wait.
    Lead(Lead<'a>),
}

/// The request in flight identical ones wait for: once it is answered or
/// dropped, it is no longer in flight.
struct Lead<'a> {
    redundant: &'a Redundant,
    identity: Identity,
    answer: watch::Sender<Option<Outcome>>,
}

impl Lead<'_> {
    fn answer(self, outcome: Outcome) {
        self.answer.send_replace(Some(outcome));
    }
}

impl Drop for Lead<'_> {
    fn drop(&mut self) {
        lock(&self.redundant.in_flight).remove(&self.identity);
    }
}

/// Which copy of a hedged request answered.
#[derive(Clone, Copy)]
enum AnsweredBy {
    First,
    Hedge,
}

impl AnsweredBy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Hedge => "hedge",
        }
    }
}

/// `first`, unanswered past `threshold`, hedged once the Run has room for
/// a second copy: the first copy that answers wins, a failed one waiting
/// for the other. With no room, `first` alone.
#[expect(
    clippy::too_many_arguments,
    reason = "one step of `Redundant::sent`, which hands it everything a hedge needs"
)]
async fn hedged(
    hedges: &Hedges,
    transport: &dyn Transport,
    bounds: &FetchBounds,
    run: &Run,
    (hop, own): (&Hop, Option<OwnDocument<'_>>),
    requester: &Requester<'_>,
    threshold: Duration,
    mut first: std::pin::Pin<&mut impl Future<Output = Outcome>>,
) -> Outcome {
    let clock = run.clock();
    let copy = loop {
        match requester
            .second_copies
            .take(requester.domain_key, clock.now())
            .await
        {
            Room::Taken(copy) => break copy,
            Room::Full => return first.await,
            Room::Until(until) => tokio::select! {
                biased;
                outcome = &mut first => return outcome,
                () = clock.sleep_until(until) => {}
            },
        }
    };
    let governed = run.governor().try_admit(WorkKind::Http);
    let spent = governed.is_some() && lock(&hedges.budget).spend();
    if !spent || run.charge(REQUESTS, 1).await.is_err() {
        requester.second_copies.give_back(copy).await;
        return first.await;
    }
    let hedge = fetch_anew(transport, hop, bounds, &clock, own);
    tokio::pin!(hedge);
    let (outcome, answered_by) = tokio::select! {
        biased;
        outcome = &mut first => match outcome {
            Ok(fetched) => (Ok(fetched), AnsweredBy::First),
            Err(error) => (hedge.await.map_err(|_| error), AnsweredBy::Hedge),
        },
        outcome = &mut hedge => match outcome {
            Ok(fetched) => (Ok(fetched), AnsweredBy::Hedge),
            Err(_) => (first.await, AnsweredBy::First),
        },
    };
    drop(governed);
    requester.second_copies.give_back(copy).await;
    goodfetch_page::provenance(
        run,
        json!({
            "action": HEDGED,
            "session_id": requester.session,
            "crawl_request_id": requester.request_id,
            "method": hop.method.as_str(),
            "url": hop.url.as_str(),
            "after_ms": u64::try_from(threshold.as_millis()).unwrap_or(u64::MAX),
            "answered_by": answered_by.as_str(),
        }),
    )
    .await;
    outcome
}

/// Records in `run`'s Provenance that `requester` was answered with the
/// response of `shared_from`, an identical request in flight from its
/// Session.
async fn record_shared(run: &Run, hop: &Hop, requester: &Requester<'_>, shared_from: &str) {
    goodfetch_page::provenance(
        run,
        json!({
            "action": RESPONSE_SHARED,
            "session_id": requester.session,
            "crawl_request_id": requester.request_id,
            "method": hop.method.as_str(),
            "url": hop.url.as_str(),
            "shared_from": shared_from,
        }),
    )
    .await;
}
