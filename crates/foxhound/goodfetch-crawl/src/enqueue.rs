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

use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_frontier::document_base;
use goodfetch_frontier::payload::Payload;
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CrawlRequestEnqueueParams, Enqueued,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails};

use crate::run_state::StateError;
use crate::run_state::admission::Enqueue;
use crate::serving::{Crawls, LiveCrawl};

/// The handler of `crawl_request.enqueue` on the live crawls of `crawls`.
pub(crate) fn handler(crawls: Crawls) -> Handler {
    Handler::new(&CRAWL_REQUEST_ENQUEUE, move |params, context| {
        enqueue(params, context, crawls.clone())
    })
}

async fn enqueue(
    params: CrawlRequestEnqueueParams,
    _context: CallContext,
    crawls: Crawls,
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
    let document = params
        .document
        .as_ref()
        .map(|document| {
            document_base(&document.url, document.base_href.as_deref()).ok_or_else(|| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("The document's URL `{}` is no absolute URL.", document.url),
                )
            })
        })
        .transpose()?;
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

pub(crate) mod links;

#[cfg(test)]
mod tests;
