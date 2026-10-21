//! A crawl's delivery (ADRs 0003, 0004, 0024 and 0093): the Run's driver,
//! which admits pending work items through the Run's admission, the
//! per-domain queues Foxhound's composition passes in as the gate in
//! admission under the Run's rate cap, and leases them to the program's
//! handler, at most `crawl.handler.concurrency` at once, each lease
//! committed in run state
//! before its `crawl_request.handle` request is sent; commits an answered
//! attempt's Records once, through its lease, while the lease holds; and
//! ends an attempt whose handler failed, or that got no answer before its
//! lease ended, with nothing committed, its work item retried under the
//! Run's policy, which each admission point reads again from the Run's
//! settings. When a lease ends first, the attempt's request is cancelled
//! with `$/cancel`, and an answer that still comes is a late commit,
//! refused with `lease_lost`. Once every work item is done, it finishes
//! the Run. When the Run is asked to stop at its next safe point, such as
//! once its `timeout.run` passed, it admits nothing new, lets each attempt
//! in flight commit or be abandoned under its lease, and then stops the
//! Run. When the Run stops at once, by an abort, the attempts in flight are
//! abandoned and their work items return to pending without counting; when the program's
//! Connection ends, the driver delivers nothing more and waits for the Run
//! to stop.

use goodfetch_engine::admission::{Admission, Ended};
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::runs::{Ending, Run};
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    HANDLER_LOST, LEASE_LOST,
};
use goodfetch_protocol::run::StopReason;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::CrawlPolicy;
use crate::work_items::{self, Leased};

/// How an attempt ended, before its outcome is committed.
enum Answer {
    /// The handler answered.
    Handled(CrawlRequestHandled),
    /// The handler failed, or answered no result.
    Failed { code: String, retryable: bool },
    /// The lease ended with no answer.
    LeaseEnded,
    /// The program's Connection ended.
    ClientGone,
}

struct Attempt {
    leased: Leased,
    lease: WorkItemLease,
    answer: Answer,
}

/// Drives `run`'s work items to the handler of `client` until each is done
/// or the Run stops.
///
/// # Cancel safety
///
/// Run it as one of the Run's tasks to its end: dropped, it leaves its
/// leases to the next writer, which counts them as lost.
pub(crate) async fn drive(run: Run, client: Client, settings: RunSettings) {
    let run_leases = Leases::new(run.clock(), run.id().as_str(), run.state().epoch());
    let mut admission = match open(&run, &settings).await {
        Ok(admission) => admission,
        Err(error) => {
            tracing::error!(%error, run_id = %run.id(), "the crawl's admission could not open");
            run.end(Ending::Failed).await;
            return;
        }
    };
    let mut attempts: JoinSet<Attempt> = JoinSet::new();
    let mut client_gone = false;
    let mut stop: Option<StopReason> = None;
    loop {
        let resolved = settings.at_admission().await;
        let policy = CrawlPolicy::of(&resolved);
        admission.update(&resolved);
        let admitting = !client_gone && stop.is_none();
        let room = if admitting {
            policy.handler_concurrency.saturating_sub(attempts.len())
        } else {
            0
        };
        match admit(&run, &mut admission, room).await {
            Ok(leased) => {
                for leased in leased {
                    let lease = run_leases.grant(
                        &leased.crawl_request.id,
                        leased.lease_epoch,
                        policy.lease_length(),
                    );
                    attempts.spawn(attempt(run.clone(), client.clone(), leased, lease));
                }
            }
            Err(error) => {
                tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be leased");
                run.end(Ending::Failed).await;
                return;
            }
        }
        if attempts.is_empty() {
            if let Some(reason) = stop {
                run.end(Ending::Stopped(reason)).await;
                return;
            }
            if client_gone {
                run.cancellation().cancelled().await;
                release(&run).await;
                return;
            }
            match when_idle(&run, &mut admission).await {
                Ok(Idle::Admit) => continue,
                Ok(Idle::Wait) => {}
                Ok(Idle::Done) => {
                    run.end(Ending::Finished).await;
                    return;
                }
                Err(error) => {
                    tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                    run.end(Ending::Failed).await;
                    return;
                }
            }
        }
        let free = admitting && attempts.len() < policy.handler_concurrency;
        let wake = free
            .then(|| admission.ready_at(run.clock().now()))
            .flatten();
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => {
                attempts.shutdown().await;
                release(&run).await;
                return;
            }
            reason = run.stopping(), if stop.is_none() => {
                stop = Some(reason);
            }
            Some(ended) = attempts.join_next() => {
                let Ok(attempt) = ended else {
                    continue;
                };
                if let Answer::ClientGone = attempt.answer {
                    client_gone = true;
                }
                settled(&run, &settings, &mut admission, attempt).await;
            }
            () = sleep_until(run.clock(), wake), if wake.is_some() => {}
        }
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

/// Settles an attempt that ended under the Run's policy now, returns its
/// domain's turn, puts a retried work item back in its queue and journals
/// what the queues changed.
async fn settled(run: &Run, settings: &RunSettings, admission: &mut Admission, attempt: Attempt) {
    let waiting = attempt.leased.waiting.clone();
    let policy = CrawlPolicy::of(&settings.at_admission().await);
    let retried = settle(run, policy, attempt).await;
    admission.ended(&waiting.key, run.clock().now(), Ended::Done);
    if retried {
        admission.enqueue(waiting);
    }
    journal(run, admission).await;
}

/// Opens the Run's admission, the gate Foxhound's composition passes in,
/// keys the work items not keyed yet and fills each domain's queue.
async fn open(run: &Run, settings: &RunSettings) -> Result<Admission, String> {
    let resolved = settings.at_admission().await;
    let mut admission = run
        .admission(&resolved)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("the composition passes in no gate in admission")?;
    let unkeyed = run
        .state()
        .read(work_items::unkeyed)
        .await
        .map_err(|error| error.to_string())?;
    let keys: Vec<(String, String)> = unkeyed
        .into_iter()
        .map(|(id, url)| {
            let key = admission.key(&url);
            (id, key)
        })
        .collect();
    run.state()
        .write(move |connection| work_items::set_keys(connection, &keys))
        .await
        .map_err(|error| error.to_string())?;
    fill(run, &mut admission, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(admission)
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
/// again each domain's queue that drained.
async fn admit(
    run: &Run,
    admission: &mut Admission,
    room: usize,
) -> Result<Vec<Leased>, RunStateError> {
    let now = run.clock().now();
    let mut ids = Vec::new();
    let mut drained = Vec::new();
    while ids.len() < room {
        let Some(admitted) = admission.admit(now) else {
            break;
        };
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
async fn attempt(run: Run, client: Client, leased: Leased, lease: WorkItemLease) -> Attempt {
    let params = CrawlRequestHandleParams {
        run_id: run.id().clone(),
        crawl_request: leased.crawl_request.clone(),
        attempt: leased.attempt,
        lease_epoch: leased.lease_epoch,
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
                Err(ClientRequestError::Failed { code, retryable, .. }) => Answer::Failed { code, retryable },
                Err(ClientRequestError::Malformed(_)) => Answer::Failed {
                    code: HANDLER_FAILED.name.to_owned(),
                    retryable: false,
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
        leased,
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
                && let Err(CommitError::LeaseLost) = lease.commit(run.state(), handled.records).await
            {
                run.record_outcome(LEASE_LOST.name);
            }
        }
    }
}

/// Commits an attempt's outcome: its Records once while its lease holds, or
/// its work item retried or failed; then releases its lease. Answers
/// whether its work item waits again for a retry.
async fn settle(run: &Run, policy: CrawlPolicy, attempt: Attempt) -> bool {
    let Attempt {
        leased,
        lease,
        answer,
    } = attempt;
    let id = leased.crawl_request.id.clone();
    let epoch = leased.lease_epoch;
    let retry = |code: &str, retryable: bool| {
        let retryable = if code == HANDLER_FAILED.name {
            policy.retry_handler_failed
        } else {
            retryable
        };
        retryable && policy.may_retry(leased.attempt)
    };
    let (code, retryable) = match answer {
        Answer::Handled(handled) => match lease.commit(run.state(), handled.records).await {
            Ok(()) => {
                lease.release();
                return false;
            }
            Err(CommitError::LeaseLost) => (LEASE_LOST.name.to_owned(), LEASE_LOST.retryable),
            Err(CommitError::RunState(error)) => {
                tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                return false;
            }
        },
        Answer::LeaseEnded | Answer::ClientGone => {
            (HANDLER_LOST.name.to_owned(), HANDLER_LOST.retryable)
        }
        Answer::Failed { code, retryable } => (code, retryable),
    };
    let retry = retry(&code, retryable);
    let recorded = code.clone();
    let written = run
        .state()
        .write(move |connection| work_items::fail(connection, &id, epoch, &code, retry))
        .await;
    lease.release();
    match written {
        Ok(true) => {
            run.record_outcome(&recorded);
            retry
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

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run) {
    if let Err(error) = run.state().write(work_items::release_leases).await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
