//! What a fetched page teaches the Frontier (ADRs 0005 and 0120): each
//! redirect hop its chain followed, its page signature and
//! `rel=canonical`; nothing under `frontier.learning: false`, which signs
//! no page (ADR 0138). The page's task notes it with the Frontier and
//! reads and writes nothing of the run file for it: the end of the batch
//! the writer commits next teaches it, and ends each pending request a
//! rule it taught merges `skipped` with `duplicate`, one batch late
//! (#2169; ADR 0142's note of 2026-11-10).

use std::sync::Arc;

use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_fetch_tier::{FetchTiers, Redirected};
use goodfetch_frontier::{PageSeen, Redirect, Url};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::frontier::{FRONTIER_LEARNING, HttpBody, HttpMethod};
use goodfetch_protocol::page::PageId;

/// Where a crawl's pages note what they teach the Frontier: its
/// invariants, when the Run learns ([`learns`]).
#[derive(Clone, Debug)]
pub(in crate::delivery) struct Lessons(Option<Arc<goodfetch_frontier::Invariants>>);

impl Lessons {
    /// Noted with `invariants` when `run` learns, else nowhere.
    pub(in crate::delivery) fn of(
        run: &RunIdentity,
        invariants: Arc<goodfetch_frontier::Invariants>,
    ) -> Self {
        Self(learns(run).then_some(invariants))
    }

    /// Notes what the page `page_id` the tiers fetched for `request`
    /// teaches: each redirect hop its chain followed, its page signature
    /// and its `rel=canonical`. A Run that does not learn reads no
    /// signature and notes nothing.
    pub(super) async fn learn(&self, tiers: &FetchTiers, page_id: &PageId, request: &CrawlRequest) {
        let Some(invariants) = &self.0 else {
            return;
        };
        let Some(lesson) = tiers.lesson(page_id).await else {
            return;
        };
        let mut noted = lesson_of(&lesson.redirects);
        if let Some(signed) = lesson.signed
            && let Ok(url) = Url::parse(&signed.url)
        {
            noted.page(PageSeen {
                crawl_request: &request.id,
                url: &url,
                method: &request.method,
                body: request.body.as_ref().map(HttpBody::bytes),
                signature: signed.signature,
                canonical: signed.canonical.as_ref(),
            });
        }
        invariants.note(noted);
    }

    /// Notes what the redirect hops `redirects` a chain followed before a
    /// hop ended its request `skipped` teach: each still counts as evidence
    /// of an equivalent form (ADR 0120). Nothing when the Run does not
    /// learn.
    pub(super) fn skipped(&self, redirects: &[Redirected]) {
        if let Some(invariants) = &self.0 {
            invariants.note(lesson_of(redirects));
        }
    }
}

/// The lesson of the redirect hops `redirects`, in order.
fn lesson_of(redirects: &[Redirected]) -> goodfetch_frontier::Lesson {
    let mut lesson = goodfetch_frontier::Lesson::default();
    for hop in redirects {
        // A method the protocol cannot name is no `GET`, which alone is
        // evidence.
        let Ok(method) = HttpMethod::parse(hop.method.as_str()) else {
            continue;
        };
        lesson.redirect(Redirect {
            from: &hop.from,
            to: &hop.to,
            method: &method,
            body: hop.body.as_deref(),
        });
    }
    lesson
}

/// Whether a crawl under `run`'s settings learns from its pages:
/// `frontier.learning`, on unless set off (ADR 0138).
pub(in crate::delivery) fn learns(run: &RunIdentity) -> bool {
    get_or_default(
        run.settings().map(RunSettings::resolved),
        &FRONTIER_LEARNING,
    )
}
