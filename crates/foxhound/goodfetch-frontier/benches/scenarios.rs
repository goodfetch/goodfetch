//! The Frontier's benchmark scenarios (ADR 0071), shared by its Callgrind
//! and DHAT targets: admission through the per-domain queues, from a
//! seed-free literal pattern, with no network, no sleep and no timing. One
//! heavy domain holds most of the requests and many small domains one or
//! two each, the shape fair admission exists for (ADR 0004); every request
//! is enqueued, admitted in round-robin turns and ended.

use goodfetch_engine::admission::{Ended, Gate, Waiting};
use goodfetch_frontier::{DomainKeys, DomainQueues};
use tokio::time::Instant;

/// How many requests a domain's queue holds at most, as a crawl's do.
const BOUND: usize = 64;

/// The requests of the scenario: `heavy` to one domain, then one or two to
/// each of `small` other domains, every third one at a higher priority.
pub(crate) fn requests(heavy: usize, small: usize) -> Vec<(String, i64)> {
    let heavy = (0..heavy).map(|page| (format!("https://www.heavy.test/pages/{page}"), 0));
    let small = (0..small).flat_map(|domain| {
        let pages = if domain % 2 == 0 { 1 } else { 2 };
        (0..pages).map(move |page| {
            let priority = i64::from(page % 3 == 0);
            (format!("https://shop.site-{domain}.test/{page}"), priority)
        })
    });
    heavy.chain(small).collect()
}

/// Enqueues `requests` in order, each into its domain's queue while it has
/// room, then admits every one in turn and ends it; returns how many it
/// admitted and how many domains it still keeps state for, none once
/// every queue drained.
#[expect(
    clippy::disallowed_methods,
    reason = "a pure rule's readings may start from any moment (ADR 0059)"
)]
pub(crate) fn admit_all(requests: Vec<(String, i64)>) -> (usize, usize) {
    let now = Instant::now();
    let mut queues = DomainQueues::new(DomainKeys::default(), BOUND);
    for (order, (url, priority)) in requests.into_iter().enumerate() {
        let key = queues.key(&url);
        queues.enqueue(Waiting {
            id: url,
            key,
            priority,
            order: i64::try_from(order).unwrap_or(i64::MAX),
            forefront: false,
        });
    }
    let mut admitted = 0;
    while let Some(request) = queues.admit(now) {
        queues.ended(&request.key, now, Ended::Done);
        admitted += 1;
    }
    (admitted, queues.domains())
}
