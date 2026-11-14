//! What a fetched page teaches the Frontier before its handler runs
//! (ADRs 0005 and 0120): its page signature and `rel=canonical`, and the
//! pending requests a rule it taught merges, ended `skipped` with
//! `duplicate`; nothing under `frontier.learning: false`, which signs no
//! page (ADR 0138). A handler's attempt learns in a job of its own; a
//! code-free attempt's outcome learns in its own job (ADR 0138).

use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_protocol::foxhound::frontier::FRONTIER_LEARNING;
use goodfetch_protocol::page::PageId;

use crate::attempt_outcome::{Learning, Moved};
use crate::work_items::{Changed, Leased};

/// What a fetched page taught the Frontier, or teaches it with its
/// attempt's outcome.
#[derive(Debug, Default)]
pub(in crate::delivery) struct Taught {
    /// The pending requests a rule it taught merged, which it ended
    /// `skipped` with `duplicate`, by ID with their domain key: they leave
    /// their queues. Beside them, those a rule it stopped made pending
    /// again, which their queues take back when filled again.
    pub(in crate::delivery) merged: Vec<(String, String)>,
    /// What ending those and giving these back changed of the crawl's
    /// requests by state, in the lesson's own job, when it changed any:
    /// the delivery's loop has the crawl's Run state count it once the
    /// attempt ends (#2167).
    pub(in crate::delivery) changed: Option<Box<Changed>>,
    /// What it teaches in its attempt's outcome, which a code-free attempt
    /// leaves to it.
    pub(in crate::delivery) learning: Option<Box<Learning>>,
}

/// Hands the Frontier the page `page_id` the tiers fetched for `leased`'s
/// crawl request, with its page signature and its `rel=canonical`, before
/// its handler runs, so the links the handler then enqueues count against
/// what the page taught (ADRs 0005 and 0120); in the same commit, each
/// pending request a rule it taught merges ends `skipped` with
/// `duplicate`. Answers those requests, by ID with the domain key they
/// wait under, which leave their queues. A run file that cannot take it
/// loses only what the page would have taught. A Run that does not learn
/// ([`learns`]) reads no signature and teaches nothing. A code-free
/// attempt, `in_outcome`, whose links are admitted in its outcome, leaves
/// the lesson to that outcome, which teaches it before those links.
pub(super) async fn learn(
    run: &Run,
    tiers: &FetchTiers,
    page_id: &PageId,
    leased: &Leased,
    in_outcome: bool,
) -> Taught {
    if !learns(run) {
        return Taught::default();
    }
    let Some(signed) = tiers.signature(page_id).await else {
        return Taught::default();
    };
    let Ok(url) = goodfetch_frontier::Url::parse(&signed.url) else {
        return Taught::default();
    };
    let learning = Learning {
        request: leased.crawl_request.clone(),
        url,
        signature: signed.signature,
        canonical: signed.canonical,
        ended_at: run.point_in_time(),
        work: run.metrics().counted_work().clone(),
    };
    if in_outcome {
        return Taught {
            learning: Some(Box::new(learning)),
            ..Taught::default()
        };
    }
    let learned = run
        .state()
        // Its answer promises nothing outside (ADR 0138's note of #1926):
        // the handler reads what it taught, and the outcome's sync covers it.
        .write_deferred(move |connection| learning.teach(connection))
        .await;
    let (merged, changed) = learned.unwrap_or_else(|error| {
        tracing::warn!(%error, run_id = %run.id(), "the Frontier could not learn from a fetched page");
        Moved::default()
    });
    Taught {
        merged,
        changed: (!changed.is_empty()).then(|| Box::new(changed)),
        learning: None,
    }
}

/// Whether a crawl under `run`'s settings learns from its pages:
/// `frontier.learning`, on unless set off (ADR 0138).
pub(in crate::delivery) fn learns(run: &Run) -> bool {
    get_or_default(
        run.settings().map(RunSettings::resolved),
        &FRONTIER_LEARNING,
    )
}
