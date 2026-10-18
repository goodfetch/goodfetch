//! The Frontier's benchmark scenarios (ADR 0071), shared by its Callgrind
//! and DHAT targets. Until area 3 fills the Frontier, one fixture scenario
//! stands in, so `xtask perf` has a benchmark to measure against its base:
//! queueing synthetic URLs and dropping the duplicates, from a seed-free
//! literal pattern, with no network, no sleep and no timing.

use std::collections::BTreeSet;

/// The URLs the fixture scenario queues: `count` of them, every tenth one
/// a repeat of the one before.
pub(crate) fn synthetic_urls(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            let page = if index % 10 == 9 { index - 1 } else { index };
            format!("https://proving-ground.test/pages/{page}")
        })
        .collect()
}

/// Queues `urls` in order, keeping the first of each, and returns how many
/// were queued.
pub(crate) fn queue_unique(urls: Vec<String>) -> usize {
    let mut seen = BTreeSet::new();
    let mut queue = Vec::with_capacity(urls.len());
    for url in urls {
        if seen.insert(url.clone()) {
            queue.push(url);
        }
    }
    queue.len()
}
