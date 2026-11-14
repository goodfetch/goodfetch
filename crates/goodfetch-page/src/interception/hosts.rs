//! Each live Run's host cache (spec #2164): the rule lists its pages'
//! hosts built, kept up to its `filter_lists.host_cache`, so a page on a
//! host already seen builds none. Every page of the Run takes the one
//! cache, whichever Session or fetch tier reads it; it is emptied at each
//! sample that finds the Resource governor's memory critical, and what it
//! holds as the Run ends is counted into the Run's counted work.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use goodfetch_clean_up_rules::removal::HostCache;
use goodfetch_counted_work::Row;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{RunSettings, Values, value_as};
use goodfetch_protocol::governor::GovernorLevel;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::FILTER_LISTS_HOST_CACHE;
use tokio_util::sync::CancellationToken;

use super::clean_up::effective;
use super::lock;

/// The host cache of each live Run, held until the Run ends.
#[derive(Default)]
pub(super) struct HostCaches(Mutex<BTreeMap<RunId, Held>>);

/// One live Run's host cache.
struct Held {
    /// Cancelled once the Run ends.
    live: CancellationToken,
    cache: Arc<HostCache>,
}

impl HostCaches {
    /// `run`'s host cache, made on its first page, bounded by its
    /// `filter_lists.host_cache`.
    pub(super) fn of(&self, run: &Run) -> Arc<HostCache> {
        let mut held = lock(&self.0);
        held.retain(|_, held| !held.live.is_cancelled());
        if let Some(held) = held.get(run.id()) {
            return Arc::clone(&held.cache);
        }
        let cache = Arc::new(HostCache::new(bound(run)));
        // A Run already ended keeps nothing for a later page of its own.
        if run.cancellation().is_cancelled() {
            return cache;
        }
        watch(run, &cache);
        held.insert(
            run.id().clone(),
            Held {
                live: run.cancellation().clone(),
                cache: Arc::clone(&cache),
            },
        );
        cache
    }
}

/// `run`'s `filter_lists.host_cache`; its catalog default without settings.
fn bound(run: &Run) -> u64 {
    let resolved = run.settings().map(RunSettings::resolved);
    value_as(
        &FILTER_LISTS_HOST_CACHE,
        effective(resolved, &Values::default(), &FILTER_LISTS_HOST_CACHE),
    )
}

/// Empties `cache` at each governor sample of `run` that finds memory
/// critical, and counts what it holds into `run`'s counted work as the Run
/// ends: its end waits for its tasks, so the count lands before its report.
fn watch(run: &Run, cache: &Arc<HostCache>) {
    let mut samples = run.governor().samples();
    let ending = run.cancellation().clone();
    let work = run.metrics().counted_work().clone();
    let cache = Arc::clone(cache);
    run.spawn(async move {
        loop {
            tokio::select! {
                () = ending.cancelled() => break,
                changed = samples.changed() => if changed.is_err() {
                    ending.cancelled().await;
                    break;
                },
            }
            if *samples.borrow_and_update() == GovernorLevel::Critical {
                cache.clear();
            }
        }
        work.count_many(Row::PageHostCacheEntries, cache.entries());
        work.count_many(Row::PageHostCacheBytes, cache.bytes());
    });
}
