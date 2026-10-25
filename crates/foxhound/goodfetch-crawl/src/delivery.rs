//! A crawl's delivery (ADRs 0003, 0004, 0024 and 0093): the Run's driver,
//! which admits pending work items through the Run's admission, the
//! per-domain queues Foxhound's composition passes in as the gate in
//! admission under the Run's rate cap, and leases them to the handler of
//! each Connection serving the crawl (the one that started it, and each `workers=N` handler process
//! that joined it with `crawl.serve`), at most `crawl.handler.concurrency`
//! at once on each, each lease committed in run state
//! before its `crawl_request.handle` request is sent, served by a Session
//! the crawl's Session pool draws, which the attempt's ending then scores
//! (ADRs 0006 and 0021); commits an answered
//! attempt's Records once, through its lease, while the lease holds; and
//! ends an attempt whose handler failed, or that got no answer before its
//! lease ended, with nothing committed, its work item retried under the
//! Run's policy, which each admission point reads again from the Run's
//! settings; a handler's skip ends its work item `skipped`, and a failure
//! with the Error code `blocked` ends it `blocked`. Each ending's outcome
//! is committed with it. When a lease ends first, the attempt's request is cancelled
//! with `$/cancel`, and an answer that still comes is a late commit,
//! refused with `lease_lost`. An attempt whose Connection ended is
//! `client_gone`, never the handler's own `handler_failed`, and is retried
//! on a Connection still serving. Each admission also takes a Permit of its
//! domain key from the Politeness the Run shares with the other runs of
//! its user, given back once its attempt ended, with what the attempt met:
//! a site that did not answer in time backs the domain off, and a handler
//! failing with `blocked` for a `rate_limit` (a 429) halves its concurrency,
//! for every sharing run (ADR 0053), as one failing with
//! `response_unavailable` for a 502, 503 or 504 backs it off; the time a
//! failure's `retry_after` names holds the domain until then (ADR 0118). A
//! domain the Politeness holds for want of room waits for the wake of a
//! Permit given back. When that time is further away than the Run waits
//! (`politeness.retry_after.max`), the request whose response asked ends
//! `failed` with no retry spent, and each request of the domain admitted
//! meanwhile ends `failed` with `response_unavailable`, unsent: no handler
//! request goes for it. Once every work item
//! is done, it finishes the Run; with `crawl.keep_alive` (RT-12) it waits
//! for new work instead until `run.finish` lets the Run finish, so its idle
//! time counts toward its `timeout.run`. A seed added to the running crawl
//! (`run.add_seed`, `run.add_seeds`) wakes it, idle or not, to key the new
//! work items and queue them under their domains. When the Run is asked to stop at its next safe point,
//! such as once its `timeout.run` passed, it admits nothing new, lets each
//! attempt in flight commit or be abandoned under its lease, and then stops
//! the Run. When the Run stops at once, by an abort, the attempts in flight
//! are abandoned and their work items return to pending without counting;
//! when
//! no Connection serves the crawl any more, the driver delivers nothing
//! more and waits for the Run to stop or a Connection to join. Each attempt
//! counts in `goodfetch.work_item.leased` until it ends, and each retry in
//! `goodfetch.work_item.retried`, on the meter the Run hands out (ADR 0050).
//! A pending work item a rule the Frontier learned later merged leaves its
//! domain's queue, and one the gate already took gives its turn back
//! unsent. Each attempt runs on the tier the crawl's tier choice plans for
//! its Label on its domain (`tier_choice`); a sample's outcome is
//! classified, and the choice written in the run file, as its attempt
//! settles, before the next admission.
//!
//! Size: one delivery loop, the admission, leases, attempts and settling it
//! drives, kept whole so every work item's way from pending to its ending
//! passes through one loop.

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_engine::CallContext;
use goodfetch_engine::admission::{Admission, Ended, SecondCopies, Waiting, Wakes};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::ledger::Refusal;
use goodfetch_engine::mask::Scope;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::runs::{ChargeError, Ending, Run};
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_fetch_tier::{Choosing, FetchTiers, HttpTier, LabelTiers, RobotsTxtFetch, SetTier};
use goodfetch_frontier::Bounds;
use goodfetch_protocol::foxhound::crawl::{
    CLIENT_GONE, CRAWL_KEEP_ALIVE, CrawlRequestHandled, HANDLER_FAILED, HANDLER_LOST, LEASE_LOST,
    WORK_ITEM_RETRIED,
};
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::SITE_PREFERENCES_REFUSED;
use goodfetch_protocol::run::{
    EndingError, NothingDone, RUN_STATE_UNAVAILABLE, RunDetails, StopReason,
};
use goodfetch_protocol::usage::REQUESTS;
use goodfetch_session_pool::{PoolSettings, SessionPool};
use serde_json::{Map, Value};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::completeness;
use crate::labels;
use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::{CrawlPolicy, resumed_domain_policy};
use crate::progress::{self, InFlight, Room, SharedInFlight};
use crate::serving::{Crawls, Opened, Server};
use crate::tier_choice::{self, AttemptPlan, TierChoices};
use crate::work_items::{self, Failure, Leased, Unsuccessful};
use goodfetch_protocol::foxhound::progress::{InFlightPhase, InFlightRequest};

mod attempt;
mod politeness;

use attempt::{Answer, Attempt, Gating, Tiering, attempt, block_cause, effect, leased_count};
use politeness::{RETRY_AFTER_FIX, domain_ending, end_unsent, held_past_bound};

/// A Connection serving the crawl's handler requests, as the delivery
/// keeps it.
struct Serving {
    /// The context of the call that joined it, whose Connection's Client
    /// its handler requests go to.
    context: CallContext,
    /// Closed once it takes no new work item; `None` for the Connection
    /// that started the crawl, which serves until it ends.
    left: Option<oneshot::Receiver<()>>,
    /// Its attempts in flight.
    in_flight: usize,
    /// Whether its Connection ended.
    gone: bool,
}

impl Serving {
    /// A handler process that joined the crawl with `crawl.serve`.
    fn joined(server: Server) -> Self {
        Self {
            context: server.context,
            left: Some(server.left),
            in_flight: 0,
            gone: false,
        }
    }

    /// Whether it takes new work items.
    fn serves(&mut self) -> bool {
        !self.gone
            && self.left.as_mut().is_none_or(|left| {
                matches!(left.try_recv(), Err(oneshot::error::TryRecvError::Empty))
            })
    }
}

/// Drives `run`'s work items to the handler of `client`, and of each
/// Connection that `arrivals` brings, keying the seeds added since whenever
/// it says so, until each is done or the Run stops; then closes the crawl
/// in `crawls`.
///
/// # Cancel safety
///
/// Run it as one of the Run's tasks to its end: dropped, it leaves its
/// leases to the next writer, which counts them as lost.
pub(crate) async fn drive(
    run: Run,
    context: CallContext,
    mut arrivals: Opened,
    crawls: Crawls,
    settings: RunSettings,
    (tiers, labels, choices): (FetchTiers, LabelTiers, TierChoices),
) {
    let mut servers = vec![Serving {
        context,
        left: None,
        in_flight: 0,
        gone: false,
    }];
    let run_tier = FetchTiers::run_tier(settings.resolved());
    // One HTTP tier for the Run, so its hops share connections (#763); the
    // navigation gate fetches robots.txt on it too.
    let http = match FetchTiers::built(&run, settings.resolved()) {
        Ok(http) => Some(http),
        // A request may take the `http` tier: the Run cannot go on.
        Err(error) if labels.any() || run_tier != SetTier::Unset => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's HTTP tier could not be built");
            run.end(failed(&run, &error)).await;
            crawls.close(run.id()).await;
            return;
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "the navigation gate has no transport for robots.txt; every robots.txt it reads is unreachable");
            None
        }
    };
    let fetching = Fetching {
        tiers: &tiers,
        http: http.as_ref(),
        robots: http.as_ref().map(HttpTier::robots_txt_fetch),
        egress: std::sync::Arc::clone(&arrivals.egress),
        labels: &labels,
        run_tier,
        choices: tokio::sync::Mutex::new(choices),
        in_flight: std::sync::Arc::new(tokio::sync::Mutex::new(InFlight::default())),
    };
    deliver(&run, &mut servers, &mut arrivals, &settings, &fetching).await;
    crawls.close(run.id()).await;
}

/// The delivery's loop, which returns once the Run ended or stopped.
async fn deliver(
    run: &Run,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    settings: &RunSettings,
    fetching: &Fetching<'_>,
) {
    let run_leases = Leases::new(run.clock(), run.id().as_str(), run.state().epoch());
    let Some((mut admission, mut pool)) = opened(run, settings).await else {
        return;
    };
    let mut wakes = admission.wakes();
    let mut attempts: JoinSet<Attempt> = JoinSet::new();
    let mut stop: Option<StopReason> = None;
    loop {
        let resolved = settings.at_admission().await;
        let policy = CrawlPolicy::of(&resolved);
        admission.update(&resolved);
        if stop.is_none()
            && let Err(error) = lease_to_servers(
                run,
                &run_leases,
                policy,
                Pooled::of(&mut pool, &resolved, fetching),
                &mut admission,
                servers,
                &mut attempts,
            )
            .await
        {
            tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be leased");
            end(run, &mut admission, failed(run, &error)).await;
            return;
        }
        let starved = run.governor().waits(WorkKind::Http);
        if attempts.is_empty() {
            if let Some(reason) = stop {
                end(run, &mut admission, Ending::Stopped(reason)).await;
                return;
            }
            if !servers.iter_mut().any(Serving::serves) {
                match waited(run, &mut admission, servers, arrivals, false).await {
                    Waited::Ended => return,
                    Waited::Stop(reason) => stop = Some(reason),
                    Waited::Again => {}
                }
                continue;
            }
            match when_idle(run, &mut admission).await {
                Ok(Idle::Admit) if !starved => continue,
                Ok(Idle::Admit | Idle::Wait) => {}
                Ok(Idle::Done) => {
                    match drained(run, &resolved, &mut admission, servers, arrivals).await {
                        Waited::Ended => return,
                        Waited::Stop(reason) => stop = Some(reason),
                        Waited::Again => {}
                    }
                    continue;
                }
                Err(error) => {
                    tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                    end(run, &mut admission, failed(run, &error)).await;
                    return;
                }
            }
        }
        let free = stop.is_none()
            && servers
                .iter_mut()
                .any(|server| server.serves() && server.in_flight < policy.handler_concurrency);
        let wake = (free && !starved)
            .then(|| admission.ready_at(run.clock().now()))
            .flatten();
        let waits = [stop.is_some(), free, starved];
        show(run, &admission, &resolved, servers, waits);
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => {
                attempts.shutdown().await;
                admission.close().await;
                release(run).await;
                return;
            }
            reason = run.stopping(), if stop.is_none() => {
                stop = Some(reason);
            }
            Some(server) = arrivals.joined.recv() => {
                servers.push(Serving::joined(server));
            }
            () = arrivals.seeded.notified() => {
                if let Err(error) = key_new(run, &mut admission).await {
                    tracing::warn!(%error, run_id = %run.id(), "the seeds added were not keyed yet");
                }
            }
            Some(ended) = attempts.join_next() => {
                if let Ok(attempt) = ended {
                    let pool = (&mut pool, fetching);
                    settled(run, settings, &mut admission, pool, servers, attempt).await;
                }
            }
            () = sleep_until(run.clock(), wake), if wake.is_some() => {}
            () = run.governor().turn(WorkKind::Http), if free && starved => {}
            key = next_wake(wakes.as_mut()) => admission.wake(key.as_deref()),
        }
    }
}

/// Sets the crawl's live parts of its Run's progress (ADR 0102) as its
/// `servers` stand, given whether it stops, whether a serving Connection
/// has room and whether the governor starves it.
fn show(
    run: &Run,
    admission: &Admission,
    resolved: &Resolved,
    servers: &mut [Serving],
    [stopping, free, starved]: [bool; 3],
) {
    let serving = servers.iter_mut().any(Serving::serves);
    let handlers = servers.iter().map(|server| server.in_flight).sum();
    let room = progress::room(stopping || !serving, free, starved);
    let concurrency = CrawlPolicy::of(resolved).handler_concurrency;
    progress::show(run, admission, resolved, room, (handlers, concurrency));
}

/// The ending of a crawl whose state or admission failed with `error`:
/// `failed`, its ending calls raising `run_state_unavailable` (ADR 0102).
fn failed(run: &Run, error: &dyn std::fmt::Display) -> Ending {
    let details = RunDetails {
        run_id: run.id().clone(),
        path: Some(run.folder().display().to_string()),
    };
    Ending::Failed(EndingError {
        code: RUN_STATE_UNAVAILABLE.name.to_owned(),
        message: format!("The crawl `{}` failed: {error}", run.id()),
        details: match serde_json::to_value(details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        },
    })
}

/// Ends the Run as `ending` says, once its admission gave back what it held.
async fn end(run: &Run, admission: &mut Admission, ending: Ending) {
    admission.close().await;
    run.end(ending).await;
}

/// The next key of the Run that a Permit given back woke, `None` for every
/// held key; never, when the Run shares no Politeness.
async fn next_wake(wakes: Option<&mut Wakes>) -> Option<String> {
    match wakes {
        Some(wakes) => wakes.next().await,
        None => std::future::pending().await,
    }
}

/// Drives a resumed crawl (ADRs 0002 and 0102), its bounds recorded again
/// from its settings (an unreadable path budget fails it) and its Labels'
/// tier choices read again from its run file: each attempt its
/// committed state shows in flight, whose engine stopped without settling
/// it, counts as `handler_lost` against `crawl.retry.max` and is retried or
/// failed, then delivery goes on from the committed state, so the work
/// since that state is redone. Its domain policy is read again from its
/// settings; one that cannot be read admits no seed or enqueue.
///
/// # Cancel safety
///
/// As [`drive`]'s.
pub(crate) async fn resume(
    run: Run,
    context: CallContext,
    settings: RunSettings,
    crawls: Crawls,
    tiers: FetchTiers,
) {
    let policy = CrawlPolicy::of(&settings.at_admission().await);
    let retry_max = policy.retry_max;
    let bounds = Bounds::of_settings(settings.resolved());
    let resolved = settings.resolved().clone();
    let left = run
        .state()
        .write(move |connection| {
            bounds.record(connection)?;
            goodfetch_frontier::payload::record_max_size(connection, &resolved)?;
            work_items::recover_leases(connection, HANDLER_LOST.name, retry_max)?;
            Ok((
                work_items::left(connection)?,
                labels::read(connection)?,
                tier_choice::read(connection)?,
            ))
        })
        .await;
    let left = left.map(|(left, declared, choices)| {
        let labels = LabelTiers::of(&declared).unwrap_or_else(|error| {
            tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's Labels no longer read; its requests take the Run's tier");
            LabelTiers::default()
        });
        (left, (labels, choices))
    });
    let (labels, choices) = match left {
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the resumed crawl's work items could not be recovered");
            run.end(failed(&run, &error)).await;
            return;
        }
        Ok((0, labels)) => {
            if let Err(error) = run.nothing_done(NothingDone::NothingPending).await {
                tracing::warn!(%error, run_id = %run.id(), "the resumed crawl's `nothingDone` was not kept");
            }
            labels
        }
        Ok((_, labels)) => labels,
    };
    let egress = resumed_domain_policy(settings.resolved(), run.id());
    let opened = crawls.open(&run, egress).await;
    drive(
        run,
        context,
        opened,
        crawls,
        settings,
        (tiers, labels, choices),
    )
    .await;
}

/// The tiers a Run's crawl requests are fetched on: its HTTP tier, and the
/// browser's pages through `tiers`, each request on the tier its Label and
/// the Run's `fetch.tier` choose.
struct Fetching<'a> {
    tiers: &'a FetchTiers,
    http: Option<&'a HttpTier>,
    /// The Labels the crawl declares.
    labels: &'a LabelTiers,
    /// The tier the Run's own `fetch.tier` forces.
    run_tier: SetTier,
    /// What each Label on each domain learned under `auto`.
    choices: tokio::sync::Mutex<TierChoices>,
    /// How the navigation gate fetches robots.txt.
    robots: Option<RobotsTxtFetch>,
    /// The Run's domain policy, which judges each redirect hop.
    egress: std::sync::Arc<goodfetch_engine::egress::Policy>,
    /// The requests in flight, as the Run's progress shows them.
    in_flight: SharedInFlight,
}

impl Fetching<'_> {
    /// What one attempt's handler pages are gated with.
    fn gating(&self, second_copies: &SecondCopies) -> Gating {
        Gating {
            tiers: self.tiers.clone(),
            second_copies: second_copies.clone(),
            robots: self.robots.clone(),
            egress: std::sync::Arc::clone(&self.egress),
        }
    }

    /// How `leased`'s attempt runs, as its Label and the Run's
    /// `fetch.tier` choose and, under `auto`, as its Label on its domain
    /// learned, and what it fetches its document with.
    async fn served(&self, leased: &Leased, second_copies: &SecondCopies) -> Tiering {
        let request = &leased.crawl_request;
        let plan = match self.labels.choose(&request.label, self.run_tier) {
            Choosing::Alone => AttemptPlan::Alone,
            Choosing::Forced(choice) => AttemptPlan::On(choice),
            Choosing::Engine => self.choices.lock().await.serve_attempt(
                &request.label,
                &leased.waiting.key,
                &request.url,
            ),
        };
        Tiering::of(self.tiers, self.http, plan, second_copies)
    }

    /// Classifies the sample `attempt` was, if it was one, and writes what
    /// its Label on its domain now learned into the run file.
    async fn classify(&self, run: &Run, attempt: &mut Attempt) {
        let Some((sampled, matched)) = attempt.sampled.take() else {
            return;
        };
        let unit = self
            .choices
            .lock()
            .await
            .classify(&sampled, matched)
            .clone();
        if let Err(error) = run
            .state()
            .write(move |connection| {
                tier_choice::record(connection, &sampled.label, &sampled.domain_key, &unit)
            })
            .await
        {
            tracing::warn!(%error, run_id = %run.id(), "a Label's tier choice was not written; a resumed crawl samples it again");
        }
    }
}

/// What each attempt is given: a Session drawn from the crawl's Session
/// pool under the Run's settings now, and the tier that fetches its
/// request's document.
struct Pooled<'a> {
    pool: &'a mut SessionPool,
    settings: PoolSettings,
    fetching: &'a Fetching<'a>,
}

impl<'a> Pooled<'a> {
    /// Draws from `pool` under `resolved`, fetching on `fetching`.
    fn of(pool: &'a mut SessionPool, resolved: &Resolved, fetching: &'a Fetching<'a>) -> Self {
        Self {
            pool,
            settings: PoolSettings::of(resolved),
            fetching,
        }
    }
}

/// Admits work items up to each serving Connection's room and the HTTP
/// admissions the engine's Resource governor gives the Run now, draws the
/// Session of the pool that serves each, written in run state before any
/// handler request is sent, and starts an attempt for each on that
/// Connection, which holds its admission until it ends (ADR 0040). A Run
/// the governor gave none waits for its turn.
async fn lease_to_servers(
    run: &Run,
    run_leases: &Leases,
    policy: CrawlPolicy,
    pooled: Pooled<'_>,
    admission: &mut Admission,
    servers: &mut [Serving],
    attempts: &mut JoinSet<Attempt>,
) -> Result<(), ChargeError> {
    for (place, server) in servers.iter_mut().enumerate() {
        if !server.serves() || server.in_flight >= policy.handler_concurrency {
            continue;
        }
        let mut permits: Vec<Permit> =
            std::iter::from_fn(|| run.governor().try_admit(WorkKind::Http))
                .take(policy.handler_concurrency - server.in_flight)
                .collect();
        if permits.is_empty() {
            return Ok(());
        }
        let served: Vec<(Leased, ServingSession)> = admit(run, admission, permits.len())
            .await?
            .into_iter()
            .map(|leased| {
                let session =
                    pooled
                        .pool
                        .draw(&leased.waiting.key, &pooled.settings, run.clock().wall());
                (leased, session)
            })
            .collect();
        let changes = pooled.pool.changes();
        if !changes.is_empty() {
            run.state()
                .write(move |connection| changes.write(connection))
                .await?;
        }
        for (leased, session) in served {
            let lease = run_leases.grant(
                &leased.crawl_request.id,
                leased.lease_epoch,
                policy.lease_length(),
            );
            server.in_flight += 1;
            let tiering = pooled
                .fetching
                .served(&leased, &admission.second_copies())
                .await;
            let fetches = tiering.tier();
            pooled.fetching.in_flight.lock().await.began(
                run,
                InFlightRequest {
                    request_id: leased.crawl_request.id.clone(),
                    url: leased.crawl_request.url.clone(),
                    phase: if fetches.is_some() {
                        InFlightPhase::Fetching
                    } else {
                        InFlightPhase::Handling
                    },
                    tier: fetches.map(|tier| tier.as_str().to_owned()),
                    attempt: leased.attempt,
                    started_at: run.point_in_time(),
                },
            );
            let permit = permits
                .pop()
                .expect("invariant: one admission per work item admitted");
            attempts.spawn(holding(
                permit,
                attempt(
                    run.clone(),
                    place,
                    server.context.clone(),
                    (leased, session, leased_count(run)),
                    lease,
                    (tiering, pooled.fetching.gating(&admission.second_copies())),
                    std::sync::Arc::clone(&pooled.fetching.in_flight),
                ),
            ));
        }
    }
    Ok(())
}

/// Runs `attempt`, holding `permit` until it ends.
async fn holding(permit: Permit, attempt: impl Future<Output = Attempt>) -> Attempt {
    let ended = attempt.await;
    drop(permit);
    ended
}

/// What a crawl with nothing to deliver now does next.
enum Waited {
    /// It ended the Run, or the Run stopped at once.
    Ended,
    /// The Run is asked to stop at its next safe point.
    Stop(StopReason),
    /// `run.finish` let it finish, a Connection joined to serve it, or a
    /// seed was added: look for work once more.
    Again,
}

/// Finishes a crawl with no work item left, with what its bounds and caps
/// cut as its Completeness reasons, unless `crawl.keep_alive`,
/// read at each admission point as its policy is, holds it until
/// `run.finish` lets it finish: until then it waits for new work, as the
/// Run's own timeout and a stop still end it (RT-12).
async fn drained(
    run: &Run,
    resolved: &Resolved,
    admission: &mut Admission,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
) -> Waited {
    let keeps_alive = resolved
        .boolean(CRAWL_KEEP_ALIVE.name)
        .expect("invariant: `crawl.keep_alive` resolves to a boolean");
    if !keeps_alive || run.is_let_finish() {
        let ending = match run.state().read(completeness::short_of_scope).await {
            Ok(reasons) => Ending::Finished(reasons),
            Err(error) => {
                tracing::error!(%error, run_id = %run.id(), "what cut the crawl could not be read");
                failed(run, &error)
            }
        };
        end(run, admission, ending).await;
        return Waited::Ended;
    }
    progress::show_waits(run, &[], resolved, run.clock().now(), Room::None);
    progress::show_idle(run, true);
    waited(run, admission, servers, arrivals, true).await
}

/// Waits, with nothing to deliver now, until the Run stops at once, is
/// asked to stop at its next safe point, a Connection joins to serve it, a
/// seed is added or, `until_finish`, `run.finish` lets it finish.
async fn waited(
    run: &Run,
    admission: &mut Admission,
    servers: &mut Vec<Serving>,
    arrivals: &mut Opened,
    until_finish: bool,
) -> Waited {
    tokio::select! {
        biased;
        () = run.cancellation().cancelled() => {
            admission.close().await;
            release(run).await;
            Waited::Ended
        }
        reason = run.stopping() => Waited::Stop(reason),
        () = run.let_finish(), if until_finish => Waited::Again,
        Some(server) = arrivals.joined.recv() => {
            servers.push(Serving::joined(server));
            Waited::Again
        }
        () = arrivals.seeded.notified() => Waited::Again,
    }
}

/// What a crawl with no attempt in flight does next.
enum Idle {
    /// Admit the work items its queues now hold.
    Admit,
    /// Wait until its admission is ready or the Run stops.
    Wait,
    /// Finish: no work item is pending.
    Done,
}

/// What a crawl with no attempt in flight does next: its queues filled
/// again from the pending work items when they hold none ready.
async fn when_idle(run: &Run, admission: &mut Admission) -> Result<Idle, RunStateError> {
    if admission.ready_at(run.clock().now()).is_some() {
        return Ok(Idle::Wait);
    }
    key_new(run, admission).await?;
    fill(run, admission, None).await?;
    if admission.ready_at(run.clock().now()).is_some() {
        return Ok(Idle::Admit);
    }
    Ok(if run.state().read(work_items::any_pending).await? {
        Idle::Wait
    } else {
        Idle::Done
    })
}

/// Settles an attempt that ended under the Run's policy now, takes it out
/// of the requests in flight its progress shows, frees its place on its
/// Connection, returns its domain's turn, puts a retried work item back in
/// its queue and journals what the queues changed.
async fn settled(
    run: &Run,
    settings: &RunSettings,
    admission: &mut Admission,
    (pool, fetching): (&mut SessionPool, &Fetching<'_>),
    servers: &mut [Serving],
    mut attempt: Attempt,
) {
    fetching.classify(run, &mut attempt).await;
    fetching
        .in_flight
        .lock()
        .await
        .ended(run, &attempt.leased.crawl_request.id);
    let server = &mut servers[attempt.server];
    server.in_flight -= 1;
    if let Answer::ClientGone = attempt.answer {
        server.gone = true;
    }
    let waiting = attempt.leased.waiting.clone();
    let merged = std::mem::take(&mut attempt.merged);
    let domain = domain_ending(&attempt.answer, run.clock().now(), run.clock().wall());
    let resolved = settings.at_admission().await;
    pool.settle(
        &attempt.session,
        &waiting.key,
        effect(&attempt.answer),
        &PoolSettings::of(&resolved),
    );
    let changes = pool.changes();
    if !changes.is_empty()
        && let Err(error) = run
            .state()
            .write(move |connection| changes.write(connection))
            .await
    {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's Session pool was not written");
    }
    let retried = settle(
        run,
        CrawlPolicy::of(&resolved),
        goodfetch_frontier::longest_wait(&resolved),
        attempt,
    )
    .await;
    admission
        .ended(&waiting.id, &waiting.key, run.clock().now(), domain)
        .await;
    if retried {
        count_retry(run);
        admission.enqueue(waiting);
    }
    let mut keys = BTreeSet::new();
    for (id, key) in merged {
        admission.withdraw(&key, &id);
        keys.insert(key);
    }
    for key in keys {
        if let Err(error) = fill(run, admission, Some(key)).await {
            tracing::warn!(%error, run_id = %run.id(), "a queue a learned rule thinned was not filled again");
        }
    }
    if let Err(error) = key_new(run, admission).await {
        tracing::warn!(%error, run_id = %run.id(), "the requests a handler enqueued were not keyed yet");
    }
    journal(run, admission).await;
}

/// Opens the Run's admission and Session pool, or ends the Run as failed
/// when they could not open.
async fn opened(run: &Run, settings: &RunSettings) -> Option<(Admission, SessionPool)> {
    match open(run, settings).await {
        Ok(opened) => Some(opened),
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's admission could not open");
            run.end(failed(run, &error)).await;
            None
        }
    }
}

/// Opens the Run's admission, the gate Foxhound's composition passes in,
/// keys the work items not keyed yet and fills each domain's queue; and
/// opens its Session pool from its run state, on an `Entropy` of its own.
async fn open(run: &Run, settings: &RunSettings) -> Result<(Admission, SessionPool), String> {
    let resolved = settings.at_admission().await;
    let mut admission = run
        .admission(&resolved)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("the composition passes in no gate in admission")?;
    key_new(run, &mut admission)
        .await
        .map_err(|error| error.to_string())?;
    fill(run, &mut admission, None)
        .await
        .map_err(|error| error.to_string())?;
    let entropy = run.fork_entropy().await;
    let pool = run
        .state()
        .read(move |connection| SessionPool::open(connection, entropy))
        .await
        .map_err(|error| error.to_string())?;
    Ok((admission, pool))
}

/// How many work items not keyed yet [`key_new`] reads at a time, so a
/// stream of added seeds never sits in memory whole.
const KEYED_AT_ONCE: usize = 1024;

/// Keys each work item not keyed yet, the seeds at the start, those a
/// handler enqueued since and those added to the running crawl, and offers
/// it to its domain's queue, which takes it unless full; one it refuses
/// waits in run state until its queue fills again.
async fn key_new(run: &Run, admission: &mut Admission) -> Result<(), RunStateError> {
    loop {
        let unkeyed = run
            .state()
            .read(|connection| work_items::unkeyed(connection, KEYED_AT_ONCE))
            .await?;
        let read = unkeyed.len();
        key(run, admission, unkeyed).await?;
        if read < KEYED_AT_ONCE {
            return Ok(());
        }
    }
}

/// Keys each of `unkeyed`, as [`key_new`] does.
async fn key(
    run: &Run,
    admission: &mut Admission,
    unkeyed: Vec<(String, Waiting)>,
) -> Result<(), RunStateError> {
    if unkeyed.is_empty() {
        return Ok(());
    }
    let keyed: Vec<Waiting> = unkeyed
        .into_iter()
        .map(|(url, waiting)| Waiting {
            key: admission.key(&url),
            ..waiting
        })
        .collect();
    let keys: Vec<(String, String)> = keyed
        .iter()
        .map(|waiting| (waiting.id.clone(), waiting.key.clone()))
        .collect();
    run.state()
        .write(move |connection| work_items::set_keys(connection, &keys))
        .await?;
    for waiting in keyed {
        admission.enqueue(waiting);
    }
    Ok(())
}

/// Fills the queue of `key`, or of every key when `None`, from the pending
/// work items, at most its bound.
async fn fill(
    run: &Run,
    admission: &mut Admission,
    key: Option<String>,
) -> Result<(), RunStateError> {
    let bound = admission.bound();
    let waiting = run
        .state()
        .read(move |connection| work_items::waiting(connection, key.as_deref(), bound))
        .await?;
    for waiting in waiting {
        admission.enqueue(waiting);
    }
    Ok(())
}

/// Admits up to `room` work items ready now and leases them, then fills
/// again each domain's queue that drained. Each one first charges the
/// request it is against the Limits that hold the Run (ADR 0041): once one
/// has nothing left, or one the engine cannot evaluate covers it, the Run
/// is asked to stop at its next safe point and nothing more is admitted.
async fn admit(
    run: &Run,
    admission: &mut Admission,
    room: usize,
) -> Result<Vec<Leased>, ChargeError> {
    let now = run.clock().now();
    let mut ids = Vec::new();
    let mut keys = Vec::new();
    let mut drained = Vec::new();
    let mut refused = Vec::new();
    while ids.len() < room {
        let Some(admitted) = admission.admit(now).await else {
            break;
        };
        if let Some(until) = admitted.refused_until {
            if admitted.drained {
                drained.push(admitted.key.clone());
            }
            refused.push((admitted, until));
            continue;
        }
        match run.charge(REQUESTS, 1).await {
            Ok(()) => {}
            Err(ChargeError::Refused(Refusal::Reached(_) | Refusal::NotUnderstood(_))) => break,
            Err(error) => return Err(error),
        }
        if admitted.drained {
            drained.push(admitted.key.clone());
        }
        ids.push(admitted.id);
        keys.push(admitted.key);
    }
    if !refused.is_empty() {
        end_unsent(run, admission, refused).await?;
    }
    let asked = ids.clone();
    let leased = if ids.is_empty() {
        Vec::new()
    } else {
        run.state()
            .write(move |connection| work_items::lease(connection, &ids))
            .await?
    };
    // A work item a rule learned later merged after the gate took it is
    // no longer pending: it gives its domain's turn back unsent.
    for (id, key) in asked.iter().zip(&keys) {
        if !leased.iter().any(|leased| leased.crawl_request.id == *id) {
            admission.ended(id, key, now, Ended::Done).await;
        }
    }
    for key in drained {
        fill(run, admission, Some(key)).await?;
    }
    Ok(leased)
}

/// Writes what the gate changed into the run file.
async fn journal(run: &Run, admission: &mut Admission) {
    let clock = run.clock();
    if let Some(journal) = admission.journal(clock.wall(), clock.now())
        && let Err(error) = run
            .state()
            .write(move |connection| journal(connection))
            .await
    {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's per-domain queues were not journaled");
    }
}

/// Sleeps until `moment`, if any.
async fn sleep_until(clock: Clock, moment: Option<Instant>) {
    if let Some(moment) = moment {
        clock.sleep_until(moment).await;
    }
}

/// One more retry in `goodfetch.work_item.retried`.
fn count_retry(run: &Run) {
    run.metrics()
        .meter()
        .u64_counter(WORK_ITEM_RETRIED.name)
        .with_description(WORK_ITEM_RETRIED.description)
        .with_unit(WORK_ITEM_RETRIED.unit)
        .build()
        .add(1, &[]);
}

/// Commits an attempt's outcome: its Records once while its lease holds,
/// its handler's skip, its block, or its work item retried or failed; then
/// releases its lease. Answers whether its work item waits again for a
/// retry.
async fn settle(
    run: &Run,
    policy: CrawlPolicy,
    longest_wait: Option<Duration>,
    attempt: Attempt,
) -> bool {
    let Attempt {
        leased,
        lease,
        answer,
        ..
    } = attempt;
    let failure =
        |failure, retryable| retried_or_failed(policy, leased.attempt, failure, retryable);
    let unsuccessful = match answer {
        Answer::Handled(CrawlRequestHandled {
            skipped: Some(skip),
            ..
        }) => Unsuccessful::Skipped {
            reason: SkipReason::Handler,
            message: Some(skip.message),
        },
        Answer::Refused(reason) => refused(reason),
        Answer::RobotsTxtUnreachable(message) if policy.may_retry(leased.attempt) => {
            Unsuccessful::Retried(Failure::new(SITE_PREFERENCES_REFUSED.name, &message))
        }
        Answer::RobotsTxtUnreachable(_) => refused(SkipReason::RobotsTxt),
        Answer::Handled(handled) => {
            match lease
                .commit(run.state(), handled.records, run.point_in_time())
                .await
            {
                Ok(()) => {
                    lease.release();
                    return false;
                }
                Err(CommitError::LeaseLost) => failure(
                    Failure::new(LEASE_LOST.name, LEASE_LOST.description),
                    LEASE_LOST.retryable,
                ),
                Err(CommitError::RunState(error)) => {
                    tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                    return false;
                }
            }
        }
        Answer::ClientGone => failure(
            Failure::new(CLIENT_GONE.name, CLIENT_GONE.description),
            CLIENT_GONE.retryable,
        ),
        Answer::LeaseEnded => failure(
            Failure::new(HANDLER_LOST.name, HANDLER_LOST.description),
            HANDLER_LOST.retryable,
        ),
        Answer::Failed {
            code,
            retryable,
            message,
            details,
        } => {
            let held = held_past_bound(&details, run.clock().wall(), longest_wait).is_some();
            let failed = kept_failure(run, code, &message, details);
            match block_cause(&failed.code, &failed.details) {
                Some(cause) => Unsuccessful::Blocked(cause),
                None if held => Unsuccessful::Failed(failed.with_fix(RETRY_AFTER_FIX)),
                None => failure(failed, retryable),
            }
        }
    };
    end_unsuccessful(run, &lease, &leased, unsuccessful).await
}

/// A handler's failure as the Run's state keeps it: no value the Run
/// resolved stays in its message or details (spec #564).
fn kept_failure(run: &Run, code: String, message: &str, details: Map<String, Value>) -> Failure {
    let masks = run.secrets().mask().masks();
    let mut kept = Value::Object(details);
    masks.apply_value(&mut kept, Scope::Kept);
    let Value::Object(details) = kept else {
        unreachable!("invariant: masking an object leaves an object")
    };
    Failure {
        code,
        message: masks.apply(message, Scope::Kept).into_owned(),
        details,
    }
}

/// Ends `leased`'s work item `unsuccessful` while its lease holds,
/// counting its failure's code in the Run's outcomes, then releases
/// `lease`. Answers whether the work item waits again for a retry.
async fn end_unsuccessful(
    run: &Run,
    lease: &WorkItemLease,
    leased: &Leased,
    unsuccessful: Unsuccessful,
) -> bool {
    let id = leased.crawl_request.id.clone();
    let epoch = leased.lease_epoch;
    let retried = matches!(unsuccessful, Unsuccessful::Retried(_));
    let recorded = match &unsuccessful {
        Unsuccessful::Retried(failure) | Unsuccessful::Failed(failure) => {
            Some(failure.code.clone())
        }
        Unsuccessful::Skipped { .. } | Unsuccessful::Blocked(_) => None,
    };
    let ended_at = run.point_in_time();
    let written = run
        .state()
        .write(move |connection| work_items::end(connection, &id, epoch, unsuccessful, &ended_at))
        .await;
    lease.release();
    match written {
        Ok(true) => {
            if let Some(code) = recorded {
                run.record_outcome(&code);
            }
            retried
        }
        Ok(false) => {
            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "an attempt past its lease commits nothing");
            false
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
            false
        }
    }
}

/// A request the site's preferences refused before its handler ran,
/// skipped with `reason` (ADR 0086).
fn refused(reason: SkipReason) -> Unsuccessful {
    Unsuccessful::Skipped {
        reason,
        message: None,
    }
}

/// How `failure` on a work item's `attempt`th attempt ends it under
/// `policy`: retried while it is `retryable` and retries are left, failed
/// otherwise; a handler's own failure is retryable only as
/// `crawl.retry.handler_failed` says.
fn retried_or_failed(
    policy: CrawlPolicy,
    attempt: u32,
    failure: Failure,
    retryable: bool,
) -> Unsuccessful {
    let retryable = if failure.code == HANDLER_FAILED.name {
        policy.retry_handler_failed
    } else {
        retryable
    };
    if retryable && policy.may_retry(attempt) {
        Unsuccessful::Retried(failure)
    } else {
        Unsuccessful::Failed(failure)
    }
}

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run) {
    if let Err(error) = run.state().write(work_items::release_leases).await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
