//! A crawl's delivery (ADRs 0003, 0024 and 0093): the Run's driver, which
//! leases pending work items to the program's handler, at most
//! `crawl.handler.concurrency` at once, each lease committed in run state
//! before its `crawl_request.handle` request is sent; commits an answered
//! attempt's Records once, while its lease holds; and ends an attempt whose
//! handler failed, or that got no answer before its lease ended, with
//! nothing committed, its work item retried under the Run's policy, which
//! each admission point reads again from the Run's settings. Once
//! every work item is done, it finishes the Run. When the Run stops, the
//! attempts in flight are abandoned and their work items return to pending
//! without counting; when the program's Connection ends, the driver
//! delivers nothing more and waits for the Run to stop.

use goodfetch_engine::client::{Client, ClientRequestError};
use goodfetch_engine::runs::{Ending, Run};
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    HANDLER_LOST,
};
use tokio::task::JoinSet;

use crate::lease::Lease;
use crate::policy::CrawlPolicy;
use crate::work_items::{self, Leased};

/// How an attempt ended, before its outcome is committed.
enum Answer {
    /// The handler answered while the lease held.
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
    lease: Lease,
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
                        let lease = Lease::granted(
                            leased.lease_epoch,
                            run.clock().now(),
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

/// One attempt: the handler request, bounded by its lease.
async fn attempt(run: Run, client: Client, leased: Leased, lease: Lease) -> Attempt {
    let params = CrawlRequestHandleParams {
        run_id: run.id().clone(),
        crawl_request: leased.crawl_request.clone(),
        attempt: leased.attempt,
        lease_epoch: leased.lease_epoch,
    };
    let clock = run.clock();
    let answer = tokio::select! {
        biased;
        answer = client.request::<CrawlRequestHandled>(&CRAWL_REQUEST_HANDLE, &params) => match answer {
            Ok(handled) => Answer::Handled(handled),
            Err(ClientRequestError::Failed { code, retryable, .. }) => Answer::Failed { code, retryable },
            Err(ClientRequestError::Malformed(_)) => Answer::Failed {
                code: HANDLER_FAILED.name.to_owned(),
                retryable: false,
            },
            Err(ClientRequestError::Gone) => Answer::ClientGone,
            Err(_) => Answer::LeaseEnded,
        },
        () = clock.sleep_until(lease.expires_at()) => Answer::LeaseEnded,
    };
    Attempt {
        leased,
        lease,
        answer,
    }
}

/// Commits an attempt's outcome: its Records once while its lease holds, or
/// its work item retried or failed.
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
    // `failed`: the Error code the work item records, which no reply carries.
    let (written, failed) = match answer {
        Answer::Handled(handled) if lease.holds_at(run.clock().now()) => {
            let written = run
                .state()
                .write(move |connection| {
                    work_items::commit(connection, &id, epoch, &handled.records)
                })
                .await;
            (written, None)
        }
        Answer::Handled(_) | Answer::LeaseEnded | Answer::ClientGone => {
            let retry = retry(HANDLER_LOST.name, HANDLER_LOST.retryable);
            let written = run
                .state()
                .write(move |connection| {
                    work_items::fail(connection, &id, epoch, HANDLER_LOST.name, retry)
                })
                .await;
            (written, Some(HANDLER_LOST.name.to_owned()))
        }
        Answer::Failed { code, retryable } => {
            let retry = retry(&code, retryable);
            let recorded = code.clone();
            let written = run
                .state()
                .write(move |connection| work_items::fail(connection, &id, epoch, &code, retry))
                .await;
            (written, Some(recorded))
        }
    };
    match written {
        Ok(true) => {
            if let Some(code) = failed {
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

/// Returns the leased work items to pending, without counting them.
async fn release(run: &Run) {
    if let Err(error) = run.state().write(work_items::release_leases).await {
        tracing::warn!(%error, run_id = %run.id(), "the crawl's leases were not released");
    }
}
