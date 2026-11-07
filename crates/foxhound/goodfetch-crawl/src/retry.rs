//! `run.retry_failed` (ADRs 0102 and 0118; spec #581's `readmit_failed`):
//! re-admits, in the same Run and its Datasets, every work item whose
//! latest outcome is `failed`, and with `blocked` also `blocked`, the same
//! predicate `run.failed_requests` lists by. Each keeps its ID, Label,
//! Payload and depth, its attempts counting on from a fresh
//! `crawl.retry.max`, and the call answers how many. A crawl this engine
//! runs takes them at once, its delivery woken as a seed added wakes it,
//! while its intake is open; once the crawl closed it to end its Run, the
//! call waits for that end and re-admits as for an ended Run, so a
//! re-admission is never answered and then left pending (#1005).
//! On an ended Run with something to retry, the engine reopens it under the
//! Run's event `RetryFailed`, as a resume takes its writer role, the
//! re-admission commits under that writer, and the crawl's driver starts
//! again with the calling Connection as its server; with nothing to retry
//! the Run stays as it is and the answer is 0.

use goodfetch_engine::run::{checkpoint_interval, run_error_of, settings_error, through_owner};
use goodfetch_engine::runs::{Resume, Resumed, Run, RunError};
use goodfetch_engine::settings::Opening;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::outcomes::{
    RUN_RETRY_FAILED, RetriedFailed, RunRetryFailedParams,
};
use goodfetch_protocol::run::{
    Lifecycle, RUN_INTERRUPTED, RUN_STATE_UNAVAILABLE, RunDetails, RunId,
};
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_RUN};

use crate::intake::Intake;
use crate::serving::{Crawls, LiveCrawl};
use crate::work_items;

/// The handler of `run.retry_failed` on the crawls of `crawls`.
pub(crate) fn handler(crawls: Crawls) -> Handler {
    Handler::new(&RUN_RETRY_FAILED, move |params, context| {
        retry_failed(params, context, crawls.clone())
    })
}

async fn retry_failed(
    params: RunRetryFailedParams,
    context: CallContext,
    crawls: Crawls,
) -> Result<RetriedFailed, CallError> {
    if let Some(LiveCrawl {
        run,
        seeded,
        intake,
        ..
    }) = crawls.live(&params.run_id).await
    {
        if let Some(readmitted) = readmit(&run, params.blocked, Some(intake)).await? {
            if readmitted > 0 {
                seeded.notify_one();
            }
            return Ok(RetriedFailed { readmitted });
        }
        // The crawl is ending its Run: once it ended, its failures are
        // re-admitted as an ended Run's are.
        if let Err(error) = context.runs().ended(params.run_id.clone()).await {
            tracing::debug!(%error, run_id = %params.run_id, "the ending crawl's report could not be read");
        }
    }
    if let Some(answer) =
        through_owner(&context, &RUN_RETRY_FAILED, &params.run_id, &params).await?
    {
        return Ok(answer);
    }
    let failed = |error: RunError| run_error_of(error, context.identity(), RUN_RETRY_FAILED.name);
    let blocked = params.blocked;
    let waiting = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?
        .read(move |connection| work_items::readmittable(connection, blocked))
        .await
        .map_err(failed)?;
    if waiting == 0 {
        return Ok(RetriedFailed { readmitted: 0 });
    }
    reopened(params, context).await
}

/// Reopens the ended crawl `params` names under the Run's event
/// `RetryFailed`, re-admits its failures and starts its driver again, with
/// the calling Connection as its server.
async fn reopened(
    params: RunRetryFailedParams,
    context: CallContext,
) -> Result<RetriedFailed, CallError> {
    let RunRetryFailedParams {
        run_id,
        blocked,
        settings,
        project,
    } = params;
    let (settings, _) = context
        .runs()
        .settings()
        .open(Opening {
            run: None,
            project,
            recipe: None,
            settings,
            origin: Some(RunOrigin::Settings),
            acting_for_model: context.surface().acts_for_model(),
        })
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let resumed = context
        .runs()
        .resume(Resume {
            run_id: run_id.clone(),
            checkpoint: None,
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().get(&TIMEOUT_RUN),
            settings: Some(settings.clone()),
            retry_failed: true,
        })
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_RETRY_FAILED.name))?;
    match resumed {
        Resumed::New { run, kind, .. } => {
            let readmitted = readmit(&run, blocked, None).await;
            if let Some(driver) = context.driver(kind) {
                run.spawn(driver(*run.clone(), context.clone(), settings));
            }
            readmitted.map(|readmitted| RetriedFailed {
                readmitted: readmitted.unwrap_or_default(),
            })
        }
        Resumed::Existing(existing) if existing.lifecycle == Lifecycle::Running => {
            Err(interrupted(&run_id))
        }
        // Never, as the Run's table reopens every ended Run: nothing to
        // retry in a Run that stays as it is.
        Resumed::Existing(_) => Ok(RetriedFailed { readmitted: 0 }),
    }
}

/// Re-admits `run`'s failures, and with `blocked` its blocked requests, in
/// one write of its state, and answers how many; `None`, re-admitting
/// none, when its live crawl's `intake` is closed by then.
async fn readmit(
    run: &Run,
    blocked: bool,
    intake: Option<Intake>,
) -> Result<Option<u64>, CallError> {
    let readmitted = run
        .state()
        .write(move |connection| {
            let readmit = || work_items::readmit(connection, blocked);
            match intake {
                Some(intake) => intake.admitting(readmit),
                None => readmit().map(Some),
            }
        })
        .await
        .map_err(|error| {
            CallError::new(
                &RUN_STATE_UNAVAILABLE,
                format!(
                    "The state of the Run `{}` could not take the re-admission: {error}",
                    run.id()
                ),
                &RunDetails {
                    run_id: run.id().clone(),
                    path: Some(run.folder().display().to_string()),
                },
            )
        })?;
    Ok(readmitted.map(|readmitted| u64::try_from(readmitted.len()).unwrap_or(u64::MAX)))
}

/// `run_interrupted` for a Run whose state says `running` with no writer
/// in this engine: a resume takes its writer role first.
fn interrupted(run_id: &RunId) -> CallError {
    CallError::new(
        &RUN_INTERRUPTED,
        format!(
            "The Run `{run_id}` has no live writer and has not ended, so nothing re-admits its failures: resume it with its Run ID, then retry them."
        ),
        &RunDetails {
            run_id: run_id.clone(),
            path: None,
        },
    )
}
