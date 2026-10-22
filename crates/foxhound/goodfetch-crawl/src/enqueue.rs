//! A handler's enqueue (ADRs 0093 and 0120): `crawl_request.enqueue` adds
//! one request to a live crawl from inside an attempt, while its lease
//! holds, through the Frontier, which resolves a relative URL against the
//! document's base URL and answers added or not added with its reason. The
//! request is one level deeper than the attempt's, and is committed at once
//! in the Run's state; the crawl's delivery keys it and queues it under its
//! domain once the attempt settles.

use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_frontier::{Candidate, Declared, admit, document_base};
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE, CrawlRequestEnqueueParams, Enqueued, HttpBody,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails};
use rusqlite::Connection;

use crate::serving::Crawls;
use crate::work_items::{self, Adding};

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
    let Some(run) = crawls.run(&params.run_id).await else {
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
    let request = params.clone();
    let answered = run
        .state()
        .write(move |connection| add(connection, &request, document.as_ref()))
        .await;
    match answered {
        Ok(Some(enqueued)) => Ok(enqueued),
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

/// Adds `params`'s request through the Frontier while its attempt holds
/// its lease, against `document`'s base URL, else the attempt's own URL;
/// `None` once the attempt ended.
fn add(
    connection: &Connection,
    params: &CrawlRequestEnqueueParams,
    document: Option<&goodfetch_frontier::Url>,
) -> rusqlite::Result<Option<Enqueued>> {
    let Some((url, depth)) =
        work_items::attempt(connection, &params.crawl_request_id, params.lease_epoch)?
    else {
        return Ok(None);
    };
    let Some(base) = document.cloned().or_else(|| document_base(&url, None)) else {
        return Ok(None);
    };
    let candidate = Candidate::Enqueue {
        url: &params.url,
        base: &base,
        declared: Declared {
            method: &params.method,
            body: params.body.as_ref().map(HttpBody::bytes),
            key: params.key.as_deref(),
        },
    };
    admit(connection, candidate, |url| {
        work_items::add(
            connection,
            url,
            Adding {
                method: &params.method,
                body: params.body.as_ref(),
                label: &params.label,
                payload: params.payload.as_ref(),
                priority: params.priority,
                forefront: params.forefront,
                depth: depth + 1,
            },
        )
    })
    .map(Some)
}
