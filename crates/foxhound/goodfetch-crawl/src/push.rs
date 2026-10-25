//! A handler's push (ADR 0103): `crawl_request.push` checks one Record
//! against the Dataset it names, as the run file declares it, and stages
//! it with its attempt while the attempt's lease holds; it commits once
//! with the attempt or is discarded with it. A Record that does not hold
//! is refused with `dataset_unknown` or `record_invalid` and not staged,
//! the attempt keeping what it staged before; a `record_invalid` Record's
//! rejection is staged in its place, so it enters its Dataset's rejection
//! table if the attempt commits; an attempt that ended
//! refuses it with `attempt_ended`, and a declared Effect's body with
//! `effect_staging_refused`.

use goodfetch_dataset::{Datasets, Refused};
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, PushedRecord};
use goodfetch_protocol::foxhound::dataset::{
    CRAWL_REQUEST_PUSH, CrawlRequestPushParams, CrawlRequestPushed,
};
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails};

use crate::serving::{Crawls, LiveCrawl};

/// The handler of `crawl_request.push` on the live crawls of `crawls`.
pub(crate) fn handler(crawls: Crawls) -> Handler {
    Handler::new(&CRAWL_REQUEST_PUSH, move |params, context| {
        push(params, context, crawls.clone())
    })
}

async fn push(
    params: CrawlRequestPushParams,
    _context: CallContext,
    crawls: Crawls,
) -> Result<CrawlRequestPushed, CallError> {
    let CrawlRequestPushParams {
        run_id,
        crawl_request_id,
        lease_epoch,
        dataset,
        data,
    } = params;
    let ended = || {
        CallError::new(
            &ATTEMPT_ENDED,
            format!(
                "Attempt {lease_epoch} of crawl request `{crawl_request_id}` of the crawl `{run_id}` already ended, so the Record was not staged."
            ),
            &(),
        )
    };
    let Some(LiveCrawl { run, staging, .. }) = crawls.live(&run_id).await else {
        return Err(ended());
    };
    let work_item = crawl_request_id.clone();
    let answered = run
        .state()
        .write(move |connection| {
            if !staging.holds(&work_item, lease_epoch) {
                return Ok(None);
            }
            match Datasets::read(connection)?.check_pushed(PushedRecord { dataset, data }) {
                Ok(checked) => Ok(staging.push(&work_item, lease_epoch, checked)),
                Err(Refused { error, rejected }) => Ok(staging
                    .push(&work_item, lease_epoch, rejected)
                    .map(|staged| staged.and(Err(error)))),
            }
        })
        .await;
    match answered {
        Ok(Some(Ok(()))) => Ok(CrawlRequestPushed {}),
        Ok(Some(Err(refused))) => Err(refused),
        Ok(None) => Err(ended()),
        Err(error) => Err(CallError::new(
            &RUN_STATE_UNAVAILABLE,
            format!("The state of the Run `{run_id}` could not take the push: {error}"),
            &RunDetails {
                run_id: run_id.clone(),
                path: None,
            },
        )),
    }
}
