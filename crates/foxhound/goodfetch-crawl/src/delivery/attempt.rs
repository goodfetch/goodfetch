//! One attempt of a crawl's delivery (ADRs 0004, 0006 and 0024): its
//! `crawl_request.handle` request to a serving Connection, bounded by its
//! lease, and what its answer is. When the lease ends first, the request
//! is cancelled with `$/cancel` and an answer that still comes is a late
//! commit, refused with `lease_lost`. Its answer also says what the Session
//! that served it scored: a success, a block or a failed Page check.
//!
//! Under `fetch.tier: http` (spec #579), the attempt first fetches its
//! request's document on the Run's HTTP tier, under the same lease, and
//! hands the handler the page served from it, closed once the attempt
//! ends; a fetch that fails ends the attempt with its error, as a
//! handler's failure would, and sends the handler nothing. The fetch is
//! sent from the attempt's Session, whose identical requests in flight it
//! may share, and hedged under its domain key's room (ADR 0040).

use std::collections::BTreeSet;

use goodfetch_engine::admission::SecondCopies;
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::metrics::Counted;
use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::{FetchTiers, HttpTier, Requester};
use goodfetch_frontier::{EndMerged, PageSeen};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_REQUEST_HANDLE, CrawlRequestHandleParams, CrawlRequestHandled, HANDLER_FAILED,
    LEASE_LOST, WORK_ITEM_LEASED,
};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::frontier::HttpBody;
use goodfetch_protocol::foxhound::progress::InFlightPhase;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::PointInTime;
use goodfetch_session_pool::Effect;
use serde_json::{Map, Value};

use crate::lease::{CommitError, WorkItemLease};
use crate::progress::SharedInFlight;
use crate::work_items::{self, Leased};

/// The Error code of a page the site refused, which Collie's page raises
/// with the Block cause in its details (ADRs 0035 and 0079); declared with
/// the page's errors (#711).
const BLOCKED: &str = "blocked";

/// The Error code of a page that failed its Page check (ADR 0006), which
/// scores its Session down; declared with the Page check (area 5).
const PAGE_CHECK_FAILED: &str = "page_check_failed";

/// How an attempt ended, before its outcome is committed.
pub(super) enum Answer {
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

/// What an attempt fetches its request's document with under `fetch.tier:
/// http`: the Run's HTTP tier, and the room its hedges take beside it.
pub(super) struct DocumentFetch {
    tiers: FetchTiers,
    http: HttpTier,
    second_copies: SecondCopies,
}

impl DocumentFetch {
    /// The Run's `tiers` and `http` tier, its hedges taking
    /// `second_copies`.
    pub(super) fn of(tiers: &FetchTiers, http: &HttpTier, second_copies: &SecondCopies) -> Self {
        Self {
            tiers: tiers.clone(),
            http: http.clone(),
            second_copies: second_copies.clone(),
        }
    }
}

pub(super) struct Attempt {
    /// The server, by its place in the delivery's list, its request went to.
    pub(super) server: usize,
    pub(super) leased: Leased,
    /// Its count in `goodfetch.work_item.leased`, until it drops.
    _counted: Counted,
    /// The Session of the crawl's Session pool that served it.
    pub(super) session: ServingSession,
    pub(super) lease: WorkItemLease,
    pub(super) answer: Answer,
    /// The pending requests a rule its page taught merged, which it ended
    /// `skipped` with `duplicate`, by ID with their domain key: they leave
    /// their queues.
    pub(super) merged: Vec<(String, String)>,
}

/// One attempt: the handler request, bounded by its lease. When the lease
/// ends first, the request is cancelled and its answer left to [`late`].
pub(super) async fn attempt(
    run: Run,
    server: usize,
    client: Client,
    (leased, session, counted): (Leased, ServingSession, Counted),
    lease: WorkItemLease,
    fetching: Option<DocumentFetch>,
    in_flight: SharedInFlight,
) -> Attempt {
    let fetched = match &fetching {
        None => Ok(None),
        Some(fetch) => {
            let requester = Requester {
                request_id: &leased.crawl_request.id,
                session: &session.id,
                label: &leased.crawl_request.label,
                domain_key: &leased.waiting.key,
                second_copies: &fetch.second_copies,
                // No work item declares an Effect before area 9's Effect
                // journal (ADR 0106), and no crawl request is a Paid fetch
                // before ADR 0045's.
                declares_effect: false,
                paid: false,
            };
            tokio::select! {
            biased;
            document = fetch.tiers.serve(&fetch.http, &run, &leased.crawl_request, &requester) => document
                .map(Some)
                .map_err(|error| Answer::Failed {
                    code: error.code().to_owned(),
                    retryable: error.retryable(),
                    message: error.message().to_owned(),
                    details: error.details().cloned().unwrap_or_default(),
                }),
            _ = lease.ended() => Err(Answer::LeaseEnded),
            }
        }
    };
    let mut merged = Vec::new();
    let answer = match fetched {
        Err(answer) => answer,
        Ok(document) => {
            if let (Some(fetch), Some(page_id)) = (&fetching, &document) {
                merged = learn(&run, &fetch.tiers, page_id, &leased).await;
            }
            if document.is_some() {
                in_flight.lock().await.went_on(
                    &run,
                    &leased.crawl_request.id,
                    InFlightPhase::Handling,
                );
            }
            let params = CrawlRequestHandleParams {
                run_id: run.id().clone(),
                crawl_request: leased.crawl_request.clone(),
                attempt: leased.attempt,
                lease_epoch: leased.lease_epoch,
                session: session.clone(),
                tier: document.as_ref().map(|_| FetchTier::Http),
                page_id: document.clone(),
            };
            let answer = handled(&run, &client, &params, &lease).await;
            if let (Some(fetch), Some(page_id)) = (&fetching, &document) {
                fetch.tiers.close(page_id);
            }
            answer
        }
    };
    Attempt {
        server,
        leased,
        _counted: counted,
        session,
        lease,
        answer,
        merged,
    }
}

/// Hands the Frontier the page `page_id` the tiers fetched for `leased`'s
/// crawl request, with its page signature and its `rel=canonical`, before
/// its handler runs, so the links the handler then enqueues count against
/// what the page taught (ADRs 0005 and 0120); in the same commit, each
/// pending request a rule it taught merges ends `skipped` with
/// `duplicate`. Answers those requests, by ID with the domain key they
/// wait under, which leave their queues. A run file that cannot take it
/// loses only what the page would have taught.
async fn learn(
    run: &Run,
    tiers: &FetchTiers,
    page_id: &PageId,
    leased: &Leased,
) -> Vec<(String, String)> {
    let Some(signed) = tiers.signature(page_id).await else {
        return Vec::new();
    };
    let Ok(url) = goodfetch_frontier::Url::parse(&signed.url) else {
        return Vec::new();
    };
    let request = leased.crawl_request.clone();
    let ended_at = run.point_in_time();
    let learned = run
        .state()
        .write(move |connection| {
            let mut merged = Merged {
                ended_at,
                ended: Vec::new(),
            };
            goodfetch_frontier::learn(
                connection,
                PageSeen {
                    crawl_request: &request.id,
                    url: &url,
                    method: &request.method,
                    body: request.body.as_ref().map(HttpBody::bytes),
                    signature: signed.signature,
                    canonical: signed.canonical.as_ref(),
                },
                &mut merged,
            )?;
            Ok(merged.ended)
        })
        .await;
    learned.unwrap_or_else(|error| {
        tracing::warn!(%error, run_id = %run.id(), "the Frontier could not learn from a fetched page");
        Vec::new()
    })
}

/// The crawl's side of a rule learned later: the pending requests it
/// merged, ended at `ended_at`, by ID with their domain key.
struct Merged {
    ended_at: PointInTime,
    ended: Vec<(String, String)>,
}

impl EndMerged for Merged {
    fn end_merged(
        &mut self,
        connection: &rusqlite::Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<String>> {
        let ended = work_items::end_merged(connection, crawl_requests, &self.ended_at)?;
        Ok(ended
            .into_iter()
            .map(|(id, url, key)| {
                if let Some(key) = key {
                    self.ended.push((id, key));
                }
                url
            })
            .collect())
    }
}

/// The handler request of one attempt, bounded by its lease.
async fn handled(
    run: &Run,
    client: &Client,
    params: &CrawlRequestHandleParams,
    lease: &WorkItemLease,
) -> Answer {
    match client
        .send::<CrawlRequestHandled>(&CRAWL_REQUEST_HANDLE, params)
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
    }
}

/// One more attempt in `goodfetch.work_item.leased`, until the answer
/// drops.
pub(super) fn leased_count(run: &Run) -> Counted {
    let counter = run
        .metrics()
        .meter()
        .i64_up_down_counter(WORK_ITEM_LEASED.name)
        .with_description(WORK_ITEM_LEASED.description)
        .with_unit(WORK_ITEM_LEASED.unit)
        .build();
    Counted::new(counter, Vec::new())
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

/// What an attempt's answer says of the Session that served it: a
/// committed answer is a success, a block or a failed Page check scores it
/// down, and any other ending, a skip or the handler's own failure, says
/// nothing of it.
pub(super) fn effect(answer: &Answer) -> Effect {
    match answer {
        Answer::Handled(CrawlRequestHandled { skipped: None, .. }) => Effect::Success,
        Answer::Failed { code, details, .. } => match block_cause(code, details) {
            Some(cause) => Effect::Blocked(cause),
            None if code == PAGE_CHECK_FAILED => Effect::PageCheckFailed,
            None => Effect::Neutral,
        },
        Answer::Handled(CrawlRequestHandled {
            skipped: Some(_), ..
        })
        | Answer::LeaseEnded
        | Answer::ClientGone => Effect::Neutral,
    }
}

/// The Block cause of a handler's failure with the Error code `blocked`,
/// which its details name.
pub(super) fn block_cause(code: &str, details: &Map<String, Value>) -> Option<BlockCause> {
    if code != BLOCKED {
        return None;
    }
    details
        .get("cause")
        .and_then(|cause| serde_json::from_value(cause.clone()).ok())
}
