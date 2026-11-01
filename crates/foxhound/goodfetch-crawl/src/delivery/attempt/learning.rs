//! What a fetched page teaches the Frontier before its handler runs
//! (ADRs 0005 and 0120): its page signature and `rel=canonical`, and the
//! pending requests a rule it taught merges, ended `skipped` with
//! `duplicate`.

use std::collections::BTreeSet;

use goodfetch_engine::runs::Run;
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_frontier::{EndMerged, PageSeen};
use goodfetch_protocol::foxhound::frontier::HttpBody;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::PointInTime;

use crate::work_items::{self, Leased};

/// Hands the Frontier the page `page_id` the tiers fetched for `leased`'s
/// crawl request, with its page signature and its `rel=canonical`, before
/// its handler runs, so the links the handler then enqueues count against
/// what the page taught (ADRs 0005 and 0120); in the same commit, each
/// pending request a rule it taught merges ends `skipped` with
/// `duplicate`. Answers those requests, by ID with the domain key they
/// wait under, which leave their queues. A run file that cannot take it
/// loses only what the page would have taught.
pub(super) async fn learn(
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
