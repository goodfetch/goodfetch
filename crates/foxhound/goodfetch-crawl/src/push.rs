//! A handler's push (ADRs 0103 and 0147): `crawl_request.push` checks the
//! Records of one push, one or several, against the Dataset they name, as
//! the live crawl declares it, and stages those that hold with their
//! attempt while the attempt's lease holds, in one step; they commit once
//! with the attempt or are discarded with it. A push of no Record is refused
//! with `invalid_params`; a Dataset the crawl does not
//! declare is refused with `dataset_unknown`, nothing staged; a Record that
//! does not hold is not staged and the push fails with `record_invalid`,
//! naming it by its position, the attempt keeping what it staged before
//! and the push's other Records; its rejection is staged in its place, so
//! it enters its Dataset's rejection table if the attempt commits; an
//! attempt that ended refuses the push with `attempt_ended`, and a declared
//! Effect's body with `effect_staging_refused`.

use goodfetch_dataset::Refused;
use goodfetch_engine::staging::AttemptWrites as _;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::dataset::{
    CRAWL_REQUEST_PUSH, CrawlRequestPushParams, CrawlRequestPushed,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;

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
        records,
    } = params;
    if records.is_empty() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("The push into the Dataset `{dataset}` holds no Record: push one or several."),
        ));
    }
    let ended = || {
        CallError::new(
            &ATTEMPT_ENDED,
            format!(
                "Attempt {lease_epoch} of crawl request `{crawl_request_id}` of the crawl `{run_id}` already ended, so no Record was staged."
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
            match datasets.check_pushed(&dataset, records, &work) {
                Ok(checked) => staging.push(&work_item, lease_epoch, checked),
                Err(Refused { error, staged }) => staging
                    .push(&work_item, lease_epoch, staged)
                    .map(|pushed| pushed.and(Err(error))),
            }
        })
        .await;
    match staged {
        Some(Ok(())) => Ok(CrawlRequestPushed {}),
        Some(Err(refused)) => Err(refused),
        None => Err(ended()),
    }
}
