//! A handler's enqueue (ADRs 0093 and 0120): `crawl_request.enqueue` adds
//! one request to a live crawl from inside an attempt, while its lease
//! holds, through the Frontier, which resolves a relative URL against the
//! document's base URL, measures it against the crawl's scope, which the
//! Run's invariants hold, then the Run's domain policy, and answers added or not added
//! with its reason; a Payload that does not fit its Label's JSON Schema or
//! `crawl.payload.max_size` fails the call with `payload_invalid` first
//! (ADR 0082). The
//! request is one level deeper than the attempt's, and is staged with the
//! attempt, answered as the Frontier will admit it, and committed with the
//! attempt, which adds it as its staging keyed it and runs this admission
//! no second time (#2245), or discarded with it (ADR 0093); the crawl's
//! delivery keys it and queues it under its domain once the attempt
//! settles. Its call's task reads its params and serializes its Payload,
//! then asks the crawl's Run state, which its delivery loop owns, for the
//! part that needs the Run's state (ADR 0142; `run_state::admission`): no
//! call queues a job with the run-state writer itself.

use std::sync::Arc;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::{Url, document_base};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CrawlRequestEnqueueParams, Enqueued,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails};

use crate::run_state::StateError;
use crate::run_state::admission::Enqueue;
use crate::serving::{Crawls, LiveCrawl};

/// The handler of `crawl_request.enqueue` on the live crawls of `crawls`,
/// whose pages `tiers` serve.
pub(crate) fn handler(crawls: Crawls, tiers: FetchTiers) -> Handler {
    Handler::new(&CRAWL_REQUEST_ENQUEUE, move |params, context| {
        enqueue(params, context, crawls.clone(), tiers.clone())
    })
}

async fn enqueue(
    params: CrawlRequestEnqueueParams,
    _context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
) -> Result<Enqueued, CallError> {
    let ended = || {
        CallError::new(
            &ATTEMPT_ENDED,
            format!(
                "Attempt {} of crawl request `{}` of the crawl `{}` already ended, so nothing was enqueued.",
                params.lease_epoch, params.crawl_request_id, params.run_id
            ),
            &(),
        )
    };
    let Some(LiveCrawl { run, asking, .. }) = crawls.live(&params.run_id).await else {
        return Err(ended());
    };
    // An absolute URL needs no base, so a browser page is not read for it.
    let document = match &params.page_id {
        Some(page_id) if Url::parse(&params.url).is_err() => {
            Some(base_of(&tiers, page_id, run.metrics().counted_work()).await?)
        }
        _ => None,
    };
    let mut request = params.clone();
    // Serialized once, for its check, the run file and what its staging
    // keeps for its attempt's commit (#1896, #2245).
    let payload = request
        .payload
        .take()
        .map(|payload| Arc::new(Payload::of(payload, run.metrics().counted_work())));
    let answered = asking
        .enqueue(Enqueue {
            request,
            payload,
            document,
        })
        .await;
    match answered {
        Ok(Some(answered)) => answered,
        // The crawl ended with its attempts.
        Ok(None) | Err(StateError::Closed) => Err(ended()),
        Err(error) => Err(CallError::new(
            &RUN_STATE_UNAVAILABLE,
            format!(
                "The state of the Run `{}` could not take the enqueue: {error}",
                params.run_id
            ),
            &RunDetails {
                run_id: params.run_id.clone(),
                path: None,
            },
        )),
    }
}

/// The base URL of the document on the page `page_id`, which `tiers`
/// serve, against which a relative enqueue resolves (ADR 0120).
async fn base_of(
    tiers: &FetchTiers,
    page_id: &PageId,
    work: &CountedWork,
) -> Result<Url, CallError> {
    let base_url = tiers.base_url(page_id, work).await.ok_or_else(|| {
        CallError::new(
            &CLOSED,
            format!(
                "No fetch tier serves the page `{}` now, so a relative URL cannot resolve against its document: enqueue from the page its attempt was given while the attempt runs.",
                page_id.0
            ),
            &(),
        )
    })??;
    document_base(&base_url, None).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("The page's base URL `{base_url}` is no absolute URL."),
        )
    })
}

pub(crate) mod links;

#[cfg(test)]
mod tests;
