//! A crawl's delivery (ADRs 0003, 0024 and 0093): the Run's driver, which
//! leases pending work items to the program's handler, at most
//! `crawl.handler.concurrency` at once, each lease committed in run state
//! before its `crawl_request.handle` request is sent; commits an answered
//! attempt's Records once, through its lease, while the lease holds; and
//! ends an attempt whose handler failed, or that got no answer before its
//! lease ended, with nothing committed, its work item retried under the
//! Run's policy, which each admission point reads again from the Run's
//! settings. When a lease ends first, the attempt's request is cancelled
//! with `$/cancel`, and an answer that still comes is a late commit,
//! refused with `lease_lost`. Once every work item is done, it finishes
//! the Run. When the Run stops, the attempts in flight are abandoned and
//! their work items return to pending without counting; when the program's
//! Connection ends, the driver delivers nothing more and waits for the Run
//! to stop.

use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::runs::{Ending, Run};
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    HANDLER_LOST, LEASE_LOST,
};
use tokio::task::JoinSet;

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
    let mut attempts: JoinSet<Attempt> = JoinSet::new();
    let mut client_gone = false;
    loop {
        let policy = CrawlPolicy::of(&settings.at_admission().await);
        if !client_gone && attempts.len() < policy.handler_concurrency {
            let room = policy.handler_concurrency - attempts.len();
            match run
                .state()
                .write(move |connection| work_items::lease(connection, room))
                .await
            {
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
        }
        if attempts.is_empty() {
            if client_gone {
                run.cancellation().cancelled().await;
                release(&run).await;
                return;
            }
            match run.state().read(work_items::any_pending).await {
                Ok(false) => {
                    run.end(Ending::Finished).await;
                    return;
                }
                Ok(true) => continue,
                Err(error) => {
                    tracing::error!(%error, run_id = %run.id(), "the crawl's work items could not be read");
                    run.end(Ending::Failed).await;
                    return;
                }
            }
        }
        tokio::select! {
            biased;
            () = run.cancellation().cancelled() => {
                attempts.shutdown().await;
                release(&run).await;
                return;
            }
            Some(ended) = attempts.join_next() => {
                let Ok(attempt) = ended else {
                    continue;
                };
                if let Answer::ClientGone = attempt.answer {
                    client_gone = true;
                }
                let policy = CrawlPolicy::of(&settings.at_admission().await);
                settle(&run, policy, attempt).await;
            }
        }
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
pub(crate) async fn resume(run: Run, client: Client, settings: RunSettings) {
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
    drive(run, client, settings).await;
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
/// its work item retried or failed; then releases its lease.
async fn settle(run: &Run, policy: CrawlPolicy, attempt: Attempt) {
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
                return;
            }
            Err(CommitError::LeaseLost) => (LEASE_LOST.name.to_owned(), LEASE_LOST.retryable),
            Err(CommitError::RunState(error)) => {
                tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                return;
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
        Ok(true) => run.record_outcome(&recorded),
        Ok(false) => {
            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "an attempt past its lease commits nothing");
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
        }
    }
}

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run) {
    if let Err(error) = run.state().write(work_items::release_leases).await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
