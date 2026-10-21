//! A crawl's delivery (ADRs 0003, 0024 and 0093): the Run's driver, which
//! leases pending work items to the program's handler, at most
//! `crawl.handler.concurrency` at once, each lease committed in run state
//! before its `crawl_request.handle` request is sent; commits an answered
//! attempt's Records once, through its lease, while the lease holds; and
//! ends an attempt whose handler failed, or that got no answer before its
//! lease ended, with nothing committed, its work item retried under the
//! Run's policy, which each admission point reads again from the Run's
//! settings; a handler's skip ends its work item `skipped`, and a failure
//! with the Error code `blocked` ends it `blocked`. Each ending's outcome
//! is committed with it. When a lease ends first, the attempt's request is cancelled
//! with `$/cancel`, and an answer that still comes is a late commit,
//! refused with `lease_lost`. Once every work item is done, it finishes
//! the Run. When the Run stops, the attempts in flight are abandoned and
//! their work items return to pending without counting; when the program's
//! Connection ends, the driver delivers nothing more and waits for the Run
//! to stop.

use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::runs::{Ending, Run};
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    HANDLER_LOST, LEASE_LOST,
};
use serde_json::{Map, Value};
use tokio::task::JoinSet;

use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::CrawlPolicy;
use crate::work_items::{self, Failure, Leased, Unsuccessful};

/// The Error code of a page the site refused, which Collie's page raises
/// with the Block cause in its details (ADRs 0035 and 0079); declared with
/// the page's errors (#711).
const BLOCKED: &str = "blocked";

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
/// releases its lease.
async fn settle(run: &Run, policy: CrawlPolicy, attempt: Attempt) {
    let Attempt {
        leased,
        lease,
        answer,
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
                    return;
                }
                Err(CommitError::LeaseLost) => failure(
                    LEASE_LOST.name.to_owned(),
                    LEASE_LOST.retryable,
                    LEASE_LOST.description.to_owned(),
                ),
                Err(CommitError::RunState(error)) => {
                    tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
                    return;
                }
            }
        }
        Answer::LeaseEnded | Answer::ClientGone => failure(
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
        }
        Ok(false) => {
            tracing::debug!(run_id = %run.id(), work_item = %leased.crawl_request.id, "an attempt past its lease commits nothing");
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "an attempt's outcome was not committed");
        }
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
