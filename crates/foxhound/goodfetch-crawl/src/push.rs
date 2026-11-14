//! A handler's push (ADR 0103): `crawl_request.push` checks one Record
//! against the Dataset it names, as the live crawl declares it, and stages
//! it with its attempt while the attempt's lease holds; it commits once
//! with the attempt or is discarded with it. A Record that does not hold
//! is refused with `dataset_unknown` or `record_invalid` and not staged,
//! the attempt keeping what it staged before; a `record_invalid` Record's
//! rejection is staged in its place, so it enters its Dataset's rejection
//! table if the attempt commits; an attempt that ended
//! refuses it with `attempt_ended`, and a declared Effect's body with
//! `effect_staging_refused`.

use goodfetch_dataset::Refused;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, PushedRecord};
use goodfetch_protocol::foxhound::dataset::{
    CRAWL_REQUEST_PUSH, CrawlRequestPushParams, CrawlRequestPushed,
};

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
    let Some(LiveCrawl {
        run,
        staging,
        datasets,
        ..
    }) = crawls.live(&run_id).await
    else {
        return Err(ended());
    };
    // The attempt's staging is the push's only state: its entry is there
    // until the attempt's commit takes it or its end discards it, so a push
    // either rides that commit or is refused, with no write of its own. Its
    // check is a validation, so it runs off the async workers.
    let work = run.metrics().counted_work().clone();
    let work_item = crawl_request_id.clone();
    let staged = run
        .owner()
        .blocking(move || {
            if !staging.holds(&work_item, lease_epoch) {
                return None;
            }
            match datasets.check_pushed(PushedRecord { dataset, data }, &work) {
                Ok(checked) => staging.push(&work_item, lease_epoch, checked),
                Err(Refused { error, rejected }) => staging
                    .push(&work_item, lease_epoch, rejected)
                    .map(|staged| staged.and(Err(error))),
            }
        })
        .await;
    match staged {
        Some(Ok(())) => Ok(CrawlRequestPushed {}),
        Some(Err(refused)) => Err(refused),
        None => Err(ended()),
    }
}
