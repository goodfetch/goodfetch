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
//! for every sharing run (ADR 0053). A domain the Politeness holds for want
//! of room waits for the wake of a Permit given back. Once every work item
//! is done, it finishes the Run. When the Run is asked to stop at its next safe point,
//! such as once its `timeout.run` passed, it admits nothing new, lets each
//! attempt in flight commit or be abandoned under its lease, and then stops
//! the Run. When the Run stops at once, by an abort, the attempts in flight
//! are abandoned and their work items return to pending without counting;
//! when
//! no Connection serves the crawl any more, the driver delivers nothing
//! more and waits for the Run to stop or a Connection to join.

use goodfetch_engine::admission::{Admission, Ended, Waiting, Wakes};
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::ledger::Refusal;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::runs::{ChargeError, Ending, Run};
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::connection::DEADLINE_EXCEEDED;
use goodfetch_protocol::foxhound::crawl::{
    CLIENT_GONE, CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled,
    HANDLER_FAILED, HANDLER_LOST, LEASE_LOST,
};
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::run::StopReason;
use goodfetch_protocol::settings::{TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_protocol::usage::REQUESTS;
use goodfetch_session_pool::{Effect, PoolSettings, SessionPool};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::CrawlPolicy;
use crate::serving::{Crawls, Server};
use crate::work_items::{self, Failure, Leased, Unsuccessful};

/// The Error code of a page the site refused, which Collie's page raises
/// with the Block cause in its details (ADRs 0035 and 0079); declared with
/// the page's errors (#711).
const BLOCKED: &str = "blocked";

/// The Error code of a page that failed its Page check (ADR 0006), which
/// scores its Session down; declared with the Page check (area 5).
const PAGE_CHECK_FAILED: &str = "page_check_failed";

/// How an attempt ended, before its outcome is committed.
enum Answer {
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

struct Attempt {
    /// The server, by its place in the delivery's list, its request went to.
    server: usize,
    leased: Leased,
    /// The Session of the crawl's Session pool that served it.
    session: ServingSession,
    lease: WorkItemLease,
    answer: Answer,
}

/// A Connection serving the crawl's handler requests, as the delivery
/// keeps it.
struct Serving {
    client: Client,
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
            client: server.client,
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
/// Connection that `joined` brings, until each is done or the Run stops;
/// then closes the crawl in `crawls`.
///
/// # Cancel safety
///
/// Run it as one of the Run's tasks to its end: dropped, it leaves its
/// leases to the next writer, which counts them as lost.
pub(crate) async fn drive(
    run: Run,
    client: Client,
    mut joined: mpsc::UnboundedReceiver<Server>,
    crawls: Crawls,
    settings: RunSettings,
) {
    let mut servers = vec![Serving {
        client,
        left: None,
        in_flight: 0,
        gone: false,
    }];
    deliver(&run, &mut servers, &mut joined, &settings).await;
    crawls.close(run.id()).await;
}

/// The delivery's loop, which returns once the Run ended or stopped.
async fn deliver(
    run: &Run,
    servers: &mut Vec<Serving>,
    joined: &mut mpsc::UnboundedReceiver<Server>,
    settings: &RunSettings,
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
                Pooled {
                    pool: &mut pool,
                    settings: PoolSettings::of(&resolved),
                },
                &mut admission,
                servers,
                &mut attempts,
            )
            .await
        {
            tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be leased");
            end(run, &mut admission, Ending::Failed).await;
            return;
        }
        if attempts.is_empty() {
            if let Some(reason) = stop {
                end(run, &mut admission, Ending::Stopped(reason)).await;
                return;
            }
            if !servers.iter_mut().any(Serving::serves) {
                tokio::select! {
                    biased;
                    () = run.cancellation().cancelled() => {
                        admission.close().await;
                        release(run).await;
                        return;
                    }
                    reason = run.stopping() => {
                        stop = Some(reason);
                        continue;
                    }
                    Some(server) = joined.recv() => {
                        servers.push(Serving::joined(server));
                        continue;
                    }
                }
            }
            match when_idle(run, &mut admission).await {
                Ok(Idle::Admit) => continue,
                Ok(Idle::Wait) => {}
                Ok(Idle::Done) => {
                    end(run, &mut admission, Ending::Finished).await;
                    return;
                }
                Err(error) => {
                    tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                    end(run, &mut admission, Ending::Failed).await;
                    return;
                }
            }
        }
        let free = stop.is_none()
            && servers
                .iter_mut()
                .any(|server| server.serves() && server.in_flight < policy.handler_concurrency);
        let wake = free
            .then(|| admission.ready_at(run.clock().now()))
            .flatten();
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
            Some(server) = joined.recv() => {
                servers.push(Serving::joined(server));
            }
            Some(ended) = attempts.join_next() => {
                if let Ok(attempt) = ended {
                    settled(run, settings, &mut admission, &mut pool, servers, attempt).await;
                }
            }
            () = sleep_until(run.clock(), wake), if wake.is_some() => {}
            key = next_wake(wakes.as_mut()) => admission.wake(key.as_deref()),
        }
    }
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

/// Drives a resumed crawl (ADRs 0002 and 0102): each attempt its committed
/// state shows in flight, whose engine stopped without settling it, counts
/// as `handler_lost` against `crawl.retry.max` and is retried or failed,
/// then delivery goes on from the committed state, so the work since that
/// state is redone.
///
/// # Cancel safety
///
/// As [`drive`]'s.
pub(crate) async fn resume(run: Run, client: Client, settings: RunSettings, crawls: Crawls) {
    let policy = CrawlPolicy::of(&settings.at_admission().await);
    let retry_max = policy.retry_max;
    if let Err(error) = run
        .state()
        .write(move |connection| {
            work_items::recover_leases(connection, HANDLER_LOST.name, retry_max)
        })
        .await
    {
        tracing::error!(%error, run_id = %run.id(), "the resumed crawl's work items could not be recovered");
        run.end(Ending::Failed).await;
        return;
    }
    let joined = crawls.open(&run).await;
    drive(run, client, joined, crawls, settings).await;
}

/// The crawl's Session pool under the Run's settings now.
struct Pooled<'a> {
    pool: &'a mut SessionPool,
    settings: PoolSettings,
}

/// Admits work items up to each serving Connection's room, draws the
/// Session of the pool that serves each, written in run state before any
/// handler request is sent, and starts an attempt for each on that
/// Connection.
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
        let room = policy.handler_concurrency - server.in_flight;
        let served: Vec<(Leased, ServingSession)> = admit(run, admission, room)
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
            attempts.spawn(attempt(
                run.clone(),
                place,
                server.client.clone(),
                (leased, session),
                lease,
            ));
        }
    }
    Ok(())
}

/// What an attempt's answer says of its domain: a handler's failure that
/// says the site did not answer in time (a `deadline_exceeded` whose
/// `limit` is the response's first byte or the navigation) backs the domain
/// off like a refused or timed-out connection (ADR 0086); one that says the
/// site refused for too many requests (`blocked` for a `rate_limit`, a 429)
/// halves its concurrency (ADR 0021). A `Retry-After` and a 503 join with
/// the fetch tiers (area 5).
fn domain_ending(answer: &Answer) -> Ended {
    let Answer::Failed { code, details, .. } = answer else {
        return Ended::Done;
    };
    if block_cause(code, details) == Some(BlockCause::RateLimit) {
        return Ended::TooManyRequests(None);
    }
    let timed_out = code == DEADLINE_EXCEEDED.name
        && details
            .get("limit")
            .and_then(|limit| limit.get("key"))
            .and_then(Value::as_str)
            .is_some_and(|key| [TIMEOUT_FIRST_BYTE.name, TIMEOUT_NAVIGATION.name].contains(&key));
    if timed_out {
        Ended::BackOff
    } else {
        Ended::Done
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

/// Settles an attempt that ended under the Run's policy now, frees its
/// place on its Connection, returns its domain's turn, puts a retried work
/// item back in its queue and journals what the queues changed.
async fn settled(
    run: &Run,
    settings: &RunSettings,
    admission: &mut Admission,
    pool: &mut SessionPool,
    servers: &mut [Serving],
    attempt: Attempt,
) {
    let server = &mut servers[attempt.server];
    server.in_flight -= 1;
    if let Answer::ClientGone = attempt.answer {
        server.gone = true;
    }
    let waiting = attempt.leased.waiting.clone();
    let domain = domain_ending(&attempt.answer);
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
    let retried = settle(run, CrawlPolicy::of(&resolved), attempt).await;
    admission
        .ended(&waiting.id, &waiting.key, run.clock().now(), domain)
        .await;
    if retried {
        admission.enqueue(waiting);
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
            run.end(Ending::Failed).await;
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

/// Keys each work item not keyed yet, the seeds at the start and those a
/// handler enqueued since, and offers it to its domain's queue, which
/// takes it unless full; one it refuses waits in run state until its queue
/// fills again.
async fn key_new(run: &Run, admission: &mut Admission) -> Result<(), RunStateError> {
    let unkeyed = run.state().read(work_items::unkeyed).await?;
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
    let mut drained = Vec::new();
    while ids.len() < room {
        let Some(admitted) = admission.admit(now).await else {
            break;
        };
        match run.charge(REQUESTS, 1).await {
            Ok(()) => {}
            Err(ChargeError::Refused(Refusal::Reached(_) | Refusal::NotUnderstood(_))) => break,
            Err(error) => return Err(error),
        }
        if admitted.drained {
            drained.push(admitted.key);
        }
        ids.push(admitted.id);
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let leased = run
        .state()
        .write(move |connection| work_items::lease(connection, &ids))
        .await?;
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

/// One attempt: the handler request, bounded by its lease. When the lease
/// ends first, the request is cancelled and its answer left to [`late`].
async fn attempt(
    run: Run,
    server: usize,
    client: Client,
    (leased, session): (Leased, ServingSession),
    lease: WorkItemLease,
) -> Attempt {
    let params = CrawlRequestHandleParams {
        run_id: run.id().clone(),
        crawl_request: leased.crawl_request.clone(),
        attempt: leased.attempt,
        lease_epoch: leased.lease_epoch,
        session: session.clone(),
    };
    let answer = match client
        .send::<CrawlRequestHandled>(&CRAWL_REQUEST_HANDLE, &params)
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
    };
    Attempt {
        server,
        leased,
        session,
        lease,
        answer,
    }
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

/// Commits an attempt's outcome: its Records once while its lease holds,
/// its handler's skip, its block, or its work item retried or failed; then
/// releases its lease. Answers whether its work item waits again for a
/// retry.
async fn settle(run: &Run, policy: CrawlPolicy, attempt: Attempt) -> bool {
    let Attempt {
        leased,
        lease,
        answer,
        ..
    } = attempt;
    let id = leased.crawl_request.id.clone();
    let epoch = leased.lease_epoch;
    let failure = |code: String, retryable: bool, message: String| {
        let retryable = if code == HANDLER_FAILED.name {
            policy.retry_handler_failed
        } else {
            retryable
        };
        let failure = Failure { code, message };
        if retryable && policy.may_retry(leased.attempt) {
            Unsuccessful::Retried(failure)
        } else {
            Unsuccessful::Failed(failure)
        }
    };
    let unsuccessful = match answer {
        Answer::Handled(CrawlRequestHandled {
            skipped: Some(skip),
            ..
        }) => Unsuccessful::Skipped(skip.message),
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
                    LEASE_LOST.name.to_owned(),
                    LEASE_LOST.retryable,
                    LEASE_LOST.description.to_owned(),
                ),
                Err(CommitError::RunState(error)) => {
                    tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                    return false;
                }
            }
        }
        Answer::ClientGone => failure(
            CLIENT_GONE.name.to_owned(),
            CLIENT_GONE.retryable,
            CLIENT_GONE.description.to_owned(),
        ),
        Answer::LeaseEnded => failure(
            HANDLER_LOST.name.to_owned(),
            HANDLER_LOST.retryable,
            HANDLER_LOST.description.to_owned(),
        ),
        Answer::Failed {
            code,
            retryable,
            message,
            details,
        } => match block_cause(&code, &details) {
            Some(cause) => Unsuccessful::Blocked(cause),
            None => failure(code, retryable, message),
        },
    };
    let retried = matches!(unsuccessful, Unsuccessful::Retried(_));
    let recorded = match &unsuccessful {
        Unsuccessful::Retried(failure) | Unsuccessful::Failed(failure) => {
            Some(failure.code.clone())
        }
        Unsuccessful::Skipped(_) | Unsuccessful::Blocked(_) => None,
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

/// What an attempt's answer says of the Session that served it: a
/// committed answer is a success, a block or a failed Page check scores it
/// down, and any other ending, a skip or the handler's own failure, says
/// nothing of it.
fn effect(answer: &Answer) -> Effect {
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
fn block_cause(code: &str, details: &Map<String, Value>) -> Option<BlockCause> {
    if code != BLOCKED {
        return None;
    }
    details
        .get("cause")
        .and_then(|cause| serde_json::from_value(cause.clone()).ok())
}

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run) {
    if let Err(error) = run.state().write(work_items::release_leases).await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
