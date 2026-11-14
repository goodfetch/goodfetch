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
//! settles.

use std::sync::Arc;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::{Candidate, Declared, Narrowing, document_base};
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CrawlRequestEnqueueParams, Enqueued, HttpBody,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails};
use rusqlite::Connection;

use crate::invariants::Invariants;
use crate::serving::{Crawls, LiveCrawl};
use crate::staging::Answered;
use crate::work_items::{self, Adding, Positions};

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
    let Some(LiveCrawl {
        run,
        egress,
        invariants,
        staging,
        ..
    }) = crawls.live(&params.run_id).await
    else {
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
    let work = run.metrics().counted_work().clone();
    // Serialized once, for its check, the run file and what its staging
    // keeps for its attempt's commit (#1896, #2245).
    let payload = request
        .payload
        .take()
        .map(|payload| Arc::new(Payload::of(payload, &work)));
    let answered = run
        .state()
        .write(move |connection| {
            let (crawl_request_id, lease_epoch) =
                (request.crawl_request_id.clone(), request.lease_epoch);
            staging.admit(
                connection,
                &crawl_request_id,
                lease_epoch,
                move |connection, positions| {
                    add(
                        connection,
                        positions,
                        (&egress, &invariants),
                        (&request, payload.as_ref()),
                        document.as_ref(),
                        &work,
                    )
                },
            )
        })
        .await;
    match answered {
        Ok(Some(answered)) => answered,
        Ok(None) => Err(ended()),
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

/// Adds `params`'s request, carrying `payload`, through the Frontier, at
/// its position from `positions`, within the Run's `invariants` and under
/// its domain policy `egress`, while its attempt holds its lease, against
/// `document`'s base URL, else the attempt's own URL, once its Payload fits
/// its Label's schema and the cap, else `payload_invalid`; `None` once the
/// attempt ended.
fn add(
    connection: &Connection,
    positions: &mut Positions,
    (egress, invariants): (&Policy, &Invariants),
    (params, payload): (&CrawlRequestEnqueueParams, Option<&Arc<Payload>>),
    document: Option<&goodfetch_frontier::Url>,
    work: &CountedWork,
) -> Answered<Enqueued> {
    let Some((url, depth)) =
        work_items::attempt(connection, &params.crawl_request_id, params.lease_epoch)?
    else {
        return Ok(None);
    };
    let Some(from) = document_base(&url, None) else {
        return Ok(None);
    };
    let checked = payload.map(|payload| &**payload);
    if let Err(refused) = invariants.payloads().check(&params.label, checked, work) {
        return Ok(Some(Err(refused)));
    }
    let base = document.cloned().unwrap_or_else(|| from.clone());
    let candidate = Candidate::Enqueue {
        url: &params.url,
        base: &base,
        from: &from,
        declared: Declared {
            method: &params.method,
            body: params.body.as_ref().map(HttpBody::bytes),
            key: params.key.as_deref(),
            keeps_fragment: invariants.keeps_fragment(&params.label),
        },
        narrowing: Narrowing::none(),
        depth: depth + 1,
    };
    let adding = Adding {
        method: &params.method,
        body: params.body.as_ref(),
        label: &params.label,
        payload,
        priority: params.priority,
        forefront: params.forefront,
        depth: depth + 1,
    };
    work_items::admit(connection, positions, invariants, egress, candidate, adding)
        .map(|enqueued| Some(Ok(enqueued)))
}

pub(crate) mod links;

#[cfg(test)]
mod tests;
